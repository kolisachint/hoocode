//! The chat transcript: rendering messages and tool blocks, streaming, the status and notice
//! rows, and the tool output views.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

use hoocode_agent_types::AgentMessage;
use hoocode_ai_types::{
    AssistantMessage, Content, ImageContent, StopReason, UserContent, UserMessage,
};
use hoocode_ai_util::is_long_retry_delay_error;
use hoocode_code_agent_session::format::{format_duration_secs, format_tokens};
use hoocode_code_agent_session::stats::{
    add_assistant_usage, sum_assistant_usage, AssistantUsageTotals,
};
use hoocode_code_resources::skill_blocks::parse_skill_block;
use hoocode_code_settings::ToolOutputView;
use hoocode_code_tool_api::{truncate_tail, TruncationOptions, TruncationResult};
use hoocode_code_tui_keybindings::key_text;
use hoocode_code_tui_theme::{apply_block_fill, BlockFill};
use hoocode_code_tui_theme::{get_markdown_theme, theme};
use hoocode_code_tui_widgets::background_notice::{
    parse_background_notice, BackgroundNoticeComponent,
};
use hoocode_code_tui_widgets::bash_execution::BashExecutionComponent;
use hoocode_code_tui_widgets::custom_message::{
    BranchSummaryMessageComponent, CompactionSummaryMessageComponent, CustomMessageComponent,
};
use hoocode_code_tui_widgets::dynamic_border::DynamicBorder;
use hoocode_code_tui_widgets::radar::{FlightMark, FlightStats, PeekOnly};
use hoocode_code_tui_widgets::skill_block::SkillBlockComponent;
use hoocode_code_tui_widgets::tool_chain::{speaking_ends_chain, ToolChainComponent};
use hoocode_code_tui_widgets::tool_chain_summary::ChainState;
use hoocode_code_tui_widgets::tool_execution::{ToolExecutionComponent, ToolExecutionOptions};
use hoocode_code_tui_widgets::tool_output_view::cycle_tool_output_view;
use hoocode_code_tui_widgets::tool_signal::ToolResult;
use hoocode_code_tui_widgets::tools::registered_tool_definition;
use hoocode_code_tui_widgets::trajectory::{self, MissionHeader};
use hoocode_code_tui_widgets::{AssistantMessageComponent, ThinkingDisplay, UserMessageComponent};
use hoocode_tui_components::BoxComponent;
use hoocode_tui_components::{Loader, Markdown, MarkdownTheme, Spacer, Text};
use hoocode_tui_render::ComponentHandle;

use crate::hull_paint::HullPaint;
use crate::notification_panel::NotificationKind;
use crate::resource_display::show_loaded_resources;
use crate::scroll_view;

use super::*;

/// The chat transcript: the chat `Container` and the components that track it. Reset
/// (`reset_transcript_view`) replaces the whole value, keeping only the chat container.
pub(super) struct Transcript {
    pub(super) chat: Rc<RefCell<Container>>,
    pub(super) streaming: Option<Rc<RefCell<AssistantMessageComponent>>>,
    pub(super) streaming_message: Option<Rc<AssistantMessage>>,
    /// Tool blocks by call id, until their execution ends.
    pub(super) pending_tools: HashMap<String, Rc<RefCell<ToolExecutionComponent>>>,
    /// The chain collecting tool calls, if the agent is mid-run.
    pub(super) open_chain: Option<Rc<RefCell<ToolChainComponent>>>,
    /// Every chain and assistant message in the transcript, in order.
    pub(super) chains: Vec<Rc<RefCell<ToolChainComponent>>>,
    pub(super) assistant_components: Vec<Rc<RefCell<AssistantMessageComponent>>>,
    pub(super) latest_block: Option<Rc<RefCell<ToolExecutionComponent>>>,
    pub(super) latest_chain: Option<Rc<RefCell<ToolChainComponent>>>,
    /// The last status line, updated in place when nothing followed it.
    pub(super) last_status: RecordRows,
    /// Every custom message in the transcript, for the dial sweep.
    pub(super) custom_messages: Vec<Rc<RefCell<CustomMessageComponent>>>,
    /// Every `<skill>` block in the transcript, for the dial sweep.
    pub(super) skill_blocks: Vec<Rc<RefCell<SkillBlockComponent>>>,
    /// Every background-subagent notice in the transcript, for the dial sweep.
    pub(super) background_notices: Vec<Rc<RefCell<BackgroundNoticeComponent>>>,
    /// Every branch summary in the transcript, for the expand sweep.
    pub(super) branch_summaries: Vec<Rc<RefCell<BranchSummaryMessageComponent>>>,
    /// Every compaction summary in the transcript, for the expand sweep.
    pub(super) compaction_summaries: Vec<Rc<RefCell<CompactionSummaryMessageComponent>>>,
    /// The row of the latest failed attempt, until a retry event takes it out (`retry.rs`).
    pub(super) attempt_error: Option<Rc<RefCell<AssistantMessageComponent>>>,
    /// The auto-retry streak in progress (`retry.rs`).
    pub(super) retry_streak: Option<super::retry::RetryStreak>,
    /// The tool blocks of the latest failed attempt, dropped with its row.
    pub(super) attempt_tools: Vec<hoocode_code_tui_widgets::tool_chain::ToolBlock>,
    /// The last prompt actually sent: empty enter hails it again after
    /// COMMS LOST (M8e1).
    pub(super) last_prompt: Option<LastPrompt>,
    /// Mission Control (M4): one range per `▲ M{n}` header. Steps number
    /// per mission and each mission gets its own landing line; ranges only
    /// grow, and step numbers rebase from them after a retry takes blocks
    /// out.
    pub(super) missions: Vec<MissionRange>,
    /// Whether the radar flight log is showing (M5): peek-only rows hide
    /// while set. Shared with the wrappers, so rows added in either view
    /// follow later flips.
    pub(super) radar_active: Rc<Cell<bool>>,
}

/// The last prompt actually sent, text plus images: empty enter hails it
/// again after COMMS LOST.
#[derive(Debug, Clone, PartialEq, Default)]
pub(super) struct LastPrompt {
    pub(super) text: String,
    pub(super) images: Vec<ImageContent>,
}

/// The prompt a rebuilt user message stands for: its text plus the images
/// recorded with it, so hailing it again resends the whole turn.
pub(super) fn user_prompt(user: &UserMessage) -> LastPrompt {
    let mut prompt = LastPrompt::default();
    match &user.content {
        UserContent::Text(text) => prompt.text = text.clone(),
        UserContent::Blocks(blocks) => {
            for block in blocks {
                match block {
                    Content::Text(text) => prompt.text.push_str(&text.text),
                    Content::Image(image) => prompt.images.push(image.clone()),
                    _ => {}
                }
            }
        }
    }
    prompt
}

/// One M4 mission: its header number, the chain index it starts at, and
/// whether its landing line is out yet. The header handle and flight data
/// back the radar flight-log row (M5); the header itself carries the
/// prompt and skill.
#[derive(Debug, Clone)]
pub(super) struct MissionRange {
    pub(super) seq: usize,
    pub(super) start: usize,
    pub(super) landed: bool,
    pub(super) header: Rc<RefCell<MissionHeader>>,
    pub(super) flight: Rc<RefCell<FlightStats>>,
    pub(super) landing: Option<Rc<RefCell<trajectory::LandingLine>>>,
}

impl Transcript {
    /// An empty transcript over `chat`, the container the TUI tree shows.
    pub(super) fn new(chat: Rc<RefCell<Container>>) -> Self {
        Self {
            chat,
            streaming: None,
            streaming_message: None,
            pending_tools: HashMap::new(),
            open_chain: None,
            chains: Vec::new(),
            assistant_components: Vec::new(),
            latest_block: None,
            latest_chain: None,
            last_status: RecordRows::default(),
            custom_messages: Vec::new(),
            skill_blocks: Vec::new(),
            background_notices: Vec::new(),
            branch_summaries: Vec::new(),
            compaction_summaries: Vec::new(),
            attempt_error: None,
            retry_streak: None,
            attempt_tools: Vec::new(),
            last_prompt: None,
            missions: Vec::new(),
            radar_active: Rc::new(Cell::new(false)),
        }
    }

    /// The view over a cleared chat. A retry still in progress keeps its
    /// streak, so its end still reports how the streak went. The radar flag
    /// survives too, so peek-only rows stay hidden across a rebuild.
    pub(super) fn reset_view(&mut self) {
        let streak = self.retry_streak.take();
        let radar_active = self.radar_active.clone();
        *self = Transcript::new(self.chat.clone());
        self.retry_streak = streak;
        self.radar_active = radar_active;
    }

    /// The missions a compaction folds, first to last (`M1–M3`), for the
    /// JETTISONED row (M8e6). `None` when the transcript holds no missions.
    pub(super) fn jettisoned_range(&self) -> Option<(usize, usize)> {
        let first = self.missions.first()?.seq;
        let last = self.missions.last()?.seq;
        Some((first, last))
    }
}

/// One turn of a rebuilt session: what its assistant messages spent, and when
/// it started and last settled. The live path reads the same figures from its
/// request anchor and clock.
#[derive(Default)]
struct RebuiltTurn {
    started_ms: i64,
    ended_ms: i64,
    usage: AssistantUsageTotals,
    stop: Option<StopReason>,
}

/// The details a rebuilt result shows its renderer: the tool's own, with the
/// duration the loop recorded on the message (`durationMs`) added, since a
/// rebuild has no clock of its own.
fn rebuilt_details(result: &hoocode_ai_types::ToolResultMessage) -> serde_json::Value {
    let details = result.details.clone().unwrap_or(serde_json::Value::Null);
    let Some(ms) = result.duration_ms else {
        return details;
    };
    let mut map = match details {
        serde_json::Value::Object(map) => map,
        serde_json::Value::Null => serde_json::Map::new(),
        other => return other,
    };
    map.insert("durationMs".into(), ms.into());
    serde_json::Value::Object(map)
}

/// A `{ truncated: true }` result for a `!` row: the row only reads the flag.
pub(super) fn truncated_marker(content: &str) -> TruncationResult {
    TruncationResult {
        truncated: true,
        ..truncate_tail(content, TruncationOptions::default())
    }
}

/// The user message under an M4 mission header: everything after the first
/// line, which the header already carries. Empty for a one-line prompt.
fn mission_body(text: &str) -> String {
    match text.split_once('\n') {
        Some((_, rest)) => rest.to_string(),
        None => String::new(),
    }
}

impl Transcript {
    /// Steps and failed steps in `chains[start..end]`.
    fn tally_range(&self, start: usize, end: usize) -> (usize, usize) {
        let mut steps = 0;
        let mut hits = 0;
        for chain in self
            .chains
            .iter()
            .skip(start)
            .take(end.saturating_sub(start))
        {
            for block in chain.borrow().tool_blocks() {
                steps += 1;
                if block.borrow().is_error() {
                    hits += 1;
                }
            }
        }
        (steps, hits)
    }

    /// Steps and failed steps in the current (last) mission. Before the
    /// first header, every chain counts.
    pub(super) fn mission_tally(&self) -> (usize, usize) {
        let start = self.missions.last().map_or(0, |m| m.start);
        self.tally_range(start, self.chains.len())
    }

    /// Steps and failed steps of one mission by header number.
    fn mission_tally_seq(&self, seq: usize) -> (usize, usize) {
        let Some(i) = self.missions.iter().position(|m| m.seq == seq) else {
            return (0, 0);
        };
        let end = self
            .missions
            .get(i + 1)
            .map(|m| m.start)
            .unwrap_or(self.chains.len());
        self.tally_range(self.missions[i].start, end)
    }

    /// The current mission when it still needs its landing line: it has
    /// steps and none is out yet. A new header lands it first, so a queued
    /// follow-up never swallows the running mission.
    pub(super) fn unlanded_current(&self) -> Option<usize> {
        let mission = self.missions.last()?;
        (!mission.landed && self.mission_tally().0 > 0).then_some(mission.seq)
    }

    /// A peek-only row in the chat (M5): drawn in peek, hidden in radar.
    fn peek_child(&self, child: ComponentHandle) -> ComponentHandle {
        as_component(&PeekOnly::wrap(self.radar_active.clone(), child))
    }

    /// The mission's mark from its chains (M5): an open chain still flies;
    /// else an interrupted chain did not land (`⊗` with failed steps) or
    /// was aborted (`⊘`); settled chains read landed.
    fn mission_mark(&self, start: usize, end: usize) -> FlightMark {
        let end = end.min(self.chains.len());
        let start = start.min(end);
        let chains = &self.chains[start..end];
        if chains.iter().any(|c| c.borrow().is_open()) {
            return FlightMark::InFlight;
        }
        if chains
            .iter()
            .any(|c| c.borrow().chain_state() == ChainState::Interrupted)
        {
            // Hits tally as the landing lines count them.
            return if self.tally_range(start, end).1 > 0 {
                FlightMark::DidNotLand
            } else {
                FlightMark::Aborted
            };
        }
        FlightMark::Landed
    }

    /// Refreshes every mission's flight counts and mark from its chains
    /// (M5): steps and hits tally as the landing lines count them, and
    /// unlanded missions read their mark off their chains. Times and costs
    /// land with the mission; the flying mission's come from the mode.
    pub(super) fn refresh_flight_marks(&mut self) {
        let chain_count = self.chains.len();
        for i in 0..self.missions.len() {
            let end = self
                .missions
                .get(i + 1)
                .map(|m| m.start)
                .unwrap_or(chain_count);
            let start = self.missions[i].start;
            let (steps, hits) = self.tally_range(start, end);
            let mark = if self.missions[i].landed {
                self.missions[i].flight.borrow().mark
            } else {
                self.mission_mark(start, end)
            };
            let mut flight = self.missions[i].flight.borrow_mut();
            flight.steps = steps;
            flight.hits = hits;
            flight.mark = mark;
            drop(flight);
            // A landed mission whose tally moved (a retry took blocks
            // out) rebuilds its landing row, so peek and radar agree.
            if self.missions[i].landed {
                if let Some(landing) = self.missions[i].landing.clone() {
                    landing.borrow_mut().set_counts(steps, hits);
                }
            }
        }
        let first_start = self.missions.first().map(|m| m.start);
        for (i, chain) in self.chains.iter().enumerate() {
            // No mission yet: every chain stands before the first one,
            // so all keep the summary line (a restored or fed transcript
            // with no user turn never opens a mission).
            chain
                .borrow_mut()
                .set_orphan(first_start.is_none_or(|start| i < start));
        }
    }

    /// Opens a mission: the `▲ M{n}` header over the chat. In radar the
    /// header draws the mission's flight-log row instead.
    pub(super) fn begin_mission(
        &mut self,
        prompt: &str,
        skill: Option<&str>,
        view: ToolOutputView,
    ) {
        let seq = self.missions.len() + 1;
        let flight = Rc::new(RefCell::new(FlightStats {
            mark: FlightMark::InFlight,
            ..Default::default()
        }));
        let mut header = MissionHeader::new(seq, prompt, skill);
        header.set_view(view);
        header.set_flight(flight.clone());
        let header = handle(header);
        self.missions.push(MissionRange {
            seq,
            start: self.chains.len(),
            landed: false,
            header: header.clone(),
            flight,
            landing: None,
        });
        if !self.chat.borrow().children.is_empty() {
            self.chat
                .borrow_mut()
                .add_child(self.peek_child(as_component(&handle(Spacer::new(1)))));
        }
        self.chat.borrow_mut().add_child(as_component(&header));
    }

    /// Appends the mission's landing line and marks it landed. No row for
    /// a stepless mission (returns false).
    pub(super) fn land_mission(
        &mut self,
        seq: usize,
        input: i64,
        output: i64,
        cost: f64,
        secs: f64,
    ) -> bool {
        let Some(i) = self.missions.iter().position(|m| m.seq == seq) else {
            return false;
        };
        if self.missions[i].landed {
            return false;
        }
        let (steps, hits) = self.mission_tally_seq(seq);
        if steps == 0 {
            return false;
        }
        let landed_time = format_duration_secs(secs);
        let cost_text = format!(
            "${}",
            hoocode_code_agent_session::format::js_to_fixed(cost, 3)
        );
        self.chat
            .borrow_mut()
            .add_child(self.peek_child(as_component(&handle(Spacer::new(1)))));
        // The landing row draws from spans (M5 review): the transcript
        // refreshes its counts when a landed mission's tally moves.
        let tokens = format!(
            "{} tokens",
            format_tokens((input.max(0) + output.max(0)) as u64)
        );
        let landing = handle(trajectory::LandingLine::new(
            steps,
            hits,
            &landed_time,
            &tokens,
            &cost_text,
        ));
        self.missions[i].landing = Some(landing.clone());
        self.chat
            .borrow_mut()
            .add_child(self.peek_child(as_component(&landing)));
        // The chains are closed by now, so the mark reads the outcome: a
        // clean mission landed, an interrupted one keeps `⊗` or `⊘`.
        let start = self.missions[i].start;
        let end = self
            .missions
            .get(i + 1)
            .map(|m| m.start)
            .unwrap_or(self.chains.len());
        let mark = self.mission_mark(start, end);
        let mut flight = self.missions[i].flight.borrow_mut();
        flight.steps = steps;
        flight.hits = hits;
        flight.mark = mark;
        flight.landed_time = Some(landed_time);
        flight.cost = Some(cost_text);
        flight.started = None;
        drop(flight);
        self.missions[i].landed = true;
        true
    }

    /// Takes a chain out of the chat and the chain list, pulling later
    /// mission starts down so a later mission keeps its first chain.
    /// A mission starting exactly at the removed chain keeps its start:
    /// the next chain slides into that index.
    pub(super) fn remove_chain(&mut self, chain: &Rc<RefCell<ToolChainComponent>>) {
        self.chat.borrow_mut().remove_child(&as_component(chain));
        if let Some(index) = self.chains.iter().position(|c| Rc::ptr_eq(c, chain)) {
            self.chains.remove(index);
            for mission in &mut self.missions {
                if mission.start > index {
                    mission.start -= 1;
                }
            }
        }
        if self
            .open_chain
            .as_ref()
            .is_some_and(|c| Rc::ptr_eq(c, chain))
        {
            self.open_chain = None;
        }
        if self
            .latest_chain
            .as_ref()
            .is_some_and(|c| Rc::ptr_eq(c, chain))
        {
            self.latest_chain = None;
        }
        self.refresh_flight_marks();
    }

    /// Renumbers every chain from the mission ranges: a retry that takes
    /// blocks out leaves positional numbering behind otherwise.
    pub(super) fn rebase_steps(&mut self) {
        let mut current_start: Option<usize> = None;
        let mut steps_before = 0;
        for (i, chain) in self.chains.iter().enumerate() {
            let start = self
                .missions
                .iter()
                .rev()
                .find(|m| m.start <= i)
                .map(|m| m.start);
            if start != current_start {
                current_start = start;
                steps_before = 0;
            }
            chain.borrow_mut().set_base_step(steps_before);
            steps_before += chain.borrow().tool_blocks().len();
        }
    }
}

impl Mode {
    pub(super) fn add_to_chat(&mut self, component: ComponentHandle) {
        self.transcript.chat.borrow_mut().add_child(component);
        self.dirty.set(true);
    }

    /// `showError`: a filled error block in the chat, headline over detail.
    pub(super) fn show_error(&mut self, message: &str) {
        let mut lines = message.split('\n');
        let title = format!("Error: {}", lines.next().unwrap_or(""));
        let mut body: Vec<&str> = lines.collect();
        // A wait measured in days is a dead end: say what to do about it.
        if is_long_retry_delay_error(Some(message)) {
            body.push(
                "Switch to another model with /model, or wait for the provider's quota to reset.",
            );
        }
        self.show_block(BlockFill::ToolErrorBg, "error", &title, &body);
    }

    /// The startup/reload listing and the session's own state.
    pub(super) fn render_resources(&mut self) {
        let mut listing = (self.listing)(&self.session);
        listing.columns = Some(self.size.get().0 as usize);
        listing.verbose = self.verbose;
        for component in show_loaded_resources(&listing, false, true) {
            self.add_to_chat(component);
        }
    }

    /// `getMarkdownThemeWithSettings`.
    pub(super) fn markdown_theme(&self) -> Rc<dyn Fn() -> MarkdownTheme> {
        let indent = self.code_block_indent.clone();
        Rc::new(move || MarkdownTheme {
            code_block_indent: Some(indent.clone()),
            ..get_markdown_theme()
        })
    }

    /// `thinkingDisplayForView`: radar drops traces outright.
    pub(super) fn thinking_display(&self) -> ThinkingDisplay {
        if self.tool_output_view == ToolOutputView::Radar {
            ThinkingDisplay::Omit
        } else if self.hide_thinking_block {
            ThinkingDisplay::Label
        } else {
            ThinkingDisplay::Full
        }
    }

    pub(super) fn create_working_loader(&self) -> Rc<RefCell<Loader>> {
        let mut loader = Loader::new(
            Box::new(|s: &str| theme().fg("accent", s)),
            Box::new(|s: &str| theme().fg("muted", s)),
            DEFAULT_WORKING_MESSAGE,
            None,
        );
        loader.start();
        handle(loader)
    }

    pub(super) fn stop_working_loader(&mut self) {
        if let Some(loader) = self.loader.take() {
            loader.borrow_mut().stop();
        }
        self.status.borrow_mut().clear();
    }

    /// Steps and failed steps in the current mission, for the step numbers
    /// and the landing line (M4). A user message opens a mission: everything
    /// after it, up to the next one, belongs to it.
    fn mission_tally(&self) -> (usize, usize) {
        self.transcript.mission_tally()
    }

    /// The mission header opening a user turn: `▲ M{n}  prompt`, with the
    /// skill chip when the turn runs a skill (M4). A queued follow-up opens
    /// its own mission while the previous one still runs; see
    /// `land_previous_mission`.
    fn add_mission_header(&mut self, text: &str, skill: Option<&str>) {
        if let Some(seq) = self.transcript.unlanded_current() {
            self.land_previous_mission(seq);
        }
        self.transcript
            .begin_mission(text, skill, self.tool_output_view);
        self.refresh_flight_stats();
    }

    /// A queued follow-up lands the running mission where its own header
    /// lands: the open chain closes and the mission gets its own landing
    /// line. Usage since the turn anchor is that mission's; the anchor
    /// resets so the final landing at PromptDone shows only the remainder
    /// (no double counting).
    fn land_previous_mission(&mut self, seq: usize) {
        self.close_open_chain(ChainState::Done);
        let now = sum_assistant_usage(self.session.session_manager().entries());
        let (input, output, cost, secs) = match self.turn_cost_anchor.take() {
            Some((anchor, at)) => (
                now.input as i64 - anchor.input as i64,
                now.output as i64 - anchor.output as i64,
                now.cost - anchor.cost,
                at.elapsed().as_secs_f64(),
            ),
            None => (0, 0, 0.0, 0.0),
        };
        self.turn_cost_anchor = Some((now, Instant::now()));
        self.transcript.land_mission(seq, input, output, cost, secs);
        self.refresh_flight_stats();
    }

    /// Refreshes the radar flight rows (M5) after the transcript changed:
    /// counts and marks come off the missions, and the flying mission reads
    /// its elapsed time off the turn anchor and its cost off usage since
    /// the anchor. Landed missions keep the time and cost they landed with.
    pub(super) fn refresh_flight_stats(&mut self) {
        self.transcript.refresh_flight_marks();
        let Some(mission) = self.transcript.missions.last() else {
            return;
        };
        if mission.landed {
            return;
        }
        let now = sum_assistant_usage(self.session.session_manager().entries());
        let mut flight = mission.flight.borrow_mut();
        match &self.turn_cost_anchor {
            Some((anchor, at)) => {
                flight.started = Some(*at);
                flight.cost = Some(format!(
                    "${}",
                    hoocode_code_agent_session::format::js_to_fixed(now.cost - anchor.cost, 3)
                ));
            }
            None => {
                flight.started = None;
                flight.cost = None;
            }
        }
    }

    /// `addMessageToChat` for the roles this transcript draws so far.
    pub(super) fn add_message_to_chat(&mut self, message: &AgentMessage, populate_history: bool) {
        match message {
            AgentMessage::User(user) => {
                let text: String = user
                    .content
                    .blocks()
                    .iter()
                    .filter_map(|c| match c {
                        Content::Text(t) => Some(t.text.as_str()),
                        _ => None,
                    })
                    .collect();
                if text.is_empty() {
                    return;
                }
                // Every user turn opens a mission (M4): the `▲ M{n}` header
                // carries the first line, the rest stays in the message.
                match parse_skill_block(&text) {
                    Some(skill) => {
                        let prompt = skill.user_message.as_deref().unwrap_or(&text);
                        self.add_mission_header(prompt, Some(&skill.name));
                        // The message the user typed comes first; the skill it
                        // invoked is drawn under it.
                        if let Some(message) = skill.user_message {
                            let body = mission_body(&message);
                            if !body.is_empty() {
                                let component = UserMessageComponent::with_theme(
                                    &body,
                                    (self.markdown_theme())(),
                                );
                                // Radar hides the body: the flight-log row
                                // carries the first line.
                                let child = as_component(&handle(component));
                                self.add_to_chat(self.transcript.peek_child(child));
                            }
                        }
                        let component =
                            handle(SkillBlockComponent::new(&skill.name, &skill.content));
                        component
                            .borrow_mut()
                            .set_expanded(self.tool_output_view == ToolOutputView::Peek);
                        self.transcript.skill_blocks.push(component.clone());
                        self.add_to_chat(as_component(&component));
                    }
                    None => match parse_background_notice(&text) {
                        Some(notice) => {
                            self.add_mission_header(&text, None);
                            let component = handle(BackgroundNoticeComponent::new(notice));
                            component
                                .borrow_mut()
                                .set_expanded(self.tool_output_view == ToolOutputView::Peek);
                            self.transcript.background_notices.push(component.clone());
                            self.add_to_chat(as_component(&component));
                        }
                        None => {
                            self.add_mission_header(&text, None);
                            let body = mission_body(&text);
                            if !body.is_empty() {
                                let component = UserMessageComponent::with_theme(
                                    &body,
                                    (self.markdown_theme())(),
                                );
                                // Radar hides the body: the flight-log row
                                // carries the first line.
                                let child = as_component(&handle(component));
                                self.add_to_chat(self.transcript.peek_child(child));
                            }
                        }
                    },
                }
                if populate_history {
                    self.editor.borrow_mut().editor.add_to_history(&text);
                }
            }
            AgentMessage::Assistant(assistant) => {
                let component = handle(AssistantMessageComponent::with_theme(
                    Some(assistant),
                    self.thinking_display(),
                    self.markdown_theme(),
                    DEFAULT_HIDDEN_THINKING_LABEL,
                ));
                // Mission Control peek: prose sits on the `┊` rail.
                component
                    .borrow_mut()
                    .set_rail(self.tool_output_view == ToolOutputView::Peek);
                self.transcript.assistant_components.push(component.clone());
                self.add_to_chat(as_component(&component));
            }
            AgentMessage::Custom(custom) => {
                if custom.display {
                    let component = handle(CustomMessageComponent::new(
                        custom.clone(),
                        self.markdown_theme(),
                    ));
                    component
                        .borrow_mut()
                        .set_expanded(self.tool_output_view == ToolOutputView::Peek);
                    self.transcript.custom_messages.push(component.clone());
                    self.add_to_chat(as_component(&component));
                }
            }
            AgentMessage::CompactionSummary(summary) => {
                self.add_to_chat(as_component(&handle(Spacer::new(1))));
                let component = handle(CompactionSummaryMessageComponent::new(
                    summary.clone(),
                    self.markdown_theme(),
                ));
                component
                    .borrow_mut()
                    .set_expanded(self.tool_output_view == ToolOutputView::Peek);
                self.transcript.compaction_summaries.push(component.clone());
                self.add_to_chat(as_component(&component));
            }
            AgentMessage::BranchSummary(summary) => {
                self.add_to_chat(as_component(&handle(Spacer::new(1))));
                let component = handle(BranchSummaryMessageComponent::new(
                    summary.clone(),
                    self.markdown_theme(),
                ));
                component
                    .borrow_mut()
                    .set_expanded(self.tool_output_view == ToolOutputView::Peek);
                self.transcript.branch_summaries.push(component.clone());
                self.add_to_chat(as_component(&component));
            }
            AgentMessage::BashExecution(bash) => {
                let exclude = bash.exclude_from_context.unwrap_or(false);
                let mut component = BashExecutionComponent::new(&bash.command, exclude);
                if !bash.output.is_empty() {
                    component.append_output(&bash.output);
                }
                component.set_complete(
                    bash.exit_code.map(|c| c as i32),
                    bash.cancelled,
                    bash.truncated.then(|| truncated_marker("")),
                    bash.full_output_path.clone(),
                );
                let component = handle(component);
                self.add_to_chat(as_component(&component));
            }
            _ => {}
        }
    }

    /// `renderSessionContext`: the transcript of messages already in the
    /// session, with tool calls drawn as the live path draws them.
    pub(super) fn render_session_context(
        &mut self,
        messages: &[AgentMessage],
        populate_history: bool,
    ) {
        self.transcript.pending_tools.clear();
        let mut rendered_pending: Vec<(String, Rc<RefCell<ToolExecutionComponent>>)> = Vec::new();
        let mut last_stop_reason = None;
        let mut turn: Option<RebuiltTurn> = None;
        for (i, message) in messages.iter().enumerate() {
            match message {
                AgentMessage::User(user) => {
                    // The hail behind empty enter after COMMS LOST (M8e1):
                    // the last replayed user message stands for the prompt.
                    self.transcript.last_prompt = Some(user_prompt(user));
                    // A new turn: the one before it has ended, and its
                    // per-turn token row goes where the live path drew it.
                    if let Some(ended) = turn.take() {
                        self.show_rebuilt_turn_cost(ended);
                    }
                    turn = Some(RebuiltTurn {
                        started_ms: user.timestamp,
                        ended_ms: user.timestamp,
                        ..RebuiltTurn::default()
                    });
                    self.add_message_to_chat(message, populate_history);
                }
                AgentMessage::Assistant(assistant) => {
                    if let Some(turn) = turn.as_mut() {
                        add_assistant_usage(&mut turn.usage, &assistant.usage);
                        turn.ended_ms = assistant.timestamp;
                        turn.stop = Some(assistant.stop_reason);
                    }
                    // The live path's chain boundary, so a rebuilt history shows
                    // the chains it lived through.
                    if self.opens_new_chain(assistant) {
                        self.close_open_chain(ChainState::Done);
                    }
                    last_stop_reason = Some(assistant.stop_reason);
                    // A failed attempt the session retried is not shown again
                    // (`retry.rs`); the streak that gave up is one Model block.
                    if assistant.stop_reason == StopReason::Error {
                        if retry::attempt_was_retried(messages, i) {
                            continue;
                        }
                        let failed = retry::failed_attempts_before(messages, i);
                        if let Some(first) = failed.first() {
                            let elapsed = (assistant.timestamp - first.timestamp).max(0);
                            let title = assistant
                                .error_message
                                .clone()
                                .filter(|m| !m.is_empty())
                                .unwrap_or_else(|| "Error".into());
                            self.transcript.add_model_error(
                                &title,
                                &retry::model_error_body(
                                    failed.len() as u64 + 1,
                                    std::time::Duration::from_millis(elapsed as u64),
                                    false,
                                ),
                                // A rebuilt streak that gave up is COMMS LOST
                                // too (M8e1): its hails are its attempts.
                                failed.len() as u64 + 1,
                            );
                            // Without a user message behind it there is
                            // nothing to hail again: stay out of COMMS LOST.
                            if self.transcript.last_prompt.is_some() {
                                self.comms_lost.set(failed.len() as u64 + 1 >= 2);
                            }
                            continue;
                        }
                    }
                    self.add_message_to_chat(message, populate_history);
                    let trace = self.transcript.assistant_components.last().cloned();
                    if let Some(trace) = &trace {
                        if assistant
                            .content
                            .iter()
                            .any(|c| matches!(c, Content::ToolCall(_)))
                        {
                            self.place_trace_in_chain(trace);
                        }
                    }
                    for content in &assistant.content {
                        let Content::ToolCall(call) = content else {
                            continue;
                        };
                        let block =
                            self.new_tool_block(&call.name, &call.id, call.arguments.clone());
                        self.attach_tool_block(block.clone());
                        if matches!(
                            assistant.stop_reason,
                            StopReason::Aborted | StopReason::Error
                        ) {
                            let error = if assistant.stop_reason == StopReason::Aborted {
                                let attempt = self.session.retry_attempt();
                                if attempt > 0 {
                                    format!(
                                        "Aborted after {attempt} retry attempt{}",
                                        if attempt > 1 { "s" } else { "" }
                                    )
                                } else {
                                    "Operation aborted".to_string()
                                }
                            } else {
                                assistant
                                    .error_message
                                    .clone()
                                    .filter(|m| !m.is_empty())
                                    .unwrap_or_else(|| "Error".into())
                            };
                            block.borrow_mut().update_result(
                                ToolResult {
                                    content: vec![Content::text(error)],
                                    details: serde_json::Value::Null,
                                    is_error: true,
                                },
                                false,
                            );
                        } else {
                            block.borrow_mut().set_args_complete();
                            rendered_pending.push((call.id.clone(), block));
                        }
                    }
                }
                AgentMessage::ToolResult(result) => {
                    if let Some(turn) = turn.as_mut() {
                        turn.ended_ms = result.timestamp;
                    }
                    if let Some(i) = rendered_pending
                        .iter()
                        .position(|(id, _)| *id == result.tool_call_id)
                    {
                        let (_, block) = rendered_pending.remove(i);
                        block.borrow_mut().update_result(
                            ToolResult {
                                content: result.content.clone(),
                                details: rebuilt_details(result),
                                is_error: result.is_error,
                            },
                            false,
                        );
                    }
                }
                _ => self.add_message_to_chat(message, populate_history),
            }
        }
        // History has no live state: a chain still open is finished, unless
        // calls still wait for results (a resumed run in flight). A turn that
        // has settled gets its token row; one still in flight waits for it.
        if rendered_pending.is_empty() {
            if let Some(ended) = turn.take() {
                self.show_rebuilt_turn_cost(ended);
            }
            self.close_open_chain(if last_stop_reason == Some(StopReason::Stop) {
                ChainState::Done
            } else {
                ChainState::Interrupted
            });
        }
        self.transcript.pending_tools.extend(rendered_pending);
        self.dirty.set(true);
    }

    /// `renderInitialMessages`: the loaded session's transcript, and how often
    /// it was compacted.
    pub(super) fn render_initial_messages(&mut self) {
        // The hull paint lives in the session file; the provider is only
        // its render cache (M8e5).
        let paint = self
            .session
            .hull_paint()
            .and_then(|name| HullPaint::parse(&name).ok().flatten());
        self.footer_data.set_hull_paint(paint);
        let messages = self.session.messages();
        // The plan is the session's own: rebuilt from its last TodoWrite, so a
        // resumed or reloaded session shows the plan it ended with. That call
        // may sit before a compaction, so the whole branch is read, not the
        // compacted context.
        let branch: Vec<AgentMessage> = self
            .session
            .session_manager()
            .branch(None)
            .into_iter()
            .filter_map(|e| match e {
                hoocode_code_session::FileEntry::Message { message, .. } => Some(message.clone()),
                _ => None,
            })
            .collect();
        hoocode_code_tools_optin::restore_plan(hoocode_code_task_store::task_store(), &branch);
        self.update_editor_border_color();
        self.render_session_context(&messages, true);
        let compactions = self
            .session
            .session_manager()
            .entries()
            .iter()
            .filter(|e| matches!(e, hoocode_code_session::FileEntry::Compaction { .. }))
            .count();
        if compactions > 0 {
            let times = if compactions == 1 {
                "1 time".to_string()
            } else {
                format!("{compactions} times")
            };
            self.show_status(&format!("Session compacted {times}"));
        }
    }

    /// Re-render the in-flight message now, or once the throttle window passes.
    pub(super) fn schedule_streaming_render(&mut self) {
        let due = self
            .stream_render_at
            .is_none_or(|at| at.elapsed() >= STREAM_RENDER_THROTTLE);
        if due {
            self.run_streaming_render();
        } else {
            self.stream_render_pending = true;
        }
    }

    pub(super) fn run_streaming_render(&mut self) {
        self.stream_render_pending = false;
        self.stream_render_at = Some(Instant::now());
        if let (Some(component), Some(message)) = (
            &self.transcript.streaming,
            &self.transcript.streaming_message,
        ) {
            component.borrow_mut().update_shared(message.clone(), true);
            // While an attached view is on screen, the session's text is not drawn.
            if self.attached.is_none() {
                self.dirty.set(true);
            }
        }
    }

    pub(super) fn new_tool_block(
        &self,
        name: &str,
        id: &str,
        args: serde_json::Value,
    ) -> Rc<RefCell<ToolExecutionComponent>> {
        // A registered tool always has a definition, renderers or not; only an
        // unknown name gets the bare text rendering.
        let definition = self
            .session
            .get_tool_definition(name)
            .map(|_| registered_tool_definition(name));
        handle(ToolExecutionComponent::new(
            name,
            id,
            args,
            ToolExecutionOptions {
                show_images: self.show_images,
                image_width_cells: self.image_width_cells,
                view: self.tool_output_view,
            },
            definition,
            &self.session.cwd().to_string_lossy(),
        ))
    }

    /// `attachToolBlock`: into the open chain (a new one when none is open),
    /// marking the newest call and run.
    pub(super) fn attach_tool_block(&mut self, block: Rc<RefCell<ToolExecutionComponent>>) {
        let chain = match &self.transcript.open_chain {
            Some(chain) if chain.borrow().is_open() => chain.clone(),
            _ => {
                let chain = handle(ToolChainComponent::new(self.tool_output_view));
                // Steps number per mission (M4): the chain starts after the
                // steps the mission already holds.
                chain.borrow_mut().set_base_step(self.mission_tally().0);
                self.add_to_chat(as_component(&chain));
                self.transcript.chains.push(chain.clone());
                self.transcript.open_chain = Some(chain.clone());
                chain
            }
        };
        chain.borrow_mut().add(block.clone());
        if let Some(previous) = self.transcript.latest_block.replace(block.clone()) {
            previous.borrow_mut().set_latest(false);
        }
        block.borrow_mut().set_latest(true);
        let same = self
            .transcript
            .latest_chain
            .as_ref()
            .is_some_and(|c| Rc::ptr_eq(c, &chain));
        if !same {
            if let Some(previous) = self.transcript.latest_chain.replace(chain.clone()) {
                previous.borrow_mut().set_latest(false);
            }
            chain.borrow_mut().set_latest(true);
        }
        self.refresh_flight_stats();
        self.dirty.set(true);
    }

    /// Moves a message's trace into the open chain, above the calls it is
    /// about to add, so thinking reads above the call it led to. Skipped when
    /// no chain is open: the trace then sits above the chain it opens.
    pub(super) fn place_trace_in_chain(
        &mut self,
        component: &Rc<RefCell<AssistantMessageComponent>>,
    ) {
        let Some(chain) = self.transcript.open_chain.clone() else {
            return;
        };
        if !chain.borrow().is_open() || chain.borrow().has_lead_in(component) {
            return;
        }
        self.transcript
            .chat
            .borrow_mut()
            .remove_child(&as_component(component));
        chain.borrow_mut().add_lead_in(component.clone());
    }

    /// The streaming message speaks after its calls joined the open chain:
    /// its trace goes back into the transcript, after the chain, so its text
    /// is not drawn inside the run.
    pub(super) fn release_streaming_trace(&mut self) {
        let (Some(chain), Some(component)) = (
            self.transcript.open_chain.clone(),
            self.transcript.streaming.clone(),
        ) else {
            return;
        };
        if chain.borrow_mut().take_lead_in(&component) {
            self.add_to_chat(as_component(&component));
        }
    }

    /// `closeOpenChain`: settle the chain collecting calls.
    pub(super) fn close_open_chain(&mut self, outcome: ChainState) {
        let Some(chain) = self.transcript.open_chain.take() else {
            return;
        };
        if chain.borrow().is_empty() {
            self.transcript.remove_chain(&chain);
        } else {
            chain.borrow_mut().close(outcome);
        }
        self.refresh_flight_stats();
        self.dirty.set(true);
    }

    /// `opensNewChain`: speaking ends the run. A trace never does, in either
    /// view: thinking sits inside the chain it belongs to.
    pub(super) fn opens_new_chain(&self, message: &AssistantMessage) -> bool {
        speaking_ends_chain(&message.content)
    }

    /// `trimTranscriptMemory`: freeze all but the newest live tool blocks.
    pub(super) fn trim_transcript_memory(&mut self) {
        let freezable: Vec<_> = self
            .transcript
            .chains
            .iter()
            .flat_map(|c| c.borrow().tool_blocks().to_vec())
            .filter(|b| b.borrow().is_freezable())
            .collect();
        let excess = freezable.len().saturating_sub(LIVE_TOOL_WINDOW);
        for block in &freezable[..excess] {
            block.borrow_mut().freeze();
        }
    }

    /// `showNotice`: a filled warning block in the chat, for warnings that
    /// cost money if ignored (`showBlock`).
    pub(super) fn show_notice(&mut self, title: &str, body: &[&str]) {
        self.show_block(BlockFill::WarningBg, "warning", title, body);
    }

    /// `showBlock`: a filled block in the chat, bold title over muted lines.
    fn show_block(&mut self, fill: BlockFill, fg: &str, title: &str, body: &[&str]) {
        let t = theme();
        let mut block = BoxComponent::new(1, 1, None);
        apply_block_fill(&mut block, fill);
        block.add_child(as_component(&handle(Text::new(
            t.bold(&t.fg(fg, title)),
            0,
            0,
        ))));
        for line in body {
            block.add_child(as_component(&handle(Text::new(t.fg("muted", line), 0, 0))));
        }
        self.add_to_chat(as_component(&handle(Spacer::new(1))));
        self.add_to_chat(as_component(&handle(block)));
    }

    /// Every tool block in the transcript.
    pub(super) fn tool_blocks(&self) -> Vec<Rc<RefCell<ToolExecutionComponent>>> {
        let mut blocks: Vec<_> = self
            .transcript
            .chains
            .iter()
            .flat_map(|c| c.borrow().tool_blocks().to_vec())
            .collect();
        for block in self.transcript.pending_tools.values() {
            if !blocks.iter().any(|b| Rc::ptr_eq(b, block)) {
                blocks.push(block.clone());
            }
        }
        blocks
    }

    /// A bordered page in the chat: accent title over markdown
    /// (`handleHotkeys`, `handleChangelog`, the startup "What's New").
    pub(super) fn show_bordered_markdown(&mut self, title: &str, markdown: &str) {
        let t = theme();
        self.add_to_chat(as_component(&handle(Spacer::new(1))));
        self.add_to_chat(as_component(&handle(DynamicBorder::new(None))));
        self.add_to_chat(as_component(&handle(Text::new(
            t.bold(&t.fg("accent", title)),
            1,
            0,
        ))));
        self.add_to_chat(as_component(&handle(Spacer::new(1))));
        self.add_to_chat(as_component(&handle(Markdown::new(
            markdown,
            1,
            0,
            (self.markdown_theme())(),
            None,
        ))));
        self.add_to_chat(as_component(&handle(DynamicBorder::new(None))));
    }

    /// `toggleThinkingBlockVisibility`: hide or show thinking traces, saved,
    /// and the transcript rebuilt with the new display.
    pub(super) fn toggle_thinking_block_visibility(&mut self) {
        self.hide_thinking_block = !self.hide_thinking_block;
        self.session
            .settings()
            .set_hide_thinking_block(self.hide_thinking_block);
        let streaming = self.transcript.streaming.clone();
        let streaming_message = self.transcript.streaming_message.clone();
        self.reset_transcript_view();
        let messages = self.session.messages();
        self.render_session_context(&messages, false);
        // A message still streaming goes back on with the new display.
        if let (Some(component), Some(message)) = (streaming, streaming_message) {
            {
                let mut c = component.borrow_mut();
                c.set_thinking_display(self.thinking_display());
                c.update_shared(message.clone(), true);
            }
            self.add_to_chat(as_component(&component));
            self.transcript.streaming = Some(component);
            self.transcript.streaming_message = Some(message);
        }
        // Radar hides thinking whatever the setting, so the toggle has no
        // visible effect there: the notice says what was saved and why the
        // screen did not change.
        let status = if self.tool_output_view == ToolOutputView::Radar {
            format!(
                "Thinking blocks: {}. Radar hides them; ctrl+o shows peek.",
                if self.hide_thinking_block {
                    "hidden"
                } else {
                    "visible"
                }
            )
        } else if self.hide_thinking_block {
            "Thinking blocks: hidden".to_string()
        } else {
            "Thinking blocks: visible".to_string()
        };
        self.show_status(&status);
    }

    /// `resetTranscriptView`: drop every view reference into the transcript.
    /// The rows are gone, so a pinned view has nothing to hold: it follows the
    /// tail again, and the `N new` count starts over.
    pub(super) fn reset_transcript_view(&mut self) {
        self.transcript.chat.borrow_mut().clear();
        self.transcript.reset_view();
        // A fresh transcript is not a lost mission.
        self.comms_lost.set(false);
        self.tui.scroll_to_live();
        self.tui.reset_new_lines_count();
    }

    /// `renderCurrentSessionState`: the transcript of the session just
    /// swapped in, after its resource listing.
    pub(super) fn render_current_session_state(&mut self) {
        self.reset_transcript_view();
        self.render_resources();
        self.render_initial_messages();
    }

    /// `showStatus`: a passing status in the notification band above the
    /// prompt (the first line is the title, the rest its body).
    pub(super) fn show_status(&mut self, message: &str) {
        self.notify(NotificationKind::Info, message);
    }

    /// `showWarning`: on the notification band, not in the transcript.
    pub(super) fn show_warning(&mut self, message: &str) {
        self.notify(NotificationKind::Warning, message);
    }

    pub(super) fn notify(&mut self, kind: NotificationKind, message: &str) {
        let mut lines = message.split('\n');
        let title = lines.next().unwrap_or("");
        let body: Vec<&str> = lines.collect();
        self.notifications
            .borrow_mut()
            .notify(kind, title, &body, None, None, None);
        self.dirty.set(true);
    }

    /// `showRecord`: a dim line in the chat that stays; back-to-back records
    /// update the previous line instead of stacking.
    pub(super) fn show_record(&mut self, message: &str) {
        let styled = if message.contains("\x1b[") {
            message.to_string()
        } else {
            theme().fg("dim", message)
        };
        self.transcript
            .last_status
            .show(&mut self.transcript.chat.borrow_mut(), styled);
        self.dirty.set(true);
    }

    /// `showDialStep`: the stop a dial landed on, and (the first time) how to
    /// step back.
    pub(super) fn show_dial_step(&mut self, backward: &'static str, message: &str) {
        let taught = !self.dial_reverse_taught.insert(backward);
        let topic = backward
            .strip_suffix(".cycleForward")
            .or_else(|| backward.strip_suffix(".cycleBackward"))
            .unwrap_or(backward);
        let note = (!taught).then(|| format!("{} steps back", key_text(backward)));
        self.notifications.borrow_mut().notify(
            NotificationKind::Info,
            message,
            &[],
            note.as_deref(),
            None,
            Some(topic),
        );
        self.dirty.set(true);
    }

    /// `applyToolOutputView`: move every block and chain to `view`.
    pub(super) fn apply_tool_output_view(&mut self, view: ToolOutputView, persist: bool) {
        // The change moves rows above the pinned top: the same message stays on top.
        let anchor =
            scroll_view::capture_top(&self.tui, &self.transcript.chat, self.scroll_body.as_ref());
        let previous_thinking = self.thinking_display();
        self.tool_output_view = view;
        if persist {
            self.session.settings().set_tool_output_view(view);
        }
        self.footer.borrow_mut().set_tool_output_view(view);
        self.comms.borrow_mut().set_tool_output_view(view);
        // Radar hides the peek-only rows (landing lines, message bodies)
        // and switches the mission headers to their flight-log rows.
        self.transcript
            .radar_active
            .set(view == ToolOutputView::Radar);
        for mission in &self.transcript.missions {
            mission.header.borrow_mut().set_view(view);
        }
        self.refresh_flight_stats();
        let thinking = self.thinking_display();
        for chain in &self.transcript.chains {
            chain.borrow_mut().set_view(view);
        }
        if thinking != previous_thinking {
            for component in &self.transcript.assistant_components {
                component.borrow_mut().set_thinking_display(thinking);
            }
        }
        // Peek opens each message's body; radar folds it to a row. The startup
        // header and resource listing stay compact (they open under `verbose`).
        let peek = view == ToolOutputView::Peek;
        // Mission Control peek: prose sits on the `┊` rail; radar has none.
        for component in &self.transcript.assistant_components {
            component.borrow_mut().set_rail(peek);
        }
        for component in &self.transcript.custom_messages {
            component.borrow_mut().set_expanded(peek);
        }
        for component in &self.transcript.skill_blocks {
            component.borrow_mut().set_expanded(peek);
        }
        for component in &self.transcript.background_notices {
            component.borrow_mut().set_expanded(peek);
        }
        for component in &self.transcript.branch_summaries {
            component.borrow_mut().set_expanded(peek);
        }
        for component in &self.transcript.compaction_summaries {
            component.borrow_mut().set_expanded(peek);
        }
        scroll_view::reanchor_top(
            &mut self.tui,
            &self.transcript.chat,
            self.scroll_body.as_ref(),
            anchor,
        );
        self.dirty.set(true);
    }

    /// `app.tools.expand` (Ctrl+O): flip between radar and peek, and save it.
    pub(super) fn toggle_tool_output_view(&mut self) {
        let next = match self.tool_output_view {
            ToolOutputView::Radar => ToolOutputView::Peek,
            ToolOutputView::Peek => ToolOutputView::Radar,
        };
        self.apply_tool_output_view(next, true);
        // A toggle has no direction, so the notice carries no "steps back" note.
        self.notifications.borrow_mut().notify(
            NotificationKind::Info,
            &format!("Tool output: {next}"),
            &[],
            None,
            None,
            Some("app.tools.expand"),
        );
        self.dirty.set(true);
    }

    /// `cycleToolOutputView`: one stop on the dial, saved.
    pub(super) fn cycle_tool_output_view(&mut self, forward: bool) {
        let next = cycle_tool_output_view(self.tool_output_view, forward);
        self.apply_tool_output_view(next, true);
        self.show_dial_step(
            if forward {
                "app.view.cycleBackward"
            } else {
                "app.view.cycleForward"
            },
            &format!("Tool output: {next}"),
        );
    }

    /// `showTurnCost`: this request's own tokens, time and cost.
    pub(super) fn show_turn_cost(&mut self) {
        let Some((anchor, at)) = self.turn_cost_anchor.take() else {
            return;
        };
        let now = sum_assistant_usage(self.session.session_manager().entries());
        self.show_turn_cost_row(
            now.input as i64 - anchor.input as i64,
            now.output as i64 - anchor.output as i64,
            now.cost - anchor.cost,
            at.elapsed().as_secs_f64(),
        );
    }

    /// The per-turn row of a settled turn in a rebuilt transcript: the same
    /// row `show_turn_cost` draws after a live turn.
    fn show_rebuilt_turn_cost(&mut self, turn: RebuiltTurn) {
        let outcome = if turn.stop == Some(StopReason::Stop) {
            ChainState::Done
        } else {
            ChainState::Interrupted
        };
        self.close_open_chain(outcome);
        let secs = (turn.ended_ms - turn.started_ms).max(0) as f64 / 1000.0;
        self.show_turn_cost_row(
            turn.usage.input as i64,
            turn.usage.output as i64,
            turn.usage.cost,
            secs,
        );
    }

    /// The row itself: tokens in and out, the turn's time, and its cost.
    fn show_turn_cost_row(&mut self, input: i64, output: i64, cost: f64, secs: f64) {
        // Mission Control (M4): tally first. A mission with steps always
        // lands, even at zero usage (`0 tokens · $0.000`). A stepless turn
        // keeps the old accounting row, silent when nothing was accounted.
        if let Some(seq) = self.transcript.missions.last().map(|m| m.seq) {
            if self.transcript.land_mission(seq, input, output, cost, secs) {
                self.refresh_flight_stats();
                return;
            }
        }
        // A stepless mission still reads its time and cost in radar.
        self.refresh_flight_stats();
        // Nothing was accounted: stay silent rather than print zeroes.
        if input <= 0 && output <= 0 {
            return;
        }
        self.add_to_chat(as_component(&handle(Spacer::new(1))));
        let t = theme();
        let mut segs = vec![
            format!(
                "{}{}{}{}",
                t.fg("dim", "↑"),
                t.fg("muted", &format_tokens(input.max(0) as u64)),
                t.fg("dim", " ↓"),
                t.fg("muted", &format_tokens(output.max(0) as u64)),
            ),
            t.fg("muted", &format_duration_secs(secs)),
        ];
        if cost > 0.0 {
            segs.push(t.fg(
                "muted",
                &format!(
                    "${}",
                    hoocode_code_agent_session::format::js_to_fixed(cost, 3)
                ),
            ));
        }
        let separator = t.fg("dim", " · ");
        self.add_to_chat(as_component(&handle(Text::new(
            segs.join(&separator),
            1,
            0,
        ))));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoocode_ai_types::{Content, ToolResultMessage};
    use hoocode_code_tui_widgets::tool_chain::{ToolBlock, ToolChainComponent};
    use hoocode_code_tui_widgets::tool_signal::ToolResult;
    use hoocode_tui_util::strip_vt_control_characters;
    use serde_json::{json, Value};

    fn fresh_transcript() -> Transcript {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        Transcript::new(Rc::new(RefCell::new(Container::new())))
    }

    fn user_message(content: UserContent) -> UserMessage {
        UserMessage {
            content,
            timestamp: 0,
        }
    }

    #[test]
    fn user_prompt_reads_plain_text() {
        let prompt = user_prompt(&user_message(UserContent::Text("hail me".into())));
        assert_eq!(prompt.text, "hail me");
        assert!(prompt.images.is_empty());
    }

    #[test]
    fn user_prompt_reads_blocks_with_images() {
        let prompt = user_prompt(&user_message(UserContent::Blocks(vec![
            Content::Text(hoocode_ai_types::TextContent {
                text: "look ".into(),
                text_signature: None,
            }),
            Content::Image(ImageContent {
                data: "aGVsbG8=".into(),
                media_type: "image/png".into(),
            }),
        ])));
        assert_eq!(prompt.text, "look ");
        assert_eq!(prompt.images.len(), 1);
        assert_eq!(prompt.images[0].media_type, "image/png");
    }

    #[test]
    fn reset_view_clears_the_last_prompt() {
        let mut transcript = fresh_transcript();
        transcript.last_prompt = Some(LastPrompt {
            text: "hail me".into(),
            images: Vec::new(),
        });
        transcript.reset_view();
        assert_eq!(transcript.last_prompt, None);
    }

    /// The chat as plain text, 100 columns.
    fn chat_text(chat: &Rc<RefCell<Container>>) -> String {
        let mut lines = Vec::new();
        for child in chat.borrow().children.iter() {
            lines.extend(child.borrow_mut().render(100));
        }
        strip_vt_control_characters(&lines.join("\n"))
    }

    fn shell_block(id: &str) -> ToolBlock {
        let mut block = ToolExecutionComponent::new(
            "Shell",
            id,
            json!({"command": "echo hi"}),
            ToolExecutionOptions {
                view: ToolOutputView::Peek,
                ..Default::default()
            },
            None,
            "/",
        );
        block.update_result(
            ToolResult {
                content: vec![Content::text("hi")],
                details: json!({}),
                is_error: false,
            },
            false,
        );
        Rc::new(RefCell::new(block))
    }

    fn chain_with(n: usize) -> Rc<RefCell<ToolChainComponent>> {
        let chain = Rc::new(RefCell::new(ToolChainComponent::new(ToolOutputView::Peek)));
        for i in 0..n {
            chain.borrow_mut().add(shell_block(&format!("call-{i}")));
        }
        chain
    }

    /// The flight row reads the mission's own tally: steps and hits count
    /// as the landing line counts them, and landing stores the time, cost
    /// and mark (no duplicate counting).
    #[test]
    fn flight_stats_land_with_the_mission() {
        use hoocode_code_tui_widgets::radar::FlightMark;

        let mut t = fresh_transcript();
        t.begin_mission("do it", None, ToolOutputView::Peek);
        let chain = chain_with(2);
        t.chains.push(chain.clone());
        t.refresh_flight_marks();
        let flight = t.missions[0].flight.borrow();
        assert_eq!(flight.steps, 2);
        assert_eq!(flight.hits, 0);
        assert_eq!(flight.mark, FlightMark::InFlight);
        drop(flight);
        chain.borrow_mut().close(ChainState::Done);
        assert!(t.land_mission(1, 100, 200, 0.002, 0.5));
        let flight = t.missions[0].flight.borrow();
        assert_eq!(flight.steps, 2);
        assert_eq!(flight.hits, 0);
        assert_eq!(flight.mark, FlightMark::Landed);
        assert!(flight.landed_time.is_some());
        assert!(flight.cost.is_some());
        assert!(flight.started.is_none());
    }

    /// A landed mission whose tally moves rebuilds its landing row, so
    /// peek and radar agree on the counts.
    #[test]
    fn landed_counts_rebuild_the_landing_row() {
        let mut t = fresh_transcript();
        t.begin_mission("do it", None, ToolOutputView::Peek);
        let chain = chain_with(2);
        t.chains.push(chain.clone());
        chain.borrow_mut().close(ChainState::Done);
        assert!(t.land_mission(1, 100, 200, 0.002, 0.5));
        assert!(chat_text(&t.chat).contains("2 steps"));
        // A retry takes a block out after landing: both rows follow.
        let dropped = chain.borrow().tool_blocks()[0].clone();
        t.remove_tool_block(&dropped);
        t.rebase_steps();
        let text = chat_text(&t.chat);
        assert!(text.contains("1 step"), "{text}");
        assert!(!text.contains("2 steps"), "{text}");
        let flight = t.missions[0].flight.borrow();
        assert_eq!((flight.steps, flight.hits), (1, 0));
        let landing = t.missions[0].landing.as_ref().unwrap().borrow();
        assert_eq!(landing.counts(), (1, 0));
    }

    /// An interrupted mission keeps `⊗` when a step failed, `⊘` when none did.
    #[test]
    fn flight_mark_reads_the_outcome_off_interrupted_chains() {
        use hoocode_code_tui_widgets::radar::FlightMark;

        let mut t = fresh_transcript();
        t.begin_mission("run it", None, ToolOutputView::Peek);
        let mut failed = ToolExecutionComponent::new(
            "Shell",
            "call-bad",
            json!({"command": "exit 3"}),
            ToolExecutionOptions {
                view: ToolOutputView::Peek,
                ..Default::default()
            },
            None,
            "/",
        );
        failed.update_result(
            ToolResult {
                content: vec![Content::text("boom")],
                details: json!({}),
                is_error: true,
            },
            false,
        );
        let chain = Rc::new(RefCell::new(ToolChainComponent::new(ToolOutputView::Peek)));
        chain.borrow_mut().add(Rc::new(RefCell::new(failed)));
        t.chains.push(chain.clone());
        chain.borrow_mut().close(ChainState::Interrupted);
        t.refresh_flight_marks();
        assert_eq!(t.missions[0].flight.borrow().mark, FlightMark::DidNotLand);

        let mut t = fresh_transcript();
        t.begin_mission("run it", None, ToolOutputView::Peek);
        let clean = chain_with(1);
        t.chains.push(clean.clone());
        clean.borrow_mut().close(ChainState::Interrupted);
        t.refresh_flight_marks();
        assert_eq!(t.missions[0].flight.borrow().mark, FlightMark::Aborted);
    }

    /// A queued follow-up lands the running mission where its own header
    /// lands: two missions, two landing lines, each with its own steps and
    /// its own usage (no double counting).
    #[test]
    fn a_queued_follow_up_lands_each_mission_with_its_own_steps() {
        let mut t = fresh_transcript();
        t.begin_mission("first prompt", None, ToolOutputView::Peek);
        let first = chain_with(2);
        t.chains.push(first.clone());
        t.open_chain = Some(first);
        // The follow-up's header finds M1 unlanded with steps: the mode
        // closes the chain and lands it with the usage known so far.
        assert_eq!(t.unlanded_current(), Some(1));
        t.chains[0].borrow_mut().close(ChainState::Done);
        t.open_chain = None;
        assert!(t.land_mission(1, 100, 200, 0.002, 0.5));
        // M2 opens with one step; PromptDone lands it with the remainder.
        t.begin_mission("second prompt", None, ToolOutputView::Peek);
        assert_eq!(t.unlanded_current(), None);
        let second = chain_with(1);
        second.borrow_mut().set_base_step(t.mission_tally().0);
        t.chains.push(second.clone());
        t.open_chain = Some(second);
        assert!(t.land_mission(2, 50, 60, 0.001, 0.3));
        let text = chat_text(&t.chat);
        let landed: Vec<&str> = text.lines().filter(|l| l.contains("LANDED")).collect();
        assert_eq!(landed.len(), 2, "{text}");
        assert!(landed[0].contains("2 steps"), "{text}");
        assert!(landed[1].contains("1 step"), "{text}");
        assert!(
            landed[0].contains(&format!("{} tokens", format_tokens(300))),
            "{text}"
        );
        assert!(
            landed[1].contains(&format!("{} tokens", format_tokens(110))),
            "{text}"
        );
    }

    /// A mission with steps always lands, even when nothing was used.
    #[test]
    fn a_mission_with_steps_lands_even_when_nothing_was_used() {
        let mut t = fresh_transcript();
        t.begin_mission("do it", None, ToolOutputView::Peek);
        t.chains.push(chain_with(1));
        assert!(t.land_mission(1, 0, 0, 0.0, 0.5));
        let text = chat_text(&t.chat);
        assert!(text.contains("LANDED"), "{text}");
        assert!(text.contains("1 step"), "{text}");
        assert!(text.contains("0 tokens"), "{text}");
        assert!(text.contains("$0.000"), "{text}");
    }

    /// A stepless mission has no landing line.
    #[test]
    fn a_stepless_mission_has_no_landing_line() {
        let mut t = fresh_transcript();
        t.begin_mission("just talking", None, ToolOutputView::Peek);
        assert!(!t.land_mission(1, 10, 20, 0.01, 0.5));
        assert!(!chat_text(&t.chat).contains("LANDED"));
    }

    /// A removed chain pulls later mission starts down: M2 keeps its first
    /// chain and still lands with its own step, numbered 01.
    #[test]
    fn removing_a_whole_chain_keeps_the_later_missions_start() {
        let mut t = fresh_transcript();
        t.begin_mission("first", None, ToolOutputView::Peek);
        let failed = chain_with(1);
        t.chains.push(failed.clone());
        t.open_chain = Some(failed.clone());
        t.begin_mission("second", None, ToolOutputView::Peek);
        assert_eq!(t.missions[1].start, 1);
        // The failed attempt's chain leaves after M2 began.
        let block = failed.borrow().tool_blocks()[0].clone();
        t.remove_tool_block(&block);
        t.rebase_steps();
        assert!(t.chains.iter().all(|c| !Rc::ptr_eq(c, &failed)));
        assert_eq!(t.missions[1].start, 0);
        // M2 still lands with its own step, numbered 01.
        let m2 = chain_with(1);
        m2.borrow_mut().set_base_step(t.mission_tally().0);
        t.chains.push(m2.clone());
        assert!(t.land_mission(2, 10, 20, 0.001, 0.2));
        let text = chat_text(&t.chat);
        let landed: Vec<&str> = text.lines().filter(|l| l.contains("LANDED")).collect();
        assert_eq!(landed.len(), 1, "{text}");
        assert!(landed[0].contains("1 step"), "{text}");
        let rendered = strip_vt_control_characters(&m2.borrow_mut().render(100).join("\n"));
        assert!(rendered.contains(" 01 "), "{rendered}");
    }

    /// Taking a block out renumbers the later steps (01, 02 — never 01, 03).
    #[test]
    fn removing_a_block_renumbers_the_later_steps() {
        let mut t = fresh_transcript();
        t.begin_mission("m", None, ToolOutputView::Peek);
        let first = chain_with(2);
        let second = chain_with(1);
        second.borrow_mut().set_base_step(2);
        t.chains.push(first.clone());
        t.chains.push(second.clone());
        let dropped = first.borrow().tool_blocks()[0].clone();
        t.remove_tool_block(&dropped);
        // The drop path renumbers once after the loop.
        t.rebase_steps();
        let rendered = strip_vt_control_characters(&second.borrow_mut().render(100).join("\n"));
        assert!(rendered.contains(" 02 "), "{rendered}");
        assert!(!rendered.contains(" 03 "), "{rendered}");
    }

    fn result(details: Option<Value>, duration_ms: Option<u64>) -> ToolResultMessage {
        ToolResultMessage {
            tool_call_id: "call-1".into(),
            tool_name: "Shell".into(),
            content: vec![Content::text("ok")],
            details,
            is_error: false,
            timestamp: 0,
            duration_ms,
        }
    }

    #[test]
    fn rebuilt_details_carry_the_stored_duration_into_the_tool_details() {
        let details = rebuilt_details(&result(Some(json!({"truncation": null})), Some(42)));
        assert_eq!(details, json!({"truncation": null, "durationMs": 42}));
        assert_eq!(
            rebuilt_details(&result(None, Some(7))),
            json!({"durationMs": 7})
        );
    }

    #[test]
    fn rebuilt_details_without_a_duration_are_the_tools_own() {
        assert_eq!(rebuilt_details(&result(None, None)), Value::Null);
        assert_eq!(
            rebuilt_details(&result(Some(json!({"k": 1})), None)),
            json!({"k": 1})
        );
        // Non-object details cannot carry a duration and are kept as they are.
        assert_eq!(
            rebuilt_details(&result(Some(json!("text")), Some(7))),
            json!("text")
        );
    }
}

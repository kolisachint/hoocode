//! The chat transcript: rendering messages and tool blocks, streaming, the status and notice
//! rows, and the tool output views.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use hoocode_agent_types::AgentMessage;
use hoocode_ai_types::{AssistantMessage, Content, StopReason};
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
use hoocode_code_tui_widgets::skill_block::SkillBlockComponent;
use hoocode_code_tui_widgets::tool_chain::{speaking_ends_chain, ToolChainComponent};
use hoocode_code_tui_widgets::tool_chain_summary::ChainState;
use hoocode_code_tui_widgets::tool_execution::{ToolExecutionComponent, ToolExecutionOptions};
use hoocode_code_tui_widgets::tool_output_view::cycle_tool_output_view;
use hoocode_code_tui_widgets::tool_signal::ToolResult;
use hoocode_code_tui_widgets::tools::registered_tool_definition;
use hoocode_code_tui_widgets::{AssistantMessageComponent, ThinkingDisplay, UserMessageComponent};
use hoocode_tui_components::BoxComponent;
use hoocode_tui_components::{Loader, Markdown, MarkdownTheme, Spacer, Text};
use hoocode_tui_render::ComponentHandle;

use crate::notification_panel::NotificationKind;
use crate::resource_display::show_loaded_resources;

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
        }
    }

    /// The view over a cleared chat. A retry still in progress keeps its
    /// streak, so its end still reports how the streak went.
    pub(super) fn reset_view(&mut self) {
        let streak = self.retry_streak.take();
        *self = Transcript::new(self.chat.clone());
        self.retry_streak = streak;
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
                if !self.transcript.chat.borrow().children.is_empty() {
                    self.add_to_chat(as_component(&handle(Spacer::new(1))));
                }
                match parse_skill_block(&text) {
                    Some(skill) => {
                        // The message the user typed comes first; the skill it
                        // invoked is drawn under it.
                        if let Some(message) = skill.user_message {
                            let component = UserMessageComponent::with_theme(
                                &message,
                                (self.markdown_theme())(),
                            );
                            self.add_to_chat(as_component(&handle(component)));
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
                            let component = handle(BackgroundNoticeComponent::new(notice));
                            component
                                .borrow_mut()
                                .set_expanded(self.tool_output_view == ToolOutputView::Peek);
                            self.transcript.background_notices.push(component.clone());
                            self.add_to_chat(as_component(&component));
                        }
                        None => {
                            let component =
                                UserMessageComponent::with_theme(&text, (self.markdown_theme())());
                            self.add_to_chat(as_component(&handle(component)));
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
                            );
                            continue;
                        }
                    }
                    self.add_message_to_chat(message, populate_history);
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
        let messages = self.session.messages();
        // The plan is the session's own: rebuilt from its last TodoWrite, so a
        // resumed or reloaded session shows the plan it ended with.
        hoocode_code_tools_optin::restore_plan(hoocode_code_task_store::task_store(), &messages);
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
        self.dirty.set(true);
    }

    /// `closeOpenChain`: settle the chain collecting calls.
    pub(super) fn close_open_chain(&mut self, outcome: ChainState) {
        let Some(chain) = self.transcript.open_chain.take() else {
            return;
        };
        if chain.borrow().is_empty() {
            let h = as_component(&chain);
            self.transcript.chat.borrow_mut().remove_child(&h);
            self.transcript.chains.retain(|c| !Rc::ptr_eq(c, &chain));
        } else {
            chain.borrow_mut().close(outcome);
        }
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
    pub(super) fn reset_transcript_view(&mut self) {
        self.transcript.chat.borrow_mut().clear();
        self.transcript.reset_view();
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
        let previous_thinking = self.thinking_display();
        self.tool_output_view = view;
        if persist {
            self.session.settings().set_tool_output_view(view);
        }
        self.footer.borrow_mut().set_tool_output_view(view);
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
        // Nothing was accounted: stay silent rather than print zeroes.
        if input <= 0 && output <= 0 {
            return;
        }
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
        self.add_to_chat(as_component(&handle(Spacer::new(1))));
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
    use serde_json::{json, Value};

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

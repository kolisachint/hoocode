//! `components/tool-chain.ts`: a run of consecutive tool calls, one line in
//! a collapsed radar view and a plain pass-through everywhere else.
//!
//! A chain closes when the agent next speaks or the turn settles; closing
//! while it is still the bottom of the screen keeps the rewrite to one line.

use std::cell::RefCell;
use std::rc::Rc;

use hoocode_ai_types::Content;
use hoocode_tui_glyphs::status;
use hoocode_tui_render::Component;
use hoocode_tui_util::strip_vt_control_characters as strip_line;

use hoocode_tui_ratatui::core::style::Style;
use hoocode_tui_ratatui::core::text::Span;
use hoocode_tui_ratatui::{ansi_spans, spans_width, truncate_spans, with_base_style, Palette};

use crate::assistant_message::AssistantMessageComponent;
use crate::radar::{
    agent_live_row, hit_row, live_row, satellite_row, sub_hit_row, sub_live_row, sub_waiting_row,
    waiting_row,
};
use crate::tool_chain_summary::{
    chain_phrase, chain_segments, chain_stats, ChainState, SegmentTone,
};
use crate::tool_execution::ToolExecutionComponent;
use crate::tool_output_view::ToolOutputView;
use crate::trajectory;
use crate::trajectory::two_part_row;
use hoocode_tui_ratatui::row;

pub type ToolBlock = Rc<RefCell<ToolExecutionComponent>>;

/// Whether an assistant message's content ends the open chain: visible text
/// does. Thinking never does, in either view.
pub fn speaking_ends_chain(content: &[Content]) -> bool {
    content
        .iter()
        .any(|c| matches!(c, Content::Text(t) if !t.text.trim().is_empty()))
}

/// The output lines a failed call shows under its chain line.
const FAILURE_TAIL_LINES: usize = 2;

/// A message's thinking that leads into the run: drawn just above the call
/// at index `before`, so a trace sits above the call it led to. The chain
/// does not split at it.
struct LeadIn {
    before: usize,
    component: Rc<RefCell<AssistantMessageComponent>>,
}

pub struct ToolChainComponent {
    blocks: Vec<ToolBlock>,
    lead_ins: Vec<LeadIn>,
    view: ToolOutputView,
    state: ChainState,
    /// The newest run in the transcript; radar marks it.
    latest: bool,
    /// A chain before the first mission has no flight-log row above it, so
    /// radar keeps its old summary line (M5). Chains inside a mission drop
    /// the summary: the mission row carries the stats.
    orphan: bool,
    /// The mission step number this chain starts at (M4: steps are numbered
    /// per mission, so a chain opened after prose keeps counting). The
    /// transcript sets it when the chain opens.
    base_step: usize,
}

impl ToolChainComponent {
    pub fn new(view: ToolOutputView) -> Self {
        Self {
            blocks: Vec::new(),
            lead_ins: Vec::new(),
            view,
            state: ChainState::Running,
            latest: false,
            orphan: false,
            base_step: 0,
        }
    }

    /// Whether this chain stands outside every mission (M5): radar keeps
    /// its summary line, since no flight-log row covers it.
    pub fn set_orphan(&mut self, orphan: bool) {
        self.orphan = orphan;
    }

    /// The chain's settle state (M5: the transcript reads it for the
    /// mission's flight-log mark).
    pub fn chain_state(&self) -> ChainState {
        self.state
    }

    pub fn set_base_step(&mut self, base: usize) {
        self.base_step = base;
    }

    pub fn add(&mut self, block: ToolBlock) {
        self.blocks.push(block);
    }

    /// Takes a block out of the chain (a failed attempt's calls).
    pub fn remove(&mut self, block: &ToolBlock) {
        let Some(index) = self.blocks.iter().position(|b| Rc::ptr_eq(b, block)) else {
            return;
        };
        self.blocks.remove(index);
        for lead in &mut self.lead_ins {
            if lead.before > index {
                lead.before -= 1;
            }
        }
    }

    /// Draws a message's trace above the calls added from now on.
    pub fn add_lead_in(&mut self, component: Rc<RefCell<AssistantMessageComponent>>) {
        self.lead_ins.push(LeadIn {
            before: self.blocks.len(),
            component,
        });
    }

    pub fn has_lead_in(&self, component: &Rc<RefCell<AssistantMessageComponent>>) -> bool {
        self.lead_ins
            .iter()
            .any(|l| Rc::ptr_eq(&l.component, component))
    }

    /// Gives a trace back to the caller; true when it was in this chain.
    pub fn take_lead_in(&mut self, component: &Rc<RefCell<AssistantMessageComponent>>) -> bool {
        let before = self.lead_ins.len();
        self.lead_ins
            .retain(|l| !Rc::ptr_eq(&l.component, component));
        self.lead_ins.len() != before
    }

    /// True until the agent speaks or the turn settles.
    pub fn is_open(&self) -> bool {
        self.state == ChainState::Running
    }

    /// Whether a summary line stands in for this chain's calls right now.
    pub fn is_summarised(&self) -> bool {
        self.is_collapsed()
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    pub fn tool_blocks(&self) -> &[ToolBlock] {
        &self.blocks
    }

    /// Settle the chain. An interrupted chain reads `⊘ interrupted`.
    pub fn close(&mut self, outcome: ChainState) {
        self.state = outcome;
    }

    pub fn set_view(&mut self, view: ToolOutputView) {
        self.view = view;
        for block in &self.blocks {
            block.borrow_mut().set_view(view);
        }
    }

    pub fn set_latest(&mut self, latest: bool) {
        self.latest = latest;
    }

    /// Radar summarises every orphan chain, a lone call included, so one
    /// call and a run of calls read the same
    /// (`Ran npm run check   1 call · 412 lines`). Chains inside a mission
    /// collapse into the mission's flight-log rows instead.
    fn is_collapsed(&self) -> bool {
        self.view == ToolOutputView::Radar && !self.blocks.is_empty()
    }

    /// The blank row that holds an orphan radar run off whatever came
    /// before it. Chains inside a mission attach their flight-log rows
    /// straight under the mission.
    fn needs_lead_in(&self) -> bool {
        if self.view != ToolOutputView::Radar {
            return false;
        }
        if self.is_collapsed() {
            return self.orphan;
        }
        self.blocks
            .first()
            .is_some_and(|b| !b.borrow().draws_leading_gap())
    }

    /// The peek trajectory: each block a numbered step on the `┊` rail
    /// (an Agent block keeps the violet `┃` rail), up to
    /// [`trajectory::TRAJECTORY_BODY_LINES`] body lines, then a `+N more`
    /// fold. A failed step shows the `HIT` chip in place of its success
    /// stats; an Edit step shows its `+a −b` counts and a Write step the
    /// `NEW` chip; a live background job shows its satellite row.
    fn peek_rows(&mut self, width: u16, palette: &Palette) -> Vec<String> {
        let width = width as usize;
        let mut out = Vec::new();
        for (i, block) in self.blocks.iter().enumerate() {
            for lead in self.lead_ins.iter().filter(|l| l.before == i) {
                out.extend(lead.component.borrow_mut().render(width as u16));
            }
            let step = self.base_step + i + 1;
            let agent = block.borrow().is_agent();
            let lines = block.borrow_mut().render(width as u16);
            // The header is the block's first non-blank line: blocks draw
            // their own leading gap, which stays untouched.
            let first = lines.iter().position(|l| !strip_line(l).trim().is_empty());
            let Some(first) = first else {
                out.extend(lines);
                continue;
            };
            out.extend(lines[..first].iter().cloned());
            let rest = &lines[first + 1..];
            let (prefix, content) = trajectory::split_padding(&lines[first]);
            // A failed step carries `⊗` in place of its dot, and the `HIT`
            // chip replaces its success stats (never appends to them).
            let content = if block.borrow().is_error() {
                trajectory::failure_dot(&content)
            } else {
                content
            };
            out.push(trajectory::step_header(
                &trajectory::step_number(step, palette),
                &prefix,
                &content,
                &block.borrow().peek_right(palette),
                width,
            ));
            let shown = rest.len().min(trajectory::TRAJECTORY_BODY_LINES);
            for line in &rest[..shown] {
                out.push(trajectory::rail_line(line, width, agent, palette));
            }
            if rest.len() > shown {
                out.push(trajectory::fold_line(
                    rest.len() - shown,
                    width,
                    agent,
                    palette,
                ));
            }
        }
        out
    }

    /// Whether the chain's last flight row is a hit (radar): prose written
    /// under it indents to the hit's text. Settled clean calls draw no row,
    /// so they are skipped.
    pub fn ends_in_hit(&self) -> bool {
        self.blocks
            .iter()
            .rev()
            .find_map(|block| {
                let body = block.borrow();
                if body.chain_entry().is_error {
                    Some(true)
                } else if body.is_waiting_for_clearance() || body.chain_entry().is_partial {
                    Some(false)
                } else {
                    None
                }
            })
            .unwrap_or(false)
    }

    /// The mission's flight-log rows for this chain (M5, radar): one line
    /// per hit (`⊗ 04 … HIT`, indented 12), per live step (`◐ 01 … 3s`,
    /// indented 8, with a progress bar when the call's estimate is known),
    /// per waiting step (`◉ 01 … holding for clearance`, the queue's head
    /// first), per live subagent (`◈ type · task`) with its own running and
    /// failed calls under it (`01.1`), and per live background job (its
    /// satellite row), numbered by mission step. Settled clean
    /// calls get no row: the mission row above already counts them.
    /// Lead-in traces keep their think line with its gist, so thinking
    /// that led to a call still reads under its mission.
    fn flight_rows(&self, width: usize, palette: &Palette) -> Vec<String> {
        let mut out = Vec::new();
        for component in &self.lead_ins {
            if let Some(line) = component
                .component
                .borrow()
                .radar_think_line(width, palette)
            {
                out.push(line);
            }
        }
        let mut waited = false;
        for (i, block) in self.blocks.iter().enumerate() {
            let step = self.base_step + i + 1;
            let body = block.borrow();
            let entry = body.chain_entry();
            if entry.is_error {
                let snippet = body
                    .error_text()
                    .lines()
                    .map(str::trim)
                    .find(|line| !line.is_empty())
                    .unwrap_or_default()
                    .to_string();
                out.push(hit_row(
                    step,
                    &entry.tool,
                    &entry.subject,
                    &snippet,
                    palette,
                    width,
                ));
            } else if body.is_waiting_for_clearance() {
                // The queue's head holds `◉`; steps behind it read `○ next`.
                let head = !waited;
                waited = true;
                out.push(waiting_row(
                    step,
                    &entry.tool,
                    &entry.subject,
                    head,
                    palette,
                    width,
                ));
            } else if entry.is_partial {
                let secs = body.block_elapsed_secs().unwrap_or(0);
                if let Some(job) = body.background_job_id() {
                    out.push(satellite_row(
                        step,
                        &entry.tool,
                        &entry.subject,
                        &job,
                        secs,
                        palette,
                        width,
                    ));
                } else if let Some((subtype, task)) = body.agent_label() {
                    out.push(agent_live_row(step, &subtype, &task, secs, palette, width));
                    // The agent's own calls still running or failed sit under its row.
                    if let Some(run) = body.agent_run_id() {
                        let subs = hoocode_code_subagents::live::sub_steps(&run);
                        for (n, sub) in subs.iter().enumerate() {
                            let index = n + 1;
                            match &sub.failure {
                                Some(snippet) => out.push(sub_hit_row(
                                    step,
                                    index,
                                    &sub.tool,
                                    &sub.subject,
                                    snippet,
                                    palette,
                                    width,
                                )),
                                None if sub.held => out.push(sub_waiting_row(
                                    step,
                                    index,
                                    &sub.tool,
                                    &sub.subject,
                                    palette,
                                    width,
                                )),
                                None => out.push(sub_live_row(
                                    step,
                                    index,
                                    &sub.tool,
                                    &sub.subject,
                                    sub.started.elapsed().as_secs(),
                                    palette,
                                    width,
                                )),
                            }
                        }
                    }
                } else {
                    out.push(live_row(
                        step,
                        &entry.tool,
                        &entry.subject,
                        secs,
                        body.estimated_secs(),
                        palette,
                        width,
                    ));
                }
            }
        }
        out
    }

    fn rows(&mut self, width: u16) -> Vec<String> {
        // One palette per render pass, passed down to every row.
        let palette = Palette::current();
        if !self.is_collapsed() {
            return self.peek_rows(width, &palette);
        }
        if !self.orphan {
            return self.flight_rows(width as usize, &palette);
        }
        self.orphan_rows(width as usize, &palette)
    }

    /// A chain outside every mission keeps the old summary line in radar:
    /// the state glyph, the run's labels and its stats, then one folded
    /// block per failed call. Drawn from spans like every other row.
    fn orphan_rows(&self, width: usize, palette: &Palette) -> Vec<String> {
        let entries: Vec<_> = self
            .blocks
            .iter()
            .map(|b| b.borrow().chain_entry())
            .collect();
        // What the line reads as: interrupted when the turn was cut short;
        // running while any call is still going; otherwise done, even while
        // the chain stays open for the model's next call.
        // An interrupted turn does not stop a background job: an interrupted
        // chain whose job is still live keeps reading running.
        let live_job = self
            .blocks
            .iter()
            .any(|b| b.borrow().is_live_background_job());
        let shown = if self.state == ChainState::Interrupted && !live_job {
            ChainState::Interrupted
        } else if entries.iter().any(|e| e.is_partial) {
            ChainState::Running
        } else {
            ChainState::Done
        };
        let running = shown == ChainState::Running;
        let failed = entries.iter().any(|e| e.is_error);
        let glyph = match shown {
            ChainState::Running => status::RUNNING_STR,
            ChainState::Interrupted if !failed => "⊘",
            ChainState::Interrupted | ChainState::Done => status::DONE_STR,
        };
        // The legacy tones, as the chalk rows styled them (`toolTitle` is
        // "" in the theme: chalk's default opener).
        let glyph_style = if failed {
            palette.err()
        } else {
            match shown {
                ChainState::Running => palette.style("warning"),
                ChainState::Interrupted => palette.faint(),
                ChainState::Done => palette.ok(),
            }
        };
        let segment_style = |tone: SegmentTone| match tone {
            SegmentTone::Error => palette.err(),
            SegmentTone::Running => palette.style("warning"),
            SegmentTone::Ok => Style::default(),
        };
        let left: Vec<Span<'static>> = if shown == ChainState::Done {
            ansi_spans(&chain_phrase(&entries))
        } else {
            let segments = chain_segments(&entries);
            let mut spans = Vec::new();
            for (i, segment) in segments.iter().enumerate() {
                if i > 0 {
                    spans.push(Span::styled(" › ".to_string(), palette.dim()));
                }
                spans.push(Span::styled(
                    segment.label.clone(),
                    segment_style(segment.tone),
                ));
            }
            if running {
                spans.push(Span::styled("…".to_string(), palette.dim()));
            }
            spans
        };

        let stats = Span::styled(chain_stats(&entries, shown), palette.faint());
        // The latest-row stroke, merged into the label spans' background.
        let bg = if self.latest {
            palette.bg_color("activeToolBg")
        } else {
            None
        };
        let paint_bg = |mut span: Span<'static>| {
            if let Some(bg) = bg {
                span.style.bg = Some(bg);
            }
            span
        };
        let mut prefix = vec![
            paint_bg(Span::styled(" ".to_string(), Style::default())),
            paint_bg(Span::styled(glyph.to_string(), glyph_style)),
            paint_bg(Span::styled(" ".to_string(), Style::default())),
        ];
        let mut left: Vec<Span<'static>> = left.into_iter().map(paint_bg).collect();
        let right = vec![stats];
        let budget = width.saturating_sub(spans_width(&prefix));
        let stats_width = spans_width(&right);
        // Stats whole, flush right. When the label does not fit beside them it
        // is dropped; the stats are cut only when they do not fit alone.
        let line = if spans_width(&left) + 2 + stats_width <= budget {
            prefix.append(&mut left);
            two_part_row(prefix, right, width)
        } else if stats_width <= budget {
            two_part_row(prefix, right, width)
        } else {
            // Flush left like the old row: the cut stats sit right behind
            // the glyph, not flush right.
            prefix.extend(truncate_spans(right, budget, "…"));
            row(prefix, width)
        };
        let mut out = vec![line];
        out.extend(self.failure_lines(width, palette));
        out
    }

    /// Each failed call's header and the last lines of its reason, under the
    /// chain line. A longer reason folds to its tail, with the way to see it all.
    fn failure_lines(&self, width: usize, palette: &Palette) -> Vec<String> {
        // The legacy tones, as the chalk rows styled them: the tool keeps
        // chalk's default opener (`toolTitle` is ""), the subject and reason
        // keep their styling rooted in `toolOutput`.
        let output = palette.style("toolOutput");
        let mut lines = Vec::new();
        for block in &self.blocks {
            let block = block.borrow();
            let entry = block.chain_entry();
            if !entry.is_error {
                continue;
            }
            let mut header = vec![
                Span::styled("    ".to_string(), Style::default()),
                Span::styled("✗".to_string(), palette.err()),
                Span::styled(" ".to_string(), Style::default()),
                Span::styled(entry.tool.clone(), Style::default()),
                Span::styled("  ".to_string(), Style::default()),
            ];
            header.extend(with_base_style(ansi_spans(&entry.subject), output));
            lines.push(row(header, width));
            let text = block.error_text();
            let reason: Vec<&str> = text
                .split('\n')
                .filter(|raw| !crate::is_blank(raw))
                .collect();
            let hidden = reason.len().saturating_sub(FAILURE_TAIL_LINES);
            for raw in &reason[hidden..] {
                let mut line = vec![Span::styled("      ".to_string(), Style::default())];
                line.extend(with_base_style(ansi_spans(raw), output));
                lines.push(row(line, width));
            }
            if hidden > 0 {
                let noun = if hidden == 1 { "line" } else { "lines" };
                let line = vec![
                    Span::styled("      ".to_string(), Style::default()),
                    Span::styled(
                        format!(
                            "… {hidden} more {noun} · {}",
                            hoocode_code_tui_keybindings::key_text("app.tools.expand")
                        ),
                        palette.faint(),
                    ),
                ];
                lines.push(row(line, width));
            }
        }
        lines
    }
}

impl Component for ToolChainComponent {
    fn render(&mut self, width: u16) -> Vec<String> {
        let rows = self.rows(width);
        if rows.is_empty() || !self.needs_lead_in() {
            return rows;
        }
        let mut out = Vec::with_capacity(rows.len() + 1);
        out.push(String::new());
        out.extend(rows);
        out
    }

    fn invalidate(&mut self) {
        for block in &self.blocks {
            block.borrow_mut().invalidate();
        }
        for lead in &self.lead_ins {
            lead.component.borrow_mut().invalidate();
        }
    }
}

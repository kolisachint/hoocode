//! `components/tool-chain.ts`: a run of consecutive tool calls, one line in
//! a collapsed outline view and a plain pass-through everywhere else.
//!
//! A chain closes when the agent next speaks or the turn settles; closing
//! while it is still the bottom of the screen keeps the rewrite to one line.

use std::cell::RefCell;
use std::rc::Rc;

use hoocode_ai_types::Content;
use hoocode_tui_render::{fresh_stamp, mix_stamps, Component};

use hoocode_tui_ratatui::core::style::Style;
use hoocode_tui_ratatui::core::text::Span;
use hoocode_tui_ratatui::{ansi_spans, with_base_style, Palette};

use crate::assistant_message::AssistantMessageComponent;
use crate::fold::{
    earlier_lines_text, fold_rows, fold_rows_tail, more_lines_span, more_lines_text,
};
use crate::tool_chain_summary::ChainState;
use crate::tool_execution::ToolExecutionComponent;
use crate::tool_output_view::ToolOutputView;
use crate::tool_row;
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

/// Pushes one blank row unless the rows end in one (or are empty, which still
/// gets its opening blank).
fn ensure_blank(out: &mut Vec<String>) {
    if out.last().is_none_or(|row| !tool_row::is_blank_row(row)) {
        out.push(String::new());
    }
}

/// How many lines of a body show before the fold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fold {
    /// No body at all.
    Hidden,
    /// The first lines, the hint below.
    Head(usize),
    /// The last lines, the hint above.
    Tail(usize),
}

/// Bodies this short show whole, never behind a fold.
const WHOLE_BODY_LINES: usize = 4;
/// A failed call's error: its last lines.
const FAILED_BODY_LINES: usize = 4;
/// Edit and Write at peek: their first lines.
const HEAD_BODY_LINES: usize = 4;
/// A settled shell call at peek: its last lines.
const SHELL_BODY_LINES: usize = 2;

/// The peek rule for a tool's body (spec "Transcript", tool body).
fn peek_fold(tool: &str, failed: bool, tail: bool) -> Fold {
    if failed {
        return if tail {
            Fold::Tail(FAILED_BODY_LINES)
        } else {
            Fold::Head(FAILED_BODY_LINES)
        };
    }
    match tool {
        "Read" | "CodeSearch" | "WebFetch" | "WebSearch" | "Agent" | "AgentOutput" => Fold::Hidden,
        "Shell" => Fold::Tail(SHELL_BODY_LINES),
        _ if tail => Fold::Tail(SHELL_BODY_LINES),
        _ => Fold::Head(HEAD_BODY_LINES),
    }
}

/// A body's rows after the view's fold rule. `body` is the renderer's rows
/// (already at column 4); `footer` the rows after them that are not output (a
/// truncation note), always whole.
#[allow(clippy::too_many_arguments)]
fn fold_body(
    mut body: Vec<String>,
    tail: bool,
    footer: Vec<String>,
    tool: &str,
    failed: bool,
    full: bool,
    width: usize,
    palette: &Palette,
) -> Vec<String> {
    while body.last().is_some_and(|r| tool_row::is_blank_row(r)) {
        body.pop();
    }
    while body.first().is_some_and(|r| tool_row::is_blank_row(r)) {
        body.remove(0);
    }
    if full {
        body.extend(footer);
        return body;
    }
    let fold = peek_fold(tool, failed, tail);
    let mut out = Vec::new();
    match fold {
        Fold::Hidden => {}
        Fold::Head(budget) | Fold::Tail(budget) => {
            let whole = fold_rows(body.clone(), Some(WHOLE_BODY_LINES));
            if whole.hidden == 0 {
                out.extend(whole.shown);
            } else if matches!(fold, Fold::Tail(_)) {
                let folded = fold_rows_tail(body, Some(budget));
                if folded.hidden > 0 {
                    out.push(tool_row::fold_row(
                        &earlier_lines_text(folded.hidden),
                        width,
                        palette,
                    ));
                }
                out.extend(folded.shown);
            } else {
                let folded = fold_rows(body, Some(budget));
                out.extend(folded.shown);
                if folded.hidden > 0 {
                    out.push(tool_row::fold_row(
                        &more_lines_text(folded.hidden),
                        width,
                        palette,
                    ));
                }
            }
        }
    }
    if !matches!(fold, Fold::Hidden) {
        out.extend(footer);
    }
    out
}

/// A subagent's own call under its row, indented 3: `   ⊗ Grep \\"  reason`.
/// A failed call reads in red with its reason after two spaces.
fn sub_step_rows(
    step: &hoocode_code_subagents::live::SettledStep,
    width: usize,
    palette: &Palette,
) -> Vec<String> {
    let t = hoocode_code_tui_theme::theme();
    let plain = |text: &str| hoocode_tui_util::strip_vt_control_characters(text);
    let subject = plain(&step.subject);
    match step
        .failure
        .as_deref()
        .map(plain)
        .filter(|f| !f.trim().is_empty())
    {
        Some(reason) => {
            let label = format!(
                "{} {}",
                t.fg("error", &t.bold(&step.tool)),
                t.fg("error", &subject)
            );
            let facts = vec![Span::styled(reason.trim().to_string(), palette.err())];
            tool_row::tool_rows(
                tool_row::SUB_INDENT,
                tool_row::Mark::Failed,
                &label,
                facts,
                width,
                palette,
            )
        }
        None => {
            let mark = if step.failure.is_some() {
                tool_row::Mark::Failed
            } else {
                tool_row::Mark::Done
            };
            let label = format!(
                "{} {}",
                t.fg("text", &t.bold(&step.tool)),
                t.fg("text", &subject)
            );
            tool_row::tool_rows(
                tool_row::SUB_INDENT,
                mark,
                &label,
                Vec::new(),
                width,
                palette,
            )
        }
    }
}

pub struct ToolChainComponent {
    blocks: Vec<ToolBlock>,
    lead_ins: Vec<LeadIn>,
    view: ToolOutputView,
    state: ChainState,
    /// The newest run in the transcript; outline marks it.
    latest: bool,
    /// A chain before the first mission has no flight-log row above it, so
    /// outline keeps its old summary line (M5). Chains inside a mission drop
    /// the summary: the mission row carries the stats.
    orphan: bool,
    /// New whenever one of the fields above changes (the render stamp).
    rev: u64,
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
            rev: fresh_stamp(),
        }
    }

    /// Whether this chain stands outside every mission (M5): outline keeps
    /// its summary line, since no flight-log row covers it.
    pub fn set_orphan(&mut self, orphan: bool) {
        self.rev = fresh_stamp();
        self.orphan = orphan;
    }

    /// The chain's settle state (M5: the transcript reads it for the
    /// mission's flight-log mark).
    pub fn chain_state(&self) -> ChainState {
        self.state
    }

    pub fn add(&mut self, block: ToolBlock) {
        self.rev = fresh_stamp();
        self.blocks.push(block);
    }

    /// Takes a block out of the chain (a failed attempt's calls).
    pub fn remove(&mut self, block: &ToolBlock) {
        let Some(index) = self.blocks.iter().position(|b| Rc::ptr_eq(b, block)) else {
            return;
        };
        self.rev = fresh_stamp();
        self.blocks.remove(index);
        for lead in &mut self.lead_ins {
            if lead.before > index {
                lead.before -= 1;
            }
        }
    }

    /// Draws a message's trace above the calls added from now on.
    pub fn add_lead_in(&mut self, component: Rc<RefCell<AssistantMessageComponent>>) {
        self.rev = fresh_stamp();
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
        self.rev = fresh_stamp();
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
        self.rev = fresh_stamp();
        self.state = outcome;
    }

    pub fn set_view(&mut self, view: ToolOutputView) {
        self.rev = fresh_stamp();
        self.view = view;
        for block in &self.blocks {
            block.borrow_mut().set_view(view);
        }
    }

    pub fn set_latest(&mut self, latest: bool) {
        self.rev = fresh_stamp();
        self.latest = latest;
    }

    /// Outline summarises every orphan chain, a lone call included, so one
    /// call and a run of calls read the same
    /// (`Ran npm run check   1 call · 412 lines`). Chains inside a mission
    /// collapse into the mission's flight-log rows instead.
    fn is_collapsed(&self) -> bool {
        self.view == ToolOutputView::Outline && !self.blocks.is_empty()
    }

    /// The blank row that holds an orphan outline run off whatever came
    /// before it. Chains inside a mission attach their flight-log rows
    /// straight under the mission.
    fn needs_lead_in(&self) -> bool {
        if self.view != ToolOutputView::Outline {
            return false;
        }
        if self.is_collapsed() {
            return self.orphan;
        }
        self.blocks
            .first()
            .is_some_and(|b| !b.borrow().draws_leading_gap())
    }

    /// The peek and full rows: each block a tool row (` ● Name subject  facts`)
    /// with its body under it at column 4, dim, and no rail.
    ///
    /// Consecutive body-less rows stay tight; a row that shows a body is
    /// followed by one blank row, and one blank row opens the run. Peek shows
    /// no body for Read, search and web calls or for a settled agent; an Edit
    /// or Write its first four lines, a Shell call its last two (a failed
    /// call its last four, any tool's error likewise); a body of four lines or
    /// fewer shows whole. Every fold counts all the lines left out, the ones
    /// the renderer already cut included. The full stop shows every line and
    /// no fold. A subagent's own calls sit under its row, indented 3: the
    /// failed ones at peek, every one at full.
    fn peek_rows(&mut self, width: u16, palette: &Palette) -> Vec<String> {
        let w = width as usize;
        let full = self.view == ToolOutputView::Full;
        let mut out: Vec<String> = Vec::new();
        let mut spaced = true;
        for (i, block) in self.blocks.iter().enumerate() {
            for lead in self.lead_ins.iter().filter(|l| l.before == i) {
                out.extend(lead.component.borrow_mut().render(width));
            }
            let parts = block.borrow_mut().step_parts(width);
            let Some(header) = parts.header else {
                out.extend(parts.images);
                continue;
            };
            if i == 0 || !spaced {
                ensure_blank(&mut out);
            }
            let (mark, facts, tool, failed) = {
                let b = block.borrow();
                (
                    b.mark(),
                    b.facts(palette),
                    b.tool_name().to_string(),
                    b.is_error(),
                )
            };
            out.extend(tool_row::tool_rows(0, mark, &header, facts, w, palette));

            let mut shown_body = false;
            if tool == "Agent" {
                for sub in block.borrow().settled_sub_steps() {
                    if full || sub.failure.is_some() {
                        out.extend(sub_step_rows(&sub, w, palette));
                        shown_body = true;
                    }
                }
            }
            let body = fold_body(
                parts.body,
                parts.tail,
                parts.footer,
                &tool,
                failed,
                full,
                w,
                palette,
            );
            shown_body |= !body.is_empty();
            out.extend(body);
            shown_body |= !parts.images.is_empty();
            out.extend(parts.images);
            spaced = !shown_body;
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
            // A mission's calls are drawn by its header (outline, R3).
            return Vec::new();
        }
        self.orphan_rows(width as usize, &palette)
    }

    /// A chain outside every turn summary reads as one recap row in outline
    /// (` ● Read x · Edit y`), the same row the turn recaps use. Calls that
    /// failed keep their folded reason rows under it.
    fn orphan_rows(&self, width: usize, palette: &Palette) -> Vec<String> {
        let mut out = Vec::new();
        out.extend(crate::outline::recap_row(
            &self.blocks,
            true,
            palette,
            width,
        ));
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
                let line = vec![
                    Span::styled("      ".to_string(), Style::default()),
                    more_lines_span(hidden, palette),
                    Span::styled(
                        format!(
                            " · {}",
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

    /// A chain that has settled, whose calls are all frozen blocks and whose
    /// traces are settled messages, draws from its own fields and theirs. One
    /// still collecting calls shows live clocks and the job registry: no stamp.
    fn render_stamp(&self) -> Option<u64> {
        if self.state == ChainState::Running {
            return None;
        }
        let mut parts = vec![self.rev, self.blocks.len() as u64];
        for block in &self.blocks {
            parts.push(block.try_borrow().ok()?.render_stamp()?);
        }
        for lead in &self.lead_ins {
            parts.push(lead.before as u64);
            parts.push(lead.component.try_borrow().ok()?.render_stamp()?);
        }
        Some(mix_stamps(parts))
    }

    fn invalidate(&mut self) {
        self.rev = fresh_stamp();
        for block in &self.blocks {
            block.borrow_mut().invalidate();
        }
        for lead in &self.lead_ins {
            lead.component.borrow_mut().invalidate();
        }
    }
}

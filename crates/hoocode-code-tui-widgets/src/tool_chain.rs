//! `components/tool-chain.ts`: a run of consecutive tool calls, one row per
//! call, a blank row apart. A collapsed block is its header row; the toggle
//! opens the body whole.
//!
//! A chain closes when the agent next speaks or the turn settles; closing
//! while it is still the bottom of the screen keeps the rewrite to one line.

use std::cell::RefCell;
use std::rc::Rc;

use hoocode_ai_types::Content;
use hoocode_tui_render::{fresh_stamp, mix_stamps, Component, Fold, FoldHost};

use hoocode_tui_ratatui::core::text::Span;
use hoocode_tui_ratatui::Palette;

use crate::assistant_message::AssistantMessageComponent;
use crate::fold::{self, gutter_span, paint_focused};
use crate::tool_execution::ToolExecutionComponent;
use crate::tool_row;
use hoocode_tui_ratatui::row;

pub type ToolBlock = Rc<RefCell<ToolExecutionComponent>>;

/// Whether a chain still takes calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainState {
    /// Calls are still arriving or executing.
    Running,
    /// Every call finished and the turn moved on cleanly.
    Done,
    /// The turn was aborted, errored, or hit a length cap partway through.
    Interrupted,
}

/// Whether an assistant message's content ends the open chain: visible text
/// does. Thinking never does, in either view.
pub fn speaking_ends_chain(content: &[Content]) -> bool {
    content
        .iter()
        .any(|c| matches!(c, Content::Text(t) if !t.text.trim().is_empty()))
}

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

/// A block's body rows, whole: blank edges trimmed, then the footer rows
/// that are not output (a truncation note). The toggle opens the body in
/// place, so nothing folds.
fn whole_body(mut body: Vec<String>, footer: Vec<String>) -> Vec<String> {
    while body.last().is_some_and(|r| tool_row::is_blank_row(r)) {
        body.pop();
    }
    while body.first().is_some_and(|r| tool_row::is_blank_row(r)) {
        body.remove(0);
    }
    body.extend(footer);
    body
}

/// A Shell exit trailer's code: only the exact line `Command exited with
/// code 1` reads as the row's red `exit 1` fact. Anything after the code
/// (a signal note, more output) keeps the text as the dim reason instead.
fn shell_trailer_exit(text: &str) -> Option<i32> {
    text.trim()
        .strip_prefix("Command exited with code ")?
        .trim()
        .parse()
        .ok()
}

/// A subagent's own call under its row, indented 3: `   ● Read path`,
/// `   ⊗ Shell cmd  exit 1 · reason`. One row per call through the header
/// cut order: the target shortens, then a failed call's reason cuts at a
/// word (12+ columns) or drops; `exit N` never drops. Red only on the mark
/// and the exit; the reason is dim.
fn sub_step_rows(
    step: &hoocode_code_subagents::live::SettledStep,
    width: usize,
    palette: &Palette,
) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let plain = |text: &str| hoocode_tui_util::strip_vt_control_characters(text);
    let target = plain(&step.subject);
    let reason = step
        .failure
        .as_deref()
        .map(plain)
        .filter(|f| !f.trim().is_empty());
    // A failed Shell with no output names only its exit trailer; the row
    // grammar reads that as the red exit fact, with no dim reason after it.
    let (primary, reason) = match reason {
        Some(text) if step.tool == "Shell" => match shell_trailer_exit(&text) {
            Some(code) => (vec![format!("exit {code}")], None),
            None => (Vec::new(), Some(text)),
        },
        reason => (Vec::new(), reason),
    };
    let primaries: Vec<&str> = primary.iter().map(String::as_str).collect();
    let mut spans = vec![Span::raw(" ".repeat(tool_row::SUB_INDENT))];
    spans.extend(tool_row::header_spans(
        if step.failure.is_some() {
            tool_row::Mark::Failed
        } else {
            tool_row::Mark::Done
        },
        &step.tool,
        target.trim(),
        &primaries,
        &[],
        reason.as_deref(),
        width.saturating_sub(tool_row::SUB_INDENT),
        palette,
    ));
    vec![row(spans, width)]
}

/// One header row in the chain's last render: its first row and how many
/// rows the header spans, with the block it toggles.
#[derive(Debug, Clone, Copy)]
pub struct HeaderHit {
    /// First row of the header within the chain's rendered rows.
    pub start: usize,
    /// How many rows the header spans (wraps at narrow widths).
    pub len: usize,
    /// The block the header toggles.
    pub block: usize,
}

pub struct ToolChainComponent {
    blocks: Vec<ToolBlock>,
    lead_ins: Vec<LeadIn>,
    /// The header rows of the last render, for click-to-toggle.
    header_map: Vec<HeaderHit>,
    /// Every block of the last render that opens in place, in row order:
    /// the tool rows, and the thinking traces drawn above them.
    folds: Vec<Fold>,
    /// The focused tool call's id, if the transcript has one in this chain:
    /// its header draws the `›` gutter and the paneSel fill. Synced from
    /// the transcript focus after every key (see `mode/transcript/focus.rs`).
    focused_call: Option<String>,
    state: ChainState,
    /// New whenever one of the fields above changes (the render stamp).
    rev: u64,
}

impl Default for ToolChainComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolChainComponent {
    pub fn new() -> Self {
        Self {
            blocks: Vec::new(),
            lead_ins: Vec::new(),
            header_map: Vec::new(),
            folds: Vec::new(),
            focused_call: None,
            state: ChainState::Running,
            rev: fresh_stamp(),
        }
    }

    /// The chain's settle state (the transcript reads it for the mission's
    /// flight-log mark).
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

    /// Flip a block's toggle by its position in the chain. A click the
    /// hit-test placed calls this; an unknown index does nothing.
    pub fn toggle_block(&mut self, index: usize) {
        if let Some(block) = self.blocks.get(index) {
            block.borrow_mut().toggle_expanded();
        }
    }

    /// Which tool call draws focused in this chain (`None` clears it). Only
    /// bumps the render stamp on a change.
    pub fn set_focused_call(&mut self, call_id: Option<&str>) {
        let next = call_id.map(str::to_string);
        if self.focused_call != next {
            self.focused_call = next;
            self.rev = fresh_stamp();
        }
    }

    /// The tool call drawing focused in this chain, if any.
    pub fn focused_call(&self) -> Option<&str> {
        self.focused_call.as_deref()
    }

    /// A block's header start row in the last render, for pinning a click or
    /// a focus move on the header's row.
    pub fn header_start_row(&self, index: usize) -> Option<usize> {
        self.header_map
            .iter()
            .find(|hit| hit.block == index)
            .map(|hit| hit.start)
    }

    /// The block toggled by a click on `row` (a row of the last render),
    /// or `None` on a body row. Only header rows toggle.
    pub fn header_hit(&self, row: usize) -> Option<usize> {
        self.header_map
            .iter()
            .find(|hit| row >= hit.start && row < hit.start + hit.len)
            .map(|hit| hit.block)
    }

    /// The chain's rows: each block a tool row (` ● Name subject  facts`)
    /// with its body under it at column 4, dim, and no rail.
    ///
    /// Consecutive body-less rows stay tight; a row that shows a body is
    /// followed by one blank row, and one blank row opens the run. A
    /// collapsed block is its header row. The toggle opens the body whole.
    /// A subagent's own calls sit under its row, indented 3, when it is
    /// open.
    fn body_rows(&mut self, width: u16, palette: &Palette) -> Vec<String> {
        let w = width as usize;
        let mut out: Vec<String> = Vec::new();
        let mut header_map = Vec::new();
        let mut folds = Vec::new();
        let mut spaced = true;
        for (i, block) in self.blocks.iter().enumerate() {
            for lead in self.lead_ins.iter().filter(|l| l.before == i) {
                let at = out.len();
                out.extend(lead.component.borrow_mut().render(width));
                // A trace above the call opens in place too: its rows are
                // the chain's rows from `at`.
                if let Some(host) = lead.component.borrow().fold_host() {
                    folds.extend(host.folds().into_iter().map(|mut fold| {
                        for row in &mut fold.header {
                            *row += at;
                        }
                        fold.fold_line = fold.fold_line.map(|row| row + at);
                        fold
                    }));
                }
            }
            let parts = block.borrow_mut().step_parts(width);
            let Some(_) = parts.header else {
                out.extend(parts.images);
                continue;
            };
            if i == 0 || !spaced {
                ensure_blank(&mut out);
            }
            let (mark, tool, target, primary, secondary, reason, open, call_id) = {
                let b = block.borrow();
                let (target, primary, secondary, reason) = b.header_parts();
                (
                    b.mark(),
                    b.tool_name().to_string(),
                    target,
                    primary,
                    secondary,
                    reason,
                    b.is_expanded(),
                    b.tool_call_id().to_string(),
                )
            };
            // The focused header carries the row grammar's selection: `›`
            // in accent at column 0 and the paneSel fill across the row
            // (reverse video with no colour). Expanded and focused compose:
            // the body still opens under a focused header.
            let focused = self.focused_call.as_deref() == Some(call_id.as_str());
            // A row that can open ends in the indicator (`▸` closed, `▾`
            // open). Its two cells come off the header's room first, so it
            // never drops; a row that cannot open draws none.
            let can_open = block.borrow_mut().can_open(width, &parts);
            let reserve = if can_open { fold::INDICATOR_CELLS } else { 0 };
            // Single view: the header is one row, never wrapped. Its facts
            // cut through `header_spans` (secondary drops first, then the
            // target, then the reason); the gutter keeps column 0.
            let start = out.len();
            {
                let primaries: Vec<&str> = primary.iter().map(String::as_str).collect();
                let secondaries: Vec<&str> = secondary.iter().map(String::as_str).collect();
                let room = w.saturating_sub(1 + reserve).max(1);
                let mut spans = vec![gutter_span(focused, palette)];
                // Facts that cannot cut any further are cut here, so the
                // indicator never drops off the row's end.
                spans.extend(hoocode_tui_ratatui::truncate_spans(
                    tool_row::header_spans(
                        mark,
                        &tool,
                        &target,
                        &primaries,
                        &secondaries,
                        reason.as_deref(),
                        room,
                        palette,
                    ),
                    room,
                    fold::ELLIPSIS,
                ));
                if can_open {
                    spans.push(fold::indicator_span(open, palette));
                }
                if focused {
                    // The fill runs to the row's end, as a pane's selected
                    // row does.
                    let used = hoocode_tui_ratatui::spans_width(&spans);
                    if used < w {
                        spans.push(Span::raw(" ".repeat(w - used)));
                    }
                    paint_focused(&mut spans, palette);
                }
                out.push(row(spans, w));
            }
            header_map.push(HeaderHit {
                start,
                len: 1,
                block: i,
            });

            // A collapsed block is its header row. The body (and an Agent
            // call's child rows) opens on the toggle.
            let mut shown_body = false;
            if open && tool == "Agent" {
                for sub in block.borrow().settled_sub_steps() {
                    out.extend(sub_step_rows(&sub, w, palette));
                    shown_body = true;
                }
            }
            // A frozen block's capture holds the open body; shut, the
            // snapshot's header row is the whole row, as for a live block.
            let captured = if !open && block.borrow().is_frozen() {
                Vec::new()
            } else {
                parts.body
            };
            let body = whole_body(captured, parts.footer);
            shown_body |= !body.is_empty();
            // The fold line of a partly shown body opens the block too.
            let fold_line = if open {
                None
            } else {
                body.iter()
                    .position(|row| fold::is_fold_line(row))
                    .map(|at| out.len() + at)
            };
            folds.push(Fold {
                id: call_id,
                label: tool.clone(),
                header: vec![start],
                fold_line,
                expanded: open,
                can_open,
            });
            out.extend(body);
            shown_body |= !parts.images.is_empty();
            out.extend(parts.images);
            spaced = !shown_body;
        }
        self.header_map = header_map;
        self.folds = folds;
        out
    }

    fn rows(&mut self, width: u16) -> Vec<String> {
        // One palette per render pass, passed down to every row.
        let palette = Palette::current();
        self.header_map.clear();
        self.folds.clear();
        self.body_rows(width, &palette)
    }
}

impl Component for ToolChainComponent {
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn fold_host(&self) -> Option<&dyn FoldHost> {
        Some(self)
    }

    fn fold_host_mut(&mut self) -> Option<&mut dyn FoldHost> {
        Some(self)
    }

    fn render(&mut self, width: u16) -> Vec<String> {
        self.rows(width)
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

/// The chain's blocks that open in place: one per tool call, and the
/// thinking traces it draws above them. A tool call's id is its call id.
impl FoldHost for ToolChainComponent {
    fn folds(&self) -> Vec<Fold> {
        self.folds.clone()
    }

    fn set_fold_expanded(&mut self, id: &str, expanded: bool) -> bool {
        if let Some(block) = self
            .blocks
            .iter()
            .find(|block| block.borrow().tool_call_id() == id)
        {
            block.borrow_mut().set_expanded(expanded);
            self.rev = fresh_stamp();
            return true;
        }
        for lead in &self.lead_ins {
            let mut component = lead.component.borrow_mut();
            if let Some(host) = component.fold_host_mut() {
                if host.set_fold_expanded(id, expanded) {
                    self.rev = fresh_stamp();
                    return true;
                }
            }
        }
        false
    }

    fn set_fold_focus(&mut self, id: Option<&str>) {
        self.set_focused_call(id);
        for lead in &self.lead_ins {
            if let Some(host) = lead.component.borrow_mut().fold_host_mut() {
                host.set_fold_focus(id);
            }
        }
    }

    /// A tool call's raw output is the session's (a frozen call released
    /// its copy), so only a trace answers here.
    fn fold_text(&self, id: &str) -> Option<String> {
        self.lead_ins.iter().find_map(|lead| {
            lead.component
                .borrow()
                .fold_host()
                .and_then(|host| host.fold_text(id))
        })
    }
}

#[cfg(test)]
mod shell_trailer_tests {
    use super::*;

    #[test]
    fn only_the_exact_trailer_converts_to_an_exit_fact() {
        assert_eq!(shell_trailer_exit("Command exited with code 1"), Some(1));
        assert_eq!(
            shell_trailer_exit("  Command exited with code 127  "),
            Some(127)
        );
        // A suffixed trailer is a reason, not an exit fact.
        assert_eq!(
            shell_trailer_exit("Command exited with code 1 (signal: killed)"),
            None
        );
        assert_eq!(
            shell_trailer_exit("Command exited with code 1\nmore output"),
            None
        );
        assert_eq!(shell_trailer_exit("exit 1"), None);
        assert_eq!(shell_trailer_exit("Command exited with code"), None);
    }
}

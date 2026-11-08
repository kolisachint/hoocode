//! `components/tool-chain.ts`: a run of consecutive tool calls, one row per
//! call, a blank row apart. A collapsed block is its header row; the toggle
//! opens the body whole.
//!
//! A chain closes when the agent next speaks or the turn settles; closing
//! while it is still the bottom of the screen keeps the rewrite to one line.

use std::cell::RefCell;
use std::rc::Rc;

use hoocode_ai_types::Content;
use hoocode_tui_render::{fresh_stamp, mix_stamps, Component};

use hoocode_tui_ratatui::core::text::Span;
use hoocode_tui_ratatui::{Palette, Style};

use crate::assistant_message::AssistantMessageComponent;
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

/// Whether rows paint in monochrome (no colour): the focused fill is
/// reverse video instead of paneSel.
fn mono() -> bool {
    !hoocode_code_tui_theme::chalk::enabled()
}

/// The header's gutter cell: `›` in accent on the focused row (bold with
/// no colour), else the blank that keeps column 0.
fn gutter_span(focused: bool, palette: &Palette) -> Span<'static> {
    if !focused {
        return Span::raw(" ".to_string());
    }
    let mut style = palette.acc();
    if mono() {
        style = hoocode_tui_ratatui::bold(style);
    }
    Span::styled("›".to_string(), style)
}

/// The focused row's band across every span of the header: paneSel with
/// colour, reverse video without.
fn paint_focused(spans: &mut [Span<'static>], palette: &Palette) {
    let fill = if mono() {
        hoocode_tui_ratatui::reversed(Style::default())
    } else {
        palette.bg("paneSel")
    };
    for span in spans.iter_mut() {
        span.style = span.style.patch(fill);
    }
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
        let mut spaced = true;
        for (i, block) in self.blocks.iter().enumerate() {
            for lead in self.lead_ins.iter().filter(|l| l.before == i) {
                out.extend(lead.component.borrow_mut().render(width));
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
            // Single view: the header is one row, never wrapped. Its facts
            // cut through `header_spans` (secondary drops first, then the
            // target, then the reason); the gutter keeps column 0.
            let start = out.len();
            {
                let primaries: Vec<&str> = primary.iter().map(String::as_str).collect();
                let secondaries: Vec<&str> = secondary.iter().map(String::as_str).collect();
                let mut spans = vec![gutter_span(focused, palette)];
                spans.extend(tool_row::header_spans(
                    mark,
                    &tool,
                    &target,
                    &primaries,
                    &secondaries,
                    reason.as_deref(),
                    w.saturating_sub(1),
                    palette,
                ));
                if focused {
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
            out.extend(body);
            shown_body |= !parts.images.is_empty();
            out.extend(parts.images);
            spaced = !shown_body;
        }
        self.header_map = header_map;
        out
    }

    fn rows(&mut self, width: u16) -> Vec<String> {
        // One palette per render pass, passed down to every row.
        let palette = Palette::current();
        self.header_map.clear();
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

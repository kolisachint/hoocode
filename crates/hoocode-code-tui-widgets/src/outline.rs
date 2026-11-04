//! The outline view's rows (declutter spec "Transcript", "Outline view").
//!
//! A turn in outline is drawn by its header from the turn's [`FlightStats`]:
//! the user message, then one recap row per batch of calls listing what they
//! touched (` ● Read src/config.rs · Edit src/config.rs · Shell cargo test`),
//! a failed call on its own red row under its batch (` ⊗ Grep \\"  regex parse
//! error`; a subagent's failed call indented 3), the final answer, and the
//! failed or stopped end line. Steps, bodies and thinking are hidden. A
//! successful turn has no end line: its result is in the status line.
//!
//! Prose outside every turn keeps its own rows (`prose_rows`).

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

use hoocode_code_subagents::live;
use hoocode_code_tui_theme::get_markdown_theme;
use hoocode_tui_components::Markdown;
use hoocode_tui_glyphs::status;
use hoocode_tui_ratatui::core::style::Style;
use hoocode_tui_ratatui::core::text::Span;
use hoocode_tui_ratatui::{bold, row, truncate_spans, Palette};
use hoocode_tui_render::{mix_stamps, Component, ComponentHandle};
use hoocode_tui_util::{strip_vt_control_characters, visible_width};

use crate::tool_chain::ToolBlock;
use crate::tool_chain_summary::{brief_subject, ChainEntry};
use crate::tool_row::{self, Mark};

/// Indent of prose outside every turn (the orphan rows): the prose column.
pub const STEP_INDENT: usize = 1;

/// How a turn stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FlightMark {
    /// The turn finished.
    Landed,
    /// The turn is still running.
    #[default]
    InFlight,
    /// The turn ended with failed calls, or failed itself.
    DidNotLand,
    /// The user stopped the turn.
    Aborted,
}

/// The end line a turn shows: none on success, else failed or stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnEnd {
    /// The turn itself failed: ` ⊗ failed · 5.0s` (`⊗` and `failed` red).
    Failed,
    /// The user stopped the turn: dim ` ⊗ stopped · 2.0s`.
    Stopped,
}

/// One item of a turn's outline, in story order.
#[derive(Clone)]
pub enum FlightItem {
    /// The tool calls between two thinking blocks or two assistant messages.
    Batch(Vec<ToolBlock>),
    /// A thinking block: its gist (its first line of text).
    Thinking(String),
}

impl std::fmt::Debug for FlightItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FlightItem::Batch(blocks) => f.debug_tuple("Batch").field(&blocks.len()).finish(),
            FlightItem::Thinking(gist) => f.debug_tuple("Thinking").field(gist).finish(),
        }
    }
}

/// One turn's outline data. The transcript refreshes it as the turn runs
/// (items, steps, hits, the mark) and ends (time, cost, tokens, the answer).
/// Its batches read their calls on every render, so the rows stay live
/// without a refresh.
#[derive(Debug, Clone, Default)]
pub struct FlightStats {
    pub mark: FlightMark,
    pub steps: usize,
    pub hits: usize,
    /// Landed time, already formatted (`0.8s`); `None` while flying.
    pub landed_time: Option<String>,
    /// When the flying turn started.
    pub started: Option<Instant>,
    /// Cost, already formatted (`$0.003`); `None` when unknown.
    pub cost: Option<String>,
    /// Tokens, already formatted (`2.1k tokens`); `None` while flying.
    pub tokens: Option<String>,
    /// The turn's batches and thinking blocks, in story order.
    pub items: Vec<FlightItem>,
    /// The final answer's Markdown, one source per text block; empty until
    /// the turn has ended.
    pub answer: Vec<String>,
    /// Why the turn failed: the error it ended with, without `Error: `.
    pub reason: Option<String>,
    /// The final error of a turn that failed, set when its auto-retries ran out
    /// or the turn itself failed. It outranks `reason`, which the message that
    /// carried it may no longer hold (a retried attempt leaves the chat).
    pub failure: Option<String>,
    /// An older finished turn: its recap folds to one row.
    pub folded: bool,
}

impl FlightStats {
    /// Every call of the turn, in order.
    pub fn calls(&self) -> Vec<ChainEntry> {
        self.items
            .iter()
            .flat_map(|item| match item {
                FlightItem::Batch(blocks) => batch_entries(blocks),
                FlightItem::Thinking(_) => Vec::new(),
            })
            .collect()
    }

    /// A key for what the rows read, so a holder can tell when they change.
    /// `None` while anything reads a live clock or live state: a flying
    /// turn, a call still running or held, a live background job (read off
    /// the task store), or a call whose state cannot be read right now. The
    /// key hashes only settled calls' stamps and flags and the turn's own
    /// numbers, so no output is cloned per frame. A frozen call keys on its
    /// revision: its capture is fixed, and it has no result left to read.
    pub fn render_key(&self) -> Option<u64> {
        // A flying turn reads its clock and its awaited call on every
        // render. That holds with or without a turn anchor (`started`).
        if self.mark == FlightMark::InFlight
            || (self.started.is_some() && self.landed_time.is_none())
        {
            return None;
        }
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        format!("{:?}", self.mark).hash(&mut hasher);
        self.steps.hash(&mut hasher);
        self.hits.hash(&mut hasher);
        self.landed_time.hash(&mut hasher);
        self.cost.hash(&mut hasher);
        self.tokens.hash(&mut hasher);
        self.answer.hash(&mut hasher);
        self.reason.hash(&mut hasher);
        self.failure.hash(&mut hasher);
        self.folded.hash(&mut hasher);
        self.items.len().hash(&mut hasher);
        for item in &self.items {
            match item {
                FlightItem::Batch(blocks) => {
                    0u8.hash(&mut hasher);
                    blocks.len().hash(&mut hasher);
                    for block in blocks {
                        let body = block.try_borrow().ok()?;
                        if let Some(rev) = body.frozen_rev() {
                            // An agent's rows read the subagent registry live.
                            if body.is_agent() {
                                return None;
                            }
                            2u8.hash(&mut hasher);
                            rev.hash(&mut hasher);
                            continue;
                        }
                        if !body.is_settled() || body.background_job_id().is_some() {
                            return None;
                        }
                        body.render_stamp()?.hash(&mut hasher);
                        body.is_error().hash(&mut hasher);
                        body.is_waiting_for_clearance().hash(&mut hasher);
                    }
                }
                FlightItem::Thinking(gist) => {
                    1u8.hash(&mut hasher);
                    gist.hash(&mut hasher);
                }
            }
        }
        Some(hasher.finish())
    }

    /// Why a failed turn failed: the turn's failure, else the reason
    /// its message carried. One line, trimmed.
    pub fn did_not_land_reason(&self) -> Option<String> {
        self.failure
            .as_deref()
            .and_then(first_line)
            .or_else(|| self.reason.as_deref().and_then(first_line))
    }
}

/// The first non-empty line of `text`, trimmed: a reason reads on one line.
/// `None` when the text is blank.
pub fn first_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

/// The chain entries of one batch's calls, in order.
fn batch_entries(blocks: &[ToolBlock]) -> Vec<ChainEntry> {
    blocks
        .iter()
        .map(|block| block.borrow().chain_entry())
        .collect()
}

/// The end line a turn shows, if any: a stopped turn, or a failed turn (its
/// message ended in an error, or its retries gave up). A turn that only had
/// failed calls finished: the status line counts them.
pub fn turn_end(stats: &FlightStats) -> Option<TurnEnd> {
    match stats.mark {
        FlightMark::Aborted => Some(TurnEnd::Stopped),
        FlightMark::DidNotLand if stats.failure.is_some() || stats.reason.is_some() => {
            Some(TurnEnd::Failed)
        }
        _ => None,
    }
}

/// The end line's spans: ` ⊗ failed · 5.0s` (`⊗` and `failed` red, the rest
/// dim) or ` ⊗ stopped · 2.0s` (all dim). Tokens and cost are the status
/// line's.
pub fn turn_end_spans(end: TurnEnd, time: Option<&str>, palette: &Palette) -> Vec<Span<'static>> {
    let (word, style) = match end {
        TurnEnd::Failed => ("failed", palette.err()),
        TurnEnd::Stopped => ("stopped", palette.dim()),
    };
    let mut spans = vec![
        blank(1),
        span(status::FAILED_STR, style),
        blank(1),
        span(word, style),
    ];
    if let Some(time) = time.filter(|t| !t.is_empty()) {
        spans.push(span(format!(" · {time}"), palette.dim()));
    }
    spans
}

/// The end of a turn as rows: a blank row, then the end line. Empty for a
/// turn that finished or still runs.
pub fn turn_end_rows(stats: &FlightStats, palette: &Palette, width: usize) -> Vec<String> {
    let Some(end) = turn_end(stats) else {
        return Vec::new();
    };
    let spans = turn_end_spans(end, stats.landed_time.as_deref(), palette);
    vec![String::new(), row(spans, width.max(1))]
}

/// A styled run of text.
fn span(text: impl Into<String>, style: Style) -> Span<'static> {
    Span::styled(text.into(), style)
}

/// `n` unstyled blank cells.
fn blank(n: usize) -> Span<'static> {
    Span::styled(" ".repeat(n), Style::default())
}

/// What a call reads as in the recap: `Read src/config.rs`, `Shell cargo
/// test`, `Agent explore · Trace escaped quotes`. Plain text.
fn call_subject(block: &ToolBlock) -> (String, String) {
    let body = block.borrow();
    let entry = body.chain_entry();
    let subject = match body.agent_label() {
        Some((subtype, task)) => format!("{subtype} · {task}"),
        None => brief_subject(&entry),
    };
    let subject = strip_vt_control_characters(&subject)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (entry.tool, subject)
}

/// Whether a call failed itself (its own row shows it, under its batch).
fn failed_itself(block: &ToolBlock) -> bool {
    block.borrow().chain_entry().is_error
}

/// The glyph of a batch's recap row: dim `⊗` when the user stopped a call,
/// dim `●` while a call runs, red `⊗` for a folded turn whose calls failed,
/// else green `●`.
fn batch_mark(blocks: &[ToolBlock], folded: bool) -> Mark {
    let entries = batch_entries(blocks);
    if blocks.iter().any(|block| block.borrow().is_interrupted()) {
        Mark::Stopped
    } else if entries.iter().any(|entry| entry.is_partial) {
        Mark::Running
    } else if folded && entries.iter().any(|entry| entry.is_error) {
        Mark::Failed
    } else {
        Mark::Done
    }
}

/// One batch's recap row: ` ● Read src/config.rs · Edit src/config.rs ·
/// Shell cargo test`, each tool name bold, cut with `…` at the width. The
/// calls that failed have rows of their own and are left out (a folded
/// turn keeps them: it has no other rows). `None` when no call is left.
pub(crate) fn recap_row(
    blocks: &[ToolBlock],
    folded: bool,
    palette: &Palette,
    width: usize,
) -> Option<String> {
    let calls: Vec<(String, String)> = blocks
        .iter()
        .filter(|block| folded || !failed_itself(block))
        .map(call_subject)
        .collect();
    if calls.is_empty() {
        return None;
    }
    let mut spans = vec![blank(1), batch_mark(blocks, folded).span(palette), blank(1)];
    let mut seen: Vec<&(String, String)> = Vec::new();
    for call in &calls {
        // The same call twice in a batch reads once.
        if seen.contains(&call) {
            continue;
        }
        if !seen.is_empty() {
            spans.push(span(" · ", palette.dim()));
        }
        seen.push(call);
        spans.push(span(call.0.clone(), bold(palette.ink())));
        if !call.1.is_empty() {
            spans.push(span(format!(" {}", call.1), palette.ink()));
        }
    }
    let spans = truncate_spans(spans, width.saturating_sub(1).max(1), "…");
    Some(row(spans, width))
}

/// A failed call's row: ` ⊗ Grep \\"  regex parse error: unclosed group`,
/// all red, the name bold, the first line of the error after two spaces.
/// `indent` is 0 for a call of the turn, [`tool_row::SUB_INDENT`] for a call
/// a subagent made. The reason is cut to fit beside the call, or to its own
/// row when the call takes the row.
fn failed_rows(
    indent: usize,
    tool: &str,
    subject: &str,
    reason: &str,
    palette: &Palette,
    width: usize,
) -> Vec<String> {
    let t = hoocode_code_tui_theme::theme();
    let subject = strip_vt_control_characters(subject);
    let label = if subject.trim().is_empty() {
        t.fg("error", &t.bold(tool))
    } else {
        format!(
            "{} {}",
            t.fg("error", &t.bold(tool)),
            t.fg("error", subject.trim())
        )
    };
    let reason = strip_vt_control_characters(reason);
    let reason = reason.trim();
    let lead = if indent == 0 { 1 } else { indent } + 2;
    let label_width = visible_width(&strip_vt_control_characters(&label));
    let beside = width.saturating_sub(lead + label_width + 3);
    let room = if beside >= 12 {
        beside
    } else {
        width.saturating_sub(lead + 2).max(1)
    };
    let facts = if reason.is_empty() {
        Vec::new()
    } else {
        truncate_spans(vec![span(reason, palette.err())], room, "…")
    };
    tool_row::tool_rows(indent, Mark::Failed, &label, facts, width, palette)
}

/// The rows a batch's calls add under its recap row: each failed call's row,
/// then the failed calls of its subagents (settled, or of a run still going),
/// indented 3.
fn failure_rows(blocks: &[ToolBlock], palette: &Palette, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    for block in blocks {
        let body = block.borrow();
        let entry = body.chain_entry();
        if entry.is_error {
            let reason = first_line(&body.error_text()).unwrap_or_default();
            let (tool, subject) = (entry.tool.clone(), entry.subject.clone());
            out.extend(failed_rows(0, &tool, &subject, &reason, palette, width));
        }
        let subs: Vec<(String, String, String)> = if entry.is_partial {
            body.agent_run_id()
                .map(|run| live::sub_steps(&run))
                .unwrap_or_default()
                .into_iter()
                .filter_map(|sub| sub.failure.map(|f| (sub.tool, sub.subject, f)))
                .collect()
        } else {
            body.settled_sub_steps()
                .into_iter()
                .filter_map(|sub| sub.failure.map(|f| (sub.tool, sub.subject, f)))
                .collect()
        };
        for (tool, subject, reason) in subs {
            out.extend(failed_rows(
                tool_row::SUB_INDENT,
                &tool,
                &subject,
                &reason,
                palette,
                width,
            ));
        }
    }
    out
}

/// The final answer, whole: its Markdown rendered as peek and full render it,
/// at the prose indent.
fn answer_rows(blocks: &[String], width: usize) -> Vec<String> {
    if blocks.is_empty() {
        return Vec::new();
    }
    let text = blocks.join("\n\n");
    let mut markdown = Markdown::new(text, 1, 0, get_markdown_theme(), None);
    markdown.render(u16::try_from(width).unwrap_or(u16::MAX))
}

/// A turn's rows under the user message: the recap (one row per batch, the
/// failed calls under it; one row for a folded turn), the answer, a failed
/// turn's error, then the failed or stopped end line. Blocks are kept apart
/// by one blank row each.
fn body_rows(stats: &FlightStats, palette: &Palette, width: usize) -> Vec<String> {
    let mut recap = Vec::new();
    if stats.folded {
        let blocks: Vec<ToolBlock> = stats
            .items
            .iter()
            .flat_map(|item| match item {
                FlightItem::Batch(blocks) => blocks.clone(),
                FlightItem::Thinking(_) => Vec::new(),
            })
            .collect();
        recap.extend(recap_row(&blocks, true, palette, width));
    } else {
        for item in &stats.items {
            // Outline hides thinking; the block still splits batches.
            let FlightItem::Batch(blocks) = item else {
                continue;
            };
            recap.extend(recap_row(blocks, false, palette, width));
            recap.extend(failure_rows(blocks, palette, width));
        }
    }
    let mut groups = vec![recap, answer_rows(&stats.answer, width)];
    if stats.mark == FlightMark::DidNotLand {
        if let Some(reason) = stats.reason.as_deref().and_then(first_line) {
            groups.push(vec![error_row(&reason, palette, width)]);
        }
    }
    let mut rows = Vec::new();
    for group in groups.into_iter().filter(|group| !group.is_empty()) {
        rows.push(String::new());
        rows.extend(group);
    }
    rows.extend(turn_end_rows(stats, palette, width));
    rows
}

/// A provider error: ` ⊗ connection reset by peer`, the glyph red, the text
/// dim. Wraps, never cut.
pub fn error_row(reason: &str, palette: &Palette, width: usize) -> String {
    let spans = vec![
        blank(1),
        span(status::FAILED_STR, palette.err()),
        blank(1),
        span(reason.to_string(), palette.dim()),
    ];
    row(spans, width.max(1))
}

/// A turn's whole outline block: the user message band, then its body.
pub fn turn_rows(
    prompt: &str,
    stats: &FlightStats,
    palette: &Palette,
    width: usize,
) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    let mut rows = crate::user_band_rows(prompt, width, palette);
    rows.extend(body_rows(stats, palette, width));
    rows
}

/// Word-wraps plain `text` (whitespace collapsed) into rows of `room`
/// columns. A word wider than the row breaks at the edge. Always one row.
fn wrap_words(text: &str, room: usize) -> Vec<String> {
    let room = room.max(1);
    let mut rows: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_width = 0;
    for word in text.split_whitespace() {
        let word_width = visible_width(word);
        if current_width > 0 && current_width + 1 + word_width <= room {
            current.push(' ');
            current.push_str(word);
            current_width += 1 + word_width;
            continue;
        }
        if current_width > 0 {
            rows.push(std::mem::take(&mut current));
            current_width = 0;
        }
        for ch in word.chars() {
            let ch_width = visible_width(ch.encode_utf8(&mut [0; 4]));
            if current_width > 0 && current_width + ch_width > room {
                rows.push(std::mem::take(&mut current));
                current_width = 0;
            }
            current.push(ch);
            current_width += ch_width;
        }
    }
    if current_width > 0 || rows.is_empty() {
        rows.push(current);
    }
    rows
}

/// Prose outside every turn: plain lines at `indent`, each wrapped at the
/// width with its last column blank, never cut. `style` is ink for the
/// message and err for an abort or error reason.
pub fn prose_rows(lines: &[String], indent: usize, style: Style, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let room = width.saturating_sub(indent + 1).max(1);
    lines
        .iter()
        .flat_map(|line| wrap_words(line, room))
        .map(|piece| row(vec![blank(indent), span(piece, style)], width))
        .collect()
}

/// A child that only draws in peek view: outline hides it. The hidden flag is
/// shared with the transcript, which flips it with the view dial, so rows
/// added in either view follow later flips.
pub struct PeekOnly {
    children: Vec<ComponentHandle>,
    hidden: Rc<Cell<bool>>,
}

impl PeekOnly {
    /// A peek-only wrapper around `child`, hidden while `hidden` is set.
    pub fn wrap(hidden: Rc<Cell<bool>>, child: ComponentHandle) -> Rc<RefCell<Self>> {
        let mut this = Self {
            children: Vec::new(),
            hidden,
        };
        this.add_child(child);
        Rc::new(RefCell::new(this))
    }

    /// Adds another child to the wrapper.
    pub fn add_child(&mut self, child: ComponentHandle) {
        self.children.push(child);
    }
}

impl Component for PeekOnly {
    /// Hidden, it draws nothing; shown, it draws its children.
    fn render_stamp(&self) -> Option<u64> {
        let mut parts = vec![u64::from(self.hidden.get()), self.children.len() as u64];
        if !self.hidden.get() {
            for child in &self.children {
                parts.push(child.borrow().render_stamp()?);
            }
        }
        Some(mix_stamps(parts))
    }

    fn render(&mut self, width: u16) -> Vec<String> {
        if self.hidden.get() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for child in &self.children {
            out.extend(child.borrow_mut().render(width));
        }
        out
    }

    fn invalidate(&mut self) {
        for child in &self.children {
            child.borrow_mut().invalidate();
        }
    }
}

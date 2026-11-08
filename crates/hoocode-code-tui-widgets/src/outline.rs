//! The mission flight log's data and its end line (declutter spec
//! "Transcript").
//!
//! A turn's [`FlightStats`] is its header from the transcript's point of
//! view: the user message, one recap batch per run of calls, the final
//! answer, and the failed or stopped end line. The transcript refreshes it
//! as the turn runs (items, steps, hits, the mark) and ends (time, cost,
//! tokens, the answer), and the landing line reads the end line from it.
//! Steps, bodies and thinking are hidden. A successful turn has no end
//! line: its result is in the status line.
//!
//! Prose outside every turn keeps its own rows (`prose_rows`).

use std::time::Instant;

use hoocode_tui_glyphs::status;
use hoocode_tui_ratatui::core::style::Style;
use hoocode_tui_ratatui::core::text::Span;
use hoocode_tui_ratatui::{row, Palette};
use hoocode_tui_render::Component;
use hoocode_tui_util::visible_width;

use crate::tool_chain::ToolBlock;

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

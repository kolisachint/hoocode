//! Mission Control flight log (radar) rows (R3, `docs/design/radar-r3.md`).
//!
//! A mission is one block on a `┊` rail, drawn by its header from the
//! mission's [`FlightStats`]: the header `▲ Mn  prompt`; the body in story
//! order (one line per tool batch, the calls that failed nested under it as
//! `└ NN Tool  subject  → snippet  HIT`; a thinking block splits batches but
//! draws nothing: radar hides thinking, peek and full show it); the final answer on the rail; and the turn-end row closing
//! the rail with `╰`. The turn end reads the landing line's words and stats
//! (`● LANDED`, `⊗ DID NOT LAND`, `⊘ ABORTED`), `◐ IN FLIGHT` while the
//! mission flies, or `◆ WAITING` while a call waits for the user's approval.
//! An older finished mission folds to one summary line.
//!
//! Like the trajectory, every row draws through a ratatui buffer and the
//! Mission Control palette. Thinking and prose outside a mission keep their
//! own rows (`think_rows`, `prose_rows`).

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

use hoocode_code_agent_session::format::format_duration_secs;
use hoocode_code_subagents::live;
use hoocode_code_tui_theme::get_markdown_theme;
use hoocode_tui_components::Markdown;
use hoocode_tui_glyphs::status;
use hoocode_tui_ratatui::core::style::Style;
use hoocode_tui_ratatui::core::text::Span;
use hoocode_tui_ratatui::{ansi_spans, bold, reversed, spans_width, truncate_spans, Palette};
use hoocode_tui_render::{mix_stamps, Component, ComponentHandle};
use hoocode_tui_util::visible_width;

use crate::tool_chain::ToolBlock;
use crate::tool_chain_summary::{turn_phrase, ChainEntry};
use crate::trajectory::{landing_spans, short_dur, two_part_row};

/// The in-flight glyph: the mission mark, a live batch and a live agent. Radar-
/// local: `status` has no half circle, and the radar keeps this one for its
/// flying rows.
const IN_FLIGHT_STR: &str = "◐";

/// Indent of prose and thinking outside a mission (the orphan rows).
pub const STEP_INDENT: usize = 8;

/// The blank columns a stem keeps before its right-hand run.
const RUN_GAP: usize = 2;

/// Columns the rail takes before a mission row's content: `   ┊ `.
const RAIL_WIDTH: usize = 5;

/// A mission's mark in the flight log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FlightMark {
    /// The mission landed.
    Landed,
    /// The mission is still flying: `◐` in the accent style.
    #[default]
    InFlight,
    /// The mission ended with failed steps.
    DidNotLand,
    /// The mission was aborted before any step failed.
    Aborted,
}

impl FlightMark {
    /// The mark's glyph: `●`, `◐`, `⊗` or `⊘`.
    pub(crate) fn glyph(self) -> &'static str {
        match self {
            FlightMark::Landed => status::DONE_STR,
            FlightMark::InFlight => IN_FLIGHT_STR,
            FlightMark::DidNotLand => "⊗",
            FlightMark::Aborted => "⊘",
        }
    }

    /// The outcome word the landing line and the comms title print.
    pub fn word(self) -> &'static str {
        match self {
            FlightMark::Landed => "LANDED",
            FlightMark::InFlight => "IN FLIGHT",
            FlightMark::DidNotLand => "DID NOT LAND",
            FlightMark::Aborted => "ABORTED",
        }
    }

    pub(crate) fn style(self, palette: &Palette) -> Style {
        match self {
            FlightMark::Landed => palette.ok(),
            // The accent tone: a flying mission reads as the live rows do.
            FlightMark::InFlight => palette.acc(),
            FlightMark::DidNotLand => palette.err(),
            FlightMark::Aborted => palette.dim(),
        }
    }
}

/// One item of a mission's flight log, in story order.
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

/// One mission's flight-log data. The transcript refreshes it as the mission
/// runs (items, steps, hits, the mark) and lands (time, cost, tokens, the
/// answer). A flying mission reads its elapsed time off `started` on every
/// render, and its batches read their calls on every render, so the log stays
/// live without a refresh.
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
    /// The mission's batches and thinking blocks, in story order.
    pub items: Vec<FlightItem>,
    /// The final answer's Markdown, one source per text block; empty until
    /// the mission has ended.
    pub answer: Vec<String>,
    /// Why the turn did not land: the error it ended with, without `Error: `.
    pub reason: Option<String>,
    /// The final error of a turn that failed, set when its auto-retries ran out
    /// or the turn itself failed. It outranks `reason`, which the message that
    /// carried it may no longer hold (a retried attempt leaves the chat).
    pub failure: Option<String>,
    /// An older finished mission: the log folds it to one summary line.
    pub folded: bool,
}

impl FlightStats {
    /// Every call of the mission, in order.
    pub fn calls(&self) -> Vec<ChainEntry> {
        self.items
            .iter()
            .flat_map(|item| match item {
                FlightItem::Batch(blocks) => batch_entries(blocks),
                FlightItem::Thinking(_) => Vec::new(),
            })
            .collect()
    }

    /// The call the turn waits on: the first held call, or a held call of a
    /// running subagent (its Agent row shows the hold). `None` while nothing
    /// is held.
    pub fn awaiting(&self) -> Option<(String, String)> {
        for item in &self.items {
            let FlightItem::Batch(blocks) = item else {
                continue;
            };
            for block in blocks {
                let body = block.borrow();
                if body.is_waiting_for_clearance() {
                    let entry = body.chain_entry();
                    return Some((entry.tool, entry.subject));
                }
                if body.chain_entry().is_partial {
                    if let Some(run) = body.agent_run_id() {
                        if let Some(sub) = live::sub_steps(&run).into_iter().find(|s| s.held) {
                            return Some((sub.tool, sub.subject));
                        }
                    }
                }
            }
        }
        None
    }

    /// A key for what the rows read, so a holder can tell when they change.
    /// `None` while anything reads a live clock or live state: a flying
    /// mission, a call still running or held, a live background job (read off
    /// the task store), or a call whose state cannot be read right now. The
    /// key hashes only settled calls' stamps and flags and the mission's own
    /// numbers, so no output is cloned per frame. A frozen call keys on its
    /// revision: its capture is fixed, and it has no result left to read.
    pub fn render_key(&self) -> Option<u64> {
        // A flying mission reads its clock and its awaited call on every
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

    /// The reason a DID NOT LAND row names: the turn's failure, else the reason
    /// its message carried. One line, trimmed.
    pub fn did_not_land_reason(&self) -> Option<String> {
        self.failure
            .as_deref()
            .and_then(first_line)
            .or_else(|| self.reason.as_deref().and_then(first_line))
    }

    /// The row's time: the landed span, else live elapsed off the start.
    fn time_text(&self) -> Option<String> {
        if let Some(landed) = &self.landed_time {
            return Some(landed.clone());
        }
        self.started.map(|at| duration_text(at.elapsed().as_secs()))
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

/// A duration in the landing line's format (`0.8s`, `42s`, `1m12s`).
fn duration_text(secs: u64) -> String {
    format_duration_secs(secs as f64)
}

/// The radar's call count: `1 call`, `9 calls`. Radar-only: `chain_stats`
/// keeps the peek and orphan wording.
fn calls_text(count: usize) -> String {
    if count == 1 {
        "1 call".to_string()
    } else {
        format!("{count} calls")
    }
}

/// The chain entries of one batch's calls, in order.
fn batch_entries(blocks: &[ToolBlock]) -> Vec<ChainEntry> {
    blocks
        .iter()
        .map(|block| block.borrow().chain_entry())
        .collect()
}

/// Plain spans read ink: chalk-less text arrives style-less, and the
/// flight log's tools and subjects are ink in the design. Spans that
/// already carry a style (a cut subject's resets) keep it.
fn with_ink(spans: Vec<Span<'static>>, palette: &Palette) -> Vec<Span<'static>> {
    spans
        .into_iter()
        .map(|mut span| {
            if span.style == Style::default() {
                span.style = palette.ink();
            }
            span
        })
        .collect()
}

/// A styled run of text.
fn span(text: impl Into<String>, style: Style) -> Span<'static> {
    Span::styled(text.into(), style)
}

/// `n` unstyled blank cells.
fn blank(n: usize) -> Span<'static> {
    Span::styled(" ".repeat(n), Style::default())
}

/// Fits a row's stem before its right-hand run: the stem is cut with `…`
/// to leave `RUN_GAP` blank columns, and the run stays whole. A run too wide
/// for the row is cut instead (the stem then goes).
fn split_stem(
    stem: Vec<Span<'static>>,
    run: Vec<Span<'static>>,
    width: usize,
) -> (Vec<Span<'static>>, Vec<Span<'static>>) {
    let run_width = spans_width(&run);
    if spans_width(&stem) + run_width + RUN_GAP <= width {
        return (stem, run);
    }
    let budget = width.saturating_sub(run_width + RUN_GAP);
    if budget == 0 {
        if run_width <= width {
            return (Vec::new(), run);
        }
        // A cut run keeps the last column blank, like every other row.
        let mut run = truncate_spans(run, width.saturating_sub(1), "…");
        run.push(blank(1));
        return (Vec::new(), run);
    }
    (truncate_spans(stem, budget, "…"), run)
}

/// A row cut one column short of its width: the last column stays blank.
fn finish(line: Vec<Span<'static>>, width: usize) -> String {
    two_part_row(line, vec![blank(1)], width)
}

/// The number a mission step reads in its row: `01`, `12`.
pub fn step_label(step: usize) -> String {
    format!("{step:02}")
}

/// The number a subagent's own step reads under its agent's step: `01.2` is
/// the second call the step-1 agent made.
pub fn sub_label(parent: usize, index: usize) -> String {
    format!("{parent:02}.{index}")
}

/// A step row's flush-left lead: `indent` blanks, the glyph, the
/// mission-relative step number and the tool, then the gap the subject
/// follows.
fn step_lead(
    indent: usize,
    glyph: &str,
    glyph_style: Style,
    label: &str,
    tool: &str,
    palette: &Palette,
) -> Vec<Span<'static>> {
    vec![
        blank(indent),
        span(glyph, glyph_style),
        blank(1),
        span(format!("{label} "), bold(palette.gold())),
        span(tool, bold(palette.ink())),
        blank(2),
    ]
}

/// One line laid out in a single run: `lead`, the subject, an optional
/// `  → snippet`, then `tail`, in `width - 1` columns. The subject and the
/// snippet are cut with `…` to make room; the lead and the tail stay whole.
/// At a width too narrow for the lead and the tail, the tail stays whole and
/// the lead gives way.
fn inline_line(
    lead: Vec<Span<'static>>,
    subject: Vec<Span<'static>>,
    snippet: Vec<Span<'static>>,
    tail: Vec<Span<'static>>,
    width: usize,
    palette: &Palette,
) -> Vec<Span<'static>> {
    let budget = width.saturating_sub(1);
    let tail_width = spans_width(&tail);
    if spans_width(&lead) + tail_width > budget {
        let mut out = truncate_spans(lead, budget.saturating_sub(tail_width), "…");
        out.extend(truncate_spans(tail, budget, "…"));
        return out;
    }
    let room = budget - spans_width(&lead) - tail_width;
    let mut out = lead;
    if snippet.is_empty() {
        out.extend(truncate_spans(subject, room, "…"));
    } else {
        let sep = vec![span("  → ", palette.dim())];
        let room = room.saturating_sub(spans_width(&sep));
        // The snippet keeps up to half the room; the subject takes the rest.
        let keep = spans_width(&snippet).min(room / 2);
        let subject = truncate_spans(subject, room - keep, "…");
        let rest = room.saturating_sub(spans_width(&subject));
        let snippet = truncate_spans(snippet, rest, "…");
        out.extend(subject);
        if !snippet.is_empty() {
            out.extend(sep);
            out.extend(snippet);
        }
    }
    out.extend(tail);
    out
}

/// Word-wraps plain `text` (whitespace collapsed) into rows: the first holds
/// `first` columns, the rest `rest`. A word wider than its row breaks at the
/// edge. Always at least one row.
fn wrap_words(text: &str, first: usize, rest: usize) -> Vec<String> {
    let mut rows: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_width = 0;
    for word in text.split_whitespace() {
        let budget = if rows.is_empty() { first } else { rest }.max(1);
        let word_width = visible_width(word);
        if current_width > 0 && current_width + 1 + word_width <= budget {
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
            let budget = if rows.is_empty() { first } else { rest }.max(1);
            let ch_width = visible_width(ch.encode_utf8(&mut [0; 4]));
            if current_width > 0 && current_width + ch_width > budget {
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

/// A row cut to `width - 1` columns with `…`, its last column blank.
fn cut_row(spans: Vec<Span<'static>>, width: usize) -> String {
    finish(truncate_spans(spans, width.saturating_sub(1), "…"), width)
}

/// A mission row's lead: three blanks, the `┊`, a blank (the rail).
fn rail(palette: &Palette) -> Vec<Span<'static>> {
    vec![blank(3), span("┊", palette.faint()), blank(1)]
}

/// A nested row's lead: the rail, then `└` under the batch's mark. Level 1 is
/// a call of the batch; level 2 is a call a subagent made under its agent row.
fn nested(level: usize, palette: &Palette) -> Vec<Span<'static>> {
    vec![
        blank(3),
        span("┊", palette.faint()),
        blank(2 * level + 1),
        span("└", palette.faint()),
        blank(1),
    ]
}

/// The turn-end row's lead: three blanks, the `╰` that closes the rail.
fn turn_lead(palette: &Palette) -> Vec<Span<'static>> {
    vec![blank(3), span("╰", palette.faint()), blank(1)]
}

/// The header row: `▲ M1  prompt`, the `▲` and `M1` in the `you` colour. The
/// prompt is never cut: it wraps under its first word.
fn header_rows(seq: usize, prompt: &str, palette: &Palette, width: usize) -> Vec<String> {
    let lead = vec![
        span("▲", bold(palette.you())),
        blank(1),
        span(format!("M{seq}"), bold(palette.you())),
        blank(2),
    ];
    let hang = spans_width(&lead);
    let room = width.saturating_sub(hang + 1).max(1);
    let mut pieces: Vec<String> = prompt
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .flat_map(|line| wrap_words(&line, room, room))
        .collect();
    if pieces.is_empty() {
        pieces.push(String::new());
    }
    pieces
        .into_iter()
        .enumerate()
        .map(|(index, piece)| {
            let mut spans = if index == 0 {
                lead.clone()
            } else {
                vec![blank(hang)]
            };
            spans.push(span(piece, bold(palette.ink())));
            cut_row(spans, width)
        })
        .collect()
}

/// A tool's name and the gap its subject follows, with the call number first
/// when it has one: `05 Shell  `.
fn tool_lead(label: Option<&str>, tool: &str, palette: &Palette) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    if let Some(label) = label {
        out.push(span(format!("{label} "), bold(palette.gold())));
    }
    out.push(span(tool, bold(palette.ink())));
    out.push(blank(2));
    out
}

/// Whether a call's row shows a hit: the call failed, or a call its subagent
/// made failed (settled, or still running into a failure).
fn call_failed(block: &ToolBlock) -> bool {
    let body = block.borrow();
    // The chain entry, as `call_rows` reads it: a frozen call keeps its failure there.
    if body.chain_entry().is_error {
        return true;
    }
    if body
        .settled_sub_steps()
        .iter()
        .any(|sub| sub.failure.is_some())
    {
        return true;
    }
    body.agent_run_id().is_some_and(|run| {
        live::sub_steps(&run)
            .iter()
            .any(|sub| sub.failure.is_some())
    })
}

/// One tool batch's line (R3 rule 3). Settled: `● Edited lex.rs · ran cargo
/// test` with `9 calls` flush right. The mark is `⊘` (dim) when the user cut a
/// call short, else `⊗` when a call or a subagent's call failed (those calls
/// keep their own rows under it). Live, when a call still runs or the batch is
/// the last of a flying mission: `◐ … Edit › Shell › Edit › Shell` with `N
/// calls · clock`, the last four tools and the clock off the batch's earliest
/// start. It never wraps: the phrase is cut with `…`.
fn batch_row(blocks: &[ToolBlock], live_tail: bool, palette: &Palette, width: usize) -> String {
    let entries = batch_entries(blocks);
    let live = live_tail || entries.iter().any(|entry| entry.is_partial);
    let mut stem = rail(palette);
    let right = if live {
        stem.push(span(IN_FLIGHT_STR, palette.acc()));
        stem.push(blank(1));
        stem.push(span("…", palette.dim()));
        stem.push(blank(1));
        let names: Vec<&str> = entries
            .iter()
            .rev()
            .take(4)
            .rev()
            .map(|entry| entry.tool.as_str())
            .collect();
        stem.push(span(names.join(" › "), palette.ink()));
        let earliest = blocks.iter().map(|block| block.borrow().started_at()).min();
        let secs = earliest.map_or(0, |at| at.elapsed().as_secs());
        format!("{} · {}", calls_text(entries.len()), duration_text(secs))
    } else {
        let (glyph, style) = if blocks.iter().any(|block| block.borrow().is_interrupted()) {
            ("⊘", palette.dim())
        } else if blocks.iter().any(call_failed) {
            ("⊗", palette.err())
        } else {
            (status::DONE_STR, palette.ok())
        };
        stem.push(span(glyph, style));
        stem.push(blank(1));
        stem.push(span(turn_phrase(&entries), palette.ink()));
        calls_text(entries.len())
    };
    let run = vec![span(right, palette.dim()), blank(1)];
    let (stem, run) = split_stem(stem, run, width);
    two_part_row(stem, run, width)
}

/// The rows one call adds under its batch's line: a failed call's hit row
/// (`└ 05 Shell  cmd  → snippet  HIT`), a held call's hold row, a background
/// job's satellite row, a subagent's own row with its calls (failed, held or
/// running, one level deeper), and a settled subagent's failed calls.
fn call_rows(block: &ToolBlock, step: usize, palette: &Palette, width: usize) -> Vec<String> {
    let body = block.borrow();
    let entry = body.chain_entry();
    let mut out = Vec::new();
    if entry.is_error {
        let snippet = body
            .error_text()
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or_default()
            .to_string();
        out.push(hit_line(
            nested(1, palette),
            &step_label(step),
            &entry.tool,
            &entry.subject,
            &snippet,
            palette,
            width,
        ));
    } else if body.is_waiting_for_clearance() {
        out.push(hold_row(1, &entry.tool, &entry.subject, palette, width));
    } else if entry.is_partial {
        let secs = body.block_elapsed_secs().unwrap_or(0);
        if let Some(job) = body.background_job_id() {
            out.push(satellite_row(
                &entry.tool,
                &entry.subject,
                &job,
                secs,
                palette,
                width,
            ));
        } else if let Some((subtype, task)) = body.agent_label() {
            out.push(agent_row(&subtype, &task, secs, palette, width));
            if let Some(run) = body.agent_run_id() {
                for sub in live::sub_steps(&run) {
                    let label = sub_label(step, sub.index);
                    match &sub.failure {
                        Some(snippet) => out.push(hit_line(
                            nested(2, palette),
                            &label,
                            &sub.tool,
                            &sub.subject,
                            snippet,
                            palette,
                            width,
                        )),
                        None if sub.held => {
                            out.push(hold_row(2, &sub.tool, &sub.subject, palette, width));
                        }
                        None => out.push(sub_live_row(
                            &sub.tool,
                            &sub.subject,
                            sub.started.elapsed().as_secs(),
                            palette,
                            width,
                        )),
                    }
                }
            }
        }
    }
    if !entry.is_partial {
        for sub in body
            .settled_sub_steps()
            .iter()
            .filter(|sub| sub.failure.is_some())
        {
            out.push(hit_line(
                nested(2, palette),
                &sub_label(step, sub.index),
                &sub.tool,
                &sub.subject,
                sub.failure.as_deref().unwrap_or_default(),
                palette,
                width,
            ));
        }
    }
    out
}

/// A held call, parked for the user's approval: `◉ Shell  rm -rf x   holding
/// for clearance`, nested at `level`.
fn hold_row(level: usize, tool: &str, subject: &str, palette: &Palette, width: usize) -> String {
    let mut stem = nested(level, palette);
    stem.push(span(status::RUNNING_STR, palette.warn()));
    stem.push(blank(1));
    stem.extend(tool_lead(None, tool, palette));
    stem.extend(with_ink(ansi_spans(subject), palette));
    let run = vec![span("holding for clearance", palette.warn()), blank(1)];
    let (stem, run) = split_stem(stem, run, width);
    two_part_row(stem, run, width)
}

/// A running subagent: `◐ Agent  ◈ general-purpose · Compute a product   2s`.
fn agent_row(subtype: &str, task: &str, secs: u64, palette: &Palette, width: usize) -> String {
    let mut stem = nested(1, palette);
    stem.push(span(IN_FLIGHT_STR, palette.acc()));
    stem.push(blank(1));
    stem.extend(tool_lead(None, "Agent", palette));
    stem.push(span("◈", bold(palette.vio())));
    stem.push(blank(1));
    stem.push(span(subtype.to_string(), palette.ink()));
    stem.push(span(" · ", palette.dim()));
    stem.extend(with_ink(ansi_spans(task), palette));
    let run = vec![span(duration_text(secs), palette.acc()), blank(1)];
    let (stem, run) = split_stem(stem, run, width);
    two_part_row(stem, run, width)
}

/// A running call of a subagent, one level under its agent row:
/// `◐ Read  src/lib.rs   2s`.
fn sub_live_row(tool: &str, subject: &str, secs: u64, palette: &Palette, width: usize) -> String {
    let mut stem = nested(2, palette);
    stem.push(span(IN_FLIGHT_STR, palette.acc()));
    stem.push(blank(1));
    stem.extend(tool_lead(None, tool, palette));
    stem.extend(with_ink(ansi_spans(subject), palette));
    let run = vec![span(duration_text(secs), palette.dim()), blank(1)];
    let (stem, run) = split_stem(stem, run, width);
    two_part_row(stem, run, width)
}

/// A running background job: `◐ Shell  sleep 60   ⌁ sat job-1 · in orbit 4s`.
fn satellite_row(
    tool: &str,
    subject: &str,
    job: &str,
    secs: u64,
    palette: &Palette,
    width: usize,
) -> String {
    let mut stem = nested(1, palette);
    stem.push(span(IN_FLIGHT_STR, palette.acc()));
    stem.push(blank(1));
    stem.extend(tool_lead(None, tool, palette));
    stem.extend(with_ink(ansi_spans(subject), palette));
    let run = vec![
        span("⌁", palette.acc()),
        span(format!(" sat {job} "), palette.acc()),
        span("·", palette.dim()),
        span(format!(" in orbit {}", duration_text(secs)), palette.acc()),
        blank(1),
    ];
    let (stem, run) = split_stem(stem, run, width);
    two_part_row(stem, run, width)
}

/// The final answer on the rail, whole: its Markdown rendered as peek and
/// full render it (headings, lists, code, emphasis), wrapped to the room
/// after the rail, never cut.
fn answer_rows(blocks: &[String], palette: &Palette, width: usize) -> Vec<String> {
    if blocks.is_empty() {
        return Vec::new();
    }
    let room = width.saturating_sub(RAIL_WIDTH + 1).max(1);
    let text = blocks.join("\n\n");
    let mut markdown = Markdown::new(text, 0, 0, get_markdown_theme(), None);
    let room = u16::try_from(room).unwrap_or(u16::MAX);
    markdown
        .render(room)
        .into_iter()
        .map(|line| {
            let line = line.trim_end();
            // A ratatui buffer drops OSC 8, so a row with a link keeps the
            // Markdown line as it is after the drawn rail: the link stays
            // clickable. The line already fits the room after the rail.
            if line.contains("\x1b]8;") {
                // The drawn row ends at its last glyph: the rail's blank
                // goes back on by hand.
                let lead = crate::trajectory::draw_line(rail(palette), RAIL_WIDTH);
                let lead_width = visible_width(&lead);
                let gap = " ".repeat(RAIL_WIDTH.saturating_sub(lead_width));
                let pad = width.saturating_sub(RAIL_WIDTH + visible_width(line));
                return format!("{lead}{gap}{line}\x1b[0m{}", " ".repeat(pad));
            }
            let mut spans = rail(palette);
            spans.extend(with_ink(ansi_spans(line), palette));
            cut_row(spans, width)
        })
        .collect()
}

/// The turn-end rows, always last (R3 rule 2). Landed, failed or aborted, the
/// row reads the landing line (`landing_spans`, the stats of peek and full). A
/// DID NOT LAND reason stays on the row when it fits; when it does not, it goes
/// on one continuation line under the row, indented to the text after `╰ `
/// with no rail, and is cut only if it is still wider than the row. Flying, the
/// row reads `◐ IN FLIGHT` with its clock; while a call waits for approval,
/// `◆ WAITING · approve Edit src/lex.rs`.
fn turn_end_rows(stats: &FlightStats, palette: &Palette, width: usize) -> Vec<String> {
    let mut spans = turn_lead(palette);
    let flying = stats.mark == FlightMark::InFlight;
    let mut reason = None;
    let did_not_land = (stats.mark == FlightMark::DidNotLand)
        .then(|| stats.did_not_land_reason())
        .flatten();
    match flying.then(|| stats.awaiting()).flatten() {
        Some((tool, subject)) => {
            spans.push(span(status::WAITING_STR, palette.warn()));
            spans.push(blank(1));
            spans.push(span("WAITING", bold(palette.warn())));
            spans.push(span(" · ", palette.dim()));
            spans.push(span(format!("approve {tool} {subject}"), palette.ink()));
        }
        None => {
            let time = if flying {
                stats.time_text().unwrap_or_default()
            } else {
                stats.landed_time.clone().unwrap_or_default()
            };
            // A failed turn shows its reason in place of the tokens and cost,
            // as the approved mockup does, so the reason fits the row. A flying
            // row shows no cost: it is the mission's running cost.
            let failed = did_not_land.is_some();
            let (tokens, cost) = if flying || failed {
                (String::new(), String::new())
            } else {
                (
                    stats.tokens.clone().unwrap_or_default(),
                    stats.cost.clone().unwrap_or_default(),
                )
            };
            spans.extend(landing_spans(
                stats.mark,
                stats.steps,
                stats.hits,
                &time,
                &tokens,
                &cost,
                palette,
            ));
            reason = did_not_land;
        }
    }
    let Some(reason) = reason else {
        return vec![cut_row(spans, width)];
    };
    // ` · ` and the reason, inline, when they fit with the last column blank.
    let inline = spans_width(&spans) + 3 + visible_width(&reason) < width;
    if inline {
        spans.push(span(" · ", palette.dim()));
        spans.push(span(reason, palette.err()));
        return vec![cut_row(spans, width)];
    }
    vec![
        cut_row(spans, width),
        cut_row(vec![blank(RAIL_WIDTH), span(reason, palette.err())], width),
    ]
}

/// The mission's body rows, between its header and its turn end: the open
/// items in story order, or for a folded mission one summary line, then a
/// blank rail line and the answer. The last batch of a flying mission reads
/// live, so the row does not flip between calls.
fn body_rows(stats: &FlightStats, palette: &Palette, width: usize) -> Vec<String> {
    let mut rows = Vec::new();
    if stats.folded {
        let calls = stats.calls();
        if !calls.is_empty() {
            let mut stem = rail(palette);
            stem.push(span(stats.mark.glyph(), stats.mark.style(palette)));
            stem.push(blank(1));
            stem.push(span(turn_phrase(&calls), palette.ink()));
            let run = vec![span(calls_text(calls.len()), palette.dim()), blank(1)];
            let (stem, run) = split_stem(stem, run, width);
            rows.push(two_part_row(stem, run, width));
        }
    } else {
        let flying = stats.mark == FlightMark::InFlight;
        let last_batch = stats
            .items
            .iter()
            .rposition(|item| matches!(item, FlightItem::Batch(_)));
        let mut step = 0;
        for (index, item) in stats.items.iter().enumerate() {
            match item {
                FlightItem::Batch(blocks) => {
                    let live_tail = flying && Some(index) == last_batch;
                    rows.push(batch_row(blocks, live_tail, palette, width));
                    for block in blocks {
                        step += 1;
                        rows.extend(call_rows(block, step, palette, width));
                    }
                }
                // Radar hides thinking (2026-10-31); the block still splits batches.
                FlightItem::Thinking(_) => {}
            }
        }
    }
    let answer = answer_rows(&stats.answer, palette, width);
    if !rows.is_empty() && !answer.is_empty() {
        rows.push(cut_row(vec![blank(3), span("┊", palette.faint())], width));
    }
    rows.extend(answer);
    rows
}

/// The mission's whole flight-log block (R3): `▲ Mn  prompt`, the body on the
/// `┊` rail, and the turn-end rows that close it. A flying mission's body
/// reads its live batch, its clock and its held call.
pub fn mission_rows(
    seq: usize,
    prompt: &str,
    stats: &FlightStats,
    palette: &Palette,
    width: usize,
) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    let mut rows = header_rows(seq, prompt, palette, width);
    rows.extend(body_rows(stats, palette, width));
    rows.extend(turn_end_rows(stats, palette, width));
    rows
}

/// A settled subagent call as one line, uncut, for the peek and full rails
/// (`⊗ 01.2 Grep  pattern  → snippet  HIT`, or `● 01.1 Read  path`): the
/// caller cuts it or wraps it. `parent` is the Agent step's number.
pub fn sub_step_line(
    parent: usize,
    step: &hoocode_code_subagents::live::SettledStep,
    palette: &Palette,
) -> String {
    let failed = step.failure.is_some();
    let (glyph, style) = if failed {
        ("⊗", palette.err())
    } else {
        (status::DONE_STR, palette.ok())
    };
    let mut spans = step_lead(
        0,
        glyph,
        style,
        &sub_label(parent, step.index),
        &step.tool,
        palette,
    );
    spans.extend(with_ink(ansi_spans(&step.subject), palette));
    if let Some(snippet) = step.failure.as_deref().filter(|s| !s.trim().is_empty()) {
        spans.push(span("  → ", palette.dim()));
        spans.extend(with_ink(ansi_spans(snippet), palette));
    }
    if failed {
        spans.push(blank(2));
        spans.push(span("HIT", reversed(bold(palette.err()))));
    }
    let width = spans_width(&spans).max(1);
    crate::trajectory::draw_line(spans, width)
}

/// A failed call's row under its batch (R3 rule 4), `└ 05 Shell  echo boom
/// → boom  HIT`: `lead` is the nesting, the number bold gold. The `HIT` chip
/// sits inline, two spaces after the snippet, and stays whole; the subject and
/// snippet are cut with `…` first. An empty snippet drops the `→` part.
fn hit_line(
    lead: Vec<Span<'static>>,
    label: &str,
    tool: &str,
    subject: &str,
    snippet: &str,
    palette: &Palette,
    width: usize,
) -> String {
    if width == 0 {
        return String::new();
    }
    let mut lead = lead;
    lead.extend(tool_lead(Some(label), tool, palette));
    // The subject and snippet keep whatever styling they arrived with (a
    // cut subject carries its resets); plain text stays ink.
    let snippet = if snippet.trim().is_empty() {
        Vec::new()
    } else {
        with_ink(ansi_spans(snippet), palette)
    };
    let tail = vec![blank(2), span("HIT", reversed(bold(palette.err())))];
    let line = inline_line(
        lead,
        with_ink(ansi_spans(subject), palette),
        snippet,
        tail,
        width,
        palette,
    );
    finish(line, width)
}

/// Thinking's rows under their mission (Mission Control spec r11), indented
/// 8: `        ✻ thought 3s · The helper is 13 lines`. The gist reads dim after
/// the violet thought; without one the row is the thought alone. The gist is
/// never cut: it wraps onto rows hanging under its first word.
pub fn think_rows(secs: u64, gist: Option<&str>, palette: &Palette, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    let mut lead = vec![
        blank(STEP_INDENT),
        span(format!("✻ thought {}", short_dur(secs)), palette.vio()),
    ];
    let Some(gist) = gist.filter(|gist| !gist.trim().is_empty()) else {
        return vec![finish(lead, width)];
    };
    lead.push(span(" · ", palette.faint()));
    let hang = spans_width(&lead);
    let room = width.saturating_sub(hang + 1).max(1);
    wrap_words(gist, room, room)
        .into_iter()
        .enumerate()
        .map(|(index, piece)| {
            let mut line = if index == 0 {
                lead.clone()
            } else {
                vec![blank(hang)]
            };
            line.push(span(piece, palette.dim()));
            finish(line, width)
        })
        .collect()
}

/// The prose rows under a mission (Mission Control spec r1/r3): the final
/// message's plain text, one entry per `lines` item, indented `indent` (8
/// under the mission row, 12 when a hit row is above it). Never cut: an
/// entry wraps at the width, the last column kept blank. `style` is ink for
/// the message and err for the abort or error reason.
pub fn prose_rows(lines: &[String], indent: usize, style: Style, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let room = width.saturating_sub(indent + 1).max(1);
    lines
        .iter()
        .flat_map(|line| wrap_words(line, room, room))
        .map(|piece| finish(vec![blank(indent), span(piece, style)], width))
        .collect()
}

/// A child that only draws in peek view: radar hides it. The hidden flag is
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

//! Mission Control flight log (radar) rows (M5, spec §4 r1–r11).
//! In radar view each mission collapses to one flight-log row: the mark
//! (`●` landed, `◐` in flight, `⊗` did not land, `⊘` aborted), the prompt,
//! and the stats (steps, `N✕` hits, time, cost). The stats sit in fixed
//! columns counted back from the right edge; the prompt is cut with `…`
//! first. Under the mission row, each hit gets its own line
//! (`⊗ 04 Shell  cmd  → snippet  HIT`, indented 12), a flying mission shows
//! its live step (`◐` with an eight-cell progress bar and `3s of ~6s` when
//! the call's estimate is known, indented 8), waiting steps read `◉ …
//! holding for clearance`, subagent steps read `◈ type · task`, background
//! jobs read their satellite row, and thinking shows `✻ thought Ns` with
//! its gist. Every row ends one column short of the width, so the last
//! column stays a blank margin.
//!
//! Like the trajectory, every row draws through a ratatui buffer and the
//! Mission Control palette. Colours keep the legacy theme tokens the peek
//! view styled with, so both views stay in step.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

use hoocode_tui_glyphs::status;
use hoocode_tui_ratatui::core::style::Style;
use hoocode_tui_ratatui::core::text::Span;
use hoocode_tui_ratatui::{ansi_spans, bold, reversed, spans_width, truncate_spans, Palette};
use hoocode_tui_render::{Component, ComponentHandle};

use crate::trajectory::{first_line, short_dur, two_part_row};

/// The in-flight glyph: the mission mark, a live step, a live agent and a
/// satellite job. Radar-local: the shared `status::RUNNING_STR` (`◉`) keeps
/// the waiting row's glyph, and the peek and task views keep it too.
const IN_FLIGHT_STR: &str = "◐";

/// Indent of a step's own line (live, waiting, agent, satellite) and of
/// prose or thinking that sits directly under a mission row.
pub const STEP_INDENT: usize = 8;

/// Indent of a hit's line, and of prose that follows a hit row.
pub const HIT_INDENT: usize = 12;

/// Cells in a live step's progress bar.
const BAR_CELLS: usize = 8;

/// The stats' fixed columns, counted back from the width: a column is
/// `width - END`. The step noun ends 21 back, the hits 17, the time 10 and
/// the cost 2; the last column (0 back) stays blank.
const STEPS_END: usize = 21;
const HITS_END: usize = 17;
const TIME_END: usize = 10;
const COST_END: usize = 2;

/// The time reads right-aligned in a field this wide.
const TIME_FIELD: usize = 4;

/// The blank columns a stem keeps before its right-hand run.
const RUN_GAP: usize = 2;

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
    fn glyph(self) -> &'static str {
        match self {
            FlightMark::Landed => status::DONE_STR,
            FlightMark::InFlight => IN_FLIGHT_STR,
            FlightMark::DidNotLand => "⊗",
            FlightMark::Aborted => "⊘",
        }
    }

    fn style(self, palette: &Palette) -> Style {
        match self {
            FlightMark::Landed => palette.ok(),
            // The accent tone: a flying mission reads as the live rows do.
            FlightMark::InFlight => palette.acc(),
            FlightMark::DidNotLand => palette.err(),
            FlightMark::Aborted => palette.dim(),
        }
    }
}

/// One mission's flight-log data. The transcript refreshes it as the
/// mission runs (steps, hits, the mark) and lands (time, cost); a flying
/// mission reads its elapsed time off `started` on every render, so the row
/// stays live without a refresh.
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
}

impl FlightStats {
    /// The row's time: the landed span, else live elapsed off the start.
    fn time_text(&self) -> Option<String> {
        if let Some(landed) = &self.landed_time {
            return Some(landed.clone());
        }
        self.started.map(|at| short_dur(at.elapsed().as_secs()))
    }

    /// The flush-right stats run (Mission Control spec r1) on fixed columns:
    /// the step noun (`1 step`, `2 steps`) ends `STEPS_END` columns from the
    /// right edge, the `N✕` hits end `HITS_END` back, the time (right-aligned
    /// in a four-column field) ends `TIME_END` back, and the cost ends
    /// `COST_END` back. Hits and times that are wider grow leftward. The run
    /// reaches the width with its last column blank.
    ///
    /// A stepless mission (`steps == 0`) has no noun, so its steps columns
    /// stay blank; its hits show only when it has any. A field the mission
    /// lacks (no time yet, no cost) leaves its columns blank. The step noun
    /// reads ink landed and accent while flying; hits faint; time dim (accent
    /// flying); cost dim.
    fn right_run(&self, width: usize, palette: &Palette) -> Vec<Span<'static>> {
        let w = width as isize;
        let flying = self.mark == FlightMark::InFlight;
        // (column where the field's text ends, its spans)
        let mut fields: Vec<(isize, Vec<Span<'static>>)> = Vec::new();
        if self.steps > 0 {
            let count = if flying { palette.acc() } else { palette.ink() };
            let noun = if self.steps == 1 { "step" } else { "steps" };
            fields.push((
                w - STEPS_END as isize,
                vec![
                    span(self.steps.to_string(), count),
                    span(format!(" {noun}"), palette.dim()),
                ],
            ));
        }
        if self.steps > 0 || self.hits > 0 {
            fields.push((
                w - HITS_END as isize,
                vec![span(format!("{}✕", self.hits), palette.faint())],
            ));
        }
        if let Some(time) = self.time_text() {
            let time_style = if flying { palette.acc() } else { palette.dim() };
            fields.push((
                w - TIME_END as isize,
                vec![span(
                    format!("{time:>width$}", width = TIME_FIELD),
                    time_style,
                )],
            ));
        }
        if let Some(cost) = &self.cost {
            fields.push((
                w - COST_END as isize,
                vec![span(cost.clone(), palette.dim())],
            ));
        }
        let Some(first_start) = fields
            .first()
            .map(|(end, spans)| end + 1 - spans_width(spans) as isize)
        else {
            return vec![blank(1)];
        };
        let mut out = Vec::new();
        let mut cursor = first_start;
        for (end, spans) in fields {
            let field_width = spans_width(&spans) as isize;
            let start = end + 1 - field_width;
            if start > cursor {
                out.push(blank((start - cursor) as usize));
                cursor = start;
            }
            out.extend(spans);
            cursor += field_width;
        }
        if cursor < w {
            out.push(blank((w - cursor) as usize));
        }
        out
    }
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

/// A step row's flush-left lead: `indent` blanks, the glyph, the
/// mission-relative step number and the tool, then the gap the subject
/// follows.
fn step_lead(
    indent: usize,
    glyph: &str,
    glyph_style: Style,
    step: usize,
    tool: &str,
    palette: &Palette,
) -> Vec<Span<'static>> {
    vec![
        blank(indent),
        span(glyph, glyph_style),
        blank(1),
        span(format!("{step:02} "), bold(palette.gold())),
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

/// The mission row (Mission Control spec r1):
/// `M1 ●  prompt                1 step  0✕   0.8s  $0.003`. `M1` reads `you`,
/// the glyph its mark, the prompt bold ink. The stats stay whole; the
/// prompt is cut with `…` first. `prompt` is one line.
pub fn mission_row(
    seq: usize,
    prompt: &str,
    stats: &FlightStats,
    palette: &Palette,
    width: usize,
) -> String {
    if width == 0 {
        return String::new();
    }
    let stem = vec![
        blank(1),
        span(format!("M{seq}"), palette.you()),
        blank(1),
        span(stats.mark.glyph(), stats.mark.style(palette)),
        blank(2),
        span(first_line(prompt), bold(palette.ink())),
    ];
    let (stem, run) = split_stem(stem, stats.right_run(width, palette), width);
    two_part_row(stem, run, width)
}

/// A hit's own line (Mission Control spec r1/r3), indented 12:
/// `            ⊗ 04 Shell  echo boom; exit 3  → boom  HIT`. `step` is the
/// mission-relative step number. The `HIT` chip sits inline, two spaces
/// after the snippet, and stays whole; the subject and snippet are cut with
/// `…` first. An empty snippet drops the `→` part.
pub fn hit_row(
    step: usize,
    tool: &str,
    subject: &str,
    snippet: &str,
    palette: &Palette,
    width: usize,
) -> String {
    if width == 0 {
        return String::new();
    }
    let lead = step_lead(HIT_INDENT, "⊗", palette.err(), step, tool, palette);
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

/// A live step's own line (Mission Control spec r4), indented 8:
/// `        ◐ 01 Shell  sleep 6; echo done  ████░░░░ 3s of ~6s`. `step` is
/// the mission-relative step number. With an estimate the bar fills its
/// eight cells by elapsed share; without one the row reads elapsed only.
pub fn live_row(
    step: usize,
    tool: &str,
    subject: &str,
    elapsed_secs: u64,
    estimate_secs: Option<u64>,
    palette: &Palette,
    width: usize,
) -> String {
    if width == 0 {
        return String::new();
    }
    let lead = step_lead(
        STEP_INDENT,
        IN_FLIGHT_STR,
        palette.acc(),
        step,
        tool,
        palette,
    );
    let mut tail = vec![blank(2)];
    if let Some(estimate) = estimate_secs.filter(|estimate| *estimate > 0) {
        tail.push(span(progress_bar(elapsed_secs, estimate), palette.acc()));
        tail.push(span(
            progress_track(elapsed_secs, estimate),
            palette.faint(),
        ));
        tail.push(span(
            format!(" {} of ~{}", short_dur(elapsed_secs), short_dur(estimate)),
            palette.dim(),
        ));
    } else {
        tail.push(span(short_dur(elapsed_secs), palette.dim()));
    }
    let line = inline_line(
        lead,
        with_ink(ansi_spans(subject), palette),
        Vec::new(),
        tail,
        width,
        palette,
    );
    finish(line, width)
}

/// The live bar's filled cells: `BAR_CELLS` blocks by elapsed share, capped.
fn progress_bar(elapsed_secs: u64, estimate_secs: u64) -> String {
    "█".repeat(progress_filled(elapsed_secs, estimate_secs))
}

/// The live bar's remaining cells.
fn progress_track(elapsed_secs: u64, estimate_secs: u64) -> String {
    "░".repeat(BAR_CELLS.saturating_sub(progress_filled(elapsed_secs, estimate_secs)))
}

fn progress_filled(elapsed_secs: u64, estimate_secs: u64) -> usize {
    let share = elapsed_secs as f64 / estimate_secs.max(1) as f64;
    ((share * BAR_CELLS as f64).round() as usize).min(BAR_CELLS)
}

/// A step parked for clearance (Mission Control spec r5), indented 8:
/// `        ◉ 01 Shell  wc -l src/wrap.rs      holding for clearance`. The
/// head of the queue reads `◉` warn; steps queued behind it read `○` with
/// `next`. The right text ends one column short of the width.
pub fn waiting_row(
    step: usize,
    tool: &str,
    subject: &str,
    head: bool,
    palette: &Palette,
    width: usize,
) -> String {
    if width == 0 {
        return String::new();
    }
    let (glyph, right_text, right_style) = if head {
        (status::RUNNING_STR, "holding for clearance", palette.warn())
    } else {
        (status::TODO_STR, "next", palette.dim())
    };
    let mut stem = step_lead(STEP_INDENT, glyph, palette.warn(), step, tool, palette);
    stem.extend(with_ink(ansi_spans(subject), palette));
    let run = vec![span(right_text, right_style), blank(1)];
    let (stem, run) = split_stem(stem, run, width);
    two_part_row(stem, run, width)
}

/// A live subagent step (Mission Control spec r7), indented 8:
/// `        ◐ 01 Agent  ◈ general-purpose · Compute a product        2s`.
pub fn agent_live_row(
    step: usize,
    subtype: &str,
    task: &str,
    elapsed_secs: u64,
    palette: &Palette,
    width: usize,
) -> String {
    if width == 0 {
        return String::new();
    }
    let mut stem = step_lead(
        STEP_INDENT,
        IN_FLIGHT_STR,
        palette.acc(),
        step,
        "Agent",
        palette,
    );
    stem.extend([
        span("◈", bold(palette.vio())),
        blank(1),
        span(subtype.to_string(), palette.ink()),
        span(" · ", palette.dim()),
    ]);
    stem.extend(with_ink(ansi_spans(task), palette));
    let run = vec![span(short_dur(elapsed_secs), palette.acc()), blank(1)];
    let (stem, run) = split_stem(stem, run, width);
    two_part_row(stem, run, width)
}

/// A live background job (Mission Control spec r9), indented 8:
/// `        ◐ 01 Shell  sleep 60                 ⌁ sat job-1 · in orbit 4s`.
pub fn satellite_row(
    step: usize,
    tool: &str,
    subject: &str,
    job: &str,
    elapsed_secs: u64,
    palette: &Palette,
    width: usize,
) -> String {
    if width == 0 {
        return String::new();
    }
    let mut stem = step_lead(
        STEP_INDENT,
        IN_FLIGHT_STR,
        palette.acc(),
        step,
        tool,
        palette,
    );
    stem.extend(with_ink(ansi_spans(subject), palette));
    let run = vec![
        span("⌁", palette.acc()),
        span(format!(" sat {job} "), palette.acc()),
        span("·", palette.dim()),
        span(
            format!(" in orbit {}", short_dur(elapsed_secs)),
            palette.acc(),
        ),
        blank(1),
    ];
    let (stem, run) = split_stem(stem, run, width);
    two_part_row(stem, run, width)
}

/// Thinking's line under its mission (Mission Control spec r11), indented
/// 8: `        ✻ thought 3s · The helper is 13 lines…`. The gist reads dim after
/// the violet thought; without one the row is the thought alone.
pub fn think_row(secs: u64, gist: Option<&str>, palette: &Palette, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let mut spans = vec![
        blank(STEP_INDENT),
        span(format!("✻ thought {}", short_dur(secs)), palette.vio()),
    ];
    if let Some(gist) = gist.filter(|gist| !gist.trim().is_empty()) {
        spans.push(span(" · ", palette.faint()));
        spans.push(span(gist.to_string(), palette.dim()));
    }
    finish(spans, width)
}

/// The prose line under a mission (Mission Control spec r1/r3): the
/// message's last paragraph on one line, indented `indent` (8 under the
/// mission row, 12 when a hit row is above it) and cut with `…` to leave
/// the last column blank. `text` is already one line.
pub fn prose_row(text: &str, indent: usize, palette: &Palette, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    finish(
        vec![blank(indent), span(text.to_string(), palette.ink())],
        width,
    )
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

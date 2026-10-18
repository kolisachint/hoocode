//! Mission Control flight log (radar) rows (M5, spec §4 r1–r11).
//! In radar view each mission collapses to one flight-log row: the mark
//! (`●` landed, `◉` in flight, `⊗` did not land, `⊘` aborted), the prompt,
//! and the stats (steps, `N✕` hits, time, cost). The stats stay whole; the
//! prompt is cut with `…` first. Under the mission row, each hit gets its
//! own line (`⊗ 04 Shell  cmd  → snippet  HIT`), a flying mission shows its
//! live step (a progress bar plus `3s of ~6s` when the call's estimate is
//! known), waiting steps read `holding for clearance`, subagent steps
//! read `◈ type · task`, background jobs read their satellite row, and
//! thinking shows `✻ thought Ns` with its gist.
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
use hoocode_tui_ratatui::{bold, reversed, Palette};
use hoocode_tui_render::{Component, ComponentHandle};

use crate::trajectory::two_part_row;
use crate::trajectory::{first_line, header_parts, short_dur};
use hoocode_tui_ratatui::row;

/// A mission's mark in the flight log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FlightMark {
    /// The mission landed.
    Landed,
    /// The mission is still flying.
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
            FlightMark::InFlight => status::RUNNING_STR,
            FlightMark::DidNotLand => "⊗",
            FlightMark::Aborted => "⊘",
        }
    }

    fn style(self, palette: &Palette) -> Style {
        match self {
            FlightMark::Landed => palette.ok(),
            // The running tone, as in the peek Think row and the old chain
            // summary glyph.
            FlightMark::InFlight => palette.warn(),
            FlightMark::DidNotLand => palette.err(),
            FlightMark::Aborted => palette.dim(),
        }
    }
}

/// One mission's flight-log data. The transcript refreshes it as the mission
/// runs (steps, hits, the mark) and lands (time, cost); a flying mission
/// reads its elapsed time off `started` on every render, so the row stays
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
}

impl FlightStats {
    /// The row's time: the landed span, else live elapsed off the start.
    fn time_text(&self) -> Option<String> {
        if let Some(landed) = &self.landed_time {
            return Some(landed.clone());
        }
        self.started.map(|at| short_dur(at.elapsed().as_secs()))
    }

    /// The flush-right stats (Mission Control spec r1): `1 step 0✕ 0.8s
    /// $0.003`, space-separated. The step count reads ink landed, acc
    /// while flying; hits faint; time dim (acc flying); cost dim.
    /// A stepless mission omits `0 steps 0✕` entirely: the row reads the
    /// mark, the prompt, then the time and cost when known.
    fn spans(&self, palette: &Palette) -> Vec<Span<'static>> {
        let flying = self.mark == FlightMark::InFlight;
        let mut parts: Vec<Vec<Span<'static>>> = Vec::new();
        if self.steps > 0 || self.hits > 0 {
            let count = if flying { palette.acc() } else { palette.ink() };
            let noun = if self.steps == 1 { "step" } else { "steps" };
            parts.push(vec![
                Span::styled(self.steps.to_string(), count),
                Span::styled(" ".to_string(), Style::default()),
                Span::styled(noun.to_string(), palette.dim()),
            ]);
            parts.push(vec![Span::styled(
                format!("{}✕", self.hits),
                palette.faint(),
            )]);
        }
        if let Some(time) = self.time_text() {
            let time_style = if flying { palette.acc() } else { palette.dim() };
            parts.push(vec![Span::styled(time, time_style)]);
        }
        if let Some(cost) = &self.cost {
            parts.push(vec![Span::styled(cost.clone(), palette.dim())]);
        }
        let mut out = Vec::new();
        for (i, part) in parts.into_iter().enumerate() {
            if i > 0 {
                out.push(Span::styled(" ".to_string(), Style::default()));
            }
            out.extend(part);
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

/// The mission row (Mission Control spec r1):
/// `M1 ●  prompt  1 step 0✕ 0.8s $0.003`. The number reads `you`, the
/// prompt bold ink. The stats stay whole; the prompt is cut with `…`
/// first. `prompt` is one line.
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
    let number = vec![Span::styled(format!("M{seq}"), palette.you())];
    let prefix = vec![
        Span::styled(stats.mark.glyph().to_string(), stats.mark.style(palette)),
        Span::styled(" ".to_string(), Style::default()),
    ];
    let left = vec![Span::styled(first_line(prompt), bold(palette.ink()))];
    let (left, right) = header_parts(number, prefix, left, stats.spans(palette), width);
    two_part_row(left, right, width)
}

/// A hit's own line (Mission Control spec r1/r3):
/// `⊗ 04 Shell  echo boom; exit 3  → boom  HIT`. `step` is the
/// mission-relative step number. The `HIT` chip is the inverted block and
/// stays whole; the tool and subject are cut with `…` first. An empty
/// snippet drops the `→` part.
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
    let number = vec![Span::styled("⊗".to_string(), palette.err())];
    let prefix = vec![Span::styled(format!("{step:02} "), bold(palette.gold()))];
    let mut left = vec![
        Span::styled(tool.to_string(), bold(palette.ink())),
        Span::styled("  ".to_string(), Style::default()),
    ];
    // The subject and snippet keep whatever styling they arrived with
    // (a cut subject carries its resets); plain text stays ink.
    left.extend(with_ink(hoocode_tui_ratatui::ansi_spans(subject), palette));
    if !snippet.trim().is_empty() {
        left.push(Span::styled("  →  ".to_string(), palette.dim()));
        left.extend(with_ink(hoocode_tui_ratatui::ansi_spans(snippet), palette));
    }
    let right = vec![Span::styled(
        " HIT ".to_string(),
        reversed(bold(palette.err())),
    )];
    let (left, right) = header_parts(number, prefix, left, right, width);
    two_part_row(left, right, width)
}

/// A live step's own line (Mission Control spec r4):
/// `◉ 01 Shell  sleep 6; echo done  ████████░░░░░░░░ 3s of ~6s`. `step` is
/// the mission-relative step number. With an estimate the bar fills
/// sixteen cells by elapsed share; without one the row reads elapsed only.
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
    let number = vec![Span::styled(status::RUNNING_STR.to_string(), palette.acc())];
    let prefix = vec![Span::styled(format!("{step:02} "), bold(palette.gold()))];
    let mut left = vec![
        Span::styled(tool.to_string(), bold(palette.ink())),
        Span::styled("  ".to_string(), Style::default()),
    ];
    left.extend(with_ink(hoocode_tui_ratatui::ansi_spans(subject), palette));
    let mut right: Vec<Span<'static>> = Vec::new();
    if let Some(estimate) = estimate_secs.filter(|estimate| *estimate > 0) {
        right.push(Span::styled(
            progress_bar(elapsed_secs, estimate),
            palette.acc(),
        ));
        right.push(Span::styled(
            progress_track(elapsed_secs, estimate),
            palette.faint(),
        ));
        right.push(Span::styled(" ".to_string(), Style::default()));
        right.push(Span::styled(
            format!("{} of ~{}", short_dur(elapsed_secs), short_dur(estimate)),
            palette.dim(),
        ));
    } else {
        right.push(Span::styled(short_dur(elapsed_secs), palette.dim()));
    }
    let (left, right) = header_parts(number, prefix, left, right, width);
    two_part_row(left, right, width)
}

/// The live bar's filled cells: sixteen blocks by elapsed share, capped.
fn progress_bar(elapsed_secs: u64, estimate_secs: u64) -> String {
    "█".repeat(progress_filled(elapsed_secs, estimate_secs))
}

/// The live bar's remaining cells.
fn progress_track(elapsed_secs: u64, estimate_secs: u64) -> String {
    const CELLS: usize = 16;
    "░".repeat(CELLS.saturating_sub(progress_filled(elapsed_secs, estimate_secs)))
}

fn progress_filled(elapsed_secs: u64, estimate_secs: u64) -> usize {
    const CELLS: usize = 16;
    let share = elapsed_secs as f64 / estimate_secs.max(1) as f64;
    ((share * CELLS as f64).round() as usize).min(CELLS)
}

/// A step parked for clearance (Mission Control spec r5):
/// `01 ◉ Shell  wc -l src/wrap.rs  holding for clearance`. The head of the
/// queue reads `◉`; steps queued behind it read `○` with `next`.
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
        (
            status::RUNNING_STR,
            "holding for clearance".to_string(),
            palette.warn(),
        )
    } else {
        (status::TODO_STR, "next".to_string(), palette.dim())
    };
    let number = vec![Span::styled(glyph.to_string(), palette.warn())];
    let prefix = vec![Span::styled(format!("{step:02} "), bold(palette.gold()))];
    let mut left = vec![
        Span::styled(tool.to_string(), bold(palette.ink())),
        Span::styled("  ".to_string(), Style::default()),
    ];
    left.extend(with_ink(hoocode_tui_ratatui::ansi_spans(subject), palette));
    let right = vec![Span::styled(right_text, right_style)];
    let (left, right) = header_parts(number, prefix, left, right, width);
    two_part_row(left, right, width)
}

/// A live subagent step (Mission Control spec r7):
/// `01 ◉ Agent  ◈ general-purpose · Compute a product  2s`.
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
    let number = vec![Span::styled(status::RUNNING_STR.to_string(), palette.acc())];
    let prefix = vec![Span::styled(format!("{step:02} "), bold(palette.gold()))];
    let mut left = vec![
        Span::styled("Agent".to_string(), bold(palette.ink())),
        Span::styled("  ".to_string(), Style::default()),
        Span::styled("◈".to_string(), bold(palette.vio())),
        Span::styled(" ".to_string(), Style::default()),
        Span::styled(subtype.to_string(), palette.ink()),
        Span::styled(" · ".to_string(), palette.dim()),
    ];
    left.extend(with_ink(hoocode_tui_ratatui::ansi_spans(task), palette));
    let right = vec![Span::styled(short_dur(elapsed_secs), palette.acc())];
    let (left, right) = header_parts(number, prefix, left, right, width);
    two_part_row(left, right, width)
}

/// A live background job (Mission Control spec r9):
/// `01 ◉ Shell  sleep 60  ⌁ sat job-1 · in orbit 4s`.
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
    let number = vec![Span::styled(status::RUNNING_STR.to_string(), palette.acc())];
    let prefix = vec![Span::styled(format!("{step:02} "), bold(palette.gold()))];
    let mut left = vec![
        Span::styled(tool.to_string(), bold(palette.ink())),
        Span::styled("  ".to_string(), Style::default()),
    ];
    left.extend(with_ink(hoocode_tui_ratatui::ansi_spans(subject), palette));
    let right = vec![
        Span::styled("⌁".to_string(), palette.acc()),
        Span::styled(format!(" sat {job} "), palette.acc()),
        Span::styled("·".to_string(), palette.dim()),
        Span::styled(
            format!(" in orbit {}", short_dur(elapsed_secs)),
            palette.acc(),
        ),
    ];
    let (left, right) = header_parts(number, prefix, left, right, width);
    two_part_row(left, right, width)
}

/// Thinking's line under its mission (Mission Control spec r11):
/// `✻ thought 3s · The helper is 13 lines…`. The gist reads dim after
/// the violet thought; without one the row is the thought alone.
pub fn think_row(secs: u64, gist: Option<&str>, palette: &Palette, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let mut spans = vec![Span::styled(
        format!("✻ thought {}", short_dur(secs)),
        palette.vio(),
    )];
    if let Some(gist) = gist.filter(|gist| !gist.trim().is_empty()) {
        spans.push(Span::styled(" · ".to_string(), palette.faint()));
        spans.push(Span::styled(gist.to_string(), palette.dim()));
    }
    row(spans, width)
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

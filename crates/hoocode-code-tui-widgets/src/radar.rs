//! Mission Control flight log (radar) rows (M5, spec §4 r1–r11).
//!
//! In radar view each mission collapses to one flight-log row: the mark
//! (`●` landed, `◐` in flight, `⊗` did not land, `⊘` aborted), the prompt,
//! and the stats (steps, `N✕` hits, time, cost). The stats stay whole; the
//! prompt is cut with `…` first. Under the mission row, each hit gets its
//! own line (`⊗ 04 Shell  cmd  → snippet  HIT`), a flying mission shows its
//! live step (elapsed only, no progress bar), and thinking shows
//! `✻ thought Ns` with no gist.
//!
//! Like the trajectory, every row draws through a ratatui buffer and the
//! Mission Control palette. Colours keep the legacy theme tokens the peek
//! view styled with, so both views stay in step.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

use hoocode_tui_ratatui::core::style::Style;
use hoocode_tui_ratatui::core::text::Span;
use hoocode_tui_ratatui::{bold, Palette};
use hoocode_tui_render::{Component, ComponentHandle};

use crate::trajectory::{first_line, header_parts, short_dur, steps_text};
use crate::trajectory::{row_line, two_part_row};

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
            FlightMark::Landed => "●",
            FlightMark::InFlight => "◐",
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

    /// The stats, muted with dim separators, as the landing line joins them.
    /// A stepless mission omits `0 steps 0✕` entirely: the row reads the
    /// mark, the prompt, then the time and cost when known.
    fn spans(&self, palette: &Palette) -> Vec<Span<'static>> {
        let mut parts = Vec::new();
        if self.steps > 0 || self.hits > 0 {
            parts.push(Span::styled(steps_text(self.steps), palette.faint()));
            parts.push(Span::styled(format!("{}✕", self.hits), palette.faint()));
        }
        if let Some(time) = self.time_text() {
            parts.push(Span::styled(time, palette.faint()));
        }
        if let Some(cost) = &self.cost {
            parts.push(Span::styled(cost.clone(), palette.faint()));
        }
        let mut out = Vec::with_capacity(parts.len() * 2);
        for (i, part) in parts.drain(..).enumerate() {
            if i > 0 {
                out.push(Span::styled(" · ".to_string(), palette.dim()));
            }
            out.push(part);
        }
        out
    }
}

/// The mission row: `M1 ●  prompt  1 step · 0✕ · 0.8s · $0.003`. The stats
/// stay whole; the prompt is cut with `…` first. `prompt` is one line.
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
    // The legacy `agent2` mission colour, as in the peek header.
    let number = vec![Span::styled(
        format!("M{seq}"),
        bold(palette.style("agent2")),
    )];
    let prefix = vec![
        Span::styled(stats.mark.glyph().to_string(), stats.mark.style(palette)),
        Span::styled(" ".to_string(), Style::default()),
    ];
    let left = vec![Span::styled(first_line(prompt), Style::default())];
    let (left, right) = header_parts(number, prefix, left, stats.spans(palette), width);
    two_part_row(left, right, width)
}

/// A hit's own line: `⊗ 04 Shell  echo boom; exit 3  → boom  HIT`. `step`
/// is the mission-relative step number. The `HIT` chip stays whole; the
/// tool and subject are cut with `…` first. An empty snippet drops the
/// `→` part.
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
    let prefix = vec![Span::styled(format!("{step:02} "), bold(palette.warn()))];
    // `toolTitle` is "" in the theme: chalk's default opener, as in the
    // peek step headers.
    let mut left = vec![
        Span::styled(tool.to_string(), Style::default()),
        Span::styled("  ".to_string(), Style::default()),
    ];
    // The subject and snippet keep whatever styling they arrived with
    // (a cut subject carries its resets); plain text stays default.
    left.extend(hoocode_tui_ratatui::ansi_spans(subject));
    if !snippet.trim().is_empty() {
        left.push(Span::styled("  →  ".to_string(), palette.dim()));
        left.extend(hoocode_tui_ratatui::ansi_spans(snippet));
    }
    let right = vec![Span::styled("HIT".to_string(), bold(palette.err()))];
    let (left, right) = header_parts(number, prefix, left, right, width);
    two_part_row(left, right, width)
}

/// A live step's line: `◐ 01 Shell  sleep 6; echo done  3s`. Elapsed only,
/// no progress bar (design Q6 item 7).
pub fn live_row(
    step: usize,
    tool: &str,
    subject: &str,
    elapsed_secs: u64,
    palette: &Palette,
    width: usize,
) -> String {
    if width == 0 {
        return String::new();
    }
    let number = vec![Span::styled("◐".to_string(), palette.warn())];
    let prefix = vec![Span::styled(format!("{step:02} "), bold(palette.warn()))];
    // `toolTitle` is "" in the theme: chalk's default opener, as in the
    // peek step headers.
    let mut left = vec![
        Span::styled(tool.to_string(), Style::default()),
        Span::styled("  ".to_string(), Style::default()),
    ];
    left.extend(hoocode_tui_ratatui::ansi_spans(subject));
    let right = vec![Span::styled(short_dur(elapsed_secs), palette.dim())];
    let (left, right) = header_parts(number, prefix, left, right, width);
    two_part_row(left, right, width)
}

/// Thinking's line under its mission: `✻ thought 3s`, no gist (M5).
pub fn think_row(secs: u64, palette: &Palette, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    row_line(
        vec![Span::styled(
            format!("✻ thought {}", short_dur(secs)),
            palette.style("thinkingText"),
        )],
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

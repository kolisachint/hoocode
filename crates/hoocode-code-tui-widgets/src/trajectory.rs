//! Mission Control trajectory (peek) helpers (M4, spec §4 b1–b7, a2, a3).
//!
//! The peek view draws every mission as a trajectory: a `▲ M1` mission
//! header, numbered steps on a `┊` rail (an agent's sub-steps keep their
//! violet `┃` rail), and one `● LANDED` line closing the mission.
//!
//! Every row is drawn through a ratatui buffer (`hoocode-tui-ratatui`):
//! fresh chrome goes through the Mission Control palette, block bodies
//! still arrive as chalk-styled strings and are parsed back into spans, so
//! the goldens stay identical. Colours keep the legacy theme tokens M4
//! styled with (via the palette's `from_token`); the palette's newer MC
//! tokens (`you`, `vio`, `gold`) are a later recolor.

use std::cell::RefCell;
use std::rc::Rc;

use hoocode_tui_glyphs::status;
use hoocode_tui_ratatui::core::buffer::Buffer;
use hoocode_tui_ratatui::core::layout::Rect;
use hoocode_tui_ratatui::core::style::{Color, Style};
use hoocode_tui_ratatui::core::text::{Line, Span};
use hoocode_tui_ratatui::{
    ansi_spans, bold, italic, paint, reversed, row, spans_width, truncate_spans, Palette,
    RatatuiComponent, RowCache,
};
use hoocode_tui_render::Component;

use crate::radar::FlightMark;
use crate::tool_output_view::ToolOutputView;
#[cfg(test)]
use hoocode_tui_util::visible_width;
use hoocode_tui_util::{strip_vt_control_characters, wrap_text_with_ansi};

/// Draws one row of `spans` through a ratatui buffer exactly `width`
/// wide, clipping (never truncating) at the edge. For rows laid out
/// elsewhere (wrapping components): unlike [`row`], no `…`.
pub(crate) fn draw_line(spans: Vec<Span<'static>>, width: usize) -> String {
    let width = u16::try_from(width).unwrap_or(u16::MAX).max(1);
    RatatuiComponent::new(1, move |area: Rect, buf: &mut Buffer| {
        buf.set_line(area.x, area.y, &Line::from(spans.clone()), area.width);
    })
    .render(width)
    .pop()
    .unwrap_or_default()
}

/// Draws `left` flush left and `right` flush right through a ratatui
/// buffer. The gap between them stays blank cells.
pub(crate) fn two_part_row(
    left: Vec<Span<'static>>,
    right: Vec<Span<'static>>,
    width: usize,
) -> String {
    let width = u16::try_from(width).unwrap_or(u16::MAX).max(1);
    RatatuiComponent::new(1, move |area: Rect, buf: &mut Buffer| {
        let room = usize::from(area.width);
        let right_width = spans_width(&right).min(room);
        let x = (room - right_width) as u16;
        let left = truncate_spans(left.clone(), usize::from(x), "…");
        buf.set_line(area.x, area.y, &Line::from(left), area.width);
        buf.set_line(
            area.x + x,
            area.y,
            &Line::from(right.clone()),
            area.width.saturating_sub(x),
        );
    })
    .render(width)
    .pop()
    .unwrap_or_default()
}

/// A full-width band row (Mission Control `▲ M1` mission headers, spec
/// a2–a4): `left` flush left, `right` flush right, every cell — including
/// the gap — on `bg`. `bg` is a background token's colour; `None` keeps
/// the row transparent (colour off, or a theme without the token).
pub fn band_row(
    left: Vec<Span<'static>>,
    right: Vec<Span<'static>>,
    width: usize,
    bg: Option<Color>,
) -> String {
    let mut right = truncate_spans(right, width, "…");
    let right_width = spans_width(&right);
    let left_budget = width
        .saturating_sub(right_width)
        .saturating_sub(usize::from(!right.is_empty()));
    let mut left = truncate_spans(left, left_budget, "…");
    let gap = width.saturating_sub(spans_width(&left) + right_width);
    // Every span shares the band: merges into each style, so resets the
    // text arrived with keep the fill.
    if let Some(bg) = bg {
        for span in left.iter_mut().chain(right.iter_mut()) {
            span.style.bg = Some(bg);
        }
    }
    left.push(Span::styled(
        " ".repeat(gap.max(usize::from(!right.is_empty()))),
        Style::default().bg(bg.unwrap_or(Color::Reset)),
    ));
    left.extend(right);
    row(left, width)
}

/// How many output lines of a step the rail shows before the fold.
pub const TRAJECTORY_BODY_LINES: usize = 3;

/// The trajectory rail: every step row and body line sits on it.
pub const RAIL: &str = "┊";

/// An agent's sub-steps keep their own rail, in violet.
pub const AGENT_RAIL: &str = "┃";

/// The slot thinking takes: it is not a tool call, so it takes no step number.
pub const THINK_SLOT: &str = "··";

/// A step number, 1-based and gold: `01` (Mission Control spec b1).
pub fn step_number(index: usize, palette: &Palette) -> String {
    paint(bold(palette.gold()), &format!("{index:02}"))
}

/// The rail prefix a body line sits on: `   ┊  ` (or the agent `┃`), as
/// spans. An agent's sub-steps keep the violet rail (Mission Control spec
/// b5).
fn rail_prefix_spans(agent: bool, palette: &Palette) -> Vec<Span<'static>> {
    let (rail, style) = if agent {
        (AGENT_RAIL, palette.vio())
    } else {
        (RAIL, palette.faint())
    };
    vec![
        Span::styled("   ".to_string(), Style::default()),
        Span::styled(rail.to_string(), style),
        Span::styled("  ".to_string(), Style::default()),
    ]
}

/// The container padding (two cells) past leading styling: block lines are
/// already width-padded and styled, so the indent hides among the ANSI
/// openers and padding spaces trail at the end. Returns the styling prefix
/// and the line without its padding, so the rail keeps the line's colours.
/// Step headers split the same way: the number goes first, then the styling.
// The slices below cut at the ASCII `m` ending an SGR opener, which is
// always a char boundary.
#[allow(clippy::string_slice)]
/// Trailing padding of a width-padded block line, ignoring the styling:
/// spaces go, and the SGR resets around them are re-attached after, so the
/// line still resets its colours. Byte-safe: the scan only walks ASCII
/// (`m`, digits, `;`, `\x1b[`).
pub(crate) fn trim_end_styled(line: &str) -> String {
    use hoocode_tui_util::text_slice as ts;
    let mut s = line.trim_end_matches(' ');
    let mut tail = String::new();
    loop {
        let bytes = s.as_bytes();
        if !bytes.ends_with(b"m") {
            break;
        }
        let mut i = bytes.len() - 1;
        while i > 0 && (bytes[i - 1].is_ascii_digit() || bytes[i - 1] == b';') {
            i -= 1;
        }
        if !(i >= 2 && bytes[i - 1] == b'[' && bytes[i - 2] == 0x1b) {
            break;
        }
        let esc = i - 2;
        tail = format!("{}{tail}", ts::suffix_from(s, esc));
        s = ts::range(s, 0, esc);
        s = s.trim_end_matches(' ');
    }
    format!("{s}{tail}")
}

// The slices below cut at ASCII boundaries only: the `m` ending an SGR
// opener, and one ASCII space of container padding.
#[allow(clippy::string_slice)]
pub fn split_padding(line: &str) -> (String, String) {
    let trimmed = trim_end_styled(line);
    let mut prefix = String::new();
    let mut rest = trimmed.as_str();
    let mut spaces = 0;
    loop {
        if let Some(after) = rest.strip_prefix("\x1b[") {
            if let Some(end) = after.find('m') {
                let len = 2 + end + 1;
                prefix.push_str(&rest[..len]);
                rest = &rest[len..];
                continue;
            }
            break;
        }
        if spaces < 2 && rest.starts_with(' ') {
            rest = &rest[1..];
            spaces += 1;
            continue;
        }
        break;
    }
    (prefix, rest.to_string())
}

/// `line` on the rail, cut to `width` columns. Container padding (two
/// cells) is dropped first, so relative indent inside the line survives; the
/// line keeps its own styling, only the rail is added. A blank line keeps
/// the rail bare, so the rail reads unbroken.
pub fn rail_line(line: &str, width: usize, agent: bool, palette: &Palette) -> String {
    if strip_vt_control_characters(line).trim().is_empty() {
        let spans = rail_prefix_spans(agent, palette);
        return draw_line(truncate_spans(spans, width, ""), width);
    }
    // The openers the split kept belong to the content that follows: one
    // parse keeps them on it.
    let (prefix, content) = split_padding(line);
    let mut spans = rail_prefix_spans(agent, palette);
    spans.extend(ansi_spans(&format!("{prefix}{content}")));
    row(spans, width)
}

/// Whether a rendered line is a thinking-slot header (`·· …`): the prose
/// rail skips those, they carry their own slot.
pub fn is_think_header(line: &str) -> bool {
    strip_vt_control_characters(line)
        .trim_start()
        .starts_with(THINK_SLOT)
}

/// The fold under a step with more output: `   ┊  … N more lines`.
pub fn fold_line(hidden: usize, width: usize, agent: bool, palette: &Palette) -> String {
    let mut spans = rail_prefix_spans(agent, palette);
    spans.push(crate::fold::more_lines_span(hidden, palette));
    row(spans, width)
}

/// The chip replacing a failed step's success stats: the inverted
/// ` HIT ` block (Mission Control spec b2: err fill, bold).
pub fn hit_chip(palette: &Palette) -> String {
    format!(" {}", paint(reversed(bold(palette.err())), " HIT "))
}

/// The chip on a step that wrote a new file: ` NEW`, in green.
pub fn new_chip(palette: &Palette) -> String {
    format!(" {}", paint(bold(palette.ok()), "NEW"))
}

/// A background job's row, in place of its stats: `⌁ sat job-1 · in orbit
/// 4s` (Mission Control spec r9). Without an elapsed time the tail is
/// dropped.
pub fn satellite_text(job: &str, secs: Option<u64>, palette: &Palette) -> String {
    let mut out = format!(
        "{} sat {} {} in orbit",
        paint(palette.acc(), "⌁"),
        paint(palette.acc(), job),
        paint(palette.dim(), "·"),
    );
    if let Some(secs) = secs {
        out.push_str(&format!(" {}", paint(palette.acc(), &short_dur(secs))));
    }
    out
}

/// A short duration: `4s`, `1m12s`.
pub fn short_dur(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else {
        format!("{}m{}s", secs / 60, secs % 60)
    }
}

/// A Think row (spec §4 a2): the `··` slot with the trace's gist. Running
/// reads `·· ◉ Think  gist  ✻ Ns` with the time right-aligned; done reads
/// `·· ● Think  gist  ✻ thought Ns · ctrl+t`. Unknown time (a resumed
/// session never timed the trace) is omitted; no gist keeps the header bare.
pub struct ThinkHeader {
    running: bool,
    secs: Option<u64>,
    gist: Option<String>,
    cache: RefCell<RowCache>,
}

impl ThinkHeader {
    pub fn new(running: bool, secs: Option<u64>) -> Self {
        Self {
            running,
            secs,
            gist: None,
            cache: RefCell::new(RowCache::default()),
        }
    }

    /// The trace's gist (its first line): faint italic after `Think`.
    /// Clears the settled cache, so a gist set before the first render
    /// is what the header keeps.
    pub fn set_gist(&mut self, gist: Option<String>) {
        if self.gist != gist {
            self.gist = gist;
            self.cache.borrow_mut().clear();
        }
    }
}

impl Component for ThinkHeader {
    fn render(&mut self, width: u16) -> Vec<String> {
        let theme = hoocode_tui_ratatui::current_theme_key();
        // A done header never changes: cache it. A running header reads a
        // live clock, so it redraws on every tick.
        if !self.running {
            if let Some(cached) = self.cache.borrow().get(width, &theme) {
                return cached.to_vec();
            }
        }
        let palette = Palette::current();
        if width == 0 {
            return Vec::new();
        }
        let width = width as usize;
        let slot = Span::styled(THINK_SLOT.to_string(), palette.gold());
        // The trace's gist, faint italic between `Think` and the right time.
        let gist: Vec<Span<'static>> = self.gist.as_deref().map_or_else(Vec::new, |gist| {
            vec![
                Span::styled("  ".to_string(), Style::default()),
                Span::styled(gist.to_string(), italic(palette.faint())),
            ]
        });
        if self.running {
            let mut left = vec![
                slot,
                Span::styled(" ".to_string(), Style::default()),
                Span::styled(status::RUNNING_STR.to_string(), palette.style("warning")),
                Span::styled(" ".to_string(), Style::default()),
                Span::styled("Think".to_string(), bold(palette.ink())),
            ];
            left.extend(gist);
            let Some(secs) = self.secs else {
                return vec![row(left, width)];
            };
            let right = vec![Span::styled(
                format!("✻ {}", short_dur(secs)),
                palette.vio(),
            )];
            let gap = width.saturating_sub(spans_width(&left) + spans_width(&right) + 2);
            let mut spans = left;
            spans.push(Span::styled(" ".repeat(gap + 2), Style::default()));
            spans.extend(right);
            return vec![row(spans, width)];
        }
        let thought = match self.secs {
            Some(s) => format!("✻ thought {}", short_dur(s)),
            None => "✻ thought".to_string(),
        };
        let mut spans = vec![
            slot,
            Span::styled(" ".to_string(), Style::default()),
            Span::styled(status::DONE_STR.to_string(), palette.ok()),
            Span::styled(" ".to_string(), Style::default()),
            Span::styled("Think".to_string(), bold(palette.ink())),
        ];
        spans.extend(gist);
        spans.extend([
            Span::styled("  ".to_string(), Style::default()),
            Span::styled(thought, palette.style("thinkingText")),
            Span::styled(" ".to_string(), Style::default()),
            Span::styled("·".to_string(), palette.dim()),
            Span::styled(" ".to_string(), Style::default()),
            // `text` is "" in the theme: chalk's default opener.
            Span::styled("ctrl+t".to_string(), Style::default()),
        ]);
        let lines = vec![row(spans, width)];
        let key = u16::try_from(width).unwrap_or(u16::MAX);
        self.cache.borrow_mut().set(key, theme, lines.clone());
        lines
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

/// The `⊗` a failed step carries in place of its status dot (spec §4 b2).
/// Only the leading dot is swapped; its error styling stays.
pub fn failure_dot(content: &str) -> String {
    content.replacen('●', "⊗", 1)
}

/// `N lines`.
pub fn lines_text(total: usize) -> String {
    if total == 1 {
        "1 line".to_string()
    } else {
        format!("{total} lines")
    }
}

/// `N files`.
pub fn files_text(total: usize) -> String {
    if total == 1 {
        "1 file".to_string()
    } else {
        format!("{total} files")
    }
}

/// A header row split into its flush-left and flush-right runs:
/// ` {number} {prefix}{left}` left, `right` (the stats) right. The label is
/// cut with `…` first and the stats stay whole; the stats are cut only when
/// they fit nowhere.
pub fn header_parts(
    number: Vec<Span<'static>>,
    left_prefix: Vec<Span<'static>>,
    left: Vec<Span<'static>>,
    right: Vec<Span<'static>>,
    width: usize,
) -> (Vec<Span<'static>>, Vec<Span<'static>>) {
    let mut stem = vec![Span::styled(" ".to_string(), Style::default())];
    stem.extend(number);
    stem.push(Span::styled(" ".to_string(), Style::default()));
    stem.extend(left_prefix);
    stem.extend(left);
    if right.is_empty() {
        return (truncate_spans(stem, width, "…"), Vec::new());
    }
    let gap = 2;
    let pad = width.saturating_sub(spans_width(&stem) + spans_width(&right));
    if pad >= gap {
        return (stem, right);
    }
    let budget = width.saturating_sub(spans_width(&right) + gap);
    if budget == 0 {
        return (Vec::new(), truncate_spans(right, width, "…"));
    }
    (truncate_spans(stem, budget, "…"), right)
}

/// A step header row at exactly the row width: ` NN {left}` flush left,
/// `right` (the stats) flush right. The label is cut with `…` first and the
/// stats stay whole; the stats are cut only when they fit nowhere.
pub fn step_header(
    number: &str,
    left_prefix: &str,
    left: &str,
    right: &str,
    width: usize,
) -> String {
    if width == 0 {
        return String::new();
    }
    // The openers the split kept belong to the label that follows: one
    // parse keeps them on it.
    let (left, right) = header_parts(
        ansi_spans(number),
        Vec::new(),
        ansi_spans(&format!("{left_prefix}{left}")),
        ansi_spans(right),
        width,
    );
    two_part_row(left, right, width)
}

/// Columns a wrapped header row hangs in under its first row: past the
/// status dot.
const HEADER_HANG: usize = 2;
/// Room the label keeps beside the stats before the stats move to a row of
/// their own.
const MIN_LABEL_BESIDE_STATS: usize = 16;

/// A step header as rows, never cut: ` NN {label}` with `right` (the stats)
/// flush right on the first row. The label wraps (a continuation row hangs
/// under the text, past the status dot) instead of ending in `…`. When the
/// stats leave the label too little room beside them, the label takes the
/// full row and the stats close the header on the last row that has room,
/// or on a row of their own. `label_bg` paints the label (a file tool's band).
pub fn step_header_rows(
    number: &str,
    left_prefix: &str,
    left: &str,
    right: &str,
    width: usize,
    label_bg: Option<Color>,
) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let number = ansi_spans(number);
    let stem = 1 + spans_width(&number) + 1;
    let stats = ansi_spans(right);
    let stats_width = spans_width(&stats);
    let label = format!("{left_prefix}{left}");
    let full = width.saturating_sub(stem).max(1);
    let beside = (stats_width > 0 && full >= stats_width + 2 + MIN_LABEL_BESIDE_STATS)
        .then(|| full - stats_width - 2);
    let avail = beside.unwrap_or(full);
    let mut wrapped = wrap_text_with_ansi(&label, avail);
    if wrapped.len() > 1 {
        // Continuation rows hang in; wrap again at the width they have.
        wrapped = wrap_text_with_ansi(&label, avail.saturating_sub(HEADER_HANG).max(1));
    }
    let count = wrapped.len();
    // The row the stats sit on: the first when they fit beside the label,
    // else the last row with room for them, else a row of their own.
    let stats_row = if stats_width == 0 {
        None
    } else if beside.is_some() {
        Some(0)
    } else if hoocode_tui_util::visible_width(&wrapped[count - 1]) + 2 + stats_width <= full {
        Some(count - 1)
    } else {
        Some(count)
    };
    let mut rows = Vec::with_capacity(count + 1);
    for (index, piece) in wrapped.into_iter().enumerate() {
        let mut left_spans = if index == 0 {
            let mut spans = vec![Span::styled(" ".to_string(), Style::default())];
            spans.extend(number.clone());
            spans.push(Span::styled(" ".to_string(), Style::default()));
            spans
        } else {
            vec![Span::styled(
                " ".repeat(stem + HEADER_HANG),
                Style::default(),
            )]
        };
        let mut label = ansi_spans(&piece);
        if let Some(bg) = label_bg {
            for span in &mut label {
                span.style.bg = Some(bg);
            }
        }
        left_spans.extend(label);
        rows.push(if stats_row == Some(index) {
            two_part_row(left_spans, stats.clone(), width)
        } else {
            row(left_spans, width)
        });
    }
    if stats_row == Some(count) {
        rows.push(two_part_row(Vec::new(), stats, width));
    }
    rows
}

/// `1 step`, `N steps`.
pub fn steps_text(steps: usize) -> String {
    if steps == 1 {
        "1 step".to_string()
    } else {
        format!("{steps} steps")
    }
}

/// `1 hit`, `N hits`.
pub fn hits_text(hits: usize) -> String {
    if hits == 1 {
        "1 hit".to_string()
    } else {
        format!("{hits} hits")
    }
}

/// The landing line's spans: `● LANDED · 2 steps · 1 hit · …`, muted with
/// dim separators, as the row joins them. The mark is the mission's outcome,
/// the one the radar row and the comms title read: `● LANDED` (ok),
/// `⊗ DID NOT LAND` (err) or `⊘ ABORTED` (dim). A stepless mission (no steps,
/// no hits) leaves those counts out.
pub fn landing_spans(
    mark: FlightMark,
    steps: usize,
    hits: usize,
    secs: &str,
    tokens: &str,
    cost: &str,
    palette: &Palette,
) -> Vec<Span<'static>> {
    let word_style = match mark {
        FlightMark::Aborted => mark.style(palette),
        _ => bold(mark.style(palette)),
    };
    let mut out = vec![
        Span::styled(mark.glyph().to_string(), mark.style(palette)),
        Span::styled(" ".to_string(), Style::default()),
        Span::styled(mark.word().to_string(), word_style),
    ];
    let mut pieces = Vec::new();
    if steps > 0 || hits > 0 {
        pieces.push(steps_text(steps));
        pieces.push(hits_text(hits));
    }
    pieces.extend([secs.to_string(), tokens.to_string(), cost.to_string()]);
    for piece in pieces {
        out.push(Span::styled(" · ".to_string(), palette.dim()));
        out.push(Span::styled(piece, palette.faint()));
    }
    out
}

/// The landing line ending every mission: steps, hits, time, tokens, cost.
pub fn landing_line(
    mark: FlightMark,
    steps: usize,
    hits: usize,
    secs: &str,
    tokens: &str,
    cost: &str,
    palette: &Palette,
) -> String {
    // Unbounded: the row below (`LandingLine`) lays the spans out at the
    // render width. The spans carry no truncation, so this matches the
    // laid-out row cell for cell whenever it fits.
    let spans = landing_spans(mark, steps, hits, secs, tokens, cost, palette);
    let width = spans_width(&spans);
    row(spans, width.max(1))
}

/// A mission's landing line as a component (M5): the peek row the
/// transcript refreshes when a landed mission's counts change, so peek
/// and radar agree. The spans render at the render width; overlong rows
/// wrap exactly the way the old padded `Text` did. A mission that did
/// something also gets one dim line right after, saying what the turn
/// amounted to (`Edited 3 files · ran 2 commands`).
#[derive(Debug)]
pub struct LandingLine {
    mark: FlightMark,
    steps: usize,
    hits: usize,
    secs: String,
    tokens: String,
    cost: String,
    summary: Option<String>,
    cache: RefCell<RowCache>,
}

impl LandingLine {
    pub fn new(
        mark: FlightMark,
        steps: usize,
        hits: usize,
        secs: &str,
        tokens: &str,
        cost: &str,
    ) -> Self {
        Self {
            mark,
            steps,
            hits,
            secs: secs.to_string(),
            tokens: tokens.to_string(),
            cost: cost.to_string(),
            summary: None,
            cache: RefCell::new(RowCache::default()),
        }
    }

    /// The counts the row reads. The transcript calls this when a landed
    /// mission's tally moves; a change drops the cached render.
    pub fn set_counts(&mut self, steps: usize, hits: usize) {
        if self.steps != steps || self.hits != hits {
            self.steps = steps;
            self.hits = hits;
            self.cache.borrow_mut().clear();
        }
    }

    /// What the turn amounted to, the line after the landing row. `None`
    /// (or blank) draws no line.
    pub fn set_summary(&mut self, summary: Option<String>) {
        let summary = summary.filter(|text| !text.trim().is_empty());
        if self.summary != summary {
            self.summary = summary;
            self.cache.borrow_mut().clear();
        }
    }

    /// The counts the row reads, for the peek/radar agreement check.
    pub fn counts(&self) -> (usize, usize) {
        (self.steps, self.hits)
    }

    /// The outcome the row shows.
    pub fn mark(&self) -> FlightMark {
        self.mark
    }

    /// The summary line, when there is one.
    pub fn summary(&self) -> Option<&str> {
        self.summary.as_deref()
    }
}

impl Component for LandingLine {
    fn render(&mut self, width: u16) -> Vec<String> {
        let theme = hoocode_tui_ratatui::current_theme_key();
        if let Some(cached) = self.cache.borrow().get(width, &theme) {
            return cached.to_vec();
        }
        let palette = Palette::current();
        // One leading cell, as the old padded `Text` drew.
        let mut spans = vec![Span::styled(" ".to_string(), Style::default())];
        spans.extend(landing_spans(
            self.mark,
            self.steps,
            self.hits,
            &self.secs,
            &self.tokens,
            &self.cost,
            &palette,
        ));
        let room = spans_width(&spans).max(1);
        let full = draw_line(spans, room);
        let content_width = (width as usize).saturating_sub(2).max(1);
        let mut lines: Vec<String> = hoocode_tui_util::wrap_text_with_ansi(&full, content_width)
            .into_iter()
            .map(|line| format!(" {line} "))
            .collect();
        if let Some(summary) = &self.summary {
            // Dim, under the word the mark makes; wrapped, never cut.
            let styled = paint(palette.dim(), summary);
            lines.extend(
                hoocode_tui_util::wrap_text_with_ansi(
                    &styled,
                    content_width.saturating_sub(2).max(1),
                )
                .into_iter()
                .map(|line| format!("   {line} ")),
            );
        }
        self.cache.borrow_mut().set(width, theme, lines.clone());
        lines
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

/// Added and removed lines in a unified diff: `+` and `-` first-column
/// lines, skipping the `+++` / `---` file headers.
pub fn diff_stats(diff: &str) -> (usize, usize) {
    let mut added = 0;
    let mut removed = 0;
    for line in diff.lines() {
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        if line.starts_with('+') {
            added += 1;
        } else if line.starts_with('-') {
            removed += 1;
        }
    }
    (added, removed)
}

/// `+1 −1`: the changed-line counts on an Edit step's header.
pub fn diff_counts_text(added: usize, removed: usize) -> String {
    format!("+{added} \u{2212}{removed}")
}

/// A band whose text is never cut: `prefix` and the first piece of `text`
/// share the first row with `right` flush right; the rest of `text` wraps
/// across continuation band rows (same `bg`, indented to the prefix width,
/// text only). Every `\n` in `text` starts a row. No room beside the prefix
/// (or empty text) keeps one row.
pub(crate) fn wrapped_band_rows(
    prefix: Vec<Span<'static>>,
    text: &str,
    text_style: Style,
    right: Vec<Span<'static>>,
    width: usize,
    bg: Option<Color>,
) -> Vec<String> {
    let budget = width.saturating_sub(
        spans_width(&prefix) + spans_width(&right) + usize::from(!right.is_empty()),
    );
    if budget == 0 || text.is_empty() {
        let mut spans = prefix;
        spans.push(Span::styled(text.to_string(), text_style));
        return vec![band_row(spans, right, width, bg)];
    }
    let mut wrapped: Vec<String> = Vec::new();
    for source in text.split('\n') {
        wrapped.extend(wrap_text_with_ansi(source, budget));
    }
    let indent = " ".repeat(spans_width(&prefix));
    let mut lines = Vec::with_capacity(wrapped.len());
    for (index, piece) in wrapped.into_iter().enumerate() {
        let mut row = if index == 0 {
            prefix.clone()
        } else {
            vec![Span::styled(indent.clone(), Style::default())]
        };
        row.push(Span::styled(piece, text_style));
        let right = if index == 0 {
            right.clone()
        } else {
            Vec::new()
        };
        lines.push(band_row(row, right, width, bg));
    }
    lines
}

/// The mission header: `▲ M1  prompt`, the `▲` and `M1` in the `you` colour.
/// A skill chip (`✦ name`) sits right-aligned. The prompt is never cut: it
/// wraps across continuation band rows so the whole mission stays visible.
/// In radar view the header draws the mission's flight-log row instead (M5);
/// see `radar`.
#[derive(Debug)]
pub struct MissionHeader {
    seq: usize,
    prompt: String,
    skill: Option<String>,
    view: ToolOutputView,
    flight: Option<Rc<RefCell<crate::radar::FlightStats>>>,
    cache: RefCell<RowCache>,
}

impl MissionHeader {
    pub fn new(seq: usize, prompt: &str, skill: Option<&str>) -> Self {
        // The full prompt: peek wraps it across band rows, never cutting it.
        Self {
            seq,
            prompt: prompt.to_string(),
            skill: skill.map(str::to_string),
            view: ToolOutputView::Peek,
            flight: None,
            cache: RefCell::new(RowCache::default()),
        }
    }

    /// The view dial's current stop: peek draws the `▲ M{n}` header, radar
    /// the flight-log row.
    pub fn set_view(&mut self, view: ToolOutputView) {
        self.view = view;
        self.cache.borrow_mut().clear();
    }

    /// The mission's flight-log data, refreshed by the transcript as the
    /// mission runs and lands; the header reads it on every render.
    pub fn set_flight(&mut self, flight: Rc<RefCell<crate::radar::FlightStats>>) {
        self.flight = Some(flight);
    }
}

impl Component for MissionHeader {
    fn render(&mut self, width: u16) -> Vec<String> {
        if width == 0 {
            return Vec::new();
        }
        let palette = Palette::current();
        let theme = hoocode_tui_ratatui::current_theme_key();
        let width = width as usize;
        if self.view == ToolOutputView::Radar {
            if let Some(flight) = &self.flight {
                let stats = flight.borrow();
                // Live data behind shared state: redraw every tick.
                let mut rows =
                    crate::radar::mission_rows(self.seq, &self.prompt, &stats, &palette, width);
                // The turn's progress row sits under the whole prompt.
                rows.extend(crate::radar::chain_rows(&stats, &palette, width));
                return rows;
            }
        } else if let Some(cached) = self
            .cache
            .borrow()
            .get(u16::try_from(width).unwrap_or(u16::MAX), &theme)
        {
            // The peek header is fixed at construction: cache it.
            return cached.to_vec();
        }
        // Mission Control (spec a2–a4): the full-width `bU` band, `▲ M1`
        // in the `you` token, the prompt in bold ink.
        let bg = palette.bg_color("userMessageBg");
        let spans = vec![
            Span::styled("▲".to_string(), bold(palette.you())),
            Span::styled(" ".to_string(), Style::default()),
            Span::styled(format!("M{}", self.seq), bold(palette.you())),
            Span::styled("  ".to_string(), Style::default()),
        ];
        let chip: Vec<Span<'static>> = self
            .skill
            .as_deref()
            .map(|name| {
                vec![
                    Span::styled("✦".to_string(), palette.acc()),
                    Span::styled(" ".to_string(), Style::default()),
                    Span::styled(name.to_string(), palette.acc()),
                ]
            })
            .unwrap_or_default();
        let lines = wrapped_band_rows(spans, &self.prompt, bold(palette.ink()), chip, width, bg);
        let key = u16::try_from(width).unwrap_or(u16::MAX);
        self.cache.borrow_mut().set(key, theme, lines.clone());
        lines
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_numbers_are_gold_and_two_wide() {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        let palette = Palette::current();
        assert!(step_number(1, &palette).contains("01"));
        assert!(step_number(12, &palette).contains("12"));
    }

    #[test]
    fn diff_stats_skip_file_headers() {
        assert_eq!(diff_stats("--- a\n+++ b\n-old\n+new\n ctx\n"), (1, 1));
        assert_eq!(diff_stats(""), (0, 0));
    }

    #[test]
    fn landing_line_counts_read_right() {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        let palette = Palette::current();
        let line = landing_line(
            FlightMark::Landed,
            2,
            1,
            "0.5s",
            "0.8k tokens",
            "$0.002",
            &palette,
        );
        let bare = strip_vt_control_characters(&line);
        assert!(bare.contains("LANDED"), "{bare}");
        assert!(bare.contains("2 steps"), "{bare}");
        assert!(bare.contains("1 hit"), "{bare}");
        assert!(bare.contains("0.5s"), "{bare}");
    }

    #[test]
    fn step_header_right_aligns_stats_and_cuts_the_label_first() {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        // Fits: stats flush right, label whole.
        let line = step_header("01", "", "● Shell  npm test", "exit 0", 40);
        let bare = strip_vt_control_characters(&line);
        assert_eq!(
            bare,
            format!(" 01 ● Shell  npm test{}exit 0", " ".repeat(13))
        );
        assert_eq!(visible_width(&line), 40);
        // Short: the label is cut with `…`, the stats stay whole.
        let line = step_header("01", "", "● Shell  npm test", "exit 0", 24);
        let bare = strip_vt_control_characters(&line);
        assert!(bare.ends_with("exit 0"), "{bare}");
        assert!(bare.contains('…'), "{bare}");
        assert_eq!(visible_width(&line), 24);
        // No room for stats either: the stats are cut last.
        let line = step_header("01", "", "● Shell  npm test", "exit 0", 5);
        assert_eq!(visible_width(&line), 5);
    }

    #[test]
    fn failure_dot_swaps_only_the_status_dot() {
        assert_eq!(failure_dot("● Shell  ok"), "⊗ Shell  ok");
    }

    #[test]
    fn short_durations_read_like_the_spec() {
        assert_eq!(short_dur(4), "4s");
        assert_eq!(short_dur(72), "1m12s");
    }

    #[test]
    fn think_header_carries_the_time_when_known() {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        let mut running = ThinkHeader::new(true, Some(4));
        let bare = strip_vt_control_characters(&running.render(40).join("\n"));
        assert!(bare.contains("◉ Think"), "{bare}");
        assert!(bare.ends_with("✻ 4s"), "{bare}");
        let mut done = ThinkHeader::new(false, Some(72));
        let bare = strip_vt_control_characters(&done.render(40).join("\n"));
        assert!(bare.contains("● Think"), "{bare}");
        assert!(bare.contains("✻ thought 1m12s"), "{bare}");
        assert!(bare.contains("ctrl+t"), "{bare}");
        let mut unknown = ThinkHeader::new(false, None);
        let bare = strip_vt_control_characters(&unknown.render(40).join("\n"));
        assert!(bare.contains("✻ thought · ctrl+t"), "{bare}");
    }

    #[test]
    fn mission_header_names_the_mission_and_chips_the_skill() {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        let mut header = MissionHeader::new(1, "check wrap.rs", Some("review"));
        let line = header.render(100).join("\n");
        let bare = strip_vt_control_characters(&line);
        assert!(bare.contains("M1"), "{bare}");
        assert!(bare.contains("check wrap.rs"), "{bare}");
        assert!(bare.contains("review"), "{bare}");
        assert_eq!(visible_width(&line), 100);
    }

    #[test]
    fn mission_header_wraps_long_prompts_instead_of_cutting() {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        let prompt =
            "fix the overflow bug in the wrap helper so long mission prompts never get cut";
        let mut header = MissionHeader::new(1, prompt, Some("review"));
        let lines = header.render(40);
        let bare = strip_vt_control_characters(&lines.join("\n"));
        assert!(lines.len() >= 2, "{bare}");
        assert!(!bare.contains('…'), "{bare}");
        let flat = bare.split_whitespace().collect::<Vec<_>>().join(" ");
        for word in prompt.split_whitespace() {
            assert!(flat.contains(word), "{bare}");
        }
        assert!(bare.contains("review"), "{bare}");
        for line in &lines {
            assert_eq!(visible_width(line), 40);
        }
    }

    #[test]
    fn mission_header_keeps_every_prompt_line() {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        let mut header =
            MissionHeader::new(2, "first half of the mission\nsecond half of it", None);
        let lines = header.render(80);
        let bare = strip_vt_control_characters(&lines.join("\n"));
        assert!(lines.len() >= 2, "{bare}");
        assert!(!bare.contains('…'), "{bare}");
        assert!(bare.contains("first half of the mission"), "{bare}");
        assert!(bare.contains("second half of it"), "{bare}");
    }
}

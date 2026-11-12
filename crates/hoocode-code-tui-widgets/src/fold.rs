//! The one place a transcript row says "there is more": the `… N more lines`
//! hint every renderer folds with (`… N earlier lines` where the tail of the
//! output is what shows), the `…` ellipsis glyph, the fold of a block's
//! body rows that the peek rail and the trajectory share, and the line
//! budget a collapsed body shows before the hint.
//!
//! Every renderer that leaves lines out builds its hint here, so the text,
//! the colour and the glyph read the same at every site. A reader that must
//! find a renderer's own hint again (the chain's fold adds the lines the
//! renderer left out to its own count) uses [`parse_more_lines`].

use hoocode_code_tui_theme::theme;
use hoocode_tui_ratatui::core::text::Span;
use hoocode_tui_ratatui::Palette;
use hoocode_tui_util::strip_vt_control_characters;

/// The ellipsis every cut and every fold ends with (never ASCII `...`).
pub const ELLIPSIS: &str = "…";

/// How many lines of a result a collapsed body shows before the hint.
/// An open toggle shows everything: renderers get `expanded` and trim
/// nothing.
pub const PEEK_LINES: usize = 5;

/// The lines of a result a renderer shows: [`PEEK_LINES`], or all of them
/// when `expanded` (its toggle is open).
pub fn line_budget(expanded: bool) -> usize {
    if expanded {
        usize::MAX
    } else {
        PEEK_LINES
    }
}

/// The body a tool result shows while collapsed, shared by every renderer
/// that trims its own output: the first [`line_budget`] lines, styled by
/// `style`, then the dim `… N more lines` hint when lines were left out.
/// Callers put any leading newline themselves, and skip the call for an
/// empty output.
pub fn peek_block(
    expanded: bool,
    lines: &[String],
    style: impl FnOnce(&[String]) -> Vec<String>,
) -> String {
    let shown = lines.len().min(line_budget(expanded));
    peek_window(lines.len(), &lines[..shown], style)
}

/// [`peek_block`] for a caller that has built only the lines it shows: `shown`
/// is the first lines (at most the budget) of `total` lines, styled by
/// `style`.
pub fn peek_window(
    total: usize,
    shown: &[String],
    style: impl FnOnce(&[String]) -> Vec<String>,
) -> String {
    let body = style(shown).join("\n");
    if total <= shown.len() {
        return body;
    }
    format!("{body}\n{}", more_lines_hint(total - shown.len()))
}

/// `… N more lines` (`… 1 more line`), plain text.
pub fn more_lines_text(hidden: usize) -> String {
    let noun = if hidden == 1 { "line" } else { "lines" };
    format!("{ELLIPSIS} {hidden} more {noun}")
}

/// `… N earlier lines` (`… 1 earlier line`), plain text: the hint above an
/// output that shows its last lines.
pub fn earlier_lines_text(hidden: usize) -> String {
    let noun = if hidden == 1 { "line" } else { "lines" };
    format!("{ELLIPSIS} {hidden} earlier {noun}")
}

/// The hint as a themed string (dim): `… N more lines`.
pub fn more_lines_hint(hidden: usize) -> String {
    theme().fg("dim", &more_lines_text(hidden))
}

/// The hint as a ratatui span (dim), for rows drawn from spans.
pub fn more_lines_span(hidden: usize, palette: &Palette) -> Span<'static> {
    Span::styled(more_lines_text(hidden), palette.dim())
}

/// The earlier-lines hint as a themed string (dim): `… N earlier lines`.
pub fn earlier_lines_hint(hidden: usize) -> String {
    theme().fg("dim", &earlier_lines_text(hidden))
}

/// The earlier-lines hint as a ratatui span (dim), for rows drawn from spans.
pub fn earlier_lines_span(hidden: usize, palette: &Palette) -> Span<'static> {
    Span::styled(earlier_lines_text(hidden), palette.dim())
}

/// The count in a `… N more lines` or `… N earlier lines` row (styling,
/// padding and a gutter ignored), or `None` when the row is anything else.
pub fn parse_more_lines(row: &str) -> Option<usize> {
    let plain = strip_vt_control_characters(row);
    // A hint may sit behind a gutter (`  │ `) or a rail.
    let rest = plain
        .trim_start_matches(|c: char| c.is_whitespace() || matches!(c, '│' | '┃' | '┊'))
        .strip_prefix(ELLIPSIS)?
        .trim();
    ["more lines", "more line", "earlier lines", "earlier line"]
        .iter()
        .find_map(|noun| rest.strip_suffix(noun))?
        .trim()
        .parse()
        .ok()
}

/// Whether a row is the placeholder a result shows in place of an image
/// the terminal cannot draw (`[Image: image/png 100×100]`).
pub fn is_image_placeholder(row: &str) -> bool {
    strip_vt_control_characters(row)
        .trim_start()
        .starts_with("[Image: ")
}

/// Splits the image placeholder rows a result ends with (`[Image: …]`, one
/// per image the terminal cannot draw) from the text above them. A renderer
/// that trims the text shows the placeholders whole after it: they stand in
/// for an image, which is never cut.
pub fn split_image_placeholders(output: &str) -> (String, Vec<String>) {
    let lines: Vec<&str> = output.split('\n').collect();
    let keep = lines
        .iter()
        .rposition(|line| !line.starts_with("[Image: "))
        .map_or(0, |i| i + 1);
    (
        lines[..keep].join("\n"),
        lines[keep..].iter().map(|l| l.to_string()).collect(),
    )
}

/// A body folded to a row budget.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Folded {
    /// The rows to draw, in their original order.
    pub shown: Vec<String>,
    /// How many output lines the body leaves out, counting the lines the
    /// renderer itself left out (its own `… N more lines` row).
    pub hidden: usize,
}

/// Folds the body `rows` of a block to `budget` rows (`None` keeps every
/// row). The renderer's own `… N more lines` row is lifted out and its count
/// joins the fold's, so the hint is shown once and reports the true number.
/// Image placeholder rows are never folded and do not count against the
/// budget.
pub fn fold_rows(rows: Vec<String>, budget: Option<usize>) -> Folded {
    // No budget: every row stays as it is (a renderer given the full budget
    // leaves out nothing, so there is no hint of its own to add up).
    let Some(budget) = budget else {
        return Folded {
            shown: rows,
            hidden: 0,
        };
    };
    let mut hidden = 0;
    let mut shown = Vec::with_capacity(rows.len().min(budget + 1));
    let mut counted = 0;
    for row in rows {
        if let Some(n) = parse_more_lines(&row) {
            hidden += n;
        } else if is_image_placeholder(&row) {
            shown.push(row);
        } else if counted < budget {
            counted += 1;
            shown.push(row);
        } else {
            hidden += 1;
        }
    }
    Folded { shown, hidden }
}

/// [`fold_rows`] for a body that shows its last rows: the first rows go and
/// the last `budget` stay, so the hint belongs above them. Same counting: the
/// renderer's own hint joins the count and image placeholders stay.
pub fn fold_rows_tail(rows: Vec<String>, budget: Option<usize>) -> Folded {
    let Some(budget) = budget else {
        return Folded {
            shown: rows,
            hidden: 0,
        };
    };
    let mut hidden = 0;
    let mut content = Vec::with_capacity(rows.len());
    for row in rows {
        match parse_more_lines(&row) {
            Some(n) => hidden += n,
            None => content.push(row),
        }
    }
    let counted = content.iter().filter(|r| !is_image_placeholder(r)).count();
    let mut skip = counted.saturating_sub(budget);
    hidden += skip;
    let shown = content
        .into_iter()
        .filter(|row| {
            if is_image_placeholder(row) {
                return true;
            }
            if skip > 0 {
                skip -= 1;
                return false;
            }
            true
        })
        .collect();
    Folded { shown, hidden }
}

// ---- Blocks that open in place: the indicator, the focus and the ids ----
//
// One look for every block that opens (`hoocode_tui_render::FoldHost`): a
// dim `▸` after the header's last fact while the block is closed and can
// open, a dim `▾` while it is open, and nothing on a block that cannot
// open. The focused header takes the `›` gutter at column 0 and the
// selection fill (reverse video with no colour).

/// The cells the indicator takes at the end of a header: a space and the
/// glyph. A header that can open is laid out this much narrower, so the
/// indicator never drops.
pub const INDICATOR_CELLS: usize = 2;

/// The indicator's glyph: `▾` open, `▸` closed.
fn indicator_glyph(expanded: bool) -> &'static str {
    if expanded {
        hoocode_tui_glyphs::status::OPEN_STR
    } else {
        hoocode_tui_glyphs::status::CLOSED_STR
    }
}

/// The indicator as a span: a space, then the glyph in dim. It is a fact
/// about the row, so it takes the facts' colour and no state colour.
pub fn indicator_span(expanded: bool, palette: &Palette) -> Span<'static> {
    Span::styled(format!(" {}", indicator_glyph(expanded)), palette.dim())
}

/// Whether rows paint in monochrome (no colour): the focused fill is
/// reverse video instead of the selection colour.
pub(crate) fn mono() -> bool {
    !hoocode_code_tui_theme::chalk::enabled()
}

/// The header's gutter cell: `›` in accent on the focused row (bold with
/// no colour), else the blank that keeps column 0.
pub fn gutter_span(focused: bool, palette: &Palette) -> Span<'static> {
    if !focused {
        return Span::raw(" ".to_string());
    }
    let mut style = palette.acc();
    if mono() {
        style = hoocode_tui_ratatui::bold(style);
    }
    Span::styled(hoocode_tui_glyphs::status::MARKER_STR.to_string(), style)
}

/// The focused row's band across every span of a header: the selection
/// fill with colour, reverse video without.
pub fn paint_focused(spans: &mut [Span<'static>], palette: &Palette) {
    let fill = if mono() {
        hoocode_tui_ratatui::reversed(hoocode_tui_ratatui::Style::default())
    } else {
        palette.bg("paneSel")
    };
    for span in spans.iter_mut() {
        span.style = span.style.patch(fill);
    }
}

/// A rendered header row drawn focused: `›` in the gutter (column 0, which
/// the row keeps blank) and the fill across all `width` cells.
pub fn focus_row(row: &str, width: usize, palette: &Palette) -> String {
    let mut rest = hoocode_tui_ratatui::ansi_spans(row);
    // Take the blank gutter cell off the front; a row that starts with
    // text keeps it and gains the marker in front.
    if let Some(first) = rest.first_mut() {
        if let Some(tail) = first.content.strip_prefix(' ') {
            *first = Span::styled(tail.to_string(), first.style);
        }
    }
    // A row that starts with text moves right by the marker's cell: the
    // blank cells at its end make room, so no text is cut.
    let mut overflow = (1 + hoocode_tui_ratatui::spans_width(&rest)).saturating_sub(width);
    for span in rest.iter_mut().rev() {
        if overflow == 0 {
            break;
        }
        let trimmed = span.content.trim_end_matches(' ');
        let cut = (span.content.len() - trimmed.len()).min(overflow);
        if cut > 0 {
            let keep = span.content.len() - cut;
            span.content.to_mut().truncate(keep);
            overflow -= cut;
        }
        if !span.content.is_empty() {
            break;
        }
    }
    let mut spans = vec![gutter_span(true, palette)];
    spans.extend(rest);
    let used = hoocode_tui_ratatui::spans_width(&spans);
    if used < width {
        spans.push(Span::raw(" ".repeat(width - used)));
    }
    paint_focused(&mut spans, palette);
    hoocode_tui_ratatui::row(spans, width)
}

/// Whether a rendered row is a fold line (`… N more lines`, `… N earlier
/// lines`): the one body row a click opens the block from.
pub fn is_fold_line(row: &str) -> bool {
    parse_more_lines(row).is_some()
}

/// A new id for a block that opens in place and has no id of its own (a
/// tool call has its call id): `thinking-12`. Unique in the process.
pub fn next_fold_id(kind: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!("{kind}-{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

/// The glyph a paper sheet draws down its right edge: the indicator stays
/// left of it.
const SHEET_EDGE: &str = "\u{258f}";

/// One cell of a rendered row.
struct RowCell {
    text: String,
    width: usize,
    style: hoocode_tui_ratatui::Style,
}

/// A rendered header row with the indicator after its last visible cell,
/// for a block that draws on a fill (a summary's sheet): the indicator
/// takes the two blank cells after the text, so the fill, the padding and a
/// sheet's right edge stay as they were. A row too full for it is cut by
/// the cells it needs first. The row comes back `width` cells wide.
pub fn stamp_indicator(row: &str, expanded: bool, width: usize, palette: &Palette) -> String {
    let mut cells: Vec<RowCell> = Vec::new();
    for span in hoocode_tui_ratatui::ansi_spans(row) {
        for ch in span.content.chars() {
            let text = ch.to_string();
            let cell_width = hoocode_tui_util::visible_width(&text);
            cells.push(RowCell {
                text,
                width: cell_width,
                style: span.style,
            });
        }
    }
    let edge_cells = usize::from(cells.last().is_some_and(|cell| cell.text == SHEET_EDGE));
    let limit = cells.len() - edge_cells;
    let edge_width: usize = cells[limit..].iter().map(|cell| cell.width).sum();
    let room = width.saturating_sub(edge_width);
    // Where the text ends: after the last cell that shows something.
    let mut keep = cells[..limit]
        .iter()
        .rposition(|cell| !cell.text.trim().is_empty())
        .map_or(0, |at| at + 1);
    let mut column: usize = cells[..keep].iter().map(|cell| cell.width).sum();
    // Cut the text until the indicator fits before the edge.
    if column + INDICATOR_CELLS > room && keep > 0 {
        while keep > 1 && column + INDICATOR_CELLS > room {
            keep -= 1;
            column -= cells[keep].width;
        }
        // Replace the last kept cell by the ellipsis, keeping its style.
        if column + INDICATOR_CELLS > room {
            keep = 0;
        } else {
            let last = &mut cells[keep - 1];
            last.text = ELLIPSIS.to_string();
            last.width = 1;
        }
    }
    let blank_style = |cells: &[RowCell], at: usize| {
        cells
            .get(at)
            .map_or_else(Default::default, |cell| hoocode_tui_ratatui::Style {
                fg: cell.style.fg,
                bg: cell.style.bg,
                ..Default::default()
            })
    };
    let space_style = blank_style(&cells, keep);
    let mark_style = {
        let mut style = indicator_span(expanded, palette).style;
        if let Some(bg) = blank_style(&cells, keep + 1).bg.or(space_style.bg) {
            style = style.bg(bg);
        }
        style
    };
    let glyph = indicator_glyph(expanded).to_string();
    // Cells from `keep` on: the two the indicator takes are replaced, the
    // rest (padding, the edge) follow unchanged.
    let skip = (keep + INDICATOR_CELLS).min(cells.len());
    let mut out: Vec<RowCell> = cells.drain(..keep).collect();
    out.push(RowCell {
        text: " ".to_string(),
        width: 1,
        style: space_style,
    });
    out.push(RowCell {
        text: glyph,
        width: 1,
        style: mark_style,
    });
    let rest: Vec<RowCell> = cells.drain(skip.saturating_sub(keep)..).collect();
    out.extend(rest);
    let mut spans: Vec<Span<'static>> = Vec::new();
    for cell in out {
        match spans.last_mut() {
            Some(last) if last.style == cell.style => last.content.to_mut().push_str(&cell.text),
            _ => spans.push(Span::styled(cell.text, cell.style)),
        }
    }
    hoocode_tui_ratatui::row(spans, width)
}

/// Draws the header rows of a block that opens in place: every header row
/// takes the focused look when `focused`, and the last one ends in the
/// indicator when the block `can_open`. `rows` are the block's rendered
/// rows; `header` indexes into them (rows out of range are left alone).
pub fn decorate_header(
    rows: &mut [String],
    header: &[usize],
    can_open: bool,
    expanded: bool,
    focused: bool,
    width: usize,
    palette: &Palette,
) {
    if width == 0 {
        return;
    }
    if can_open {
        if let Some(last) = header.last().copied().filter(|at| *at < rows.len()) {
            rows[last] = stamp_indicator(&rows[last], expanded, width, palette);
        }
    }
    if focused {
        for &at in header {
            if at < rows.len() {
                rows[at] = focus_row(&rows[at], width, palette);
            }
        }
    }
}

/// What every block that opens in place holds besides its own flag: its id
/// and whether it draws focused.
#[derive(Debug, Clone)]
pub struct FoldState {
    id: String,
    focused: bool,
}

impl FoldState {
    /// A new state with a fresh id of `kind` (`skill-4`).
    pub fn new(kind: &str) -> Self {
        Self {
            id: next_fold_id(kind),
            focused: false,
        }
    }

    /// The block's id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Whether `id` names this block.
    pub fn is(&self, id: &str) -> bool {
        self.id == id
    }

    /// Whether the block draws focused.
    pub fn focused(&self) -> bool {
        self.focused
    }

    /// Takes the transcript focus: focused when it names this block.
    pub fn set_focus(&mut self, id: Option<&str>) {
        self.focused = id == Some(self.id.as_str());
    }
}

/// The index of the first row that shows something: the header of a block
/// that draws on a fill and pads above its content.
pub fn first_shown_row(rows: &[String]) -> Option<usize> {
    rows.iter().position(|row| {
        !strip_vt_control_characters(row)
            .chars()
            .all(char::is_whitespace)
    })
}

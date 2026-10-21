//! The one place a transcript row says "there is more": the `… N more lines`
//! hint every renderer folds with (`… N earlier lines` where the tail of the
//! output is what shows), the `…` ellipsis glyph, and the fold of a block's
//! body rows that the peek rail and the trajectory share.
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

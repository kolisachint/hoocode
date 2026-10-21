//! The one place a transcript row says "there is more": the `… N more lines`
//! hint every renderer folds with, the `…` ellipsis glyph, and the fold of a
//! block's body rows that the peek rail and the trajectory share.
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

/// The hint as a themed string (dim): `… N more lines`.
pub fn more_lines_hint(hidden: usize) -> String {
    theme().fg("dim", &more_lines_text(hidden))
}

/// The hint as a ratatui span (dim), for rows drawn from spans.
pub fn more_lines_span(hidden: usize, palette: &Palette) -> Span<'static> {
    Span::styled(more_lines_text(hidden), palette.dim())
}

/// The count in a `… N more lines` row (styling, padding and a gutter ignored), or
/// `None` when the row is anything else.
pub fn parse_more_lines(row: &str) -> Option<usize> {
    let plain = strip_vt_control_characters(row);
    // A hint may sit behind a gutter (`  │ `) or a rail.
    let rest = plain
        .trim_start_matches(|c: char| c.is_whitespace() || matches!(c, '│' | '┃' | '┊'))
        .strip_prefix(ELLIPSIS)?
        .trim();
    rest.strip_suffix("more lines")
        .or_else(|| rest.strip_suffix("more line"))?
        .trim()
        .parse()
        .ok()
}

/// Whether a row is the placeholder a result shows in place of an image
/// the terminal cannot draw (`[Image: [image/png] 100x100]`).
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

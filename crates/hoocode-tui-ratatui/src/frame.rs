//! Line helpers shared by the pane and the console rows: cutting a styled
//! [`Line`] to a width and flattening it to plain text.

use ratatui_core::style::Style;
use ratatui_core::text::{Line, Span};

use crate::ansi::{spans_width, truncate_spans};
use hoocode_tui_util::visible_width;

/// `line` cut to `max` cells, ending in `…` when cut. The `…` keeps the cut
/// span's style. One span-level cutter: [`truncate_spans`] does the cutting
/// (the same cells [`hoocode_tui_util::truncate_to_width`] keeps), this
/// only re-roots the trailing `…` onto the cut span's style for the frame.
pub fn truncate_line(line: &Line<'static>, max: usize) -> Line<'static> {
    if spans_width(&line.spans) <= max {
        return line.clone();
    }
    if max == 0 {
        return Line::from(Vec::new());
    }
    // The kept cells and the cut span, walked the way the old cutter did:
    // whole spans while they fit, then the span holding the cut.
    let mut used = 0;
    let mut cut = Style::default();
    for span in &line.spans {
        let width = visible_width(span.content.as_ref());
        if used + width <= max {
            used += width;
        } else {
            cut = span.style;
            break;
        }
    }
    if used == max {
        // The kept cells already fill the budget: no room for the ellipsis.
        let mut out: Vec<Span<'static>> = Vec::new();
        let mut kept = 0;
        for span in &line.spans {
            let width = visible_width(span.content.as_ref());
            if kept + width <= max {
                kept += width;
                out.push(span.clone());
            } else {
                break;
            }
        }
        return Line::from(out);
    }
    let mut out = truncate_spans(line.spans.clone(), max, "…");
    // The cutter always appends a default-styled ellipsis when it cuts.
    let appended = out
        .pop()
        .is_some_and(|last| last.content.as_ref() == "…" && last.style == Style::default());
    debug_assert!(appended);
    if spans_width(&out) == used {
        // Nothing of the cut span survived: the ellipsis stands alone in
        // the cut span's style.
        out.push(Span::styled("…", cut));
    } else if let Some(prev) = out.last_mut() {
        // The bare cell prefix keeps the cut span's style; rejoin it.
        prev.content.to_mut().push('…');
    }
    Line::from(out)
}

/// `line` as plain text (the bare fallback under ten columns).
pub fn flatten(line: &Line<'static>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<Vec<_>>()
        .concat()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_line_keeps_the_cut_spans_style() {
        let style = Style::default();
        let line = Line::from(vec![Span::styled("abcdef", style)]);
        let cut = truncate_line(&line, 4);
        assert_eq!(flatten(&cut), "abc…");
        assert_eq!(cut.spans[0].style, style);
    }

    #[test]
    fn truncate_line_keeps_fitting_spans_whole() {
        let line = Line::from(vec![Span::raw("ab"), Span::raw("cdef")]);
        let cut = truncate_line(&line, 4);
        assert_eq!(flatten(&cut), "abc…");
    }
}

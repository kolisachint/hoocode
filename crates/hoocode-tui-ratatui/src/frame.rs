//! The Mission Control thick frame shared by cards (M6) and consoles (M7):
//! heavy `┏┓┗┛` borders with a title and right label in the top edge and a
//! foot in the bottom edge, drawn with ratatui's [`Block`] (thick borders)
//! and [`Paragraph`] from styled [`Line`]s. No SGR strings are built here;
//! every colour is a [`Style`] from the [`Palette`].
//!
//! The chrome matches the M6 card frame byte for byte: the title goes first
//! when the row is short (past that both sides are cut with `…`), and below
//! ten columns the frame has no room so the body draws bare.

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_core::style::{Color, Style};
use ratatui_core::text::{Line, Span};
use ratatui_core::widgets::Widget;
use ratatui_widgets::block::Block;
use ratatui_widgets::borders::BorderType;
use ratatui_widgets::paragraph::Paragraph;

use crate::lines::buffer_to_lines;
use hoocode_tui_util::{truncate_to_width, visible_width};

/// The chrome around a body: the frame style plus the title, right label
/// and foot contents (already styled by the caller; the padding and corners
/// are added here).
#[derive(Debug, Clone)]
pub struct Chrome {
    /// The frame colour (warn for cards, faint for consoles).
    pub frame: Style,
    /// The top-left title (bold warn on cards: `◉ CLEARANCE NEEDED`).
    pub title: Line<'static>,
    /// The top-right label (`BUILD asks for Shell`; empty on consoles).
    pub right: Line<'static>,
    /// The bottom-left foot (dim hint row; empty when there is none).
    pub foot: Line<'static>,
}

/// `line` cut to `max` cells, ending in `…` when cut. The `…` keeps the cut
/// span's style.
pub fn truncate_line(line: &Line<'static>, max: usize) -> Line<'static> {
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut used = 0;
    for span in &line.spans {
        let width = visible_width(span.content.as_ref());
        if used + width <= max {
            used += width;
            out.push(span.clone());
            continue;
        }
        let room = max.saturating_sub(used);
        if room == 0 {
            break;
        }
        let mut piece = String::new();
        let mut piece_width = 0;
        for c in span.content.chars() {
            let w = visible_width(&c.to_string());
            if piece_width + w > room.saturating_sub(1) {
                break;
            }
            piece_width += w;
            piece.push(c);
        }
        if !piece.is_empty() || room > 0 {
            piece.push('…');
            out.push(Span::styled(piece, span.style));
        }
        break;
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

/// A gap cell: plain text with the foreground reset, so the thick border
/// underneath does not bleed its colour into the title's padding (the M6
/// frame's gaps are unstyled literal spaces).
fn gap() -> Span<'static> {
    Span::styled(" ", Style::new().fg(Color::Reset))
}

/// The padded top-left / bottom-left title: `━`, a gap, the content, a gap.
/// The `━` carries the frame style; the gaps are unstyled, like the M6
/// frame's literal spaces.
fn pad_left(frame: Style, content: Line<'static>) -> Line<'static> {
    let mut spans = vec![Span::styled("━", frame), gap()];
    spans.extend(content.spans);
    spans.push(gap());
    Line::from(spans)
}

/// The padded top-right title: a gap, the content, a gap, `━`.
fn pad_right(frame: Style, content: Line<'static>) -> Line<'static> {
    let mut spans = vec![gap()];
    spans.extend(content.spans);
    spans.push(gap());
    spans.push(Span::styled("━", frame));
    Line::from(spans)
}

/// Draws the chrome around `body`: the thick [`Block`] with the title and
/// right label up top and the foot below, the body rows inside the `┃`
/// edges. Every row is exactly `width` cells; rows that do not fit are cut
/// with `…`, the title first. Below ten columns the frame has no room, so
/// the body draws bare (truncated, like the M6 frame).
pub fn render_chrome(chrome: &Chrome, body: &[Line<'static>], width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    if width < 10 {
        return body
            .iter()
            .map(|line| truncate_to_width(&flatten(line), width, "…", false))
            .collect();
    }
    // The title goes first when the row is short; past that both sides are cut.
    let budget = width.saturating_sub(9);
    let right = truncate_line(&chrome.right, budget);
    let used_right: usize = right.spans.iter().map(|s| s.width()).sum();
    let title = truncate_line(&chrome.title, budget.saturating_sub(used_right));
    let foot = truncate_line(&chrome.foot, budget);
    let inner = width.saturating_sub(2);
    let block = Block::bordered()
        .border_type(BorderType::Thick)
        .border_style(chrome.frame)
        .title_top(pad_left(chrome.frame, title).left_aligned())
        .title_top(pad_right(chrome.frame, right).right_aligned())
        .title_bottom(pad_left(chrome.frame, foot).left_aligned())
        .title_bottom(pad_right(chrome.frame, Line::raw("")).right_aligned());
    let height = body.len().saturating_add(2);
    let area = Rect::new(0, 0, width as u16, height as u16);
    let mut buf = Buffer::empty(area);
    let inner_area = Rect::new(1, 1, inner as u16, body.len() as u16);
    let rows: Vec<Line<'static>> = body.iter().map(|line| truncate_line(line, inner)).collect();
    let paragraph = Paragraph::new(rows);
    block.render(area, &mut buf);
    paragraph.render(inner_area, &mut buf);
    buffer_to_lines(&buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Palette;

    fn chrome() -> Chrome {
        let pal = Palette::current();
        Chrome {
            frame: pal.warn(),
            title: Line::from(vec![Span::styled(
                "◉ CLEARANCE NEEDED",
                pal.warn().add_modifier(ratatui_core::style::Modifier::BOLD),
            )]),
            right: Line::from(vec![Span::styled("BUILD asks for Shell", pal.warn())]),
            foot: Line::from(vec![Span::styled("or type a reply", pal.dim())]),
        }
    }

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

    #[test]
    fn narrow_frames_draw_the_body_bare() {
        let body = vec![Line::raw("hello world, this is long")];
        let rows = render_chrome(&chrome(), &body, 9);
        assert_eq!(rows.len(), 1);
        assert!(visible_width(&rows[0]) <= 9);
    }

    #[test]
    fn chrome_rows_fit_their_width() {
        let body = vec![Line::raw("detail"), Line::raw("")];
        for width in [10, 19, 40, 100] {
            let rows = render_chrome(&chrome(), &body, width);
            assert_eq!(rows.len(), body.len() + 2, "width {width}");
            for row in &rows {
                assert!(visible_width(row) <= width, "width {width}: {row:?}");
            }
        }
    }
}

//! The Mission Control card chrome (M6): the heavy warn frame every card
//! draws when it replaces the comms frame (spec §4 c1–c2, c8). The top
//! border holds the title and a right label; the body rows sit between `┃`
//! edges; the bottom border holds the foot. Titles and edges read the
//! `warn` token, so `/theme` and NO_COLOR keep working (shapes carry the
//! state with no colour).
//!
//! The frame is drawn with ratatui ([`render_chrome`]: a thick [`Block`] with
//! the title, right label and foot as edge titles). Callers build the body
//! and foot as styled [`Line`]s from the [`Palette`].

use hoocode_tui_ratatui::{bold, render_chrome, Chrome, Line, Palette, Span};

/// The card's title, already styled (bold warn): `◉ …` plus `· step NN`
/// when the host knows the held step.
pub fn card_title(title: &str) -> Line<'static> {
    let pal = Palette::current();
    Line::from(vec![Span::styled(title.to_string(), bold(pal.warn()))])
}

/// The card's right label, already styled (warn).
fn card_right(right: &str) -> Line<'static> {
    let pal = Palette::current();
    Line::from(vec![Span::styled(right.to_string(), pal.warn())])
}

/// The card's foot, already styled (dim).
pub fn card_foot(foot: &str) -> Line<'static> {
    let pal = Palette::current();
    Line::from(vec![Span::styled(foot.to_string(), pal.dim())])
}

/// Draws the card at `width` columns: the warn top edge with `title` and
/// `right`, `body` between `┃` edges, and the `foot` edge. Rows that do not
/// fit are cut with `…`, the title first. Below ten columns the frame has
/// no room, so the body draws bare.
pub fn render_card(
    title: &str,
    right: &str,
    body: &[Line<'static>],
    foot: &Line<'static>,
    width: usize,
) -> Vec<String> {
    let pal = Palette::current();
    render_chrome(
        &Chrome {
            frame: pal.warn(),
            title: card_title(title),
            right: card_right(right),
            foot: foot.clone(),
        },
        body,
        width,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoocode_tui_util::visible_width;

    fn body() -> Vec<Line<'static>> {
        vec![
            Line::raw("$ printf 'alpha\\nbeta\\ngamma\\n' | sort -r"),
            Line::raw("REACH   ■ pipe 2 programs"),
            Line::raw("y go once · a always printf in BUILD"),
        ]
    }

    /// Every emitted line fits its width, even where the frame has no room.
    #[test]
    fn narrow_cards_fit() {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        for width in [3, 19] {
            let rows = render_card(
                "◉ CLEARANCE NEEDED",
                "BUILD asks for Shell",
                &body(),
                &card_foot("or type a reply"),
                width,
            );
            assert!(!rows.is_empty());
            for row in &rows {
                assert!(
                    visible_width(row) <= width,
                    "width {width}: {row:?} is too wide"
                );
            }
        }
    }
}

//! The card pane: the pane every blocking card draws when it takes the
//! prompt's place (permission, question, quit; declutter spec v2.4). The
//! title is a warn (yellow) chip, the right label its dim facts, the body
//! rows sit in the 3-column gutter and the foot is the hint row. Drawn by
//! [`Pane`], so `/theme` and NO_COLOR keep working (monochrome panes get
//! light borders with the title in the top border).
//!
//! Callers build the body and foot as styled [`Line`]s from the
//! [`Palette`].

use hoocode_tui_ratatui::{bold, Line, Palette, Span};

use crate::pane::{Chip, Pane, PaneStyle};

/// The card's title, already styled (bold warn): `◉ …` plus `· step NN`
/// when the host knows the held step.
pub fn card_title(title: &str, pal: &Palette) -> Line<'static> {
    Line::from(vec![Span::styled(title.to_string(), bold(pal.warn()))])
}

/// The card's foot, already styled (dim).
pub fn card_foot(foot: &str) -> Line<'static> {
    let st = PaneStyle::current();
    Line::from(vec![Span::styled(foot.to_string(), st.dim_style())])
}

/// Draws the card at `width` columns: the warn chip with `title`, `right`
/// as dim facts after it, `body` in the row gutter and the `foot` as the
/// hint row. Rows that do not fit are cut with `…`.
pub fn render_card(
    title: &str,
    right: &str,
    body: &[Line<'static>],
    foot: &Line<'static>,
    width: usize,
) -> Vec<String> {
    let st = PaneStyle::current();
    let mut pane = Pane::new(title);
    pane.chip = Chip::Warn;
    if !right.is_empty() {
        pane.facts = Line::from(vec![Span::styled(right.to_string(), st.dim_style())]);
    }
    pane.rows = body.to_vec();
    pane.hint = foot.clone();
    pane.style = Some(st);
    pane.render(width)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoocode_tui_util::{strip_vt_control_characters, visible_width};

    fn body() -> Vec<Line<'static>> {
        vec![
            Line::raw("$ printf 'alpha\\nbeta\\ngamma\\n' | sort -r"),
            Line::raw("REACH   ■ pipe 2 programs"),
            Line::raw("y go once · a always printf in BUILD"),
        ]
    }

    /// Every emitted line fits its width, even where the pane has no room.
    #[test]
    fn narrow_cards_fit() {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        for width in [3, 19, 100] {
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

    #[test]
    fn card_rows_sit_in_the_gutter() {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        let rows: Vec<String> = render_card(
            "◉ CLEARANCE NEEDED",
            "BUILD asks for Shell",
            &body(),
            &card_foot("or type a reply"),
            60,
        )
        .iter()
        .map(|row| strip_vt_control_characters(row))
        .collect();
        assert!(
            rows[0].starts_with("  ◉ CLEARANCE NEEDED   BUILD asks for Shell"),
            "{rows:?}"
        );
        assert!(rows[1].starts_with("   $ printf"), "{rows:?}");
        assert!(rows[4].starts_with(" or type a reply"), "{rows:?}");
    }
}

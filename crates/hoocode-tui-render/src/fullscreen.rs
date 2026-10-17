//! Row arithmetic for the full-screen mode of [`Tui`](crate::Tui)
//! (`docs/design/mission-control.md`, M1a).
//!
//! Pure functions: no terminal, no components. A full-screen frame is rows,
//! top to bottom: the top chrome (HUD, then the gap and attach header when
//! they fit), the body (the transcript), the scroll status row (pinned only)
//! and the bottom (the prompt and what sits under it).

use std::ops::Range;

/// Screens shorter than this have no body and no chrome: only the bottom shows.
const MIN_HEIGHT_WITH_BODY: usize = 4;
/// From this height the gap row is shown; below it only the HUD stays of the top.
const MIN_HEIGHT_WITH_GAP: usize = 6;
/// The status row needs a body row and a bottom row beside it.
const MIN_FREE_ROWS_FOR_BAR: usize = 3;

/// How the rows of a full-screen frame are shared out, top to bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rows {
    /// Top chrome rows shown: the HUD, and the gap and attach rows when the screen is tall enough.
    pub top: usize,
    /// Body rows. Zero only on screens under four rows.
    pub body: usize,
    /// The scroll status row: 1 while pinned and there is room, else 0.
    pub bar: usize,
    /// Bottom rows shown. Trimmed from the top when the screen is short.
    pub bottom: usize,
}

/// Shares `height` rows between the regions of a full-screen frame.
///
/// `top_natural` is the rows the top chrome wants (HUD, gap, attach),
/// `bottom_natural` the rows the bottom wants, `bar` whether the view is
/// pinned. The bottom keeps at least one row and the body at least one, so
/// the result always sums to `height` (screens under four rows show only the
/// bottom, which may leave rows blank).
pub fn split_rows(height: usize, top_natural: usize, bar: bool, bottom_natural: usize) -> Rows {
    if height < MIN_HEIGHT_WITH_BODY {
        return Rows {
            top: 0,
            body: 0,
            bar: 0,
            bottom: bottom_natural.min(height),
        };
    }
    // Under six rows the gap goes; the HUD (the first top row) is the last of the top to go.
    let top = if height >= MIN_HEIGHT_WITH_GAP {
        top_natural
    } else {
        top_natural.min(1)
    };
    let free = height - top;
    let bar = usize::from(bar && free >= MIN_FREE_ROWS_FOR_BAR);
    let bottom = bottom_natural.min(free - bar - 1);
    let body = free - bar - bottom;
    Rows {
        top,
        body,
        bar,
        bottom,
    }
}

/// Columns the body lays out at: the last column is reserved for the
/// scrollbar, blank while live, so pinning does not reflow the text.
pub fn body_width(width: usize) -> usize {
    width.saturating_sub(1)
}

/// First body row to show in a window of `rows` rows over `total` body rows.
/// Live (`pinned_top` is `None`) follows the tail. Pinned holds the top row,
/// clamped to the tail. A short transcript starts at row 0.
pub fn body_start(total: usize, rows: usize, pinned_top: Option<usize>) -> usize {
    let last = total.saturating_sub(rows);
    pinned_top.map_or(last, |top| top.min(last))
}

/// The bottom rows to show: `shown` of `natural`, dropped from the top, as
/// the spec wants. If that would cut the cursor row (`cursor`, an index into
/// the bottom), rows are dropped from the bottom instead, so the footer goes
/// first and the cursor stays on screen.
pub fn bottom_window(natural: usize, shown: usize, cursor: Option<usize>) -> Range<usize> {
    let shown = shown.min(natural);
    let tail = natural - shown;
    match cursor {
        Some(row) if row < natural && row < tail => {
            let end = (row + 1).max(shown);
            (end - shown)..end
        }
        _ => tail..natural,
    }
}

/// The pinned top row after a resize: the same fraction of the transcript,
/// `round(top * new_total / old_total)`, clamped to the last full window of
/// `rows` rows. An empty old transcript anchors at the top.
pub fn anchor_on_resize(top: usize, old_total: usize, new_total: usize, rows: usize) -> usize {
    let last = new_total.saturating_sub(rows);
    if old_total == 0 {
        return 0;
    }
    let scaled = top
        .saturating_mul(new_total)
        .saturating_mul(2)
        .saturating_add(old_total)
        / old_total.saturating_mul(2);
    scaled.min(last)
}

/// Body lines added since the user last pinned or scrolled (the `↓ N new` count).
pub fn new_lines_since(seen_total: usize, total: usize) -> usize {
    total.saturating_sub(seen_total)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bottom natural height the spec's worked table uses.
    const B: usize = 7;

    #[test]
    fn layout_100x30_live() {
        let rows = split_rows(30, 2, false, B);
        assert_eq!(
            rows,
            Rows {
                top: 2,
                body: 21,
                bar: 0,
                bottom: 7
            }
        );
        assert_eq!(
            body_width(100),
            99,
            "last column reserved for the scrollbar"
        );
    }

    #[test]
    fn layout_100x30_pinned() {
        let rows = split_rows(30, 2, true, B);
        assert_eq!(
            rows,
            Rows {
                top: 2,
                body: 20,
                bar: 1,
                bottom: 7
            }
        );
        assert_eq!(rows.top + rows.body + rows.bar + rows.bottom, 30);
    }

    #[test]
    fn layout_80x24_attached() {
        let rows = split_rows(24, 3, false, B);
        assert_eq!(
            rows,
            Rows {
                top: 3,
                body: 14,
                bar: 0,
                bottom: 7
            }
        );
    }

    #[test]
    fn layout_80x24_autocomplete_open() {
        // A bottom of 30 rows (the autocomplete list) takes all but the HUD, gap and one body row.
        let rows = split_rows(24, 2, false, 30);
        assert_eq!(
            rows,
            Rows {
                top: 2,
                body: 1,
                bar: 0,
                bottom: 21
            }
        );
    }

    #[test]
    fn layout_80x12() {
        assert_eq!(
            split_rows(12, 2, false, B),
            Rows {
                top: 2,
                body: 3,
                bar: 0,
                bottom: 7
            }
        );
    }

    #[test]
    fn layout_bottom_taller_than_screen() {
        // At 24 rows with a 30-row bottom: the bottom is trimmed from its top, the body keeps one row.
        let rows = split_rows(24, 2, false, 30);
        assert_eq!(rows.bottom, 21);
        assert_eq!(rows.body, 1);
        // Pinned, the status row takes its row too, and the body still keeps one.
        let pinned = split_rows(24, 2, true, 30);
        assert_eq!(
            pinned,
            Rows {
                top: 2,
                body: 1,
                bar: 1,
                bottom: 20
            }
        );
    }

    #[test]
    fn layout_hud_is_last_to_go() {
        // The HUD stays through 4 and 5 rows, with a bottom taller than the screen.
        assert_eq!(
            split_rows(5, 2, false, 30),
            Rows {
                top: 1,
                body: 1,
                bar: 0,
                bottom: 3
            }
        );
        assert_eq!(
            split_rows(4, 2, false, 30),
            Rows {
                top: 1,
                body: 1,
                bar: 0,
                bottom: 2
            }
        );
        // Under four rows the HUD goes too: only the bottom shows.
        assert_eq!(
            split_rows(3, 2, false, 30),
            Rows {
                top: 0,
                body: 0,
                bar: 0,
                bottom: 3
            }
        );
    }

    #[test]
    fn layout_short_heights() {
        // 6 rows: the gap stays, body 1, bottom 3.
        assert_eq!(
            split_rows(6, 2, false, B),
            Rows {
                top: 2,
                body: 1,
                bar: 0,
                bottom: 3
            }
        );
        // 4 and 5 rows drop the gap.
        assert_eq!(
            split_rows(4, 2, false, B),
            Rows {
                top: 1,
                body: 1,
                bar: 0,
                bottom: 2
            }
        );
        assert_eq!(
            split_rows(5, 2, false, B),
            Rows {
                top: 1,
                body: 1,
                bar: 0,
                bottom: 3
            }
        );
        // 2 and 3 rows: the bottom only, no body.
        assert_eq!(
            split_rows(2, 2, false, B),
            Rows {
                top: 0,
                body: 0,
                bar: 0,
                bottom: 2
            }
        );
        assert_eq!(
            split_rows(3, 2, false, B),
            Rows {
                top: 0,
                body: 0,
                bar: 0,
                bottom: 3
            }
        );
    }

    #[test]
    fn layout_bar_needs_three_free_rows() {
        // Status row, one body row and one bottom row: 4 rows gives all three.
        assert_eq!(
            split_rows(4, 2, true, B),
            Rows {
                top: 1,
                body: 1,
                bar: 1,
                bottom: 1
            }
        );
        // Under four rows there is no body, so no status row either.
        assert_eq!(split_rows(3, 2, true, B).bar, 0);
    }

    #[test]
    fn layout_resize_smaller_then_larger_restores_rows() {
        let big = split_rows(30, 2, false, B);
        let small = split_rows(24, 3, false, B);
        assert_eq!(small.body, 14);
        assert_eq!(
            split_rows(30, 2, false, B),
            big,
            "growing back gives the same split"
        );
    }

    #[test]
    fn body_window_is_tail_when_live() {
        assert_eq!(body_start(50, 20, None), 30);
    }

    #[test]
    fn body_window_holds_when_pinned() {
        assert_eq!(
            body_start(60, 20, Some(10)),
            10,
            "output grows, the window stays"
        );
        assert_eq!(
            body_start(60, 20, Some(55)),
            40,
            "clamped to the last full window"
        );
    }

    #[test]
    fn body_window_short_transcript_starts_at_top() {
        assert_eq!(body_start(5, 20, None), 0);
        assert_eq!(body_start(5, 20, Some(3)), 0);
    }

    #[test]
    fn bottom_window_drops_from_top_without_cursor() {
        assert_eq!(bottom_window(7, 3, None), 4..7);
        assert_eq!(bottom_window(7, 7, None), 0..7);
    }

    #[test]
    fn bottom_window_keeps_cursor_row_in_view() {
        // The cursor in the tail is kept by the top drop.
        assert_eq!(bottom_window(7, 3, Some(5)), 4..7);
        // A cursor the top drop would cut: drop from the bottom instead.
        assert_eq!(bottom_window(7, 3, Some(0)), 0..3);
        assert_eq!(bottom_window(7, 3, Some(3)), 1..4);
        assert_eq!(bottom_window(30, 21, Some(0)), 0..21);
    }

    #[test]
    fn bottom_window_when_bottom_fits() {
        assert_eq!(bottom_window(2, 5, None), 0..2);
        assert_eq!(bottom_window(0, 3, None), 0..0);
    }

    #[test]
    fn anchor_keeps_fraction_on_resize() {
        // Top 10 of 100 is a tenth; at 200 rows the same tenth is row 20.
        assert_eq!(anchor_on_resize(10, 100, 200, 20), 20);
        // Shrinking the transcript scales the top down.
        assert_eq!(anchor_on_resize(10, 100, 50, 20), 5);
    }

    #[test]
    fn anchor_rounds_half_up_and_clamps() {
        assert_eq!(anchor_on_resize(1, 2, 3, 1), 2, "1.5 rounds up");
        assert_eq!(
            anchor_on_resize(90, 100, 50, 20),
            30,
            "clamped to the last full window"
        );
    }

    #[test]
    fn anchor_from_empty_transcript_is_top() {
        assert_eq!(anchor_on_resize(0, 0, 10, 5), 0);
    }

    #[test]
    fn new_lines_since_seen() {
        assert_eq!(new_lines_since(50, 58), 8);
        assert_eq!(
            new_lines_since(60, 58),
            0,
            "a shorter transcript has no new lines"
        );
    }
}

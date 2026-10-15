//! The cursor and visible window of a picker list, shared by every picker.
//!
//! Decision 29: the cursor clamps at both ends, nothing wraps. PgUp/PgDn move
//! a page, Home/End jump to the ends.

use std::ops::Range;

use hoocode_tui_keys::{matches_key, KeybindingsManager};

/// A selected row over `len` rows, with a window of `max_visible` rows that
/// keeps the selected row in view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickerCursor {
    selected: usize,
    len: usize,
    max_visible: usize,
}

impl PickerCursor {
    pub fn new(len: usize, max_visible: usize) -> Self {
        Self {
            selected: 0,
            len,
            max_visible,
        }
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn max_visible(&self) -> usize {
        self.max_visible
    }

    /// Moves the cursor to `index`, clamped to the last row.
    pub fn select(&mut self, index: usize) {
        self.selected = index.min(self.len.saturating_sub(1));
    }

    /// The rows changed (a query narrowed or widened them): the new length,
    /// with the cursor kept where it is and clamped into the list.
    pub fn set_query(&mut self, len: usize) {
        self.len = len;
        self.select(self.selected);
    }

    /// The rows were replaced by a new list: the new length, cursor on the
    /// first row.
    pub fn reset(&mut self, len: usize) {
        self.len = len;
        self.selected = 0;
    }

    /// Moves by `delta` rows, clamped at both ends. Returns whether the
    /// cursor moved.
    pub fn step(&mut self, delta: isize) -> bool {
        if self.len == 0 {
            return false;
        }
        let max = (self.len - 1) as isize;
        let next = (self.selected as isize + delta).clamp(0, max) as usize;
        let moved = next != self.selected;
        self.selected = next;
        moved
    }

    /// Moves one window up or down, clamped. Returns whether the cursor moved.
    pub fn page(&mut self, down: bool) -> bool {
        let rows = self.max_visible.max(1) as isize;
        self.step(if down { rows } else { -rows })
    }

    /// Jumps to the first row. Returns whether the cursor moved.
    pub fn home(&mut self) -> bool {
        self.jump_to(0)
    }

    /// Jumps to the last row. Returns whether the cursor moved.
    pub fn end(&mut self) -> bool {
        self.jump_to(self.len.saturating_sub(1))
    }

    fn jump_to(&mut self, index: usize) -> bool {
        if self.len == 0 {
            return false;
        }
        let moved = index != self.selected;
        self.selected = index;
        moved
    }

    /// Handles the navigation keys: up and down (one row), PgUp and PgDn (a
    /// page), Home and End (the ends). Returns whether the key was one of them.
    /// Whether the cursor moved is not reported; callers that notify on a
    /// change compare [`Self::selected`] before and after.
    pub fn handle_key(&mut self, data: &str, kb: &KeybindingsManager) -> bool {
        self.navigate(data, kb, true)
    }

    /// As [`Self::handle_key`], for a picker with a query line: Home and End
    /// are left to the query's text cursor, so only PgUp and PgDn jump the
    /// list there.
    pub fn handle_query_key(&mut self, data: &str, kb: &KeybindingsManager) -> bool {
        self.navigate(data, kb, false)
    }

    fn navigate(&mut self, data: &str, kb: &KeybindingsManager, ends: bool) -> bool {
        if kb.matches(data, "tui.select.up") {
            self.step(-1);
        } else if kb.matches(data, "tui.select.down") {
            self.step(1);
        } else if kb.matches(data, "tui.select.pageUp") {
            self.page(false);
        } else if kb.matches(data, "tui.select.pageDown") {
            self.page(true);
        } else if ends && matches_key(data, "home") {
            self.home();
        } else if ends && matches_key(data, "end") {
            self.end();
        } else {
            return false;
        }
        true
    }

    /// The rows drawn: the window of `max_visible` rows that holds the cursor.
    pub fn window(&self) -> Range<usize> {
        let start = self
            .selected
            .saturating_sub(self.max_visible / 2)
            .min(self.len.saturating_sub(self.max_visible));
        let end = start.saturating_add(self.max_visible).min(self.len);
        start..end
    }

    /// Whether the window leaves rows out, so the position footer is shown.
    pub fn is_scrolled(&self) -> bool {
        self.window() != (0..self.len)
    }

    /// The position, `(n/len)`, always.
    pub fn position(&self) -> String {
        format!("({}/{})", self.selected + 1, self.len)
    }

    /// The position footer, `(n/len)`, when the window is scrolled.
    pub fn footer(&self) -> Option<String> {
        self.is_scrolled().then(|| self.position())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoocode_tui_keys::default_tui_keybindings;
    use std::collections::HashMap;

    fn kb() -> KeybindingsManager {
        KeybindingsManager::new(default_tui_keybindings(), HashMap::new())
    }

    #[test]
    fn step_clamps_at_both_ends() {
        let mut c = PickerCursor::new(3, 5);
        assert!(!c.step(-1), "up at the first row does not move");
        assert_eq!(c.selected(), 0);
        assert!(c.step(1));
        assert!(c.step(1));
        assert_eq!(c.selected(), 2);
        assert!(!c.step(1), "down at the last row does not wrap");
        assert_eq!(c.selected(), 2);
    }

    #[test]
    fn empty_list_never_moves() {
        let mut c = PickerCursor::new(0, 5);
        assert!(c.is_empty());
        assert!(!c.step(1));
        assert!(!c.page(true));
        assert!(!c.home());
        assert!(!c.end());
        assert_eq!(c.selected(), 0);
        assert_eq!(c.window(), 0..0);
        assert_eq!(c.footer(), None);
    }

    #[test]
    fn page_jumps_one_window_and_clamps() {
        let mut c = PickerCursor::new(20, 5);
        c.page(true);
        assert_eq!(c.selected(), 5);
        c.page(true);
        c.page(true);
        c.page(true);
        c.page(true);
        assert_eq!(
            c.selected(),
            19,
            "a page past the end stops on the last row"
        );
        c.page(false);
        assert_eq!(c.selected(), 14);
        for _ in 0..10 {
            c.page(false);
        }
        assert_eq!(
            c.selected(),
            0,
            "a page past the top stops on the first row"
        );
    }

    #[test]
    fn home_and_end_jump_to_the_ends() {
        let mut c = PickerCursor::new(12, 4);
        assert!(c.end());
        assert_eq!(c.selected(), 11);
        assert!(!c.end());
        assert!(c.home());
        assert_eq!(c.selected(), 0);
        assert!(!c.home());
    }

    #[test]
    fn handle_key_routes_navigation_keys() {
        let kb = kb();
        let mut c = PickerCursor::new(12, 4);
        assert!(c.handle_key("\x1b[B", &kb), "down is handled");
        assert_eq!(c.selected(), 1);
        assert!(c.handle_key("\x1b[6~", &kb), "pageDown is handled");
        assert_eq!(c.selected(), 5);
        assert!(c.handle_key("\x1b[F", &kb), "end is handled");
        assert_eq!(c.selected(), 11);
        assert!(c.handle_key("\x1b[H", &kb), "home is handled");
        assert_eq!(c.selected(), 0);
        assert!(
            c.handle_key("\x1b[A", &kb),
            "up at the top is still handled"
        );
        assert_eq!(c.selected(), 0);
        assert!(!c.handle_key("x", &kb), "a text key is not navigation");
        assert_eq!(c.selected(), 0);
    }

    #[test]
    fn query_key_leaves_home_and_end_to_the_query() {
        let kb = kb();
        let mut c = PickerCursor::new(12, 4);
        assert!(!c.handle_query_key("\x1b[F", &kb), "end is not navigation");
        assert!(!c.handle_query_key("\x1b[H", &kb), "home is not navigation");
        assert_eq!(c.selected(), 0);
        assert!(c.handle_query_key("\x1b[6~", &kb), "pageDown is handled");
        assert_eq!(c.selected(), 4);
        assert!(c.handle_query_key("\x1b[B", &kb), "down is handled");
        assert_eq!(c.selected(), 5);
    }

    #[test]
    fn set_query_clamps_and_reset_returns_to_top() {
        let mut c = PickerCursor::new(10, 5);
        c.select(7);
        c.set_query(4);
        assert_eq!(
            c.selected(),
            3,
            "a narrower list keeps the cursor on its last row"
        );
        c.set_query(8);
        assert_eq!(
            c.selected(),
            3,
            "a wider list keeps the cursor where it was"
        );
        c.reset(8);
        assert_eq!(c.selected(), 0);
        assert_eq!(c.len(), 8);
    }

    #[test]
    fn window_keeps_the_cursor_visible() {
        let mut c = PickerCursor::new(30, 5);
        for target in [0, 1, 2, 3, 10, 17, 28, 29] {
            c.select(target);
            let w = c.window();
            assert!(w.contains(&target), "row {target} in window {w:?}");
            assert_eq!(w.len(), 5, "a full window at {target}");
        }
        c.select(0);
        assert_eq!(c.window(), 0..5);
        c.select(29);
        assert_eq!(c.window(), 25..30);
        c.select(15);
        assert_eq!(c.window(), 13..18, "the cursor sits in the middle");
    }

    #[test]
    fn window_holds_the_cursor_for_every_size_and_position() {
        let kb = kb();
        for len in 0..14 {
            for max in 0..9 {
                let mut c = PickerCursor::new(len, max);
                for _ in 0..(len + 2) {
                    let w = c.window();
                    assert_eq!(w.len(), max.min(len), "len {len}, max {max}");
                    if len > 0 && max > 0 {
                        assert!(w.contains(&c.selected()), "len {len}, max {max}: {w:?}");
                    }
                    c.handle_key("\x1b[B", &kb);
                }
                // Then a shrink (set_query), then a page up from the end.
                c.set_query(len / 2);
                assert!(c.is_empty() || c.selected() < c.len());
                c.handle_key("\x1b[6~", &kb);
                assert!(c.is_empty() || c.selected() < c.len());
            }
        }
    }

    #[test]
    fn empty_list_handles_every_key() {
        let kb = kb();
        let mut c = PickerCursor::new(0, 0);
        for key in ["\x1b[A", "\x1b[B", "\x1b[5~", "\x1b[6~", "\x1b[H", "\x1b[F"] {
            assert!(c.handle_key(key, &kb));
            assert_eq!(c.selected(), 0);
            assert_eq!(c.window(), 0..0);
            assert_eq!(c.footer(), None);
        }
    }

    #[test]
    fn window_shows_everything_when_it_fits() {
        let c = PickerCursor::new(3, 5);
        assert_eq!(c.window(), 0..3);
        assert!(!c.is_scrolled());
        assert_eq!(c.footer(), None);
    }

    #[test]
    fn footer_text_is_position_when_scrolled() {
        let mut c = PickerCursor::new(30, 5);
        assert_eq!(c.footer(), Some("(1/30)".to_string()));
        c.select(14);
        assert_eq!(c.footer(), Some("(15/30)".to_string()));
        assert_eq!(c.position(), "(15/30)");
        let short = PickerCursor::new(4, 5);
        assert_eq!(short.footer(), None);
        assert_eq!(short.position(), "(1/4)");
    }
}

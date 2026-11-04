//! The scrolled-back and search bars in the status line's left slot
//! (declutter v2.5): `scrolled 43–61 of 78 · ↓ 3 new  esc live` and
//! `search  query  1 match  enter next · esc close`, with the session facts
//! on the right. The theme lock pins the palette; the keybindings manager
//! pins the key names.

use hoocode_code_tui_app::scroll_view::format_scroll_status;
use hoocode_code_tui_app::status_line::StatusLineData;
use hoocode_code_tui_keybindings::AppKeybindingsManager;
use hoocode_tui_render::golden::render_golden;
use hoocode_tui_render::{assert_golden, Component, ScrollSearchStatus, ScrollStatus};

use crate::support::{lock, plain};

/// One status row as a component, so the golden harness can draw it.
struct BarLine(String);

impl Component for BarLine {
    fn render(&mut self, _width: u16) -> Vec<String> {
        vec![self.0.clone()]
    }
}

fn base() -> StatusLineData {
    StatusLineData {
        view: "peek".into(),
        session: Some("amber-harbor".into()),
        model: Some("claude-haiku-5-5".into()),
        context_percent: Some(42.0),
        cost: Some(1.23),
        ..Default::default()
    }
}

fn scrolled(picker: bool) -> ScrollStatus {
    ScrollStatus {
        top: 43,
        bottom: 61,
        total: 78,
        view_height: 19,
        at_top: false,
        at_bottom: false,
        width: 100,
        search: None,
        picker,
        new_lines: 3,
    }
}

fn searching(query: &str, typing: bool, count: usize) -> ScrollStatus {
    ScrollStatus {
        top: 60,
        bottom: 78,
        total: 78,
        view_height: 19,
        at_top: false,
        at_bottom: true,
        width: 100,
        search: Some(ScrollSearchStatus {
            query: query.to_string(),
            count,
            index: 1,
            typing,
        }),
        picker: false,
        new_lines: 0,
    }
}

/// The status row, plain, at 100 columns.
fn row(status: &ScrollStatus) -> String {
    AppKeybindingsManager::default().install();
    plain(&format_scroll_status(status, &base()))
}

#[test]
fn scrolled_golden() {
    let _g = lock(Some("dark"));
    AppKeybindingsManager::default().install();
    let mut line = BarLine(format_scroll_status(&scrolled(false), &base()));
    assert_golden!("scroll_rewind_100x1", render_golden(&mut line, 100));
}

#[test]
fn search_golden() {
    let _g = lock(Some("dark"));
    AppKeybindingsManager::default().install();
    let mut line = BarLine(format_scroll_status(
        &searching("beta item 25", false, 1),
        &base(),
    ));
    assert_golden!("scroll_scan_100x1", render_golden(&mut line, 100));
}

#[test]
fn scrolled_shows_the_range_new_lines_and_the_live_key() {
    let _g = lock(Some("dark"));
    let text = row(&scrolled(false));
    assert!(
        text.starts_with(" scrolled 43–61 of 78 · ↓ 3 new  esc live"),
        "{text:?}"
    );
    assert!(
        // Too wide for the cost at 100 columns: it drops first.
        text.ends_with("peek · amber-harbor · claude-haiku-5-5 · 42% context"),
        "{text:?}"
    );
}

#[test]
fn a_focused_picker_keeps_its_keys() {
    let _g = lock(Some("dark"));
    let text = row(&scrolled(true));
    assert!(!text.contains("esc"), "{text:?}");
}

#[test]
fn search_shows_the_query_matches_and_keys() {
    let _g = lock(Some("dark"));
    let text = row(&searching("beta item 25", false, 1));
    assert!(
        text.starts_with(" search  beta item 25  1 match  enter next · esc close"),
        "{text:?}"
    );
    let typing = row(&searching("beta", true, 3));
    assert!(
        typing.starts_with(" search  beta  3 matches  enter find · esc close"),
        "{typing:?}"
    );
}

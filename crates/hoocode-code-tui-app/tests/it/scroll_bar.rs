//! The Mission Control rewind and scan bars (M8 e2/e3) as component
//! goldens: one inverse row above the comms frame while the view is pinned,
//! drawn with ratatui. The theme lock pins the palette; the keybindings
//! manager pins the key names.

use hoocode_code_tui_app::scroll_view::format_scroll_status;
use hoocode_code_tui_keybindings::AppKeybindingsManager;
use hoocode_tui_render::golden::render_golden;
use hoocode_tui_render::{assert_golden, Component, ScrollSearchStatus, ScrollStatus};

use crate::support::{lock, plain};

/// One bar row as a component, so the golden harness can draw it.
struct BarLine(String);

impl Component for BarLine {
    fn render(&mut self, _width: u16) -> Vec<String> {
        vec![self.0.clone()]
    }
}

fn rewind_status() -> ScrollStatus {
    rewind_status_picker(false)
}

/// The rewind bar with `picker` focus: picker keys are the literal paging
/// keys, so the golden holds on every platform.
fn rewind_status_picker(picker: bool) -> ScrollStatus {
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

fn scan_status(query: &str, typing: bool) -> ScrollStatus {
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
            count: 1,
            index: 1,
            typing,
        }),
        picker: false,
        new_lines: 0,
    }
}

/// The bar's plain row at 100 columns.
fn row(status: &ScrollStatus) -> String {
    AppKeybindingsManager::default().install();
    plain(&format_scroll_status(status, true))
}

#[test]
fn rewind_bar_golden() {
    let _g = lock(Some("dark"));
    // The bar reads the global bindings: pin the defaults so the golden
    // holds whatever else runs beside it.
    AppKeybindingsManager::default().install();
    let mut line = BarLine(format_scroll_status(&rewind_status_picker(true), true));
    assert_golden!("scroll_rewind_100x1", render_golden(&mut line, 100));
}

#[test]
fn scan_bar_golden() {
    let _g = lock(Some("dark"));
    AppKeybindingsManager::default().install();
    let mut line = BarLine(format_scroll_status(
        &scan_status("beta item 25", false),
        true,
    ));
    assert_golden!("scroll_scan_100x1", render_golden(&mut line, 100));
}

#[test]
fn rewind_bar_shows_the_range_percent_and_new_lines() {
    let _g = lock(Some("dark"));
    let text = row(&rewind_status());
    assert!(
        text.contains("◂◂ REWIND  43–61 of 78  55%  ↓ 3 new"),
        "{text:?}"
    );
    assert!(text.contains("PgUp/PgDn page"), "{text:?}");
    assert!(text.contains("live"), "{text:?}");
}

#[test]
fn scan_bar_shows_the_query_contacts_and_keys() {
    let _g = lock(Some("dark"));
    let text = row(&scan_status("beta item 25", false));
    assert!(text.contains("⌕ SCAN  beta item 25  1 contact"), "{text:?}");
    assert!(text.contains("enter next · escape clear · n/p"), "{text:?}");
}

#[test]
fn scan_bar_typing_shows_the_caret_and_lock_keys() {
    let _g = lock(Some("dark"));
    let text = row(&scan_status("beta", true));
    assert!(text.contains("⌕ SCAN  beta█  1 contact"), "{text:?}");
    assert!(text.contains("enter lock · escape clear"), "{text:?}");
}

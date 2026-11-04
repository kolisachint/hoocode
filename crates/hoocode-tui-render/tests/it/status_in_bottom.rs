//! The scroll status in the app's status line (declutter v2.5): with
//! `set_scroll_status_in_bottom`, a pinned full-screen view draws the status
//! over the bottom's last row instead of a row of its own above the bottom.

use crate::support::*;
use hoocode_tui_render::Tui;

const WIDTH: u16 = 100;
const HEIGHT: u16 = 12;
const H: usize = HEIGHT as usize;

/// The fullscreen tree: a blank top row, the body, the prompt and the status
/// line. The formatter writes a recognisable status.
fn setup(in_bottom: bool) -> (Tui, Handle) {
    let (terminal, term) = virtual_terminal(WIDTH, HEIGHT);
    let mut tui = Tui::new(terminal, None);
    tui.set_fullscreen(true);
    tui.add_child(Lines::new(&[""]));
    let body = Lines::body(30);
    let handle: hoocode_tui_render::ComponentHandle = body.clone();
    tui.add_child(handle.clone());
    tui.set_fullscreen_body(Some(handle));
    tui.add_child(Lines::new(&["prompt", "status"]));
    tui.set_scroll_status_formatter(Box::new(|s| {
        format!("scrolled {}-{} of {}", s.top, s.bottom, s.total)
    }));
    tui.set_scroll_status_in_bottom(in_bottom);
    let _events = tui.start();
    tui.request_render(false);
    (tui, term)
}

#[test]
fn live_shows_the_status_line_as_it_is() {
    let (_tui, term) = setup(true);
    let rows = term.screen();
    assert_eq!(rows[H - 2], "prompt");
    assert_eq!(rows[H - 1], "status");
}

#[test]
fn pinned_draws_the_scroll_status_over_the_last_bottom_row() {
    let (mut tui, term) = setup(true);
    assert!(tui.scroll_by_lines(-5));
    tui.request_render(false);
    let rows = term.screen();
    assert_eq!(rows[H - 2], "prompt", "the prompt keeps its row");
    assert!(rows[H - 1].starts_with("scrolled "), "{:?}", rows[H - 1]);
    // No bar row of its own: the row above the prompt is transcript.
    assert!(rows[H - 3].starts_with("body "), "{:?}", rows[H - 3]);
}

#[test]
fn without_the_switch_the_status_takes_its_own_row() {
    let (mut tui, term) = setup(false);
    assert!(tui.scroll_by_lines(-5));
    tui.request_render(false);
    let rows = term.screen();
    assert!(rows[H - 3].starts_with("scrolled "), "{:?}", rows[H - 3]);
    assert_eq!(rows[H - 1], "status");
}

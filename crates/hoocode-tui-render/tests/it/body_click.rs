//! A click on a transcript row, full-screen: the row is reported as a row
//! of the transcript (not of the screen), from anywhere on its line, and a
//! press on one row with a release on another reports nothing.

use crate::support::*;
use hoocode_tui_render::{BodyClick, ComponentHandle, Container, Tui, TuiEvent};
use std::cell::RefCell;
use std::rc::Rc;

const WIDTH: u16 = 40;
const HEIGHT: u16 = 12;

/// HUD, a body of `rows` rows, and a prompt row: the body shows 10 rows,
/// on screen rows 2 to 11 (1-based).
fn setup(rows: usize) -> (Tui, Rc<RefCell<Vec<BodyClick>>>) {
    let (terminal, _term) = virtual_terminal(WIDTH, HEIGHT);
    let mut tui = Tui::new(terminal, None);
    tui.set_fullscreen(true);
    tui.add_child(Lines::new(&["HUD"]));
    let body = Rc::new(RefCell::new(Container::new()));
    body.borrow_mut().add_child(Lines::body(rows));
    let handle: ComponentHandle = body.clone();
    tui.add_child(handle.clone());
    tui.set_fullscreen_body(Some(handle));
    tui.add_child(Lines::new(&["prompt"]));
    let clicks = Rc::new(RefCell::new(Vec::new()));
    let sink = clicks.clone();
    tui.on_body_click = Some(Box::new(move |click| sink.borrow_mut().push(click)));
    let _events = tui.start();
    tui.request_render(true);
    (tui, clicks)
}

fn send(tui: &mut Tui, data: &str) {
    tui.process_event(TuiEvent::input(data));
}

fn press(tui: &mut Tui, row: usize, column: usize) {
    send(tui, &format!("\x1b[<0;{column};{row}M"));
}

fn release(tui: &mut Tui, row: usize, column: usize) {
    send(tui, &format!("\x1b[<0;{column};{row}m"));
}

fn click(tui: &mut Tui, row: usize, column: usize) {
    press(tui, row, column);
    release(tui, row, column);
}

#[test]
fn a_live_long_transcript_reports_the_transcript_row() {
    // 100 rows, 10 on screen: the live window starts at row 90, so screen
    // row 2 (the first body row) is transcript row 90.
    let (mut tui, clicks) = setup(100);
    click(&mut tui, 2, 5);
    click(&mut tui, 6, 5);
    let seen: Vec<(i64, i64)> = clicks
        .borrow()
        .iter()
        .map(|c| (c.buffer_row, c.context))
        .collect();
    assert_eq!(seen, [(90, 0), (94, 4)]);
}

#[test]
fn a_pinned_transcript_reports_the_transcript_row() {
    let (mut tui, clicks) = setup(100);
    tui.scroll_to_top();
    tui.request_render(true);
    tui.scroll_by_lines(7);
    tui.request_render(true);
    click(&mut tui, 4, 5);
    let seen: Vec<(i64, i64)> = clicks
        .borrow()
        .iter()
        .map(|c| (c.buffer_row, c.context))
        .collect();
    assert_eq!(seen, [(9, 2)]);
    assert_eq!(tui.transcript_screen_row(9), Some(2));
    assert_eq!(tui.transcript_screen_row(6), None, "above the window");
    assert_eq!(tui.transcript_screen_row(17), None, "below the window");
}

#[test]
fn a_short_live_transcript_reports_rows_under_its_blank_top() {
    // 3 rows in a 10-row body: bottom-anchored, on screen rows 9 to 11.
    let (mut tui, clicks) = setup(3);
    click(&mut tui, 4, 5);
    assert!(clicks.borrow().is_empty(), "a blank row above is no row");
    click(&mut tui, 9, 5);
    click(&mut tui, 11, 5);
    let seen: Vec<(i64, i64)> = clicks
        .borrow()
        .iter()
        .map(|c| (c.buffer_row, c.context))
        .collect();
    assert_eq!(seen, [(0, 7), (2, 9)]);
    assert_eq!(tui.transcript_screen_row(0), Some(7));
}

#[test]
fn the_whole_row_is_one_target() {
    let (mut tui, clicks) = setup(100);
    // Press on one cell, release on another cell of the same row.
    press(&mut tui, 5, 3);
    release(&mut tui, 5, 30);
    assert_eq!(clicks.borrow().len(), 1);
    assert_eq!(clicks.borrow()[0].buffer_row, 93);
    // The far right blank end of the row, one cell short of the scrollbar.
    click(&mut tui, 5, usize::from(WIDTH) - 1);
    assert_eq!(clicks.borrow().len(), 2);
}

#[test]
fn another_row_the_scrollbar_and_the_chrome_report_nothing() {
    let (mut tui, clicks) = setup(100);
    // Press on one row, release on another: a drag, not a click.
    press(&mut tui, 5, 3);
    release(&mut tui, 6, 3);
    // A release with no press.
    release(&mut tui, 5, 3);
    // The scrollbar column, pressed or released on.
    click(&mut tui, 5, usize::from(WIDTH));
    press(&mut tui, 5, 3);
    release(&mut tui, 5, usize::from(WIDTH));
    press(&mut tui, 5, usize::from(WIDTH));
    release(&mut tui, 5, 3);
    // The HUD row and the prompt row.
    click(&mut tui, 1, 3);
    click(&mut tui, usize::from(HEIGHT), 3);
    // The right button.
    send(&mut tui, "\x1b[<2;3;5M");
    send(&mut tui, "\x1b[<2;3;5m");
    assert!(clicks.borrow().is_empty(), "{:?}", clicks.borrow());
}

#[test]
fn each_press_and_release_pair_is_one_click() {
    let (mut tui, clicks) = setup(100);
    // A double click arriving in one chunk, then three fast clicks.
    send(&mut tui, "\x1b[<0;4;5M\x1b[<0;4;5m\x1b[<0;4;5M\x1b[<0;4;5m");
    assert_eq!(clicks.borrow().len(), 2);
    for _ in 0..3 {
        click(&mut tui, 5, 4);
    }
    assert_eq!(clicks.borrow().len(), 5);
    assert!(clicks.borrow().iter().all(|c| c.buffer_row == 93));
}

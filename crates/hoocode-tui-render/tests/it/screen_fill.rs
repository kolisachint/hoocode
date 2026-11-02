//! Port of the pin's `test/screen-fill.test.ts`: whatever the session holds,
//! the last row of the buffer is the last row of the screen.

use crate::support::*;
use hoocode_tui_render::{FlexSpacer, Tui};
use std::cell::RefCell;
use std::rc::Rc;

const WIDTH: u16 = 30;
const HEIGHT: u16 = 12;
const H: usize = HEIGHT as usize;

struct Setup {
    tui: Tui,
    term: Handle,
    body: Rc<RefCell<Lines>>,
    chrome: Rc<RefCell<Lines>>,
    fill: Rc<RefCell<FlexSpacer>>,
}

fn setup_rows(body_rows: usize, rows: u16) -> Setup {
    let (terminal, term) = virtual_terminal(WIDTH, rows);
    let mut tui = Tui::new(terminal, None);
    let body = Lines::body(body_rows);
    let fill = Rc::new(RefCell::new(FlexSpacer::new()));
    let chrome = Lines::body(0);
    tui.add_child(fill.clone());
    tui.add_child(Lines::new(&["header"]));
    tui.add_child(body.clone());
    tui.add_child(chrome.clone());
    tui.add_child(Lines::new(&["prompt"]));
    tui.set_flex_spacer(Some(fill.clone()));
    let _events = tui.start();
    Setup {
        tui,
        term,
        body,
        chrome,
        fill,
    }
}

fn setup(body_rows: usize) -> Setup {
    setup_rows(body_rows, HEIGHT)
}

impl Setup {
    fn body(&mut self, count: usize) {
        self.body.borrow_mut().set_count(count);
        self.tui.request_render(false);
    }
    fn chrome(&mut self, count: usize) {
        self.chrome.borrow_mut().set_count(count);
        self.tui.request_render(false);
    }
    fn fill(&self) -> usize {
        self.fill.borrow().current_height()
    }
}

fn blanks(n: usize) -> Vec<String> {
    vec![String::new(); n]
}

#[test]
fn packs_a_short_session_against_the_floor() {
    let s = setup(2);
    let rows = s.term.screen();
    assert_eq!(rows[H - 1], "prompt");
    assert_eq!(rows[H - 4..], ["header", "body 1", "body 2", "prompt"]);
    assert_eq!(rows[..H - 4], blanks(H - 4)[..]);
}

#[test]
fn keeps_the_prompt_on_the_last_row_as_the_session_grows() {
    let mut s = setup(2);
    for count in [3, 5, 8] {
        s.body(count);
        let rows = s.term.screen();
        assert_eq!(rows[H - 1], "prompt", "at {count}");
        assert_eq!(rows[H - 2], format!("body {count}"), "at {count}");
    }
}

#[test]
fn gives_up_the_fill_once_content_is_taller() {
    let mut s = setup(2);
    s.body(H * 2);
    assert_eq!(s.fill(), 0);
    assert_eq!(s.term.screen()[H - 1], "prompt");
}

#[test]
fn takes_the_fill_back_when_content_shrinks() {
    let mut s = setup(H * 2);
    assert_eq!(s.fill(), 0);
    s.body(1);
    assert_eq!(s.fill(), H - 3);
}

#[test]
fn refits_on_a_resize() {
    let mut s = setup(2);
    assert_eq!(s.fill(), H - 4);
    s.term.resize(WIDTH, HEIGHT + 6);
    s.tui.request_render(false);
    assert_eq!(s.fill(), H + 6 - 4);
}

#[test]
fn gives_its_rows_to_a_picker_and_takes_them_back() {
    let mut s = setup(2);
    let before = s.fill();
    s.chrome(6);
    assert_eq!(s.fill(), before - 6);
    assert_eq!(s.term.screen()[H - 1], "prompt");
    s.chrome(0);
    assert_eq!(s.fill(), before);
    assert_eq!(s.term.screen()[H - 1], "prompt");
}

#[test]
fn the_fill_is_not_transcript() {
    let mut s = setup(2);
    assert!(!s.tui.scroll_by_lines(-1));
    assert!(!s.tui.scroll_pinned());
}

#[test]
fn keeps_the_screen_the_tail_of_the_buffer_as_it_grows() {
    let mut s = setup(2);
    for count in [2, 6, H - 4, H, H + 1, H * 2, H * 3] {
        s.body(count);
        let fill = s.fill();
        let mut expected: Vec<String> = blanks(fill);
        expected.push("header".into());
        expected.extend((1..=count).map(|i| format!("body {i}")));
        expected.push("prompt".into());
        let tail = expected[expected.len() - H..].to_vec();
        assert_eq!(s.term.screen(), tail, "at {count}");
    }
}

#[test]
fn leaves_no_stale_rows_when_a_long_session_is_cleared() {
    let mut s = setup(H * 3);
    s.body(1);
    let mut expected = blanks(H - 3);
    expected.extend(["header".into(), "body 1".into(), "prompt".into()]);
    assert_eq!(s.term.screen(), expected);
}

#[test]
fn costs_no_more_full_redraws_than_without_a_fill() {
    let steps = [2, 6, H, H * 2, H + 1, 4, 1];
    let mut filled = setup(2);
    for count in steps {
        filled.body(count);
    }
    let (terminal, _plain_term) = virtual_terminal(WIDTH, HEIGHT);
    let mut plain = Tui::new(terminal, None);
    let plain_body = Lines::body(2);
    plain.add_child(Lines::new(&["header"]));
    plain.add_child(plain_body.clone());
    plain.add_child(Lines::new(&["prompt"]));
    let _events = plain.start();
    for count in steps {
        plain_body.borrow_mut().set_count(count);
        plain.request_render(false);
    }
    assert!(
        filled.tui.full_redraws() <= plain.full_redraws(),
        "filled={} plain={}",
        filled.tui.full_redraws(),
        plain.full_redraws()
    );
}

#[test]
fn has_nothing_to_do_without_a_fill() {
    let (terminal, term) = virtual_terminal(WIDTH, HEIGHT);
    let mut tui = Tui::new(terminal, None);
    tui.add_child(Lines::new(&["header"]));
    tui.add_child(Lines::new(&["prompt"]));
    let _events = tui.start();
    let rows = term.screen();
    assert_eq!(rows[0], "header");
    assert_eq!(rows[1], "prompt");
    assert_eq!(rows[H - 1], "");
}

#[test]
fn leaves_no_band_when_a_view_folds() {
    let mut s = setup(H * 4);
    let n = H * 3 / 2;
    s.body(n);
    let rows = s.term.screen();
    assert_eq!(rows[H - 1], "prompt");
    assert_eq!(rows[H - 2], format!("body {n}"));
    assert!(!rows[..H - 1].contains(&String::new()), "{rows:?}");
}

#[test]
fn empties_the_fill_while_pinned() {
    let mut s = setup(2);
    assert!(s.fill() > 0);
    s.body(H * 3);
    assert!(s.tui.scroll_by_lines(-5));
    assert_eq!(s.fill(), 0);
}

#[test]
fn puts_the_prompt_back_on_the_floor_after_a_pinned_view() {
    let mut s = setup(2);
    s.body(H * 3);
    s.tui.scroll_by_lines(-5);
    assert!(s.tui.scroll_pinned());
    s.body.borrow_mut().set_count(1);
    s.tui.scroll_to_live();
    assert!(!s.tui.scroll_pinned());
    assert_eq!(s.fill(), H - 3);
    assert_eq!(s.term.screen()[H - 1], "prompt");
}

/// A shrink of a buffer taller than the screen holds the window: the rows
/// below the prompt go blank (`blanks` of them) and nothing above moves, so
/// no row is drawn again into the scrollback.
fn assert_held(rows: &[String], blanks: usize) {
    assert_eq!(rows.len(), H);
    let (content, tail) = rows.split_at(H - blanks);
    assert!(tail.iter().all(String::is_empty), "{rows:?}");
    assert!(!content.contains(&String::new()), "{rows:?}");
    assert_eq!(content[H - blanks - 1], "prompt", "{rows:?}");
}

#[test]
fn a_picker_closing_holds_the_window_and_blanks_the_rows_below() {
    let mut s = setup(H * 2);
    assert_eq!(s.fill(), 0);
    s.chrome(6);
    assert_eq!(s.term.screen()[H - 1], "prompt");
    let top = s.term.screen()[0].clone();
    s.chrome(0);
    let rows = s.term.screen();
    assert_held(&rows, 6);
    assert_eq!(rows[0], top, "the window did not move");
    assert_eq!(s.fill(), 0);
    assert_eq!(rows[H - 8], format!("body {}", H * 2));
}

#[test]
fn a_notification_fading_holds_the_window() {
    let mut s = setup(H * 2);
    s.chrome(2);
    s.chrome(0);
    assert_held(&s.term.screen(), 2);
}

#[test]
fn growth_after_a_fold_fills_the_blank_rows_before_it_scrolls() {
    let mut s = setup(H * 2);
    s.chrome(6);
    s.chrome(0);
    s.body(H * 2 + 4);
    let rows = s.term.screen();
    assert_held(&rows, 2);
    assert_eq!(rows[H - 4], format!("body {}", H * 2 + 4));
    s.body(H * 2 + 6);
    let rows = s.term.screen();
    assert_eq!(rows[H - 1], "prompt", "{rows:?}");
    assert_eq!(rows[H - 2], format!("body {}", H * 2 + 6));
}

#[test]
fn handles_a_fold_bigger_than_the_screen() {
    let mut s = setup(H * 4);
    s.chrome(H * 2);
    s.chrome(0);
    assert_eq!(s.fill(), 0);
    let rows = s.term.screen();
    assert_eq!(rows[H - 1], "prompt", "{rows:?}");
    assert!(!rows.contains(&String::new()), "{rows:?}");
}

#[test]
fn lands_on_the_floor_after_a_resize() {
    let mut s = setup(H * 2);
    s.chrome(6);
    s.chrome(0);
    s.term.resize(WIDTH, HEIGHT - 2);
    s.tui.request_render(false);
    assert_eq!(s.fill(), 0);
    let rows = s.term.screen();
    assert_eq!(rows[H - 3], "prompt");
    assert_eq!(rows[H - 4], format!("body {}", H * 2));
}

#[test]
fn keeps_a_short_session_packed_whatever_the_chrome_does() {
    let mut s = setup(2);
    s.chrome(6);
    s.chrome(0);
    let rows = s.term.screen();
    assert_eq!(rows[0], "");
    assert_eq!(rows[H - 4..], ["header", "body 1", "body 2", "prompt"]);
}

#[allow(dead_code)]
fn unused() -> Setup {
    setup_rows(1, HEIGHT)
}

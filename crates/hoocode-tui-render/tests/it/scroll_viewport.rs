//! Port of the pin's `test/scroll-viewport.test.ts`: a pinned view stays put.

use crate::support::*;
use hoocode_tui_render::{
    default_scroll_status, scrollbar_glyphs, Component, ComponentHandle, ScrollStatus, Tui,
    TuiEvent, SCROLLBAR_THUMB, SCROLLBAR_TRACK,
};
use std::cell::RefCell;
use std::rc::Rc;

const WIDTH: u16 = 40;
const HEIGHT: u16 = 10;
const VIEW: usize = HEIGHT as usize - 1;

fn transcript(count: usize, label: &str) -> Rc<RefCell<Lines>> {
    Rc::new(RefCell::new(Lines(
        (1..=count).map(|i| format!("{label} {i}")).collect(),
    )))
}

fn setup_n(count: usize) -> (Tui, Handle, Rc<RefCell<Lines>>) {
    let (terminal, term) = virtual_terminal(WIDTH, HEIGHT);
    let mut tui = Tui::new(terminal, None);
    let t = transcript(count, "line");
    tui.add_child(t.clone());
    let _events = tui.start();
    (tui, term, t)
}

fn setup() -> (Tui, Handle, Rc<RefCell<Lines>>) {
    setup_n(100)
}

/// A screen row without the scrollbar column, trailing blanks trimmed.
fn text_of(row: &str) -> String {
    row.chars()
        .take(WIDTH as usize - 1)
        .collect::<String>()
        .trim_end()
        .to_string()
}

fn window(term: &Handle) -> Vec<String> {
    term.screen()[..VIEW].iter().map(|r| text_of(r)).collect()
}

fn status_row(term: &Handle) -> String {
    term.screen()[HEIGHT as usize - 1].trim().to_string()
}

fn send(tui: &mut Tui, data: &str) {
    tui.process_event(TuiEvent::input(data));
}

fn wheel_up(tui: &mut Tui, times: usize) {
    for _ in 0..times {
        send(tui, "\x1b[<64;1;1M");
    }
}

fn wheel_down(tui: &mut Tui, times: usize) {
    for _ in 0..times {
        send(tui, "\x1b[<65;1;1M");
    }
}

fn top(tui: &Tui) -> i64 {
    tui.get_scroll_position().unwrap().0
}

fn append(t: &Rc<RefCell<Lines>>, text: &str) {
    t.borrow_mut().0.push(text.to_string());
}

#[test]
fn does_not_pin_while_live() {
    let (mut tui, ..) = setup();
    wheel_down(&mut tui, 5);
    assert!(!tui.scroll_pinned());
}

#[test]
fn pins_on_the_first_notch_and_moves_three_lines() {
    let (mut tui, ..) = setup();
    wheel_up(&mut tui, 1);
    assert!(tui.scroll_pinned());
    let first = top(&tui);
    wheel_up(&mut tui, 1);
    assert_eq!(first - top(&tui), 3);
}

#[test]
fn refuses_to_pin_when_the_app_says_no_but_keeps_answering_once_pinned() {
    let (mut tui, ..) = setup();
    tui.can_pin_scroll = Some(Box::new(|_| false));
    wheel_up(&mut tui, 1);
    assert!(!tui.scroll_pinned());

    let (mut tui, ..) = setup();
    wheel_up(&mut tui, 1);
    tui.can_pin_scroll = Some(Box::new(|_| false));
    let before = top(&tui);
    tui.scroll_by_lines(-1);
    assert_eq!(top(&tui), before - 1);
    assert!(tui.scroll_to_live());
}

#[test]
fn shows_the_rows_the_offset_names() {
    let (mut tui, term, _) = setup();
    tui.scroll_by_lines(-20);
    let rows = window(&term);
    let t = top(&tui) as usize;
    assert_eq!(rows[0], format!("line {}", t + 1));
    assert_eq!(rows[VIEW - 1], format!("line {}", t + VIEW));
}

#[test]
fn does_not_move_when_output_arrives() {
    let (mut tui, term, t) = setup();
    tui.scroll_by_lines(-30);
    let before = window(&term);
    for i in 0..40 {
        append(&t, &format!("streamed {i}"));
        tui.request_render(false);
    }
    assert_eq!(window(&term), before);
}

#[test]
fn reports_the_growing_transcript_while_holding_its_place() {
    let (mut tui, _term, t) = setup();
    tui.scroll_by_lines(-30);
    let at = top(&tui);
    append(&t, "one more");
    tui.request_render(false);
    let (now, total, _) = tui.get_scroll_position().unwrap();
    assert_eq!((now, total), (at, 101));
}

#[test]
fn re_clamps_when_the_transcript_shrinks() {
    let (mut tui, term, t) = setup_n(200);
    tui.scroll_to_top();
    tui.scroll_by_lines(180);
    assert!(top(&tui) > 20);
    t.borrow_mut().0.truncate(30);
    tui.request_render(false);
    assert_eq!(top(&tui), (30 - VIEW) as i64);
    assert_eq!(window(&term)[0], format!("line {}", 30 - VIEW + 1));
}

#[test]
fn pages_by_a_screen_less_two() {
    let (mut tui, ..) = setup();
    tui.scroll_by_lines(-1);
    let from = top(&tui);
    tui.scroll_by_pages(-1);
    assert_eq!(from - top(&tui), VIEW as i64 - 2);
}

#[test]
fn goes_to_the_first_row() {
    let (mut tui, term, _) = setup();
    tui.scroll_to_top();
    assert_eq!(top(&tui), 0);
    assert_eq!(window(&term)[0], "line 1");
}

#[test]
fn releases_the_pin_at_the_bottom() {
    let (mut tui, ..) = setup();
    tui.scroll_by_lines(-4);
    assert!(tui.scroll_pinned());
    tui.scroll_by_lines(10);
    assert!(!tui.scroll_pinned());

    let (mut tui, ..) = setup();
    wheel_up(&mut tui, 2);
    wheel_down(&mut tui, 10);
    assert!(!tui.scroll_pinned());
}

#[test]
fn does_nothing_when_the_transcript_fits() {
    let (mut tui, ..) = setup_n(4);
    assert!(!tui.scroll_by_lines(-3));
    assert!(!tui.scroll_by_pages(-1));
    assert!(!tui.scroll_to_top());
    assert!(!tui.scroll_pinned());
}

#[test]
fn indicator_says_where_and_top_and_can_be_replaced() {
    let (mut tui, term, _) = setup();
    tui.scroll_by_lines(-20);
    let t = top(&tui) as usize;
    let status = status_row(&term);
    assert!(
        status.contains(&format!("{}\u{2013}{}/100", t + 1, t + VIEW)),
        "{status}"
    );
    tui.scroll_to_top();
    assert!(status_row(&term).contains("top"));

    let (mut tui, term, _) = setup();
    tui.set_scroll_status_formatter(Box::new(|s| format!("<<{}/{}>>", s.top, s.total)));
    tui.scroll_to_top();
    assert!(status_row(&term).contains("<<1/100>>"));
    let screen = term.screen();
    assert_eq!(screen.len(), HEIGHT as usize);
    assert_eq!(text_of(&screen[VIEW - 1]), format!("line {VIEW}"));
}

#[test]
fn takes_the_alternate_screen_and_gives_it_back() {
    let (mut tui, term, _) = setup();
    tui.scroll_by_lines(-10);
    assert!(term.alternate_screen());
    tui.scroll_to_live();
    assert!(!term.alternate_screen());
}

#[test]
fn shows_what_arrived_without_replaying() {
    let (mut tui, term, t) = setup_n(12);
    let before = tui.full_redraws();
    tui.scroll_to_top();
    append(&t, "arrived while reading");
    tui.request_render(false);
    tui.scroll_to_live();
    let screen = term.screen();
    assert_eq!(
        screen.last().unwrap(),
        "arrived while reading",
        "{screen:?}"
    );
    assert_eq!(tui.full_redraws(), before);
}

#[test]
fn full_redraw_after_a_resize_while_pinned() {
    let (mut tui, term, _) = setup_n(60);
    tui.scroll_to_top();
    let before = tui.full_redraws();
    term.resize(WIDTH - 8, HEIGHT);
    tui.process_event(TuiEvent::Resize);
    tui.scroll_to_live();
    assert!(tui.full_redraws() > before);
}

#[test]
fn leaves_the_alternate_screen_on_stop() {
    let (mut tui, term, _) = setup();
    tui.scroll_by_lines(-10);
    tui.stop();
    assert!(!term.alternate_screen());
}

struct Sink(Rc<RefCell<Vec<String>>>);
impl Component for Sink {
    fn render(&mut self, _width: u16) -> Vec<String> {
        vec!["sink".into()]
    }
    fn handle_input(&mut self, data: &str) {
        self.0.borrow_mut().push(data.to_string());
    }
}

#[test]
fn a_component_still_gets_its_keys_but_never_a_mouse_report() {
    let (mut tui, ..) = setup();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let sink: ComponentHandle = Rc::new(RefCell::new(Sink(seen.clone())));
    tui.set_focus(Some(sink));
    send(&mut tui, "\x1b[<64;1;1M\x1b[A");
    assert_eq!(*seen.borrow(), ["\x1b[A"]);
    seen.borrow_mut().clear();
    send(&mut tui, "\x1b[<0;12;4M");
    send(&mut tui, "\x1b[<0;12;4m");
    assert!(seen.borrow().is_empty());
}

fn searchable() -> (Tui, Handle) {
    let (terminal, term) = virtual_terminal(WIDTH, HEIGHT);
    let mut tui = Tui::new(terminal, None);
    let lines = (0..60)
        .map(|i| {
            if i == 5 || i == 25 || i == 45 {
                "has the Needle here".to_string()
            } else {
                format!("filler {i}")
            }
        })
        .collect();
    tui.add_child(Rc::new(RefCell::new(Lines(lines))));
    let _events = tui.start();
    (tui, term)
}

fn in_view(tui: &Tui, row: i64) -> bool {
    let (top, _, view) = tui.get_scroll_position().unwrap();
    top <= row && row < top + view
}

#[test]
fn search_finds_rows_case_insensitively_and_starts_from_the_newest() {
    let (mut tui, _) = searchable();
    assert_eq!(tui.set_scroll_search("needle", true), 3);
    assert!(in_view(&tui, 45));
}

#[test]
fn search_steps_back_and_wraps() {
    let (mut tui, _) = searchable();
    tui.set_scroll_search("needle", true);
    assert!(tui.scroll_search_step(-1));
    assert!(in_view(&tui, 25));
    tui.scroll_search_step(-1);
    assert!(in_view(&tui, 5));
    tui.scroll_search_step(-1);
    assert!(in_view(&tui, 45));
}

#[test]
fn search_with_no_match_does_not_move() {
    let (mut tui, _) = searchable();
    tui.scroll_to_top();
    let before = top(&tui);
    assert_eq!(tui.set_scroll_search("nothingmatchesthis", true), 0);
    assert_eq!(top(&tui), before);
}

#[test]
fn search_marks_matches_and_feeds_the_indicator() {
    let (mut tui, term) = searchable();
    tui.set_scroll_status_formatter(Box::new(|s| match &s.search {
        Some(q) => format!("q={} {}/{}", q.query, q.index, q.count),
        None => "no search".into(),
    }));
    tui.set_scroll_search("needle", true);
    assert!(term.last_write().contains("\x1b[7m"));
    assert!(status_row(&term).contains("q=needle 3/3"));
    tui.scroll_search_step(-1);
    assert!(status_row(&term).contains("q=needle 2/3"));
}

#[test]
fn search_re_runs_when_the_transcript_grows() {
    let (terminal, _term) = virtual_terminal(WIDTH, HEIGHT);
    let mut tui = Tui::new(terminal, None);
    let t = transcript(60, "filler");
    tui.add_child(t.clone());
    let _events = tui.start();
    tui.set_scroll_search("arrived", true);
    assert_eq!(tui.get_scroll_position(), None);
    tui.scroll_to_top();
    append(&t, "arrived later");
    tui.request_render(false);
    assert!(tui.scroll_search_step(1));
}

#[test]
fn search_ends_with_its_pinned_view() {
    let (mut tui, _) = searchable();
    tui.set_scroll_search("needle", true);
    assert!(tui.scroll_search_active());
    tui.scroll_to_live();
    assert!(!tui.scroll_search_active());
}

// --- the scrollbar ------------------------------------------------------------

#[test]
fn scrollbar_is_empty_when_everything_fits() {
    assert!(scrollbar_glyphs(10, 10, 0).is_empty());
    assert!(scrollbar_glyphs(10, 4, 0).is_empty());
}

#[test]
fn scrollbar_has_one_glyph_per_row_in_two_shapes() {
    let glyphs = scrollbar_glyphs(10, 40, 12);
    assert_eq!(glyphs.len(), 10);
    assert!(glyphs
        .iter()
        .all(|g| *g == SCROLLBAR_THUMB || *g == SCROLLBAR_TRACK));
}

#[test]
fn scrollbar_thumb_is_at_least_one_row_long() {
    let glyphs = scrollbar_glyphs(5, 1000, 0);
    assert_eq!(glyphs.iter().filter(|g| **g == SCROLLBAR_THUMB).count(), 1);
}

#[test]
fn scrollbar_thumb_runs_from_the_top_to_the_bottom_of_the_track() {
    // 10 rows over 40: a thumb of 10*10/40 = 2 rows.
    let at = |top| -> Vec<usize> {
        scrollbar_glyphs(10, 40, top)
            .iter()
            .enumerate()
            .filter(|(_, g)| **g == SCROLLBAR_THUMB)
            .map(|(i, _)| i)
            .collect()
    };
    assert_eq!(at(0), [0, 1]);
    assert_eq!(at(15), [4, 5]);
    assert_eq!(at(30), [8, 9]);
    // Out-of-range offsets are held to the ends.
    assert_eq!(at(-5), [0, 1]);
    assert_eq!(at(999), [8, 9]);
}

#[test]
fn scrollbar_thumb_moves_down_as_the_view_goes_down() {
    let start = |top| {
        scrollbar_glyphs(10, 40, top)
            .iter()
            .position(|g| *g == SCROLLBAR_THUMB)
    };
    let starts: Vec<_> = (0..=30).map(start).collect();
    assert!(starts.windows(2).all(|w| w[0] <= w[1]), "{starts:?}");
}

/// Rows of `width` cells, each `#` (the transcript fills what it is given).
struct Wide(usize);

impl Component for Wide {
    fn render(&mut self, width: u16) -> Vec<String> {
        (0..self.0).map(|_| "#".repeat(width as usize)).collect()
    }
}

#[test]
fn the_transcript_is_one_column_narrower_and_the_bar_takes_the_last() {
    let (terminal, term) = virtual_terminal(WIDTH, HEIGHT);
    let mut tui = Tui::new(terminal, None);
    tui.add_child(Rc::new(RefCell::new(Wide(60))));
    let _events = tui.start();
    tui.scroll_by_lines(-20);
    let screen = term.screen();
    for row in &screen[..VIEW] {
        let mut cells = row.chars();
        assert_eq!(
            cells.by_ref().take(WIDTH as usize - 1).collect::<String>(),
            "#".repeat(WIDTH as usize - 1),
            "{row:?}"
        );
        let bar = cells.next().unwrap();
        assert!(bar == SCROLLBAR_THUMB || bar == SCROLLBAR_TRACK, "{row:?}");
    }
    // 60 rows over 9: a one-row thumb.
    let thumbs = screen[..VIEW]
        .iter()
        .filter(|r| r.ends_with(SCROLLBAR_THUMB))
        .count();
    assert_eq!(thumbs, 1);
}

#[test]
fn a_live_frame_keeps_the_full_width_and_no_bar() {
    let (terminal, term) = virtual_terminal(WIDTH, HEIGHT);
    let mut tui = Tui::new(terminal, None);
    tui.add_child(Rc::new(RefCell::new(Wide(3))));
    let _events = tui.start();
    assert_eq!(term.screen()[0], "#".repeat(WIDTH as usize));
}

// --- the picker at the bottom -------------------------------------------------

/// A focusable picker showing fixed rows.
struct Options(Vec<String>);

impl Component for Options {
    fn render(&mut self, _width: u16) -> Vec<String> {
        self.0.clone()
    }
    fn is_focusable(&self) -> bool {
        true
    }
}

fn options(rows: &[&str]) -> Rc<RefCell<Options>> {
    Rc::new(RefCell::new(Options(
        rows.iter().map(|s| s.to_string()).collect(),
    )))
}

#[test]
fn a_focused_picker_paints_its_rows_above_the_status_row() {
    let (mut tui, term, _) = setup();
    tui.set_focus(Some(options(&["Option A", "Option B"])));
    tui.scroll_by_lines(-20);
    // Two picker rows: the transcript keeps HEIGHT - 3 rows above them.
    let view = HEIGHT as usize - 3;
    let screen = term.screen();
    assert_eq!(screen[view], "Option A");
    assert_eq!(screen[view + 1], "Option B");
    assert!(status_row(&term).contains("/100"), "{}", status_row(&term));
}

/// A picker whose rows are shared, so a test can change them later.
struct Growing(Rc<RefCell<Vec<String>>>);

impl Component for Growing {
    fn render(&mut self, _width: u16) -> Vec<String> {
        self.0.borrow().clone()
    }
    fn is_focusable(&self) -> bool {
        true
    }
}

#[test]
fn a_tall_picker_lets_go_of_the_pin_rather_than_clipping_its_top() {
    let (mut tui, term, _) = setup();
    let rows: Vec<String> = (1..=50).map(|i| format!("Row {i}")).collect();
    tui.set_focus(Some(Rc::new(RefCell::new(Options(rows)))));
    tui.scroll_by_lines(-20);
    // Room above the status row is HEIGHT - 1, and one of it stays transcript,
    // so 50 rows cannot fit: the view is live and no picker row is painted.
    assert!(!tui.scroll_pinned());
    assert!(!term.screen().iter().any(|r| r.contains("Row ")));
}

#[test]
fn a_picker_that_just_fits_keeps_the_pin_and_shows_every_row() {
    let (mut tui, term, _) = setup();
    // HEIGHT - 2 rows: one transcript row, every picker row, the status row.
    let rows: Vec<String> = (1..=HEIGHT as usize - 2)
        .map(|i| format!("Row {i}"))
        .collect();
    tui.set_focus(Some(Rc::new(RefCell::new(Options(rows)))));
    tui.scroll_by_lines(-20);
    assert!(tui.scroll_pinned());
    let screen = term.screen();
    assert_eq!(screen[1], "Row 1");
    assert_eq!(screen[HEIGHT as usize - 2], format!("Row {}", HEIGHT - 2));
}

#[test]
fn a_picker_that_grows_while_pinned_lets_go_on_the_next_frame() {
    let (mut tui, _term, _) = setup();
    let rows = Rc::new(RefCell::new(vec!["Option A".to_string()]));
    tui.set_focus(Some(Rc::new(RefCell::new(Growing(rows.clone())))));
    tui.scroll_by_lines(-20);
    assert!(tui.scroll_pinned());
    *rows.borrow_mut() = (1..=50).map(|i| format!("Row {i}")).collect();
    tui.request_render(false);
    assert!(!tui.scroll_pinned());
}

#[test]
fn the_prompt_is_not_painted_again_at_the_bottom() {
    let (mut tui, term, _) = setup();
    let prompt = options(&["prompt"]);
    tui.scroll_prompt = Some(prompt.clone());
    tui.set_focus(Some(prompt));
    tui.scroll_by_lines(-20);
    // Top is row 71 (0-based), so the last transcript row shows "line 80".
    assert_eq!(
        text_of(&term.screen()[VIEW - 1]),
        format!("line {}", 71 + VIEW)
    );
}

#[test]
fn a_focused_picker_is_not_offered_the_arrows_or_escape_in_the_indicator() {
    let (mut tui, term, _) = setup();
    tui.set_focus(Some(options(&["Option A"])));
    tui.scroll_by_lines(-20);
    let status = status_row(&term);
    assert!(status.contains("PgUp/PgDn page"), "{status}");
    assert!(!status.contains("esc"), "{status}");
    assert!(!status.contains('\u{2191}'), "{status}");
}

#[test]
fn the_default_indicator_offers_the_arrows_and_escape_only_at_the_prompt() {
    let mut status = ScrollStatus {
        top: 1,
        bottom: 9,
        total: 100,
        view_height: 9,
        at_top: true,
        at_bottom: false,
        width: 120,
        search: None,
        picker: false,
    };
    let prompt = default_scroll_status(&status);
    assert!(prompt.contains("esc live"), "{prompt}");
    status.picker = true;
    let picker = default_scroll_status(&status);
    assert!(picker.contains("PgUp/PgDn page"), "{picker}");
    assert!(!picker.contains("esc"), "{picker}");
    assert!(!picker.contains('\u{2191}'), "{picker}");
}

// --- pressing the scrollbar ---------------------------------------------------

fn press(tui: &mut Tui, row: usize, column: usize) {
    send(tui, &format!("\x1b[<0;{column};{row}M"));
}

/// The 0-based transcript row of the thumb on screen.
fn thumb_index(term: &Handle) -> usize {
    term.screen()[..VIEW]
        .iter()
        .position(|r| r.ends_with(SCROLLBAR_THUMB))
        .unwrap()
}

#[test]
fn a_press_above_the_thumb_in_the_bar_column_pages_up() {
    let (mut tui, term, _) = setup();
    tui.scroll_by_lines(-20);
    assert!(thumb_index(&term) > 0);
    let before = top(&tui);
    press(&mut tui, 1, WIDTH as usize);
    assert_eq!(before - top(&tui), VIEW as i64 - 2);
}

#[test]
fn a_press_below_the_thumb_in_the_bar_column_pages_down() {
    let (mut tui, term, _) = setup();
    tui.scroll_by_lines(-20);
    assert!(thumb_index(&term) < VIEW - 1);
    let before = top(&tui);
    press(&mut tui, VIEW, WIDTH as usize);
    assert_eq!(top(&tui) - before, VIEW as i64 - 2);
}

#[test]
fn a_press_on_the_thumb_does_nothing() {
    let (mut tui, term, _) = setup();
    tui.scroll_by_lines(-20);
    let before = top(&tui);
    let row = thumb_index(&term) + 1;
    press(&mut tui, row, WIDTH as usize);
    assert_eq!(top(&tui), before);
}

#[test]
fn a_press_elsewhere_or_while_live_does_nothing() {
    let (mut tui, _, _) = setup();
    tui.scroll_by_lines(-20);
    let before = top(&tui);
    press(&mut tui, 1, 5);
    assert_eq!(top(&tui), before);

    let (mut tui, _, _) = setup();
    press(&mut tui, 1, WIDTH as usize);
    assert!(!tui.scroll_pinned());
}

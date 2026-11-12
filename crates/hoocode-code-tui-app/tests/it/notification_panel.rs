//! Port of the pin's `test/notification-panel.test.ts`, on a fake clock.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::support::lock;
use hoocode_code_tui_app::notification_panel::*;
use hoocode_code_tui_theme::theme;
use hoocode_tui_render::Component;
use hoocode_tui_util::strip_vt_control_characters;

const WIDTH: u16 = 60;
const INFO: u64 = 3000;
const WARNING: u64 = 8000;

struct Setup {
    panel: NotificationPanel,
    renders: Rc<Cell<usize>>,
    now: Rc<Cell<Instant>>,
}

impl Setup {
    fn advance(&mut self, ms: u64) {
        self.now.set(self.now.get() + Duration::from_millis(ms));
        self.panel.poll();
    }
    fn rows(&mut self, width: u16) -> Vec<String> {
        self.panel
            .render(width)
            .iter()
            .map(|l| strip_vt_control_characters(l).trim_end().to_string())
            .collect()
    }
    fn showing(&self) -> Option<String> {
        self.panel.showing().map(|n| n.title.clone())
    }
    fn pending(&self) -> Vec<String> {
        self.panel
            .pending()
            .iter()
            .map(|n| n.title.clone())
            .collect()
    }
    fn info(&mut self, title: &str) {
        self.panel
            .notify(NotificationKind::Info, title, &[], None, None, None);
    }
    fn warning(&mut self, title: &str) {
        self.panel
            .notify(NotificationKind::Warning, title, &[], None, None, None);
    }
    fn topic(&mut self, title: &str, topic: &str) {
        self.panel
            .notify(NotificationKind::Info, title, &[], None, None, Some(topic));
    }
}

fn setup(max_body_rows: Option<Box<dyn Fn() -> usize>>) -> Setup {
    let renders = Rc::new(Cell::new(0));
    let sink = renders.clone();
    let now = Rc::new(Cell::new(Instant::now()));
    let clock = now.clone();
    let panel = NotificationPanel::with_clock(
        move || sink.set(sink.get() + 1),
        max_body_rows,
        Rc::new(move || clock.get()),
    );
    Setup {
        panel,
        renders,
        now,
    }
}

#[test]
fn draws_nothing_at_all_when_nothing_is_up() {
    let _g = lock(Some("dark"));
    let mut s = setup(None);
    assert!(s.panel.render(WIDTH).is_empty());
}

#[test]
fn a_one_row_notice_goes_to_the_status_line_and_the_band_stays_empty() {
    use hoocode_code_tui_app::status_line::{KeyHint, NoticeLevel};
    let _g = lock(Some("dark"));
    let mut s = setup(None);
    s.panel.notify(
        NotificationKind::Info,
        "View: outline",
        &[],
        Some("shift+alt+o back"),
        None,
        None,
    );
    assert!(s.panel.render(WIDTH).is_empty());
    let notice = s.panel.status_notice().expect("a status notice");
    assert_eq!(notice.level, NoticeLevel::Info);
    assert_eq!(notice.message, "View: outline");
    assert_eq!(notice.keys, [KeyHint::new("shift+alt+o", "back")]);
    // A longer notice stays in the band.
    s.panel.notify(
        NotificationKind::Warning,
        "Two rows",
        &["the second"],
        None,
        None,
        None,
    );
    s.advance(INFO);
    assert!(s.panel.status_notice().is_none());
    assert!(!s.panel.render(WIDTH).is_empty());
}

#[test]
fn leads_with_a_blank_row_and_never_pads_its_own_bottom_edge() {
    let _g = lock(Some("dark"));
    let mut s = setup(None);
    // While a turn runs the status line keeps its slot: one row goes to the band.
    s.panel.set_status_slot_free(false);
    s.info("Model: opus-5");
    let lines = s.rows(WIDTH);
    assert_eq!(lines[0], "");
    assert_eq!(lines.len(), 2);
    assert!(lines[1].contains("Model: opus-5"));
}

#[test]
fn puts_the_note_after_the_headline_and_wraps_it_when_it_does_not_fit() {
    let _g = lock(Some("dark"));
    let mut s = setup(None);
    s.panel.set_status_slot_free(false);
    s.panel.notify(
        NotificationKind::Info,
        "Thinking level: high",
        &[],
        Some("shift+alt+t steps back"),
        None,
        None,
    );
    let row = s.rows(WIDTH)[1].clone();
    assert_eq!(row, " Thinking level: high  shift+alt+t steps back");
    let narrow = s.rows(30);
    assert_eq!(narrow[1], " Thinking level: high");
    assert_eq!(narrow[2], " shift+alt+t steps back");
}

#[test]
fn fades_on_its_own_clock() {
    let _g = lock(Some("dark"));
    let mut s = setup(None);
    s.info("View: outline");
    s.advance(INFO - 1);
    assert_eq!(s.showing().as_deref(), Some("View: outline"));
    s.advance(1);
    assert_eq!(s.showing(), None);
    assert!(s.panel.render(WIDTH).is_empty());
}

#[test]
fn gives_a_warning_longer_than_a_glimpse() {
    let _g = lock(Some("dark"));
    let mut s = setup(None);
    s.warning("No previous directory to return to");
    s.advance(INFO);
    assert!(s.showing().is_some());
    s.advance(WARNING - INFO);
    assert_eq!(s.showing(), None);
}

#[test]
fn collapses_a_run_of_glimpses_to_the_one_that_is_still_true() {
    let _g = lock(Some("dark"));
    let mut s = setup(None);
    s.warning("held");
    s.info("Model: a");
    s.info("Model: b");
    s.info("Model: c");
    assert_eq!(s.showing().as_deref(), Some("held"));
    assert_eq!(s.pending(), ["Model: c"]);
}

#[test]
fn never_cuts_short_the_notification_already_on_screen() {
    let _g = lock(Some("dark"));
    let mut s = setup(None);
    s.info("Model: a");
    s.info("Model: b");
    assert_eq!(s.showing().as_deref(), Some("Model: a"));
    assert_eq!(s.pending(), ["Model: b"]);
}

#[test]
fn steps_a_dial_on_the_band_as_fast_as_the_dial_is_stepped() {
    let _g = lock(Some("dark"));
    let mut s = setup(None);
    s.topic("View: outline", "app.view");
    s.advance(1000);
    s.topic("View: full", "app.view");
    assert_eq!(s.showing().as_deref(), Some("View: full"));
    assert!(s.pending().is_empty());
    s.advance(INFO - 1);
    assert_eq!(s.showing().as_deref(), Some("View: full"));
    s.advance(1);
    assert_eq!(s.showing(), None);
}

#[test]
fn keeps_two_different_dials_apart() {
    let _g = lock(Some("dark"));
    let mut s = setup(None);
    s.topic("View: full", "app.view");
    s.topic("Thinking level: high", "app.thinking");
    assert_eq!(s.showing().as_deref(), Some("View: full"));
    assert_eq!(s.pending(), ["Thinking level: high"]);
}

#[test]
fn replaces_a_queued_glimpse_of_the_same_dial_without_jumping_the_queue() {
    let _g = lock(Some("dark"));
    let mut s = setup(None);
    s.warning("held");
    s.topic("View: outline", "app.view");
    s.topic("View: full", "app.view");
    assert_eq!(s.showing().as_deref(), Some("held"));
    assert_eq!(s.pending(), ["View: full"]);
}

#[test]
fn shows_queued_warnings_one_after_another() {
    let _g = lock(Some("dark"));
    let mut s = setup(None);
    s.warning("first");
    s.warning("second");
    s.warning("third");
    assert_eq!(s.showing().as_deref(), Some("first"));
    s.advance(WARNING);
    assert_eq!(s.showing().as_deref(), Some("second"));
    s.advance(WARNING);
    assert_eq!(s.showing().as_deref(), Some("third"));
    s.advance(WARNING);
    assert_eq!(s.showing(), None);
}

#[test]
fn stops_growing_rather_than_queueing_forever() {
    let _g = lock(Some("dark"));
    let mut s = setup(None);
    for i in 0..40 {
        s.warning(&format!("w{i}"));
    }
    assert!(s.pending().len() < 8);
    assert_eq!(s.showing().as_deref(), Some("w0"));
    assert_eq!(s.pending().last().map(String::as_str), Some("w39"));
}

#[test]
fn bounds_the_body_so_the_band_cannot_eat_the_conversation() {
    let _g = lock(Some("dark"));
    let mut s = setup(None);
    s.panel.notify(
        NotificationKind::Warning,
        "headline",
        &["a", "b", "c", "d", "e"],
        None,
        None,
        None,
    );
    assert_eq!(s.rows(WIDTH).len(), 5);
}

#[test]
fn asks_for_a_frame_when_it_appears_and_when_it_goes() {
    let _g = lock(Some("dark"));
    let mut s = setup(None);
    s.info("Model: a");
    assert_eq!(s.renders.get(), 1);
    s.advance(INFO);
    assert_eq!(s.renders.get(), 2);
}

#[test]
fn draws_plain_rows_with_no_fill_indented_by_one() {
    let _g = lock(Some("dark"));
    let mut s = setup(None);
    s.panel.notify(
        NotificationKind::Info,
        "Mode: build",
        &["described in .hoo/modes/build.md"],
        None,
        None,
        None,
    );
    let lines = s.panel.render(WIDTH);
    assert_eq!(lines[0], "");
    for line in &lines[1..] {
        assert!(!line.contains("\x1b[48;"), "no background: {line:?}");
    }
    let plain = s.rows(WIDTH);
    assert_eq!(plain[1], " Mode: build");
    assert_eq!(plain[2], " described in .hoo/modes/build.md");
}

#[test]
fn a_warning_is_ink_bold_with_a_title_over_dim_rows() {
    let _g = lock(Some("dark"));
    let mut s = setup(None);
    s.panel.notify(
        NotificationKind::Warning,
        "Sign in to start",
        &["Run /login."],
        None,
        None,
        None,
    );
    let lines = s.panel.render(WIDTH);
    assert!(
        lines[1].contains(theme().get_fg_ansi("ink")),
        "{:?}",
        lines[1]
    );
    assert!(lines[1].contains("\x1b[1m"));
    assert!(
        lines[2].contains(theme().get_fg_ansi("dim")),
        "{:?}",
        lines[2]
    );
    assert!(!lines[2].contains(theme().get_fg_ansi("warning")));
}

#[test]
fn a_long_title_wraps_to_three_rows_and_ends_in_an_ellipsis() {
    let _g = lock(Some("dark"));
    let mut s = setup(None);
    s.panel.set_status_slot_free(false);
    let long = "word ".repeat(40);
    s.warning(long.trim());
    let rows = s.rows(30);
    assert_eq!(rows.len(), 4, "{rows:?}");
    assert!(rows[3].ends_with('…'), "{rows:?}");
}

#[test]
fn a_notice_that_needs_an_action_never_times_out_until_dismissed() {
    let _g = lock(Some("dark"));
    let mut s = setup(None);
    s.panel.notify_action(
        NotificationKind::Warning,
        "Sign in again",
        &[],
        Some("/login to sign in"),
        "login",
    );
    assert_eq!(s.panel.deadline(), None);
    s.advance(60_000);
    assert_eq!(s.showing().as_deref(), Some("Sign in again"));
    s.panel.dismiss("login");
    assert_eq!(s.showing(), None);
}

#[test]
fn takes_the_rows_the_screen_can_spare_for_a_listing() {
    let _g = lock(Some("dark"));
    let mut s = setup(Some(Box::new(|| 6)));
    s.panel.notify(
        NotificationKind::Info,
        "Marketplaces",
        &["one", "two", "three", "four", "five", "six", "seven"],
        None,
        None,
        None,
    );
    assert_eq!(s.rows(WIDTH).len(), 8);
}

#[test]
fn never_takes_less_than_one_row_of_body() {
    let _g = lock(Some("dark"));
    let mut s = setup(Some(Box::new(|| 0)));
    s.panel.notify(
        NotificationKind::Info,
        "Marketplaces",
        &["one", "two"],
        None,
        None,
        None,
    );
    assert_eq!(s.rows(WIDTH).len(), 3);
}

#[test]
fn gives_a_listing_the_time_to_be_read() {
    let _g = lock(Some("dark"));
    let mut s = setup(Some(Box::new(|| 6)));
    s.panel.notify(
        NotificationKind::Info,
        "Marketplaces",
        &["one", "two", "three"],
        None,
        None,
        None,
    );
    s.advance(INFO);
    assert_eq!(s.showing().as_deref(), Some("Marketplaces"));
    s.advance(INFO * 3);
    assert_eq!(s.showing(), None);
}

#[test]
fn lets_a_listing_keep_the_colours_it_painted_itself() {
    let _g = lock(Some("dark"));
    let mut s = setup(Some(Box::new(|| 4)));
    let accent = theme().get_fg_ansi("accent").to_string();
    let coloured = format!("{accent}plugin\x1b[39m  installed");
    s.panel.notify(
        NotificationKind::Info,
        "Marketplaces",
        &[&coloured],
        None,
        None,
        None,
    );
    let body = s.panel.render(WIDTH)[2].clone();
    assert!(body.contains(&accent));
    assert!(!body.contains(theme().get_fg_ansi("muted")));
}

#[test]
fn redraws_when_the_screen_gets_shorter_not_just_narrower() {
    let _g = lock(Some("dark"));
    let budget = Rc::new(Cell::new(6));
    let b = budget.clone();
    let mut s = setup(Some(Box::new(move || b.get())));
    s.panel.notify(
        NotificationKind::Info,
        "Marketplaces",
        &["one", "two", "three", "four", "five", "six"],
        None,
        None,
        None,
    );
    assert_eq!(s.rows(WIDTH).len(), 8);
    budget.set(2);
    assert_eq!(s.rows(WIDTH).len(), 4);
}

#[test]
fn lets_a_caller_set_the_time_itself() {
    let _g = lock(Some("dark"));
    let mut s = setup(None);
    s.panel.notify(
        NotificationKind::Info,
        "quick",
        &["a", "b"],
        None,
        Some(Duration::from_millis(500)),
        None,
    );
    s.advance(500);
    assert_eq!(s.showing(), None);
}

#[test]
fn joined_to_the_task_pane_the_band_takes_the_pane_fill_at_indent_three() {
    let _g = lock(Some("dark"));
    let mut s = setup(None);
    s.panel.notify(
        NotificationKind::Info,
        "Mode: build",
        &["described in .hoo/modes/build.md"],
        None,
        None,
        None,
    );
    s.panel.set_joined_to_pane(true);
    let lines = s.panel.render(WIDTH);
    assert_eq!(lines.len(), 2, "no blank row above the band: {lines:?}");
    for line in &lines {
        assert!(line.contains(";48;2;"), "pane fill on every row: {line:?}");
    }
    let plain = s.rows(WIDTH);
    assert_eq!(plain[0], "   Mode: build");
    assert_eq!(plain[1], "   described in .hoo/modes/build.md");
}

#[test]
fn not_joined_the_band_renders_as_before() {
    let _g = lock(Some("dark"));
    let mut s = setup(None);
    s.panel.notify(
        NotificationKind::Info,
        "Mode: build",
        &["described in .hoo/modes/build.md"],
        None,
        None,
        None,
    );
    s.panel.set_joined_to_pane(true);
    s.panel.set_joined_to_pane(false);
    let lines = s.panel.render(WIDTH);
    assert_eq!(lines[0], "");
    assert!(lines.iter().skip(1).all(|l| !l.contains(";48;2;")));
    let plain = s.rows(WIDTH);
    assert_eq!(plain[1], " Mode: build");
}

#[test]
fn joined_in_monochrome_the_band_keeps_its_rows_and_drops_the_fill() {
    let _g = lock(Some("dark"));
    hoocode_code_tui_theme::chalk::set_enabled(false);
    let mut s = setup(None);
    s.panel.notify(
        NotificationKind::Info,
        "Mode: build",
        &["described in .hoo/modes/build.md"],
        None,
        None,
        None,
    );
    s.panel.set_joined_to_pane(true);
    let lines = s.panel.render(WIDTH);
    let plain = s.rows(WIDTH);
    hoocode_code_tui_theme::chalk::set_enabled(true);
    // Indent 3, no blank row above, no border and no fill.
    assert_eq!(lines[0], "   Mode: build");
    assert!(lines.iter().all(|l| !l.contains(";48;2;")));
    assert_eq!(plain[0], "   Mode: build");
}

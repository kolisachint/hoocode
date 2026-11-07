//! Dialog and notice goldens (UI-6, pinned by UI-21 P0).
//!
//! The notices the panel draws above the prompt: an info, an info with a
//! value, a warning, and one that needs an action. Dark, plus one light.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use crate::support::lock;
use hoocode_code_tui_app::notification_panel::{NotificationKind, NotificationPanel};
use hoocode_tui_render::assert_golden;
use hoocode_tui_render::golden::render_golden;

fn panel() -> NotificationPanel {
    let now = Rc::new(Cell::new(Instant::now()));
    let clock = now.clone();
    let mut p = NotificationPanel::with_clock(|| {}, None, Rc::new(move || clock.get()));
    // Body-less notices belong in the status line while its slot is free;
    // pin the band copy instead.
    p.set_status_slot_free(false);
    p
}

#[test]
fn notice_info() {
    let _g = lock(Some("dark"));
    let mut p = panel();
    p.notify(
        NotificationKind::Info,
        "Session saved to ~/.hoocode/sessions/abc123.json",
        &[],
        None,
        None,
        None,
    );
    assert_golden!("notice_info_80", render_golden(&mut p, 80));
}

#[test]
fn notice_info_with_value() {
    let _g = lock(Some("dark"));
    let mut p = panel();
    p.notify(
        NotificationKind::Info,
        "Context at 82%: 164,203 of 200,000 tokens",
        &["Summarize the thread or start a fresh session."],
        Some("ctrl+x n new session"),
        None,
        None,
    );
    assert_golden!("notice_info_value_80", render_golden(&mut p, 80));
}

#[test]
fn notice_warning() {
    let _g = lock(Some("dark"));
    let mut p = panel();
    p.notify(
        NotificationKind::Warning,
        "API key expires in 3 days",
        &["Regenerate it in the dashboard, then run /login again."],
        None,
        None,
        None,
    );
    assert_golden!("notice_warning_80", render_golden(&mut p, 80));
}

#[test]
fn notice_action_required() {
    let _g = lock(Some("dark"));
    let mut p = panel();
    p.notify_action(
        NotificationKind::Warning,
        "Background job job-1 needs input",
        &["Attach to answer it."],
        None,
        "job-1",
    );
    assert_golden!("notice_action_80", render_golden(&mut p, 80));
}

#[test]
fn notice_info_light() {
    let _g = lock(Some("light"));
    let mut p = panel();
    p.notify(
        NotificationKind::Info,
        "Session saved to ~/.hoocode/sessions/abc123.json",
        &[],
        None,
        None,
        None,
    );
    assert_golden!("notice_info_light_80", render_golden(&mut p, 80));
}

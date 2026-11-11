//! Dialog and notice goldens (UI-6, pinned by UI-21 P0).
//!
//! The notices the panel draws above the prompt: an info, an info with a
//! value, a warning, and one that needs an action. Dark, plus one light.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use crate::support::{lock, look, Look};
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

/// The notices at 100 columns in each look: the band above the prompt.
fn info_notice(p: &mut NotificationPanel) {
    p.notify(
        NotificationKind::Info,
        "Session saved to ~/.hoocode/sessions/abc123.json",
        &[],
        None,
        None,
        None,
    );
}

fn info_value_notice(p: &mut NotificationPanel) {
    p.notify(
        NotificationKind::Info,
        "Context at 82%: 164,203 of 200,000 tokens",
        &["Summarize the thread or start a fresh session."],
        Some("ctrl+x n new session"),
        None,
        None,
    );
}

fn warning_notice(p: &mut NotificationPanel) {
    p.notify(
        NotificationKind::Warning,
        "API key expires in 3 days",
        &["Regenerate it in the dashboard, then run /login again."],
        None,
        None,
        None,
    );
}

fn joined_notice(p: &mut NotificationPanel) {
    p.notify(
        NotificationKind::Info,
        "Mode: build",
        &["described in .hoo/modes/build.md"],
        None,
        None,
        None,
    );
    p.set_joined_to_pane(true);
}

/// Draws one notice at 100 columns in `variant`, and checks the golden.
fn notice_golden(name: &str, variant: Look, fill: fn(&mut NotificationPanel)) {
    let _g = lock(Some("dark"));
    look(variant);
    let mut p = panel();
    fill(&mut p);
    assert_golden!(name, render_golden(&mut p, 100));
}

#[test]
fn notice_info_dark_100() {
    notice_golden("notice_info_dark_100", Look::Dark, info_notice);
}

#[test]
fn notice_info_light_100() {
    notice_golden("notice_info_light_100", Look::Light, info_notice);
}

#[test]
fn notice_info_mono_100() {
    notice_golden("notice_info_mono_100", Look::Mono, info_notice);
}

#[test]
fn notice_info_value_dark_100() {
    notice_golden("notice_info_value_dark_100", Look::Dark, info_value_notice);
}

#[test]
fn notice_info_value_light_100() {
    notice_golden(
        "notice_info_value_light_100",
        Look::Light,
        info_value_notice,
    );
}

#[test]
fn notice_info_value_mono_100() {
    notice_golden("notice_info_value_mono_100", Look::Mono, info_value_notice);
}

#[test]
fn notice_warning_dark_100() {
    notice_golden("notice_warning_dark_100", Look::Dark, warning_notice);
}

#[test]
fn notice_warning_light_100() {
    notice_golden("notice_warning_light_100", Look::Light, warning_notice);
}

#[test]
fn notice_warning_mono_100() {
    notice_golden("notice_warning_mono_100", Look::Mono, warning_notice);
}

#[test]
fn notice_joined_dark_100() {
    notice_golden("notice_joined_dark_100", Look::Dark, joined_notice);
}

#[test]
fn notice_joined_light_100() {
    notice_golden("notice_joined_light_100", Look::Light, joined_notice);
}

#[test]
fn notice_joined_mono_100() {
    notice_golden("notice_joined_mono_100", Look::Mono, joined_notice);
}

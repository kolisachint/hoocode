//! Transmission toasts (M8 e4) as component goldens: the round frame at
//! top right, the draining bar, and the panel feeding the newest notice.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use hoocode_code_tui_app::notification_panel::{NotificationKind, NotificationPanel};
use hoocode_code_tui_app::transmission::{transmission_component, TransmissionData, TOAST_WIDTH};
use hoocode_tui_render::golden::render_golden;
use hoocode_tui_render::{assert_golden, Component};

use crate::support::{lock, plain};

fn data() -> TransmissionData {
    TransmissionData {
        title: "chrome is now compact".to_string(),
        body: vec![],
        note: Some("shift+alt+z undo".to_string()),
        remaining: 1.0,
    }
}

#[test]
fn toast_golden() {
    let _g = lock(Some("dark"));
    let mut toast = transmission_component(data(), TOAST_WIDTH);
    assert_golden!("transmission_toast_46", render_golden(&mut toast, 46));
}

#[test]
fn toast_half_drained_golden() {
    let _g = lock(Some("dark"));
    let mut half = transmission_component(
        TransmissionData {
            remaining: 0.5,
            ..data()
        },
        TOAST_WIDTH,
    );
    assert_golden!("transmission_toast_half_46", render_golden(&mut half, 46));
}

fn panel() -> (NotificationPanel, Rc<Cell<Instant>>) {
    let now = Rc::new(Cell::new(Instant::now()));
    let clock = now.clone();
    let panel = NotificationPanel::with_clock(|| {}, None, Rc::new(move || clock.get()));
    (panel, now)
}

/// The toast lines stripped of styling.
fn toast_text(panel: &NotificationPanel, width: usize) -> Vec<String> {
    panel
        .toast_lines(width)
        .iter()
        .map(|l| plain(l).trim_end().to_string())
        .collect()
}

#[test]
fn panel_floats_the_newest_notice_as_a_toast() {
    let _g = lock(Some("dark"));
    let (mut panel, _) = panel();
    panel.notify(NotificationKind::Info, "first", &[], None, None, None);
    panel.notify(NotificationKind::Info, "second", &[], None, None, None);
    let lines = toast_text(&panel, 100);
    assert!(lines[0].contains("TRANSMISSION"), "{lines:?}");
    assert!(lines[1].contains("second"), "{lines:?}");
    assert!(!lines[1].contains("first"), "{lines:?}");
}

#[test]
fn panel_drains_the_toast_as_its_time_runs_out() {
    let _g = lock(Some("dark"));
    let (mut panel, now) = panel();
    panel.notify(
        NotificationKind::Info,
        "chrome is now compact",
        &[],
        Some("shift+alt+z undo"),
        None,
        None,
    );
    let full = toast_text(&panel, 100);
    assert_eq!(
        full[2].matches('\u{2594}').count(),
        TOAST_WIDTH - 4,
        "{full:?}"
    );
    now.set(now.get() + Duration::from_millis(1500));
    let half = toast_text(&panel, 100);
    assert_eq!(
        half[2].matches('\u{2594}').count(),
        (TOAST_WIDTH - 4) / 2,
        "{half:?}"
    );
}

#[test]
fn overlay_mode_quiets_the_band_but_keeps_the_toast() {
    let _g = lock(Some("dark"));
    let (mut panel, _) = panel();
    panel.notify(
        NotificationKind::Info,
        "chrome is now compact",
        &[],
        None,
        None,
        None,
    );
    assert!(!panel.render(100).is_empty());
    panel.set_overlay(true);
    assert!(panel.render(100).is_empty());
    assert!(!panel.toast_lines(100).is_empty());
    panel.set_overlay(false);
    assert!(!panel.render(100).is_empty());
}

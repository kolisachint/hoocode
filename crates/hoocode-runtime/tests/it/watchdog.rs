//! Watchdog: UI stall detection on a fake clock, and the io starvation probe
//! on a real runtime with a deliberately blocked thread.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hoocode_runtime::watchdog::{
    probe_is_starved, run_starvation_probe, StallDetector, StallEvent, UI_FOOTER_STALL,
    UI_LOG_STALL,
};
use hoocode_runtime::{block_on_current_thread, block_on_entry};

#[test]
fn stall_detector_logs_once_at_half_a_second_and_flags_at_two() {
    let t0 = Instant::now();
    let mut d = StallDetector::new();
    let beat = Some((t0, "ui-loop:wait"));
    assert!(d.check(beat, t0).is_empty());

    let at = |ms: u64| t0 + Duration::from_millis(ms);
    assert!(d.check(beat, at(400)).is_empty(), "under 500 ms: quiet");
    assert_eq!(
        d.check(beat, at(600)),
        vec![StallEvent::Slow {
            phase: "ui-loop:wait",
            silent: Duration::from_millis(600)
        }]
    );
    assert!(
        d.check(beat, at(900)).is_empty(),
        "the phase is logged once"
    );
    assert!(d.check(beat, at(1900)).is_empty(), "not yet 2 s");
    assert_eq!(
        d.check(beat, at(2100)),
        vec![StallEvent::Stalled {
            phase: "ui-loop:wait",
            silent: Duration::from_millis(2100)
        }]
    );
    assert!(
        d.check(beat, at(3000)).is_empty(),
        "the flag is raised once"
    );
}

#[test]
fn a_new_beat_after_a_stall_reports_recovery_with_its_length() {
    let t0 = Instant::now();
    let mut d = StallDetector::new();
    let old = Some((t0, "ui-loop:wait"));
    d.check(old, t0);
    d.check(old, t0 + UI_FOOTER_STALL + Duration::from_millis(10));

    let beat_at = t0 + Duration::from_millis(2500);
    let events = d.check(Some((beat_at, "ui-loop:wait")), beat_at);
    assert_eq!(
        events,
        vec![StallEvent::Recovered {
            silent: Duration::from_millis(2500)
        }]
    );
    // The fresh beat starts a new silence: no stall right after it.
    assert!(d
        .check(
            Some((beat_at, "ui-loop:wait")),
            beat_at + Duration::from_millis(100)
        )
        .is_empty());
}

#[test]
fn a_fresh_beat_before_two_seconds_is_not_a_stall() {
    let t0 = Instant::now();
    let mut d = StallDetector::new();
    d.check(Some((t0, "a")), t0);
    let slow = d.check(Some((t0, "a")), t0 + UI_LOG_STALL);
    assert_eq!(slow.len(), 1);
    let beat_at = t0 + UI_LOG_STALL + Duration::from_millis(50);
    assert!(d.check(Some((beat_at, "b")), beat_at).is_empty());
}

#[test]
fn stopping_the_ui_ends_an_open_stall_quietly_with_a_recovery() {
    let t0 = Instant::now();
    let mut d = StallDetector::new();
    d.check(Some((t0, "a")), t0);
    d.check(Some((t0, "a")), t0 + UI_FOOTER_STALL);
    let events = d.check(None, t0 + UI_FOOTER_STALL + Duration::from_secs(1));
    assert_eq!(
        events,
        vec![StallEvent::Recovered {
            silent: UI_FOOTER_STALL + Duration::from_secs(1)
        }]
    );
    assert!(d.check(None, t0 + Duration::from_secs(10)).is_empty());
}

#[test]
fn no_beat_yet_means_no_judgement() {
    let mut d = StallDetector::new();
    assert!(d
        .check(None, Instant::now() + Duration::from_secs(60))
        .is_empty());
}

#[test]
fn probe_lateness_is_measured_against_its_schedule() {
    let due = Instant::now();
    assert!(!probe_is_starved(
        due,
        due + Duration::from_millis(249),
        Duration::from_millis(250)
    ));
    assert!(probe_is_starved(
        due,
        due + Duration::from_millis(251),
        Duration::from_millis(250)
    ));
}

#[test]
fn starvation_probe_reports_a_late_wake_up() {
    let reports = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&reports);
    block_on_current_thread(async move {
        let probe = tokio::spawn(run_starvation_probe(
            Duration::from_millis(20),
            Duration::from_millis(50),
            Some(1),
            move |late| sink.lock().unwrap().push(late),
        ));
        // Let the probe start its timer, then hold the only thread for 200 ms.
        tokio::task::yield_now().await;
        std::thread::sleep(Duration::from_millis(200));
        probe.await.expect("probe finished");
    });
    let reports = reports.lock().unwrap();
    assert_eq!(reports.len(), 1, "the blocked wake-up is reported");
    assert!(
        reports[0] >= Duration::from_millis(100),
        "late by {:?}",
        reports[0]
    );
}

#[test]
fn starvation_probe_is_quiet_when_the_runtime_is_free() {
    let reports = Arc::new(Mutex::new(0u32));
    let sink = Arc::clone(&reports);
    block_on_entry(run_starvation_probe(
        Duration::from_millis(20),
        // Generous threshold: a loaded CI box can wake a timer late.
        Duration::from_millis(500),
        Some(3),
        move |_| *sink.lock().unwrap() += 1,
    ));
    assert_eq!(*reports.lock().unwrap(), 0);
}

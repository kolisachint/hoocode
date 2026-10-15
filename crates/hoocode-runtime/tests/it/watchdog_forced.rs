//! Control run for the watchdog probes: a forced UI stall and a forced io
//! starvation must make the real probes fire. This drives the production
//! wiring (the `hoocode-watchdog` thread and the io probe that `start` spawns)
//! with short thresholds, so each check finishes well under a second.
//!
//! The watchdog is process-wide and starts once, so both tests share it and
//! tell their own lines apart by text.

use std::sync::mpsc;
use std::sync::{Mutex, Once};
use std::time::{Duration, Instant};

use hoocode_runtime::io_handle;
use hoocode_runtime::watchdog::{self, Thresholds};

/// Short enough for fast tests, long enough that a loaded box does not trip
/// the stall detector on a healthy beat (the UI test beats only while it runs).
const TEST_THRESHOLDS: Thresholds = Thresholds {
    ui_log_stall: Duration::from_millis(50),
    ui_footer_stall: Duration::from_millis(150),
    probe_period: Duration::from_millis(20),
    probe_late: Duration::from_millis(50),
};
/// Upper bound on any wait. A healthy run needs well under a second.
const DEADLINE: Duration = Duration::from_secs(5);
/// How long the forced starvation holds every io worker.
const HOLD: Duration = Duration::from_millis(400);

static LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());
static START: Once = Once::new();

fn ensure_started() {
    START.call_once(|| {
        watchdog::set_log_sink(Box::new(|line: &str| {
            LINES
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(line.to_owned());
        }));
        watchdog::start_with(TEST_THRESHOLDS).expect("watchdog starts");
    });
}

fn lines_containing(needle: &str) -> Vec<String> {
    LINES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .filter(|line| line.contains(needle))
        .cloned()
        .collect()
}

fn wait_until(what: &str, mut ready: impl FnMut() -> bool) {
    let start = Instant::now();
    while !ready() {
        assert!(start.elapsed() < DEADLINE, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_forced_ui_stall_fires_the_slow_log_the_footer_flag_and_the_recovery() {
    ensure_started();
    let (resume_tx, resume_rx) = mpsc::channel::<()>();
    let ui = std::thread::spawn(move || {
        watchdog::ui_beat("ui-loop:wait");
        // The forced stall: this "UI loop" blocks until the test resumes it,
        // without beating, like a loop stuck in a slow render.
        let _ = resume_rx.recv();
        watchdog::ui_beat("ui-loop:input-done");
    });

    // The report line is written after the flag is raised, so wait for the
    // line and then check the flag.
    wait_until("the UI stall line", || {
        !lines_containing("UI stalled").is_empty()
    });
    let slow = lines_containing("UI loop silent");
    assert_eq!(slow.len(), 1, "the slow line is logged once: {slow:?}");
    assert!(slow[0].contains("ui-loop:wait"), "phase missing: {slow:?}");
    let stalled = lines_containing("UI stalled");
    assert_eq!(stalled.len(), 1, "the stall is reported once: {stalled:?}");
    assert!(
        stalled[0].contains("ui-loop:wait"),
        "phase missing: {stalled:?}"
    );
    assert!(
        watchdog::ui_stalled(),
        "the footer flag is up during the stall"
    );

    resume_tx.send(()).expect("UI thread still waiting");
    wait_until("the resume line", || {
        !lines_containing("UI loop resumed").is_empty()
    });
    assert!(!watchdog::ui_stalled(), "the footer flag clears on resume");
    ui.join().expect("UI thread finished");
    watchdog::ui_stopped();
}

#[test]
fn a_forced_io_starvation_fires_the_starvation_probe() {
    ensure_started();
    let before = lines_containing("io runtime starved").len();
    // Hold every io worker thread: the probe's timer cannot be serviced while
    // all of them are blocked, so the probe wakes late.
    let workers = io_handle().metrics().num_workers();
    for _ in 0..workers {
        drop(io_handle().spawn(async {
            std::thread::sleep(HOLD);
        }));
    }
    wait_until("the starvation line", || {
        lines_containing("io runtime starved").len() > before
    });
}

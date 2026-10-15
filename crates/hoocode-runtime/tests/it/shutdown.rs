//! The ordered shutdown: step order, per-step deadlines, panics, and the budget.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hoocode_runtime::{
    fits_budget, run_ordered, sync_bounded_channel, ShutdownStep, StepOutcome, StepReport,
};

type Log = Arc<Mutex<Vec<&'static str>>>;

fn recorder() -> Log {
    Arc::new(Mutex::new(Vec::new()))
}

fn record(log: &Log, name: &'static str) -> impl FnOnce() + Send + 'static {
    let log = Arc::clone(log);
    move || log.lock().unwrap().push(name)
}

fn outcomes(reports: &[StepReport]) -> Vec<(&'static str, StepOutcome)> {
    reports.iter().map(|r| (r.name, r.outcome)).collect()
}

#[test]
fn steps_run_in_order_and_the_restore_runs_last() {
    let log = recorder();
    let ms = Duration::from_millis(500);
    let steps = vec![
        ShutdownStep::new("abort agent", ms, record(&log, "abort")),
        ShutdownStep::new("kill tools", ms, record(&log, "kill")),
        ShutdownStep::new("flush session", ms, record(&log, "flush")),
        ShutdownStep::new("close mcp", ms, record(&log, "mcp")),
    ];
    let reports = run_ordered(steps, record(&log, "restore"));

    assert_eq!(
        log.lock().unwrap().as_slice(),
        ["abort", "kill", "flush", "mcp", "restore"]
    );
    assert_eq!(
        outcomes(&reports),
        vec![
            ("abort agent", StepOutcome::Done),
            ("kill tools", StepOutcome::Done),
            ("flush session", StepOutcome::Done),
            ("close mcp", StepOutcome::Done),
        ]
    );
}

#[test]
fn a_hanging_step_is_cut_at_its_deadline_and_later_steps_still_run() {
    let log = recorder();
    // The hanging step waits until `hold` is dropped at the end of the test.
    let (hold, parked) = sync_bounded_channel::<()>(1);
    let hang = ShutdownStep::new("flush session", Duration::from_millis(150), move || {
        let _ = parked.recv();
    });
    let steps = vec![
        ShutdownStep::new(
            "abort agent",
            Duration::from_millis(500),
            record(&log, "abort"),
        ),
        hang,
        ShutdownStep::new(
            "release",
            Duration::from_millis(500),
            record(&log, "release"),
        ),
    ];

    let started = Instant::now();
    let reports = run_ordered(steps, record(&log, "restore"));
    let elapsed = started.elapsed();

    assert_eq!(
        outcomes(&reports),
        vec![
            ("abort agent", StepOutcome::Done),
            ("flush session", StepOutcome::TimedOut),
            ("release", StepOutcome::Done),
        ]
    );
    assert!(
        reports[1].elapsed >= Duration::from_millis(150),
        "cut at the deadline, not early: {:?}",
        reports[1].elapsed
    );
    assert!(
        reports[1].elapsed < Duration::from_millis(900),
        "cut at the deadline, not late: {:?}",
        reports[1].elapsed
    );
    assert_eq!(
        log.lock().unwrap().as_slice(),
        ["abort", "release", "restore"]
    );
    assert!(elapsed < Duration::from_secs(1), "total {elapsed:?}");
    drop(hold);
}

#[test]
fn a_panicking_step_fails_and_the_rest_still_runs() {
    let log = recorder();
    let steps = vec![
        ShutdownStep::new("kill tools", Duration::from_millis(500), || {
            panic!("kill step panicked on purpose")
        }),
        ShutdownStep::new(
            "release",
            Duration::from_millis(500),
            record(&log, "release"),
        ),
    ];
    let reports = run_ordered(steps, record(&log, "restore"));

    assert_eq!(
        outcomes(&reports),
        vec![
            ("kill tools", StepOutcome::Failed),
            ("release", StepOutcome::Done),
        ]
    );
    assert_eq!(log.lock().unwrap().as_slice(), ["release", "restore"]);
}

#[test]
fn hanging_steps_stay_within_their_deadlines_in_total() {
    let (hold_a, parked_a) = sync_bounded_channel::<()>(1);
    let (hold_b, parked_b) = sync_bounded_channel::<()>(1);
    let steps = vec![
        ShutdownStep::new("abort agent", Duration::from_millis(200), move || {
            let _ = parked_a.recv();
        }),
        ShutdownStep::new("flush session", Duration::from_millis(200), move || {
            let _ = parked_b.recv();
        }),
    ];
    let started = Instant::now();
    let reports = run_ordered(steps, || {});
    let elapsed = started.elapsed();

    assert!(reports.iter().all(|r| r.outcome == StepOutcome::TimedOut));
    assert!(
        elapsed >= Duration::from_millis(400),
        "both waited: {elapsed:?}"
    );
    assert!(elapsed < Duration::from_secs(1), "total {elapsed:?}");
    drop((hold_a, hold_b));
}

#[test]
fn the_budget_counts_the_restore_reserve() {
    let at_limit = vec![ShutdownStep::new("a", Duration::from_millis(2800), || {})];
    assert!(
        fits_budget(&at_limit),
        "2.8 s of steps plus 0.2 s restore is 3 s"
    );

    let over = vec![
        ShutdownStep::new("a", Duration::from_millis(2000), || {}),
        ShutdownStep::new("b", Duration::from_millis(801), || {}),
    ];
    assert!(!fits_budget(&over));
}

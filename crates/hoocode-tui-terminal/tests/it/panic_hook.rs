//! The installed panic hook, with real panics and no terminal: the UI thread
//! restores once and flushes, another thread only logs, and every panic chains
//! to the hook that was installed before.

use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use hoocode_runtime::spawn_named_thread;
use hoocode_tui_terminal::panic_hook::{install, Effects};

#[test]
fn installed_hook_restores_on_the_ui_thread_only_and_chains() {
    let restores = Arc::new(AtomicUsize::new(0));
    let flushes = Arc::new(AtomicUsize::new(0));
    let chained = Arc::new(AtomicUsize::new(0));
    let logged = Arc::new(Mutex::new(Vec::<String>::new()));

    // The hook installed before ours: it must still run for every panic.
    let counter = chained.clone();
    panic::set_hook(Box::new(move |_| {
        counter.fetch_add(1, Ordering::SeqCst);
    }));

    let effects = Effects {
        restore: {
            let restores = restores.clone();
            Box::new(move || {
                restores.fetch_add(1, Ordering::SeqCst);
            })
        },
        flush: {
            let flushes = flushes.clone();
            Box::new(move || {
                flushes.fetch_add(1, Ordering::SeqCst);
            })
        },
        log: {
            let logged = logged.clone();
            Box::new(move |report: &str| logged.lock().unwrap().push(report.to_owned()))
        },
    };
    assert!(install(thread::current().id(), effects), "first install");

    let second = Effects {
        restore: Box::new(|| {}),
        flush: Box::new(|| {}),
        log: Box::new(|_: &str| {}),
    };
    assert!(
        !install(thread::current().id(), second),
        "a second install changes nothing"
    );

    // Two panics on the UI thread: restore and flush run once, both are logged.
    let _ = panic::catch_unwind(AssertUnwindSafe(|| panic!("ui-marker-one")));
    let _ = panic::catch_unwind(AssertUnwindSafe(|| panic!("ui-marker-two")));
    assert_eq!(restores.load(Ordering::SeqCst), 1);
    assert_eq!(flushes.load(Ordering::SeqCst), 1);

    // A panic on another thread: logged with its name, terminal left alone.
    let worker =
        spawn_named_thread("panic-test-worker", || panic!("worker-marker")).expect("spawn worker");
    assert!(worker.join().is_err());
    assert_eq!(
        restores.load(Ordering::SeqCst),
        1,
        "a worker panic does not restore"
    );
    assert_eq!(flushes.load(Ordering::SeqCst), 1);

    let reports = logged.lock().unwrap().clone();
    assert_eq!(reports.len(), 3, "one log line per panic: {reports:?}");
    assert!(reports[0].contains("ui-marker-one"), "{}", reports[0]);
    assert!(
        reports[0].contains("panic_hook.rs"),
        "location is in the report: {}",
        reports[0]
    );
    assert!(reports[0].contains("backtrace:"), "{}", reports[0]);
    assert!(reports[1].contains("ui-marker-two"));
    assert!(
        reports[2].contains("panic-test-worker"),
        "thread name: {}",
        reports[2]
    );
    assert!(reports[2].contains("worker-marker"));

    assert_eq!(
        chained.load(Ordering::SeqCst),
        3,
        "every panic reaches the previous hook"
    );

    // Leave the process with the default hook again.
    let _ = panic::take_hook();
}

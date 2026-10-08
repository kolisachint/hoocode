//! Tests for the process runtime: worker count, thread names, the tools pool
//! cap, the parallel tool limit, the channel helpers and the session writer.

mod memory;
mod session_io;
mod watchdog;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use hoocode_runtime::{
    block_on_current_thread, block_on_entry, block_on_isolated, bounded_channel, io_handle,
    io_worker_count, run_blocking, spawn_isolated, spawn_named_thread, sync_bounded_channel,
    ParallelToolLimit, IO_CHILD_WORKERS, IO_MAX_WORKERS, IO_THREAD_PREFIX, MAX_PARALLEL_TOOLS,
    MIN_PARALLEL_TOOLS, TOOLS_MAX_THREADS, TOOLS_THREAD_PREFIX,
};

#[test]
fn sync_bridges_run_futures_and_report_panics() {
    assert_eq!(block_on_current_thread(async { 4 }), 4);
    // Callable from inside the io runtime: the bridge uses its own thread.
    let inner = block_on_entry(async { block_on_isolated(async { 6 }) });
    assert_eq!(inner.expect("no panic"), 6);
    let panicked = block_on_isolated(async { panic!("inside") });
    assert!(panicked.is_err());
    let handle = spawn_isolated("hoocode-test-bridge", async { 9 }).expect("thread starts");
    assert_eq!(handle.join().expect("no panic"), 9);
}

#[test]
fn io_worker_count_is_min_four_cores_and_two_in_children() {
    assert_eq!(io_worker_count(1, false), 1);
    assert_eq!(io_worker_count(2, false), 2);
    assert_eq!(io_worker_count(4, false), 4);
    assert_eq!(io_worker_count(16, false), IO_MAX_WORKERS);
    assert_eq!(io_worker_count(0, false), 1);
    assert_eq!(io_worker_count(1, true), IO_CHILD_WORKERS);
    assert_eq!(io_worker_count(16, true), 2);
}

#[test]
fn io_runtime_has_the_expected_worker_count() {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let expected = io_worker_count(cores, hoocode_runtime::is_subagent_child());
    assert_eq!(io_handle().metrics().num_workers(), expected);
}

#[test]
fn io_workers_have_hoocode_io_names() {
    let name = block_on_entry(
        io_handle().spawn(async { std::thread::current().name().map(str::to_owned) }),
    )
    .expect("task ran");
    let name = name.expect("io worker threads are named");
    assert!(
        name.starts_with(&format!("{IO_THREAD_PREFIX}-")),
        "unexpected worker name {name}"
    );
}

#[test]
fn run_blocking_returns_the_value_and_runs_off_the_io_workers() {
    let name = block_on_entry(run_blocking(|| {
        std::thread::current().name().map(str::to_owned)
    }))
    .expect("job ran");
    let name = name.expect("pool threads are named");
    assert!(
        name.starts_with(&format!("{TOOLS_THREAD_PREFIX}-")),
        "unexpected pool thread name {name}"
    );
    let sum = block_on_entry(run_blocking(|| 2 + 3)).expect("job ran");
    assert_eq!(sum, 5);
}

#[test]
fn run_blocking_turns_a_panic_into_an_error() {
    let result = block_on_entry(run_blocking(|| -> u32 { panic!("boom") }));
    assert!(result.is_err());
    // The pool is still usable afterwards.
    assert_eq!(block_on_entry(run_blocking(|| 7)).expect("job ran"), 7);
}

#[test]
fn run_blocking_never_runs_more_than_sixteen_jobs_at_once() {
    const JOBS: usize = 40;
    let active = Arc::new(AtomicUsize::new(0));
    let max_seen = Arc::new(AtomicUsize::new(0));
    let futures: Vec<_> = (0..JOBS)
        .map(|_| {
            let active = Arc::clone(&active);
            let max_seen = Arc::clone(&max_seen);
            run_blocking(move || {
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                max_seen.fetch_max(now, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(50));
                active.fetch_sub(1, Ordering::SeqCst);
            })
        })
        .collect();
    block_on_entry(async move {
        for f in futures {
            f.await.expect("job ran");
        }
    });
    let max = max_seen.load(Ordering::SeqCst);
    assert!(max <= TOOLS_MAX_THREADS, "{max} jobs ran at once");
    assert!(max > 1, "jobs did not run in parallel (max {max})");
}

#[test]
fn parallel_tool_limit_is_clamped() {
    assert_eq!(ParallelToolLimit::new(0).limit(), MIN_PARALLEL_TOOLS);
    assert_eq!(ParallelToolLimit::new(8).limit(), 8);
    assert_eq!(ParallelToolLimit::new(100).limit(), MAX_PARALLEL_TOOLS);
    assert_eq!(ParallelToolLimit::new(8).available(), 8);
}

#[test]
fn parallel_tool_limit_try_acquire_takes_a_free_slot_only() {
    let limit = ParallelToolLimit::new(1);
    let held = limit.try_acquire().expect("the one slot is free");
    assert!(limit.try_acquire().is_none(), "no slot left while held");
    drop(held);
    assert!(limit.try_acquire().is_some(), "the slot returns on drop");
}

#[test]
fn parallel_tool_limit_caps_concurrent_calls() {
    const CALLS: usize = 20;
    const LIMIT: usize = 3;
    let limit = ParallelToolLimit::new(LIMIT);
    let active = Arc::new(AtomicUsize::new(0));
    let max_seen = Arc::new(AtomicUsize::new(0));
    block_on_entry(async {
        let mut tasks = Vec::new();
        for _ in 0..CALLS {
            let limit = limit.clone();
            let active = Arc::clone(&active);
            let max_seen = Arc::clone(&max_seen);
            tasks.push(io_handle().spawn(async move {
                let _permit = limit.acquire().await;
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                max_seen.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(10)).await;
                active.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        for t in tasks {
            t.await.expect("call ran");
        }
    });
    assert_eq!(max_seen.load(Ordering::SeqCst), LIMIT);
    assert_eq!(limit.available(), LIMIT, "all permits are returned");
}

#[test]
fn spawn_named_thread_sets_the_name() {
    let handle = spawn_named_thread("hoocode-test-thread", || {
        std::thread::current().name().map(str::to_owned)
    })
    .expect("thread starts");
    assert_eq!(
        handle.join().expect("thread finishes").as_deref(),
        Some("hoocode-test-thread")
    );
}

#[test]
fn bounded_channel_applies_backpressure() {
    block_on_entry(async {
        let (tx, mut rx) = bounded_channel::<u32>(2);
        tx.send(1).await.expect("slot 1");
        tx.send(2).await.expect("slot 2");
        assert!(tx.try_send(3).is_err(), "third send must wait");
        assert_eq!(rx.recv().await, Some(1));
        tx.try_send(3).expect("slot freed");
    });
}

#[test]
fn bounded_channel_of_zero_still_works() {
    block_on_entry(async {
        let (tx, mut rx) = bounded_channel::<u8>(0);
        tokio::spawn(async move { tx.send(9).await.expect("sent") });
        assert_eq!(rx.recv().await, Some(9));
    });
}

#[test]
fn sync_bounded_channel_delivers_in_order() {
    let (tx, rx) = sync_bounded_channel::<u32>(1);
    let writer = std::thread::spawn(move || {
        for i in 0..5 {
            tx.send(i).expect("receiver alive");
        }
    });
    let got: Vec<u32> = rx.iter().collect();
    writer.join().expect("writer finishes");
    assert_eq!(got, vec![0, 1, 2, 3, 4]);
}

#[cfg(unix)]
#[test]
fn sigwinch_runs_the_callback_on_the_named_thread() {
    let (tx, rx) = std::sync::mpsc::channel();
    let watch = hoocode_runtime::watch_sigwinch("hoocode-test-sigwinch", move || {
        let name = std::thread::current().name().map(str::to_owned);
        let _ = tx.send(name);
    })
    .expect("watch starts");
    signal_hook::low_level::raise(signal_hook::consts::SIGWINCH).expect("raise SIGWINCH");
    let name = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("callback runs after SIGWINCH");
    assert_eq!(name.as_deref(), Some("hoocode-test-sigwinch"));
    // Dropping the watch closes the iterator, so the thread ends.
    drop(watch);
}

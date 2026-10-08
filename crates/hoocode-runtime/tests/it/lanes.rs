//! Lane assignment, OS priority per thread, `hoocode-bg` and child priority.
//! The priority checks read `/proc/thread-self/stat`, so the file runs on Linux only.

#![cfg(target_os = "linux")]

use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use hoocode_runtime::{
    block_on_entry, io_handle, lower_child_priority, run_blocking, spawn_bg, spawn_lane_thread,
    Lane, SUBAGENT_CHILD_NICE,
};

/// Niceness of the calling thread, from `/proc/thread-self/stat` (field 19).
fn thread_nice() -> i64 {
    let stat = std::fs::read_to_string("/proc/thread-self/stat").expect("read thread stat");
    nice_from_stat(&stat)
}

fn nice_from_stat(stat: &str) -> i64 {
    // The command name (field 2) may contain spaces; fields resume after the last ')'.
    let rest = &stat[stat.rfind(')').expect("stat has a comm field") + 1..];
    // Field 3 (state) is the first token here, so field 19 is index 16.
    rest.split_whitespace()
        .nth(16)
        .expect("stat has a nice field")
        .parse()
        .expect("nice is a number")
}

/// The nice a lane thread must show: its lane value, or the inherited value
/// when that is already higher (a process started at +5 is never raised).
fn expected_nice(baseline: i64, lane: Lane) -> i64 {
    baseline.max(i64::from(lane.linux_nice()))
}

#[test]
fn lane_threads_get_their_linux_niceness() {
    let baseline = thread_nice();
    for lane in [Lane::High, Lane::Medium, Lane::Low] {
        let seen = spawn_lane_thread("hoocode-test-lane", lane, thread_nice)
            .expect("thread starts")
            .join()
            .expect("no panic");
        assert_eq!(seen, expected_nice(baseline, lane), "{lane:?} lane");
    }
    // Low really is lower than Medium, which is lower than High.
    assert!(expected_nice(baseline, Lane::Low) > expected_nice(baseline, Lane::Medium));
}

#[test]
fn runtime_threads_are_assigned_their_lanes() {
    let baseline = thread_nice();
    // hoocode-io workers are High.
    let io = block_on_entry(async { io_handle().spawn(async { thread_nice() }).await })
        .expect("io task completes");
    assert_eq!(io, expected_nice(baseline, Lane::High), "io worker");
    // hoocode-tools pool threads are Medium.
    let tools = block_on_entry(run_blocking(thread_nice)).expect("tools job completes");
    assert_eq!(tools, expected_nice(baseline, Lane::Medium), "tools thread");
}

#[test]
fn bg_thread_is_low_lane_and_runs_futures() {
    let baseline = thread_nice();
    let (tx, rx) = mpsc::channel();
    let first = tx.clone();
    spawn_bg(async move {
        let name = std::thread::current().name().map(str::to_owned);
        first.send((name, thread_nice())).expect("receiver alive");
    });
    let (name, nice) = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("bg task ran");
    assert_eq!(name.as_deref(), Some(hoocode_runtime::BG_THREAD_NAME));
    assert_eq!(nice, expected_nice(baseline, Lane::Low));

    // Later futures run on the same thread, and their output comes back through the handle.
    let handle = spawn_bg(async { 21 * 2 });
    let (out_tx, out_rx) = mpsc::channel();
    spawn_bg(async move {
        out_tx
            .send(handle.await.expect("join"))
            .expect("receiver alive");
    });
    assert_eq!(
        out_rx.recv_timeout(Duration::from_secs(5)).expect("ran"),
        42
    );
}

#[test]
fn child_processes_start_at_the_requested_niceness() {
    let parent = thread_nice();
    let child_nice = |nice: u64| -> i64 {
        let mut command = Command::new("sh");
        command
            .args(["-c", "cat /proc/thread-self/stat"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped());
        lower_child_priority(&mut command, nice);
        let output = command.output().expect("sh runs");
        nice_from_stat(&String::from_utf8_lossy(&output.stdout))
    };
    // Subagent children: +5, or the parent's value if that is already higher.
    assert_eq!(
        child_nice(SUBAGENT_CHILD_NICE),
        parent.max(SUBAGENT_CHILD_NICE as i64)
    );
    // A nice of 0 leaves the child exactly as the parent is.
    assert_eq!(child_nice(0), parent);
    // Shell children at bashNice 10 (never above 19).
    assert_eq!(child_nice(10), parent.max(10));
    assert_eq!(child_nice(40), parent.max(19));
}

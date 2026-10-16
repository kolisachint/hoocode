#![allow(clippy::disallowed_methods)] // test code: #[tokio::test] expands to a runtime builder
//! lifeguard.test.ts. Children are `sleep` processes in their own process
//! group (as the pool spawns them).

use std::collections::HashMap;
use std::process::{Child, Command};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hoocode_code_subagents::lifeguard::{
    tool_limit_from_start, LifeguardEvent, SubagentLifeguard, ToolLimit, MAX_SCALED_DEADLINE_MS,
    PROGRESS_STALL_THRESHOLD_MS, SUBAGENT_DEADLINE_MS, SUBAGENT_SHELL_DEFAULT_TIMEOUT_SECS,
    TOOL_IN_FLIGHT_STALL_MS, TOOL_TIMEOUT_MARGIN_MS,
};
use hoocode_code_subagents::limits::{SubagentLimitSettings, SubagentLimits};

/// 2026-10-16: the default hard deadline is 4 hours (was 2), for every agent
/// type. A pool with another base (the `subagentDeadlineMinutes` setting or
/// `HOOCODE_SUBAGENT_DEADLINE_MS`) uses that base instead; see `limits`.
#[test]
fn the_default_deadline_is_four_hours_for_every_type() {
    const HOURS_4: u64 = 4 * 60 * 60 * 1000;
    assert_eq!(SUBAGENT_DEADLINE_MS, HOURS_4);
}

/// The load multiplier cannot stretch a run past base + 30 minutes: 4x of a
/// 4-hour base would be 16 hours. `AgentOutput` derives its reconcile age from
/// this same ceiling.
#[test]
fn the_load_scaled_deadline_is_capped_at_base_plus_thirty_minutes() {
    const MINUTE: u64 = 60 * 1000;
    assert_eq!(MAX_SCALED_DEADLINE_MS, SUBAGENT_DEADLINE_MS + 30 * MINUTE);
    assert_eq!(
        hoocode_code_subagents::lifeguard::max_run_budget_ms(SUBAGENT_DEADLINE_MS),
        MAX_SCALED_DEADLINE_MS
    );
    // Checked at compile time: uncapped, 4x the base would exceed the ceiling.
    const _: () = assert!(SUBAGENT_DEADLINE_MS * 4 > MAX_SCALED_DEADLINE_MS);
}

/// A short base scales by the multiplier (4x) below the headroom cap, and a
/// long base is capped at base + 30 minutes.
#[test]
fn the_load_ceiling_follows_the_configured_base() {
    use hoocode_code_subagents::lifeguard::max_run_budget_ms;
    const MINUTE: u64 = 60 * 1000;
    assert_eq!(max_run_budget_ms(10 * MINUTE), 40 * MINUTE);
    assert_eq!(max_run_budget_ms(120 * MINUTE), 150 * MINUTE);
}

#[test]
fn the_default_stall_threshold_is_twenty_minutes() {
    assert_eq!(PROGRESS_STALL_THRESHOLD_MS, 20 * 60 * 1000);
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn sleeper() -> Child {
    let mut command = Command::new("sleep");
    command.arg("10");
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    command.spawn().unwrap()
}

fn stalled_ids(guard: &SubagentLifeguard) -> Arc<Mutex<Vec<String>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    guard.on_event(move |event| {
        if let LifeguardEvent::Stalled { task_id, .. } = event {
            sink.lock().unwrap().push(task_id.clone());
        }
    });
    seen
}

#[tokio::test]
async fn monitors_a_child_process_and_records_heartbeats() {
    let dir = tempfile::tempdir().unwrap();
    let guard = SubagentLifeguard::new(dir.path());
    let mut child = sleeper();
    guard.monitor("t1", "explore", child.id());
    assert!(guard.is_monitoring("t1"));
    let before = guard.last_heartbeat_at("t1").unwrap();
    guard.record_heartbeat("t1");
    assert!(guard.last_heartbeat_at("t1").unwrap() >= before);
    guard.dispose();
    let _ = child.wait();
}

#[tokio::test]
async fn emits_stalled_when_heartbeat_is_missed() {
    let dir = tempfile::tempdir().unwrap();
    let guard = SubagentLifeguard::new(dir.path());
    let stalled = stalled_ids(&guard);
    let mut child = sleeper();
    guard.monitor("t1", "explore", child.id());
    guard.set_last_heartbeat_for_testing("t1", now_ms() - 70_000);
    guard.check_heartbeats();
    assert_eq!(*stalled.lock().unwrap(), vec!["t1"]);
    // The process group was killed.
    assert!(child.wait().unwrap().code().is_none());
    // Reaping: not re-reported on the next tick.
    guard.check_heartbeats();
    assert_eq!(stalled.lock().unwrap().len(), 1);
    guard.dispose();
}

#[tokio::test]
async fn does_not_stall_under_high_concurrency_until_the_scaled_threshold() {
    let dir = tempfile::tempdir().unwrap();
    let guard = SubagentLifeguard::new(dir.path());
    let stalled = stalled_ids(&guard);
    let mut children: Vec<Child> = (0..5).map(|_| sleeper()).collect();
    for (i, child) in children.iter().enumerate() {
        guard.monitor(&format!("t{i}"), "explore", child.id());
    }
    // 5 concurrent: multiplier 3, threshold 180s.
    guard.set_last_heartbeat_for_testing("t0", now_ms() - 150_000);
    guard.check_heartbeats();
    assert!(!stalled.lock().unwrap().contains(&"t0".to_string()));
    guard.set_last_heartbeat_for_testing("t0", now_ms() - 200_000);
    guard.check_heartbeats();
    assert!(stalled.lock().unwrap().contains(&"t0".to_string()));
    guard.dispose();
    for child in &mut children {
        let _ = child.wait();
    }
}

#[tokio::test]
async fn accounts_for_external_load_and_clamps_negative_load() {
    let dir = tempfile::tempdir().unwrap();
    let guard = SubagentLifeguard::new(dir.path());
    let stalled = stalled_ids(&guard);
    let mut child = sleeper();
    guard.monitor("t0", "explore", child.id());
    guard.set_external_load(4);
    guard.set_last_heartbeat_for_testing("t0", now_ms() - 150_000);
    guard.check_heartbeats();
    assert!(stalled.lock().unwrap().is_empty());
    guard.set_external_load(-5);
    guard.check_heartbeats();
    assert_eq!(*stalled.lock().unwrap(), vec!["t0"]);
    guard.dispose();
    let _ = child.wait();
}

#[tokio::test]
async fn forgives_a_stale_heartbeat_caused_by_parent_lag() {
    let dir = tempfile::tempdir().unwrap();
    let guard = SubagentLifeguard::new(dir.path());
    let stalled = stalled_ids(&guard);
    let mut child = sleeper();
    guard.monitor("t1", "explore", child.id());
    guard.set_last_heartbeat_for_testing("t1", now_ms() - 70_000);
    guard.set_last_check_for_testing(now_ms() - 200_000);
    guard.check_heartbeats();
    assert!(stalled.lock().unwrap().is_empty());
    guard.dispose();
    let _ = child.wait();
}

#[tokio::test]
async fn emits_timeout_when_the_hard_timeout_is_exceeded() {
    let dir = tempfile::tempdir().unwrap();
    let guard = SubagentLifeguard::new(dir.path());
    let timeouts = Arc::new(Mutex::new(Vec::new()));
    let sink = timeouts.clone();
    guard.on_event(move |event| {
        if let LifeguardEvent::Timeout { task_id, .. } = event {
            sink.lock().unwrap().push(task_id.clone());
        }
    });
    let mut child = sleeper();
    guard.monitor("t1", "explore", child.id());
    guard.set_timeout_for_testing("t1", Duration::from_millis(10));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(*timeouts.lock().unwrap(), vec!["t1"]);
    guard.dispose();
    let _ = child.wait();
}

#[tokio::test]
async fn untracks_a_process_on_exit() {
    let dir = tempfile::tempdir().unwrap();
    let guard = SubagentLifeguard::new(dir.path());
    let mut child = Command::new("true").spawn().unwrap();
    guard.monitor("t1", "explore", child.id());
    assert!(guard.is_monitoring("t1"));
    child.wait().unwrap();
    guard.untrack("t1");
    assert!(!guard.is_monitoring("t1"));
    guard.dispose();
}

fn backdate(path: &std::path::Path, hours: u64) {
    let past = SystemTime::now() - Duration::from_secs(hours * 3600);
    let file = std::fs::File::open(path).unwrap();
    file.set_modified(past).unwrap();
}

#[tokio::test]
async fn sweeps_old_agent_directories_on_init() {
    let dir = tempfile::tempdir().unwrap();
    let old = hoocode_code_paths::dispatch_task_dir(dir.path(), "old-task");
    std::fs::create_dir_all(&old).unwrap();
    std::fs::write(old.join("result.json"), "{}").unwrap();
    backdate(&old, 25);
    let guard = SubagentLifeguard::new(dir.path());
    wait_until_gone(&old).await;
    guard.dispose();
}

#[tokio::test]
async fn does_not_sweep_directories_with_running_pids() {
    let dir = tempfile::tempdir().unwrap();
    let old = hoocode_code_paths::dispatch_task_dir(dir.path(), "old-task-with-pid");
    std::fs::create_dir_all(&old).unwrap();
    std::fs::write(old.join("result.json"), "{}").unwrap();
    std::fs::write(old.join("pid"), std::process::id().to_string()).unwrap();
    backdate(&old, 25);
    let guard = SubagentLifeguard::new(dir.path());
    assert!(old.exists());
    guard.dispose();
}

#[tokio::test]
async fn dispose_kills_all_monitored_processes() {
    let dir = tempfile::tempdir().unwrap();
    let guard = SubagentLifeguard::new(dir.path());
    let mut child = sleeper();
    guard.monitor("t1", "explore", child.id());
    guard.dispose();
    assert!(!guard.is_monitoring("t1"));
    assert!(child.wait().unwrap().code().is_none());
}

/// A pinging child that finishes no turn and runs no tool is parked, not busy.
///
/// Measured 2026-10-05: a child blocked inside a provider call keeps writing
/// `{"ping":true}` from its heartbeat timer, so the silence threshold never
/// fired and the run could only end at the ten-minute hard deadline — 95s in,
/// the child was still alive and the ledger still empty. A second liveness
/// signal, "no forward progress", closes that hole.
#[tokio::test]
async fn stalls_a_child_that_pings_but_makes_no_progress() {
    let dir = tempfile::tempdir().unwrap();
    let guard = SubagentLifeguard::new(dir.path());
    let stalled = stalled_ids(&guard);
    let mut child = sleeper();
    guard.monitor("t1", "explore", child.id());
    // Heartbeats are current — the child is demonstrably alive — but it has
    // finished nothing for longer than the progress threshold (10 minutes).
    guard.record_heartbeat("t1");
    guard.set_last_progress_for_testing("t1", now_ms() - (PROGRESS_STALL_THRESHOLD_MS + 60_000));
    guard.check_heartbeats();
    assert_eq!(*stalled.lock().unwrap(), vec!["t1"]);
    assert!(child.wait().unwrap().code().is_none());
    guard.dispose();
}

/// Progress refreshes the second signal, so a slow-but-working child is left
/// alone: recorded subagent turns ran up to 65s.
#[tokio::test]
async fn progress_keeps_a_slow_child_alive() {
    let dir = tempfile::tempdir().unwrap();
    let guard = SubagentLifeguard::new(dir.path());
    let stalled = stalled_ids(&guard);
    let mut child = sleeper();
    guard.monitor("t1", "code-review", child.id());
    guard.set_last_progress_for_testing("t1", now_ms() - 100_000);
    guard.record_progress("t1");
    guard.record_heartbeat("t1");
    guard.check_heartbeats();
    assert!(stalled.lock().unwrap().is_empty());
    guard.dispose();
    let _ = child.wait();
}

/// A running tool is busy work, not a stall: a child inside an 18-minute
/// `cargo build` emits nothing between `tool_execution_start` and
/// `tool_execution_end`. Once the tool ends, the progress check applies again.
#[tokio::test]
async fn a_running_tool_is_not_a_progress_stall() {
    let dir = tempfile::tempdir().unwrap();
    let guard = SubagentLifeguard::new(dir.path());
    // A threshold below the tool bound (30 min), so the window "past the
    // progress threshold but inside the tool bound" exists. The test sets its
    // own, lower threshold so it need not wait out the 20-minute default.
    let threshold = 5 * 60 * 1000;
    guard.set_progress_stall_ms(threshold);
    let stalled = stalled_ids(&guard);
    let mut child = sleeper();
    guard.monitor("t1", "explore", child.id());
    guard.record_tool_start("t1", "c1", ToolLimit::Undeclared);
    guard.record_heartbeat("t1");
    // Well past the progress threshold, but inside the tool bound (30 min).
    guard.set_last_progress_for_testing("t1", now_ms() - (threshold + 60_000));
    guard.check_heartbeats();
    assert!(stalled.lock().unwrap().is_empty());
    // The tool ends and the child goes quiet: now the progress check applies.
    guard.record_tool_end("t1", "c1");
    guard.record_heartbeat("t1");
    guard.set_last_progress_for_testing("t1", now_ms() - (threshold + 60_000));
    guard.check_heartbeats();
    assert_eq!(*stalled.lock().unwrap(), vec!["t1"]);
    assert!(child.wait().unwrap().code().is_none());
    guard.dispose();
}

/// A stall reap is SIGUSR1 (the child writes its partial result and reports
/// `stalled`), then SIGKILL after the grace. `sleep` dies on SIGUSR1 by its
/// default action, which is the point: the status says which signal arrived.
#[cfg(unix)]
#[tokio::test]
async fn a_stalled_child_is_terminated_before_it_is_killed() {
    let dir = tempfile::tempdir().unwrap();
    let guard = SubagentLifeguard::new(dir.path());
    let stalled = stalled_ids(&guard);
    guard.set_term_grace_for_testing(500);
    let mut child = sleeper();
    guard.monitor("t1", "explore", child.id());
    guard.set_last_heartbeat_for_testing("t1", now_ms() - 70_000);
    guard.check_heartbeats();
    assert_eq!(*stalled.lock().unwrap(), vec!["t1"]);
    // SIGUSR1 reached it, not SIGKILL and not SIGTERM: a stall is reported as
    // a stall, never as a run that ran out of time.
    use std::os::unix::process::ExitStatusExt;
    let status = tokio::task::spawn_blocking(move || child.wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        status.signal(),
        Some(libc::SIGUSR1),
        "expected death by SIGUSR1, got {status:?}"
    );
    guard.untrack("t1");
    guard.dispose();
}

/// A hard-deadline reap is SIGTERM (not SIGUSR1), so the child reports
/// `deadline`. `sleep` dies of the signal, and the status names it.
#[cfg(unix)]
#[tokio::test]
async fn a_deadline_reap_sends_sigterm() {
    use std::os::unix::process::ExitStatusExt;
    let dir = tempfile::tempdir().unwrap();
    let guard = SubagentLifeguard::new(dir.path());
    guard.set_term_grace_for_testing(500);
    let mut child = sleeper();
    guard.monitor("t1", "explore", child.id());
    guard.set_timeout_for_testing("t1", Duration::from_millis(10));
    let status = tokio::task::spawn_blocking(move || child.wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        status.signal(),
        Some(libc::SIGTERM),
        "expected death by SIGTERM, got {status:?}"
    );
    guard.untrack("t1");
    guard.dispose();
}

/// A tool in flight is busy work up to [`TOOL_IN_FLIGHT_STALL_MS`] with no
/// progress event; beyond that it is a wedged tool and is reaped.
#[cfg(unix)]
#[tokio::test]
async fn a_tool_in_flight_past_its_bound_is_stalled() {
    let dir = tempfile::tempdir().unwrap();
    let guard = SubagentLifeguard::new(dir.path());
    let stalled = stalled_ids(&guard);
    let mut child = sleeper();
    guard.monitor("t1", "explore", child.id());
    guard.record_tool_start("t1", "c1", ToolLimit::Undeclared);
    guard.record_heartbeat("t1");
    // Just inside the bound: still busy work.
    guard.set_last_progress_for_testing("t1", now_ms() - (TOOL_IN_FLIGHT_STALL_MS - 60_000));
    guard.check_heartbeats();
    assert!(stalled.lock().unwrap().is_empty());
    // Just past the bound: wedged, reaped.
    guard.record_heartbeat("t1");
    guard.set_last_progress_for_testing("t1", now_ms() - (TOOL_IN_FLIGHT_STALL_MS + 60_000));
    guard.check_heartbeats();
    assert_eq!(*stalled.lock().unwrap(), vec!["t1"]);
    assert!(child.wait().unwrap().code().is_none());
    guard.dispose();
}

/// The backstop still holds: a child that ignores the stall signal is killed
/// after the grace period, because a wedged process can also be deaf to a
/// signal. The same child ignores SIGTERM, so the deadline path is covered too.
///
/// `/bin/sh`'s `trap ''` is not used for this: under a test harness the shell
/// does not reliably end up with SIG_IGN, and a test that passes for the wrong
/// reason is worse than no test. Python sets the disposition itself, so the
/// only thing that can end this child is the escalation.
#[cfg(unix)]
fn term_deaf_sleeper() -> Child {
    let mut command = Command::new("python3");
    command.arg("-c").arg(
        "import signal, time\nfor s in (signal.SIGTERM, signal.SIGUSR1): signal.signal(s, signal.SIG_IGN)\ntime.sleep(30)",
    );
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    command.spawn().unwrap()
}

/// The child needs a moment to install its SIGTERM handler; a SIGTERM that
/// arrives first is simply fatal, which is not what these two tests are about.
async fn deaf_sleeper_ready(_child: &Child) {
    tokio::time::sleep(Duration::from_millis(500)).await;
}

#[cfg(unix)]
#[tokio::test]
async fn a_child_that_ignores_the_stall_signal_is_killed_after_the_grace() {
    use std::os::unix::process::ExitStatusExt;
    let dir = tempfile::tempdir().unwrap();
    let guard = SubagentLifeguard::new(dir.path());
    let stalled = stalled_ids(&guard);
    guard.set_term_grace_for_testing(300);
    let mut child = term_deaf_sleeper();
    deaf_sleeper_ready(&child).await;
    guard.monitor("t1", "explore", child.id());
    guard.set_last_heartbeat_for_testing("t1", now_ms() - 70_000);
    guard.check_heartbeats();
    assert_eq!(*stalled.lock().unwrap(), vec!["t1"]);
    let status = tokio::task::spawn_blocking(move || child.wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        status.signal(),
        Some(libc::SIGKILL),
        "a signal-deaf child should have been escalated to SIGKILL, got {status:?}"
    );
    guard.untrack("t1");
    guard.dispose();
}

/// Same escalation on the deadline path: the child ignores SIGTERM, so the
/// grace timer must still end it.
#[cfg(unix)]
#[tokio::test]
async fn a_child_that_ignores_sigterm_is_killed_after_the_deadline_grace() {
    use std::os::unix::process::ExitStatusExt;
    let dir = tempfile::tempdir().unwrap();
    let guard = SubagentLifeguard::new(dir.path());
    guard.set_term_grace_for_testing(300);
    let mut child = term_deaf_sleeper();
    deaf_sleeper_ready(&child).await;
    guard.monitor("t1", "explore", child.id());
    guard.set_timeout_for_testing("t1", Duration::from_millis(10));
    let status = tokio::task::spawn_blocking(move || child.wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        status.signal(),
        Some(libc::SIGKILL),
        "a SIGTERM-deaf child should have been escalated to SIGKILL, got {status:?}"
    );
    guard.untrack("t1");
    guard.dispose();
}

/// A child that winds down on the stall signal and exits within the grace is
/// not escalated: it leaves with its own status (code 0), not SIGKILL. The
/// escalation timer is cancelled when the child is untracked at its exit.
#[cfg(unix)]
#[tokio::test]
async fn a_child_that_exits_within_the_grace_is_not_escalated() {
    let dir = tempfile::tempdir().unwrap();
    let guard = SubagentLifeguard::new(dir.path());
    guard.set_term_grace_for_testing(400);
    let ready = dir.path().join("ready");
    // Exits at once on SIGUSR1, the way a child that writes its partial result
    // does. `ready` is written after the handler is installed.
    let script = "import os,signal,time,sys; signal.signal(signal.SIGUSR1, lambda s, f: os._exit(0)); open(sys.argv[1], 'w').close(); time.sleep(30)";
    let mut command = Command::new("python3");
    command.arg("-c").arg(script).arg(&ready);
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let mut child = command.spawn().unwrap();
    for _ in 0..500 {
        if ready.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(ready.exists(), "the child never became ready");
    guard.monitor("t1", "explore", child.id());
    guard.set_last_heartbeat_for_testing("t1", now_ms() - 70_000);
    guard.check_heartbeats();
    let status = tokio::task::spawn_blocking(move || child.wait())
        .await
        .unwrap()
        .unwrap();
    guard.untrack("t1");
    // Well past the grace: a timer that was not cancelled would have fired.
    tokio::time::sleep(Duration::from_millis(1_000)).await;
    assert_eq!(
        status.code(),
        Some(0),
        "a child that exits on the stall signal should not be escalated, got {status:?}"
    );
    guard.dispose();
}

/// The configured threshold is what the progress check uses.
#[tokio::test]
async fn the_progress_check_uses_the_configured_threshold() {
    let dir = tempfile::tempdir().unwrap();
    let guard = SubagentLifeguard::new(dir.path());
    let stalled = stalled_ids(&guard);
    guard.set_progress_stall_ms(20_000);
    let mut child = sleeper();
    guard.monitor("t1", "explore", child.id());
    guard.record_heartbeat("t1");
    guard.set_last_progress_for_testing("t1", now_ms() - 25_000);
    guard.check_heartbeats();
    assert_eq!(*stalled.lock().unwrap(), vec!["t1"]);
    assert!(child.wait().unwrap().code().is_none());
    guard.dispose();
}

/// A shell as the pool spawns a child: a group leader running `script`.
#[cfg(unix)]
fn shell_child(script: &str) -> Child {
    let mut command = Command::new("sh");
    command.arg("-c").arg(script);
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    command.spawn().unwrap()
}

/// The pid a script wrote to `path` (it writes it right after it starts).
#[cfg(unix)]
fn read_pid(path: &std::path::Path) -> libc::pid_t {
    for _ in 0..500 {
        if let Some(pid) = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| text.trim().parse().ok())
        {
            return pid;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("no pid written to {}", path.display());
}

#[cfg(unix)]
fn is_alive(pid: libc::pid_t) -> bool {
    // SAFETY: signal 0 only probes for the process.
    unsafe { libc::kill(pid, 0) == 0 }
}

/// Waits for `pid` to be gone. Orphans are reparented and reaped by init.
#[cfg(unix)]
async fn wait_until_dead(pid: libc::pid_t, within: Duration) -> bool {
    let until = std::time::Instant::now() + within;
    while std::time::Instant::now() < until {
        if !is_alive(pid) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

/// A stall signal goes to the child alone. Its grandchild (a `sleep` in the
/// same group) is untouched while the child winds down, and the group is swept
/// once the child has exited, so the grandchild does not outlive the run.
#[cfg(unix)]
#[tokio::test]
async fn a_stall_signal_reaches_the_child_only_and_the_group_is_swept_on_exit() {
    use std::os::unix::process::ExitStatusExt;
    let dir = tempfile::tempdir().unwrap();
    let guard = SubagentLifeguard::new(dir.path());
    let stalled = stalled_ids(&guard);
    guard.set_term_grace_for_testing(500);
    let pid_file = dir.path().join("grandchild.pid");
    let mut child = shell_child(&format!(
        "sleep 30 & echo $! > '{}'; wait",
        pid_file.display()
    ));
    let grandchild = read_pid(&pid_file);
    guard.monitor("t1", "explore", child.id());
    guard.set_last_heartbeat_for_testing("t1", now_ms() - 70_000);
    guard.check_heartbeats();
    assert_eq!(*stalled.lock().unwrap(), vec!["t1"]);
    let status = tokio::task::spawn_blocking(move || child.wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status.signal(), Some(libc::SIGUSR1));
    assert!(
        is_alive(grandchild),
        "a stall signal must not reach the grandchild"
    );
    // The child has exited: its group is swept, so the grandchild goes too.
    guard.untrack("t1");
    assert!(
        wait_until_dead(grandchild, Duration::from_secs(3)).await,
        "the group should be swept after a stalled child exits"
    );
    guard.dispose();
}

/// A grandchild that ignores SIGTERM is still ended: the sweep escalates to
/// SIGKILL after the grace, so a stall reap never leaves an orphan behind.
#[cfg(unix)]
#[tokio::test]
async fn a_term_deaf_grandchild_is_killed_by_the_sweep_after_the_grace() {
    let dir = tempfile::tempdir().unwrap();
    let guard = SubagentLifeguard::new(dir.path());
    guard.set_term_grace_for_testing(300);
    let pid_file = dir.path().join("grandchild.pid");
    let ready = dir.path().join("ready");
    // Python sets SIG_IGN before it writes `ready`, so the SIGTERM below cannot
    // land before the disposition is in place.
    let python = format!(
        "python3 -c \"import signal,time,sys; signal.signal(signal.SIGTERM, signal.SIG_IGN); open(sys.argv[1],'w').close(); time.sleep(30)\" '{}' & echo $! > '{}'; wait",
        ready.display(),
        pid_file.display()
    );
    let mut child = shell_child(&python);
    let grandchild = read_pid(&pid_file);
    for _ in 0..500 {
        if ready.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(ready.exists(), "the grandchild never became ready");
    guard.monitor("t1", "explore", child.id());
    guard.set_last_heartbeat_for_testing("t1", now_ms() - 70_000);
    guard.check_heartbeats();
    let _ = tokio::task::spawn_blocking(move || child.wait())
        .await
        .unwrap();
    guard.untrack("t1");
    // Still there right after the child left: SIGTERM is ignored.
    assert!(is_alive(grandchild));
    assert!(
        wait_until_dead(grandchild, Duration::from_secs(5)).await,
        "the SIGTERM-deaf grandchild should have been SIGKILLed after the grace"
    );
    guard.dispose();
}

/// The tool-in-flight bound follows the load multiplier like the progress
/// bound: four external tasks make the multiplier 3, so a tool that is past
/// the unloaded bound is still busy work.
#[cfg(unix)]
#[tokio::test]
async fn the_tool_in_flight_bound_widens_under_load() {
    let dir = tempfile::tempdir().unwrap();
    let guard = SubagentLifeguard::new(dir.path());
    let stalled = stalled_ids(&guard);
    let mut child = sleeper();
    guard.monitor("t1", "explore", child.id());
    guard.set_external_load(4);
    guard.record_tool_start("t1", "c1", ToolLimit::Undeclared);
    guard.record_heartbeat("t1");
    // Past the unloaded bound, inside the loaded one (3 x 30 minutes).
    guard.set_last_progress_for_testing("t1", now_ms() - (TOOL_IN_FLIGHT_STALL_MS + 60_000));
    guard.check_heartbeats();
    assert!(stalled.lock().unwrap().is_empty());
    // Past the loaded bound too: wedged.
    guard.record_heartbeat("t1");
    guard.set_last_progress_for_testing("t1", now_ms() - (3 * TOOL_IN_FLIGHT_STALL_MS + 60_000));
    guard.check_heartbeats();
    assert_eq!(*stalled.lock().unwrap(), vec!["t1"]);
    assert!(child.wait().unwrap().code().is_none());
    guard.dispose();
}

/// What a Shell start event declares: `0` is unlimited, a positive timeout is
/// the limit, and an omitted timeout is the resolved subagent default (1200 s
/// unless the env or setting says otherwise). Other tools declare nothing.
#[test]
fn a_shell_start_declares_its_timeout() {
    let default = SUBAGENT_SHELL_DEFAULT_TIMEOUT_SECS;
    let shell = |args: serde_json::Value| tool_limit_from_start("Shell", Some(&args), default);
    assert_eq!(default, 1200.0);
    assert_eq!(
        shell(serde_json::json!({"command": "x"})),
        ToolLimit::Declared(1_200_000)
    );
    assert_eq!(
        shell(serde_json::json!({"command": "x", "timeout": 0})),
        ToolLimit::Unlimited
    );
    assert_eq!(
        shell(serde_json::json!({"command": "x", "timeout": 600})),
        ToolLimit::Declared(600_000)
    );
    // Negative values fall back to the default, as the tool does.
    assert_eq!(
        shell(serde_json::json!({"command": "x", "timeout": -5})),
        ToolLimit::Declared(1_200_000)
    );
    assert_eq!(
        tool_limit_from_start("Read", Some(&serde_json::json!({"path": "a"})), default),
        ToolLimit::Undeclared
    );
    assert_eq!(
        tool_limit_from_start("Shell", None, default),
        ToolLimit::Declared(1_200_000)
    );
}

/// The subagent default is the resolved shell timeout: the env override wins,
/// then `subagentShellTimeoutSeconds`, then 1200 s. Junk falls through.
#[test]
fn a_shell_default_timeout_is_resolved_from_env_then_setting() {
    let args = serde_json::json!({"command": "x"});
    let resolve = |settings: &SubagentLimitSettings, pairs: &[(&str, &str)]| {
        let env: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let secs = SubagentLimits::resolve(settings, &env).shell_timeout_secs;
        tool_limit_from_start("Shell", Some(&args), secs)
    };
    let none = SubagentLimitSettings::default();
    let with_setting = SubagentLimitSettings {
        shell_timeout_seconds: Some(900.0),
        ..Default::default()
    };
    assert_eq!(resolve(&none, &[]), ToolLimit::Declared(1_200_000));
    assert_eq!(
        resolve(&with_setting, &[]),
        ToolLimit::Declared(900_000),
        "the setting replaces the default"
    );
    assert_eq!(
        resolve(
            &with_setting,
            &[("HOOCODE_BASH_DEFAULT_TIMEOUT_SECS", "45")]
        ),
        ToolLimit::Declared(45_000),
        "the env override beats the setting"
    );
    assert_eq!(
        resolve(&none, &[("HOOCODE_BASH_DEFAULT_TIMEOUT_SECS", "nope")]),
        ToolLimit::Declared(1_200_000),
        "junk env falls through to the default"
    );
}

/// A Shell with a declared 20-minute timeout is busy past the 30-minute
/// default bound, and is reaped only after its timeout plus the margin.
#[cfg(unix)]
#[tokio::test]
async fn a_declared_tool_timeout_raises_the_bound() {
    let dir = tempfile::tempdir().unwrap();
    let guard = SubagentLifeguard::new(dir.path());
    let stalled = stalled_ids(&guard);
    let mut child = sleeper();
    guard.monitor("t1", "general-purpose", child.id());
    // A 40-minute declared timeout is longer than the 30-minute default bound.
    const DECLARED_MS: u64 = 2_400_000;
    guard.record_tool_start("t1", "c1", ToolLimit::Declared(DECLARED_MS));
    guard.record_heartbeat("t1");
    // Past the 30-minute default, inside 40 minutes + margin: still running.
    guard.set_last_progress_for_testing("t1", now_ms() - (TOOL_IN_FLIGHT_STALL_MS + 60_000));
    guard.check_heartbeats();
    assert!(stalled.lock().unwrap().is_empty());
    // Past the declared timeout plus the margin: wedged.
    guard.record_heartbeat("t1");
    guard.set_last_progress_for_testing(
        "t1",
        now_ms() - (DECLARED_MS + TOOL_TIMEOUT_MARGIN_MS + 60_000),
    );
    guard.check_heartbeats();
    assert_eq!(*stalled.lock().unwrap(), vec!["t1"]);
    assert!(child.wait().unwrap().code().is_none());
    guard.dispose();
}

/// A tool that declared no limit (timeout 0) is never reaped for lack of
/// progress, however long it has been silent.
#[cfg(unix)]
#[tokio::test]
async fn an_unlimited_tool_is_not_a_progress_stall() {
    let dir = tempfile::tempdir().unwrap();
    let guard = SubagentLifeguard::new(dir.path());
    let stalled = stalled_ids(&guard);
    let mut child = sleeper();
    guard.monitor("t1", "general-purpose", child.id());
    guard.record_tool_start("t1", "c1", ToolLimit::Unlimited);
    guard.record_heartbeat("t1");
    guard.set_last_progress_for_testing("t1", now_ms() - 10 * TOOL_IN_FLIGHT_STALL_MS);
    guard.check_heartbeats();
    assert!(stalled.lock().unwrap().is_empty());
    guard.dispose();
    let _ = child.wait();
}

/// Tools are matched by call id: ending one unlimited tool leaves the other
/// (undeclared) one in flight, so the 30-minute bound applies again.
#[cfg(unix)]
#[tokio::test]
async fn tools_are_tracked_by_call_id() {
    let dir = tempfile::tempdir().unwrap();
    let guard = SubagentLifeguard::new(dir.path());
    let stalled = stalled_ids(&guard);
    let mut child = sleeper();
    guard.monitor("t1", "general-purpose", child.id());
    guard.record_tool_start("t1", "c1", ToolLimit::Undeclared);
    guard.record_tool_start("t1", "c2", ToolLimit::Unlimited);
    guard.record_tool_end("t1", "c2");
    guard.record_heartbeat("t1");
    guard.set_last_progress_for_testing("t1", now_ms() - (TOOL_IN_FLIGHT_STALL_MS + 60_000));
    guard.check_heartbeats();
    assert_eq!(*stalled.lock().unwrap(), vec!["t1"]);
    assert!(child.wait().unwrap().code().is_none());
    guard.dispose();
}

/// Cleanup runs on `hoocode-bg`, so a removed dispatch dir can take a moment to disappear.
async fn wait_until_gone(path: &std::path::Path) {
    for _ in 0..500 {
        if !path.exists() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("{} was not removed", path.display());
}

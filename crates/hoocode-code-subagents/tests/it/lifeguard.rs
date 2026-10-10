#![allow(clippy::disallowed_methods)] // test code: #[tokio::test] expands to a runtime builder
//! lifeguard.test.ts. Children are `sleep` processes in their own process
//! group (as the pool spawns them).

use std::process::{Child, Command};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hoocode_code_subagents::lifeguard::{
    base_timeout_ms, LifeguardEvent, SubagentLifeguard, MAX_SCALED_DEADLINE_MS,
    SUBAGENT_DEADLINE_MS,
};

/// 2026-10-10 (user decision): every agent type, shipped or plugin, gets the
/// same 2-hour hard deadline. The per-type 5-20 minute table is gone.
#[test]
fn every_agent_type_gets_the_two_hour_deadline() {
    const HOURS_2: u64 = 2 * 60 * 60 * 1000;
    assert_eq!(SUBAGENT_DEADLINE_MS, HOURS_2);
    for agent in [
        "code-review",
        "security-review",
        "general-purpose",
        "explore",
        "plan",
    ] {
        assert_eq!(base_timeout_ms(agent), HOURS_2, "{agent}");
    }
}

/// hoocode's `edit`/`test`/`review` names are not special any more either.
#[test]
fn hoocodes_own_arm_names_get_the_same_deadline() {
    for agent in ["edit", "test", "review"] {
        assert_eq!(base_timeout_ms(agent), SUBAGENT_DEADLINE_MS, "{agent}");
    }
}

/// An unknown agent type gets the same deadline as the rest.
#[test]
fn an_unknown_agent_type_gets_the_same_deadline() {
    assert_eq!(base_timeout_ms("some-plugin-agent"), SUBAGENT_DEADLINE_MS);
}

/// The load multiplier cannot stretch a run past base + 30 minutes: 4x of a
/// 2-hour base would be 8 hours. `AgentOutput` derives its reconcile age from
/// this same ceiling.
#[test]
fn the_load_scaled_deadline_is_capped_at_base_plus_thirty_minutes() {
    const MINUTE: u64 = 60 * 1000;
    assert_eq!(MAX_SCALED_DEADLINE_MS, SUBAGENT_DEADLINE_MS + 30 * MINUTE);
    // Checked at compile time: uncapped, 4x the base would exceed the ceiling.
    const _: () = assert!(SUBAGENT_DEADLINE_MS * 4 > MAX_SCALED_DEADLINE_MS);
}

/// The warm pool's run timeout tracks the same deadline, so a warm
/// `code-review` is not killed at 180s while the cold pool would have allowed
/// it two hours.
#[test]
fn the_warm_run_timeout_tracks_the_agent_type() {
    // Asserted through the cold table because `warm_run_timeout` is private to
    // the warm module; the two must not drift apart.
    assert!(base_timeout_ms("code-review") > 180_000);
    assert!(base_timeout_ms("general-purpose") > 180_000);
    assert!(base_timeout_ms("explore") > 180_000);
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
    // finished nothing for longer than the progress threshold.
    guard.record_heartbeat("t1");
    guard.set_last_progress_for_testing("t1", now_ms() - 200_000);
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

/// A reap is SIGTERM, grace, SIGKILL — so a child that can still write its
/// result gets the chance. `sleep` dies on SIGTERM, which is the point: the
/// pool then sees the exit and settles the task from whatever it wrote.
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
    // SIGTERM reached it rather than SIGKILL: `sleep` dies of the signal, and
    // the status says which one.
    use std::os::unix::process::ExitStatusExt;
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

/// The backstop still holds: a child that ignores SIGTERM is killed after the
/// grace period, because a wedged process can also be deaf to a signal.
///
/// `/bin/sh`'s `trap ''` is not used for this: under a test harness the shell
/// does not reliably end up with SIG_IGN, and a test that passes for the wrong
/// reason is worse than no test. Python sets the disposition itself, so the
/// only thing that can end this child is the escalation.
#[cfg(unix)]
fn term_deaf_sleeper() -> Child {
    let mut command = Command::new("python3");
    command
        .arg("-c")
        .arg("import signal, time; signal.signal(signal.SIGTERM, signal.SIG_IGN); time.sleep(30)");
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
async fn a_child_that_ignores_sigterm_is_killed_after_the_grace() {
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
        "a SIGTERM-deaf child should have been escalated to SIGKILL, got {status:?}"
    );
    guard.untrack("t1");
    guard.dispose();
}

/// Untracking (the child exited) cancels the escalation, so a healthy late exit
/// is never killed after the fact.
#[cfg(unix)]
#[tokio::test]
async fn exiting_cancels_the_escalation_timer() {
    let dir = tempfile::tempdir().unwrap();
    let guard = SubagentLifeguard::new(dir.path());
    guard.set_term_grace_for_testing(400);
    let mut child = term_deaf_sleeper();
    deaf_sleeper_ready(&child).await;
    guard.monitor("t1", "explore", child.id());
    guard.set_last_heartbeat_for_testing("t1", now_ms() - 70_000);
    guard.check_heartbeats();
    guard.untrack("t1");
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    assert!(
        child.try_wait().unwrap().is_none(),
        "the escalation should have been cancelled when the child was untracked"
    );
    let _ = child.kill();
    let _ = child.wait();
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

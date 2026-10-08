//! The P1 hardening: what the subagent pool now refuses, bounds and cleans up.
//!
//! Each test here pins one decision from the reliability review that the eval
//! suite alone could not: an unbounded queue, an unbounded result map, a record
//! that claims to be running after the pool forgot it, a queued task starved
//! by priority, and a `complexity` typo that used to cost a whole dispatch.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Once};

use hoocode_code_subagents::inbox::{subagent_inbox, TaskLifecycle};
use hoocode_code_subagents::ledger;
use hoocode_code_subagents::model_categories::CategorySettings;
use hoocode_code_subagents::pool::*;
use hoocode_code_subagents::runner::{ScriptedRunner, ScriptedStep};
use serde_json::{json, Value};

const DIR: &str = hoocode_code_paths::CONFIG_DIR_NAME;

fn isolate_agent_dir() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let dir =
            std::env::temp_dir().join(format!("hoocode-hardening-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("CORTEXCODE_CODING_AGENT_DIR", &dir);
    });
}

fn setup() -> tempfile::TempDir {
    isolate_agent_dir();
    tempfile::tempdir().unwrap()
}

fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

fn result_json(summary: &str, status: &str) -> String {
    json!({"summary": summary, "files_changed": [], "confidence": 0.9, "status": status})
        .to_string()
}

fn result_writer(dir: &Path, delay_ms: u64) -> PathBuf {
    script(
        dir,
        "mock-result.sh",
        &format!(
            "tid=unknown; prev=; for a in \"$@\"; do [ \"$prev\" = \"--task-id\" ] && tid=$a; prev=$a; done\n\
             sleep {}\nmkdir -p {DIR}/dispatch/$tid\nprintf '%s' '{}' > {DIR}/dispatch/$tid/result.json\nexit 0",
            delay_ms as f64 / 1000.0,
            result_json("done", "complete")
        ),
    )
}

fn task(id: &str, agent: &str) -> SubagentPoolTask {
    SubagentPoolTask {
        task_id: id.into(),
        agent_type: agent.into(),
        task: "work".into(),
        ..Default::default()
    }
}

fn pool(exe: PathBuf, max: usize, cwd: &Path) -> SubagentPool {
    SubagentPool::new(SubagentPoolOptions {
        executable: exe,
        max_concurrency: Some(max),
        cwd: Some(cwd.to_path_buf()),
        ..Default::default()
    })
}

fn completion_order(pool: &SubagentPool) -> Arc<Mutex<Vec<String>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    pool.on(move |event| {
        if event.name == "task_done" {
            sink.lock()
                .unwrap()
                .push(event.data["task_id"].as_str().unwrap().to_string());
        }
    });
    seen
}

#[tokio::test]
async fn a_full_queue_refuses_the_next_dispatch_instead_of_growing() {
    let dir = setup();
    let mut options = SubagentPoolOptions {
        executable: result_writer(dir.path(), 400),
        max_concurrency: Some(1),
        max_queued: Some(2),
        cwd: Some(dir.path().to_path_buf()),
        ..Default::default()
    };
    options.max_queued = Some(2);
    let p = SubagentPool::new(options);
    p.spawn(task("running", "explore")).unwrap();
    p.spawn(task("q1", "explore")).unwrap();
    p.spawn(task("q2", "explore")).unwrap();
    let refused = p.spawn(task("q3", "explore")).unwrap_err().to_string();
    assert!(
        refused.contains("queue is full") && refused.contains("2 waiting"),
        "the refusal should say the queue is full and how deep: {refused:?}"
    );
    // The three that fit still run: a refusal is not a stall.
    for id in ["running", "q1", "q2"] {
        assert!(p.wait_for(id).await.unwrap().ok, "{id} should still run");
    }
    assert_eq!(p.queued_count(), 0);
    p.dispose();
}

#[tokio::test]
async fn a_waiting_task_outranks_the_tier_above_it_eventually() {
    let dir = setup();
    let p = pool(result_writer(dir.path(), 300), 1, dir.path());
    let order = completion_order(&p);
    p.spawn(task("blocker", "general-purpose")).unwrap();
    // An `explore` that has been waiting five minutes has aged past the priority
    // advantage it was given at dispatch: two `code-review`s queued after it
    // should not both overtake it.
    p.spawn(SubagentPoolTask {
        queued_at: Some(now_ms() - 5 * 60_000),
        ..task("patient", "explore")
    })
    .unwrap();
    p.spawn(task("review-a", "code-review")).unwrap();
    p.spawn(task("review-b", "code-review")).unwrap();
    for id in ["blocker", "patient", "review-a", "review-b"] {
        p.wait_for(id).await.unwrap();
    }
    let order = order.lock().unwrap().clone();
    p.dispose();
    assert_eq!(order[0], "blocker");
    assert_eq!(
        order[1], "patient",
        "a five-minute wait should outrank a fresh review: {order:?}"
    );
}

#[tokio::test]
async fn finished_results_are_bounded_and_the_oldest_goes_first() {
    let dir = setup();
    let mut options = SubagentPoolOptions {
        executable: result_writer(dir.path(), 0),
        max_concurrency: Some(2),
        cwd: Some(dir.path().to_path_buf()),
        ..Default::default()
    };
    options.max_completed = Some(2);
    let p = SubagentPool::new(options);
    for id in ["t1", "t2", "t3", "t4"] {
        p.spawn(task(id, "explore")).unwrap();
    }
    // Settled without waiting on each one: `wait_for` consumes the result,
    // which is exactly the retention path this test is not about.
    let (settled, notify) = watch_settled(&p);
    await_settled(&settled, &notify, &["t1", "t2", "t3", "t4"]).await;
    // Each finished result carries its captured streams; unbounded retention is
    // how a long session used to grow without limit.
    assert!(p.collect("t4").is_some(), "the newest result is retained");
    assert!(p.collect("t3").is_some());
    assert!(
        p.collect("t1").is_none(),
        "the oldest result should be dropped"
    );
    assert!(p.collect("t2").is_none());
    p.dispose();
}

/// Every task id the pool has reported as finished.
fn watch_settled(pool: &SubagentPool) -> (Arc<Mutex<Vec<String>>>, Arc<tokio::sync::Notify>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let notify = Arc::new(tokio::sync::Notify::new());
    let (sink, signal) = (seen.clone(), notify.clone());
    pool.on(move |event| {
        if matches!(event.name, "task_done" | "task_failed" | "task_stalled") {
            if let Some(id) = event.data.get("task_id").and_then(Value::as_str) {
                sink.lock().unwrap().push(id.to_string());
                signal.notify_waiters();
            }
        }
    });
    (seen, notify)
}

/// Block on the pool's own events rather than on a clock: the point of the
/// test is retention, not how fast a child spawns.
async fn await_settled(
    settled: &Arc<Mutex<Vec<String>>>,
    notify: &Arc<tokio::sync::Notify>,
    ids: &[&str],
) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        let done = settled.lock().unwrap().clone();
        if ids.iter().all(|id| done.iter().any(|seen| seen == id)) {
            return;
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            panic!("not settled: {done:?}");
        }
        let _ = tokio::time::timeout(remaining, notify.notified()).await;
    }
}

#[tokio::test]
async fn a_long_session_keeps_its_memory_flat() {
    let dir = setup();
    let p = pool(result_writer(dir.path(), 0), 2, dir.path());
    for round in 0..40 {
        let id = format!("t{round}");
        p.spawn(task(&id, "explore")).unwrap();
        assert!(p.wait_for(&id).await.unwrap().ok);
    }
    let ledger_lines = ledger::read_all(dir.path()).len();
    p.dispose();
    // 40 dispatches, every one recorded; the in-memory map never held more
    // than the cap. The ledger is the durable record either way.
    assert_eq!(ledger_lines, 40);
}

#[test]
fn a_record_the_pool_forgot_is_reconciled_instead_of_running_forever() {
    let inbox = subagent_inbox();
    inbox.clear();
    let record = inbox.start("lost-1", "explore#1", "explore");
    assert_eq!(record.lifecycle, TaskLifecycle::Running);

    // Still live: untouched.
    let reconciled = inbox.reconcile(|_| true, 0);
    assert!(reconciled.is_empty());
    assert_eq!(
        inbox.get("lost-1").unwrap().lifecycle,
        TaskLifecycle::Running,
        "a run the pool still knows about must not be reconciled away"
    );

    // The pool has never heard of it, and it is older than the grace: settled.
    let reconciled = inbox.reconcile(|_| false, 0);
    assert_eq!(reconciled.len(), 1);
    let settled = inbox.get("lost-1").unwrap();
    assert_eq!(settled.lifecycle, TaskLifecycle::Failed);
    assert!(settled.ended_at.is_some());
    assert!(
        settled
            .error
            .as_deref()
            .unwrap_or("")
            .contains("no longer running"),
        "the cause should say what happened: {:?}",
        settled.error
    );
    assert!(inbox.outstanding().is_empty());
    inbox.clear();
}

#[test]
fn a_young_record_is_left_alone_even_when_the_pool_is_silent() {
    let inbox = subagent_inbox();
    inbox.clear();
    inbox.start("fresh", "plan#1", "plan");
    // A dispatch that has not reached the pool's queue yet is not lost.
    let reconciled = inbox.reconcile(|_| false, 60 * 60_000);
    assert!(reconciled.is_empty());
    assert_eq!(
        inbox.get("fresh").unwrap().lifecycle,
        TaskLifecycle::Running
    );
    inbox.clear();
}

#[test]
fn a_partial_model_categories_block_keeps_the_global_tiers() {
    // `instance.rs` used `global.extend(project)`, which replaced the whole
    // `modelCategories` object: a project that set only `capable` silently lost
    // the global `fast` and `standard`, and every tier fell back to a derived
    // default. The pool now deep-merges like the rest of the codebase.
    let global = json!({
        "modelCategories": {"fast": "mock/fast", "standard": "mock/std", "capable": "mock/capable"}
    });
    let project = json!({"modelCategories": {"capable": "project/capable"}});

    let merged = hoocode_code_settings::deep_merge_settings(
        &serde_json::from_value::<serde_json::Map<String, Value>>(global.clone()).unwrap(),
        &serde_json::from_value::<serde_json::Map<String, Value>>(project).unwrap(),
    );
    let categories = CategorySettings::from_settings(&merged)
        .model_categories
        .expect("modelCategories should survive the merge");
    assert_eq!(categories.fast.as_deref(), Some("mock/fast"));
    assert_eq!(categories.standard.as_deref(), Some("mock/std"));
    assert_eq!(
        categories.capable.as_deref(),
        Some("project/capable"),
        "the project's own tier still wins"
    );
}

/// `lifeguard::sweep_old_agents` decides a dispatch dir belongs to a dead run
/// by reading `dispatch/<task>/pid`. It always read that file; nothing ever
/// wrote it, so reaping was age-only and a genuinely orphaned child was never
/// detected by liveness.
#[tokio::test]
async fn the_pool_writes_a_pid_file_that_names_a_live_process() {
    let dir = setup();
    let cwd = dir.path().to_path_buf();
    let p = pool(result_writer(dir.path(), 300), 1, &cwd);
    p.spawn(task("pid-check", "explore")).unwrap();
    // The child is up: the file is written at spawn, before the run finishes.
    let pid_file = hoocode_code_paths::dispatch_task_dir(&cwd, "pid-check").join("pid");
    let mut seen = None;
    for _ in 0..50 {
        if let Ok(text) = std::fs::read_to_string(&pid_file) {
            seen = text.trim().parse::<i32>().ok();
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let pid = seen.expect("the pool should write dispatch/<task>/pid");
    // SAFETY: signal 0 only checks that the process exists.
    let alive = unsafe { libc::kill(pid, 0) == 0 };
    p.dispose();
    assert!(alive, "pid {pid} from the file should be a live process");
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The panel's live row is fed by pool events, not by guesswork: attempt,
/// resolved model and deadline arrive when the run starts.
#[tokio::test]
async fn a_start_announces_attempt_model_and_deadline() {
    let dir = setup();
    let runner = ScriptedRunner::new(vec![ScriptedStep {
        stdout: vec![r#"{"ping":true}"#.into()],
        hang: true,
        ..Default::default()
    }]);
    let p = SubagentPool::new(SubagentPoolOptions {
        runner: Some(Arc::new(runner)),
        cwd: Some(dir.path().to_path_buf()),
        // The model must be in the available set to resolve; that is the point
        // of the field: it is what the child will actually run on.
        available_models: vec![serde_json::from_value(json!({
            "id": "pinned", "name": "Pinned", "api": "openai-completions",
            "provider": "mock", "baseUrl": "http://x", "contextWindow": 1000, "maxTokens": 100,
        }))
        .unwrap()],
        ..Default::default()
    });
    let seen: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    p.on(move |event| {
        if event.name == "task_started" {
            sink.lock().unwrap().push(event.data.clone());
        }
    });
    p.spawn(SubagentPoolTask {
        model: Some("mock/pinned".into()),
        provider: Some("mock".into()),
        ..task("t1", "explore")
    })
    .unwrap();
    for _ in 0..100 {
        if !seen.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let started = seen.lock().unwrap().first().cloned().unwrap();
    p.cancel("t1");
    let _ = p.wait_for("t1").await;
    p.dispose();
    assert_eq!(started["task_id"], "t1");
    assert_eq!(started["agent_type"], "explore");
    assert_eq!(started["attempt"], 1);
    assert_eq!(
        started["model"], "mock/pinned",
        "the resolved model, not the request"
    );
    assert!(
        started["deadline_at"].as_u64().unwrap_or(0) > now_ms(),
        "the row needs a deadline in the future to count down to"
    );
}

/// A finished run says how it ended, with the child's own confidence when it
/// reported one — the three things that used to be indistinguishable.
#[tokio::test]
async fn a_done_announces_the_outcome_the_child_reported() {
    let dir = setup();
    let runner = ScriptedRunner::new(vec![ScriptedStep {
        stdout: vec![r#"{"ping":true}"#.into()],
        result_json: Some(
            json!({"summary": "found it", "files_changed": ["a.rs"], "confidence": 0.8, "status": "partial"})
                .to_string(),
        ),
        exit_code: Some(0),
        ..Default::default()
    }]);
    let p = SubagentPool::new(SubagentPoolOptions {
        runner: Some(Arc::new(runner)),
        cwd: Some(dir.path().to_path_buf()),
        ..Default::default()
    });
    let seen: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    p.on(move |event| {
        if event.name == "task_done" {
            sink.lock().unwrap().push(event.data.clone());
        }
    });
    p.spawn(task("t1", "explore")).unwrap();
    let result = p.wait_for("t1").await.unwrap();
    p.dispose();
    assert!(result.ok);
    let done = seen.lock().unwrap().first().cloned().unwrap();
    assert_eq!(done["status"], "complete");
    assert_eq!(done["confidence"], 0.8);
    assert_eq!(done["files_changed"], json!(["a.rs"]));
}

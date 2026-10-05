//! The runner seam: the same pool, with no processes.
//!
//! These tests exist to prove the seam is real — that queueing, priority,
//! cancellation, the ledger and the lifeguard all sit *above* it. A test here
//! that needed a `/bin/sh` mock would mean the seam leaked.

use std::path::Path;
use std::sync::{Arc, Mutex, Once};

use cortexcode_code_subagents::ledger;
use cortexcode_code_subagents::pool::*;
use cortexcode_code_subagents::runner::{
    RunSpec, Runner, RunnerHandle, ScriptedRunner, ScriptedStep,
};
use serde_json::json;

fn isolate_agent_dir() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let dir = std::env::temp_dir().join(format!("cortex-runner-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("CORTEXCODE_CODING_AGENT_DIR", &dir);
    });
}

fn setup() -> tempfile::TempDir {
    isolate_agent_dir();
    tempfile::tempdir().unwrap()
}

fn result_json(summary: &str) -> String {
    json!({"summary": summary, "files_changed": [], "confidence": 0.9, "status": "complete"})
        .to_string()
}

const DONE: &str = r#"{"type":"done","reason":"stop","message":{"role":"assistant","content":[{"type":"text","text":"ok"}]}}"#;
const PING: &str = r#"{"ping":true}"#;

/// A step that answers like a finished child.
fn ok(summary: &str) -> ScriptedStep {
    ScriptedStep {
        stdout: vec![PING.into(), DONE.into()],
        result_json: Some(result_json(summary)),
        exit_code: Some(0),
        ..Default::default()
    }
}

fn pool_with(runner: ScriptedRunner, cwd: &Path, max: usize) -> SubagentPool {
    SubagentPool::new(SubagentPoolOptions {
        runner: Some(Arc::new(runner)),
        max_concurrency: Some(max),
        cwd: Some(cwd.to_path_buf()),
        ..Default::default()
    })
}

fn task(id: &str, agent: &str) -> SubagentPoolTask {
    SubagentPoolTask {
        task_id: id.into(),
        agent_type: agent.into(),
        task: "work".into(),
        ..Default::default()
    }
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
async fn a_scripted_run_settles_and_records_its_ledger_line() {
    let dir = setup();
    let runner = ScriptedRunner::new(vec![ok("found it")]);
    let p = pool_with(runner.clone(), dir.path(), 2);
    p.spawn(task("t1", "explore")).unwrap();
    let result = p.wait_for("t1").await.unwrap();
    assert!(
        result.ok,
        "the scripted child should settle complete: {result:?}"
    );
    assert_eq!(result.status, Some(ResultStatus::Complete));
    assert_eq!(runner.dispatched(), 1);

    let attempts = ledger::read_all(dir.path());
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].status, "complete");
    assert_eq!(attempts[0].agent_type, "explore");
    p.dispose();
}

#[tokio::test]
async fn the_run_spec_carries_what_a_command_line_would_have_said() {
    let dir = setup();
    let seen: Arc<Mutex<Vec<RunSpec>>> = Arc::new(Mutex::new(Vec::new()));
    // A runner that records what it was asked for, instead of running anything.
    let recorder = Arc::new(RecordingRunner {
        inner: ScriptedRunner::new(vec![ok("ok")]),
        seen: seen.clone(),
    });
    let p = SubagentPool::new(SubagentPoolOptions {
        runner: Some(recorder),
        cwd: Some(dir.path().to_path_buf()),
        ..Default::default()
    });
    p.spawn(SubagentPoolTask {
        model: Some("mock/pinned".into()),
        context: Some("from the parent".into()),
        ..task("t1", "code-review")
    })
    .unwrap();
    p.wait_for("t1").await.unwrap();

    let spec = seen.lock().unwrap().last().cloned().unwrap();
    assert_eq!(spec.task_id, "t1");
    assert_eq!(spec.agent_type, "code-review");
    assert!(
        spec.prompt.starts_with("Context from the calling agent:"),
        "the runner must see the same prompt the child would: {:?}",
        spec.prompt
    );
    assert!(spec.prompt.contains("from the parent"));
    assert!(spec.prompt.ends_with("Task: work"));
    assert_eq!(spec.max_turns, DEFAULT_SUBAGENT_MAX_TURNS);
    assert!(
        spec.argv.iter().any(|a| a == "--mode"),
        "the command line is still built for the process runner: {:?}",
        spec.argv
    );
    p.dispose();
}

#[tokio::test]
async fn queueing_and_priority_run_above_the_seam() {
    let dir = setup();
    // One slot, so the queue is observable; the first step hangs to hold it.
    let runner = ScriptedRunner::new(vec![
        ScriptedStep {
            stdout: vec![PING.into()],
            hang: true,
            ..Default::default()
        },
        ok("a"),
        ok("b"),
    ]);
    let p = pool_with(runner.clone(), dir.path(), 1);
    let order = completion_order(&p);
    p.spawn(task("running", "explore")).unwrap();
    p.spawn(task("queued-explore", "explore")).unwrap();
    p.spawn(task("queued-review", "code-review")).unwrap();
    assert_eq!(p.queued_count(), 2);

    p.cancel("running");
    let _ = p.wait_for("running").await;
    assert!(p.wait_for("queued-explore").await.unwrap().ok);
    assert!(p.wait_for("queued-review").await.unwrap().ok);

    let order = order.lock().unwrap().clone();
    p.dispose();
    assert_eq!(order.first().map(String::as_str), Some("queued-explore"));
    assert_eq!(runner.dispatched(), 3);
}

#[tokio::test]
async fn a_failing_script_is_reported_as_a_failure_with_its_stderr() {
    let dir = setup();
    let runner = ScriptedRunner::new(vec![ScriptedStep {
        stdout: vec![PING.into()],
        stderr: "429 rate limited, please retry".into(),
        exit_code: Some(1),
        ..Default::default()
    }]);
    let p = pool_with(runner, dir.path(), 1);
    p.spawn(task("t1", "explore")).unwrap();
    let result = p.wait_for("t1").await.unwrap();
    p.dispose();
    assert!(!result.ok);
    assert_eq!(result.status, Some(ResultStatus::Failed));
    assert!(
        result.stderr.contains("429"),
        "the captured stderr should reach the caller: {:?}",
        result.stderr
    );
    let attempts = ledger::read_all(dir.path());
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].status, "failed");
}

#[tokio::test]
async fn the_lifeguard_reaps_a_scripted_run_that_goes_quiet() {
    let dir = setup();
    let runner = ScriptedRunner::new(vec![ScriptedStep {
        stdout: vec![PING.into()],
        hang: true,
        ..Default::default()
    }]);
    let p = pool_with(runner, dir.path(), 1);
    p.lifeguard()
        .set_last_heartbeat_for_testing("t1", now_ms() - 70_000);
    p.spawn(task("t1", "explore")).unwrap();
    p.lifeguard()
        .set_last_heartbeat_for_testing("t1", now_ms() - 70_000);
    p.lifeguard().check_heartbeats();
    let result = p.wait_for("t1").await.unwrap();
    p.dispose();
    // A scripted handle has no process to signal, so the run is settled rather
    // than killed: the point is that the watchdog fires through the seam.
    let _ = result;
    let attempts = ledger::read_all(dir.path());
    assert_eq!(
        attempts.len(),
        1,
        "a reaped run is still recorded exactly once"
    );
}

/// Wraps the scripted runner to capture the spec it was asked for.
struct RecordingRunner {
    inner: ScriptedRunner,
    seen: Arc<Mutex<Vec<RunSpec>>>,
}

impl Runner for RecordingRunner {
    fn spawn(&self, spec: RunSpec) -> std::io::Result<Box<dyn RunnerHandle>> {
        self.seen.lock().unwrap().push(spec.clone());
        self.inner.spawn(spec)
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// A runner with no process must not signal anything. `kill(-0, SIGTERM)`
/// reaches every process in the caller's group — including the parent agent and
/// this test binary — and the seam made that reachable for the first time.
#[tokio::test]
async fn reaping_a_processless_run_signals_nothing() {
    let dir = setup();
    let runner = ScriptedRunner::new(vec![ScriptedStep {
        stdout: vec![PING.into()],
        hang: true,
        ..Default::default()
    }]);
    let p = pool_with(runner, dir.path(), 1);
    p.spawn(task("t1", "explore")).unwrap();
    p.lifeguard()
        .set_last_heartbeat_for_testing("t1", now_ms() - 70_000);
    p.lifeguard().check_heartbeats();
    // Still here: no signal was delivered to this process or its group.
    assert_eq!(p.running_count(), 1);
    p.cancel("t1");
    let _ = p.wait_for("t1").await;
    p.dispose();
}

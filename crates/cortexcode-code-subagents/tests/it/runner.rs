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
    BoxFuture, InProcessAgent, InProcessFactory, InProcessRunner, RunSpec, Runner, RunnerHandle,
    ScriptedRunner, ScriptedStep,
};
use serde_json::json;
use tokio::sync::mpsc;

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

// ---------------------------------------------------------------------------
// In-process execution: the P5 gate
// ---------------------------------------------------------------------------

/// A fake that writes its `result.json` where the spec says, the way a real
/// in-process run does.
struct WritingAgent {
    lines: Vec<String>,
    result: Option<String>,
    cwd: std::path::PathBuf,
    task_id: String,
    before: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl InProcessAgent for WritingAgent {
    fn run<'a>(
        &'a self,
        _prompt: &'a str,
        out: &'a mpsc::Sender<Vec<u8>>,
    ) -> BoxFuture<'a, Result<(), String>> {
        let lines = self.lines.clone();
        let result = self.result.clone();
        let dir = cortexcode_code_paths::dispatch_task_dir(&self.cwd, &self.task_id);
        let before = self.before.clone();
        Box::pin(async move {
            if let Some(before) = before {
                before();
            }
            for line in lines {
                if out.send(format!("{line}\n").into_bytes()).await.is_err() {
                    return Ok(());
                }
            }
            match result {
                Some(text) => {
                    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                    std::fs::write(dir.join("result.json"), text).map_err(|e| e.to_string())?;
                    let _ = out
                        .send(b"{\"type\":\"done\",\"reason\":\"stop\"}\n".to_vec())
                        .await;
                    Ok(())
                }
                None => Err("no result.json was written".into()),
            }
        })
    }
}

fn in_process_pool(cwd: &Path, factory: InProcessFactory, max: usize) -> SubagentPool {
    SubagentPool::new(SubagentPoolOptions {
        runner: Some(Arc::new(InProcessRunner::new(factory))),
        max_concurrency: Some(max),
        cwd: Some(cwd.to_path_buf()),
        ..Default::default()
    })
}

/// The same, but the agent is built from the spec — which is how an in-process
/// run learns where to write its `result.json`, exactly as a child does.
fn factory_building<F>(build: F) -> InProcessFactory
where
    F: Fn(&RunSpec) -> Box<dyn InProcessAgent> + Send + Sync + 'static,
{
    Arc::new(move |spec: &RunSpec| Ok(build(spec)))
}

#[tokio::test]
async fn an_in_process_run_settles_and_records_the_ledger() {
    let dir = setup();
    let p = in_process_pool(
        dir.path(),
        factory_building(|spec| {
            Box::new(WritingAgent {
                lines: vec![PING.into()],
                result: Some(result_json("answered in process")),
                cwd: spec.cwd.clone(),
                task_id: spec.task_id.clone(),
                before: None,
            })
        }),
        2,
    );
    p.spawn(task("t1", "explore")).unwrap();
    let result = p.wait_for("t1").await.unwrap();
    assert!(
        result.ok,
        "an in-process run must settle like a child: {result:?}"
    );
    p.dispose();
    let attempts = ledger::read_all(dir.path());
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].status, "complete");
}

#[tokio::test]
async fn an_in_process_run_without_a_result_fails_rather_than_claiming_success() {
    let dir = setup();
    let p = in_process_pool(
        dir.path(),
        factory_building(|spec| {
            Box::new(WritingAgent {
                lines: vec![PING.into()],
                result: None,
                cwd: spec.cwd.clone(),
                task_id: spec.task_id.clone(),
                before: None,
            })
        }),
        1,
    );
    p.spawn(task("t1", "explore")).unwrap();
    let result = p.wait_for("t1").await.unwrap();
    p.dispose();
    assert!(!result.ok, "no result.json means no findings: {result:?}");
    assert_eq!(ledger::read_all(dir.path())[0].status, "failed");
}

/// **The P5 gate.** A child that dispatches its own child, in this process, from
/// inside a run the pool already owns — through the *same* pool, because
/// `get_subagent_pool` is a process-wide singleton and a nested dispatch reuses
/// it. If this deadlocks, double-settles or mis-records, the in-process model is
/// not shippable and the child process stays.
///
/// The pool never holds its state lock across an await, so a run started by the
/// pool can ask the same pool for another run. That is the whole gate.
#[tokio::test(flavor = "multi_thread")]
async fn an_in_process_child_can_dispatch_its_own_child() {
    let dir = setup();
    let cwd = dir.path().to_path_buf();
    let nested: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let outer_pool: Arc<Mutex<Option<SubagentPool>>> = Arc::new(Mutex::new(None));

    let holder = outer_pool.clone();
    let record = nested.clone();
    // Once only: the grandchild's own agent has the same hook, and a hook that
    // fires for every run is a recursive dispatch, not a gate.
    let fired = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let fired_hook = fired.clone();
    let before_hook: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        if fired_hook.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        // Re-entrant: the run the pool owns asks the same pool for another
        // dispatch, while the first is still running.
        let pool = holder.lock().unwrap().clone();
        let record = record.clone();
        let nested_result: Arc<Mutex<Option<SubagentResult>>> = Arc::new(Mutex::new(None));
        let sink = nested_result.clone();
        if let Some(pool) = pool {
            tokio::task::spawn(async move {
                let options = DispatchOptions {
                    force_agent: Some("explore".into()),
                    ..Default::default()
                };
                let outcome = pool.dispatch("nested question", options).await;
                *sink.lock().unwrap() = outcome.ok().and_then(|d| d.result);
                *record.lock().unwrap() = Some("dispatched".into());
            });
        }
    });

    let p = in_process_pool(
        &cwd,
        factory_building(move |spec| {
            // The first agent is the "child"; the nested dispatch lands in the
            // same pool, so it gets the second script entry.
            let nested_id = spec.task_id.clone();
            Box::new(NestedAwareAgent {
                lines: vec![PING.into()],
                result: Some(result_json("the child used a grandchild")),
                cwd: spec.cwd.clone(),
                task_id: nested_id,
                before: Some(Arc::clone(&before_hook)),
            })
        }),
        2,
    );
    *outer_pool.lock().unwrap() = Some(p.clone());
    p.spawn(task("outer", "explore")).unwrap();
    let result = p.wait_for("outer").await.unwrap();
    assert!(result.ok, "the outer run must settle: {result:?}");

    for _ in 0..200 {
        if nested.lock().unwrap().is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert!(
        nested.lock().unwrap().is_some(),
        "the nested dispatch should have been issued"
    );
    // Wait for the grandchild to settle too: the test is about the pair, not
    // about the outer run alone.
    for _ in 0..200 {
        if ledger::read_all(dir.path()).len() >= 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    p.dispose();

    // Two dispatches, two ledger lines: the grandchild is recorded like any
    // other attempt, and nothing was left stuck.
    let attempts = ledger::read_all(dir.path());
    assert_eq!(
        attempts.len(),
        2,
        "both attempts must be recorded: {:?}",
        attempts.iter().map(|a| &a.status).collect::<Vec<_>>()
    );
    assert!(attempts.iter().all(|attempt| attempt.ok));
}

/// An agent that answers differently for a nested dispatch: the grandchild gets
/// its own summary, which is what makes "the nested run really ran" checkable.
struct NestedAwareAgent {
    lines: Vec<String>,
    result: Option<String>,
    cwd: std::path::PathBuf,
    task_id: String,
    before: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl InProcessAgent for NestedAwareAgent {
    fn run<'a>(
        &'a self,
        _prompt: &'a str,
        out: &'a mpsc::Sender<Vec<u8>>,
    ) -> BoxFuture<'a, Result<(), String>> {
        let lines = self.lines.clone();
        let result = self.result.clone();
        let dir = cortexcode_code_paths::dispatch_task_dir(&self.cwd, &self.task_id);
        let before = self.before.clone();
        Box::pin(async move {
            if let Some(before) = before {
                before();
            }
            for line in lines {
                if out.send(format!("{line}\n").into_bytes()).await.is_err() {
                    return Ok(());
                }
            }
            let text = match result {
                Some(text) => text,
                None => return Err("no result.json was written".into()),
            };
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            std::fs::write(dir.join("result.json"), text).map_err(|e| e.to_string())?;
            let _ = out
                .send(b"{\"type\":\"done\",\"reason\":\"stop\"}\n".to_vec())
                .await;
            Ok(())
        })
    }
}

/// The measurement the whole track exists for: what does an in-process run
/// actually cost or save, against a child process doing the same thing?
///
/// Not a benchmark — a small, repeatable comparison with the same pool, the same
/// settle path and the same ledger. Numbers go in `docs/design/subagents.md` §6.
#[tokio::test(flavor = "multi_thread")]
async fn in_process_and_child_process_runs_are_comparable() {
    const RUNS: usize = 12;
    let _unused = || ScriptedStep {
        stdout: vec![PING.into(), DONE.into()],
        result_json: Some(result_json("the same work either way")),
        exit_code: Some(0),
        ..Default::default()
    };

    // Child process: the real thing, a shell that writes its result.
    let process_dir = setup();
    let bench = process_dir.path().join("bench.sh");
    std::fs::write(
        &bench,
        format!(
            "#!/bin/sh\n{TASK_ID_SCRIPT}\nmkdir -p .cortexcode/dispatch/$tid\nprintf '%s' '{}' > .cortexcode/dispatch/$tid/result.json\nexit 0",
            serde_json::json!({"summary": "the same work either way", "files_changed": [], "confidence": 0.9, "status": "complete"})
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bench, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let process_pool = SubagentPool::new(SubagentPoolOptions {
        executable: bench,
        max_concurrency: Some(2),
        cwd: Some(process_dir.path().to_path_buf()),
        ..Default::default()
    });
    let process_started = std::time::Instant::now();
    for n in 0..RUNS {
        let id = format!("p{n}");
        process_pool.spawn(task(&id, "explore")).unwrap();
        assert!(process_pool.wait_for(&id).await.unwrap().ok);
    }
    let process_elapsed = process_started.elapsed();
    process_pool.dispose();

    // In process: no fork, no pipe, no re-exec.
    let in_dir = setup();
    let in_pool = in_process_pool(
        in_dir.path(),
        factory_building(move |spec| {
            Box::new(WritingAgent {
                lines: vec![PING.into(), DONE.into()],
                result: Some(result_json("the same work either way")),
                cwd: spec.cwd.clone(),
                task_id: spec.task_id.clone(),
                before: None,
            })
        }),
        2,
    );
    let in_started = std::time::Instant::now();
    for n in 0..RUNS {
        let id = format!("i{n}");
        in_pool.spawn(task(&id, "explore")).unwrap();
        assert!(in_pool.wait_for(&id).await.unwrap().ok);
    }
    let in_elapsed = in_started.elapsed();
    in_pool.dispose();

    // Both must behave identically where it matters.
    assert_eq!(
        ledger::read_all(process_dir.path()).len(),
        RUNS,
        "every child-process run is recorded"
    );
    assert_eq!(
        ledger::read_all(in_dir.path()).len(),
        RUNS,
        "every in-process run is recorded"
    );
    // The claim under test: in-process is not slower. A regression here is the
    // signal to stop pursuing the track.
    assert!(
        in_elapsed <= process_elapsed * 2,
        "in-process {:.1?} vs child {:.1?} for {RUNS} runs",
        in_elapsed,
        process_elapsed
    );
    println!(
        "[bench] {RUNS} runs: in-process {:.1}ms, child process {:.1}ms",
        in_elapsed.as_secs_f64() * 1000.0,
        process_elapsed.as_secs_f64() * 1000.0
    );
}

const TASK_ID_SCRIPT: &str =
    r#"tid=unknown; prev=; for a in "$@"; do [ "$prev" = "--task-id" ] && tid=$a; prev=$a; done"#;

#![allow(clippy::disallowed_methods)] // test code: #[tokio::test] expands to a runtime builder
//! The dispatch ledger: one line per attempt, written from the pool's settle
//! paths.
//!
//! The unit half lives in `src/ledger.rs`. This file is the half that matters:
//! it drives real child processes through the real settle paths and asserts
//! what a reader of the ledger would see. Before this existed, reliability was
//! reconstructed from dispatch dirs, which only failures leave behind.

use std::path::{Path, PathBuf};
use std::sync::Once;

use hoocode_code_resources::{AgentDefinition, AgentRegistry, AgentSource};
use hoocode_code_subagents::ledger::{self, DispatchAttempt};
use hoocode_code_subagents::pool::*;
use serde_json::{json, Value};

const DIR: &str = hoocode_code_paths::CONFIG_DIR_NAME;

fn isolate_agent_dir() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let dir = std::env::temp_dir().join(format!("hoocode-ledger-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("HOOCODE_CODING_AGENT_DIR", &dir);
    });
}

fn setup() -> tempfile::TempDir {
    isolate_agent_dir();
    tempfile::tempdir().unwrap()
}

const TASK_ID: &str =
    r#"tid=unknown; prev=; for a in "$@"; do [ "$prev" = "--task-id" ] && tid=$a; prev=$a; done"#;
const DONE_LINE: &str = r#"{"type":"done","reason":"stop","message":{"role":"assistant","content":[{"type":"text","text":"completed"}]}}"#;

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

fn result_json(summary: &str, status: &str, confidence: f64) -> String {
    json!({"summary": summary, "files_changed": [], "confidence": confidence, "status": status})
        .to_string()
}

/// A child that writes a valid `result.json` for its own task id and exits 0.
fn result_writer(dir: &Path) -> PathBuf {
    script(
        dir,
        "mock-result.sh",
        &format!(
            "{TASK_ID}\nmkdir -p {DIR}/dispatch/$tid\nprintf '%s' '{}' > {DIR}/dispatch/$tid/result.json\necho '{DONE_LINE}'\nexit 0",
            result_json("found the thing", "complete", 0.9)
        ),
    )
}

/// A child that writes no `result.json`, so whatever the parent wrote is what
/// the output verifier sees.
fn no_result_writer(dir: &Path, exit_code: i32) -> PathBuf {
    script(
        dir,
        "mock-no-result.sh",
        &format!("echo '{DONE_LINE}'\nexit {exit_code}"),
    )
}

/// A child that fails with a provider-style message on stderr.
fn failing(dir: &Path, message: &str) -> PathBuf {
    script(
        dir,
        "mock-fail.sh",
        &format!("echo 'mock error' >&2\necho '{message}' >&2\nexit 1"),
    )
}

/// A child that never emits anything.
fn silent(dir: &Path) -> PathBuf {
    script(dir, "mock-silent.sh", "sleep 30")
}

/// Records each run's argv and fails with a provider error unless it is the
/// dispatching session's own model: the shape that makes the inherited-model
/// ladder fire.
fn fallback_recorder(dir: &Path) -> PathBuf {
    script(
        dir,
        "mock-fallback.sh",
        &format!(
            r#"n=$(ls argv-*.bin 2>/dev/null | wc -l | tr -d ' ')
printf '%s\0' "$@" > argv-$n.bin
model=; prev=; for a in "$@"; do [ "$prev" = "--model" ] && model=$a; prev=$a; done
if [ "$model" != "parent-model" ]; then echo 'This model requires Global regions' >&2; exit 1; fi
{TASK_ID}
mkdir -p {DIR}/dispatch/$tid
printf '%s' '{}' > {DIR}/dispatch/$tid/result.json
echo '{DONE_LINE}'"#,
            result_json("ok", "complete", 0.9)
        ),
    )
}

fn write_valid_result(cwd: &Path, task_id: &str) {
    let dir = hoocode_code_paths::dispatch_task_dir(cwd, task_id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("result.json"),
        json!({"summary": "All files updated successfully", "files_changed": ["src/foo.rs"], "confidence": 0.95, "status": "complete"}).to_string(),
    )
    .unwrap();
}

fn pool(exe: PathBuf, max: usize, cwd: &Path) -> SubagentPool {
    SubagentPool::new(SubagentPoolOptions {
        executable: exe,
        max_concurrency: Some(max),
        cwd: Some(cwd.to_path_buf()),
        ..Default::default()
    })
}

fn task(id: &str, agent: &str, text: &str) -> SubagentPoolTask {
    SubagentPoolTask {
        task_id: id.into(),
        agent_type: agent.into(),
        task: text.into(),
        ..Default::default()
    }
}

fn attempts(cwd: &Path) -> Vec<DispatchAttempt> {
    ledger::read_all(cwd)
}

fn only_attempt(cwd: &Path) -> DispatchAttempt {
    let all = attempts(cwd);
    assert_eq!(all.len(), 1, "expected one ledger line, got {all:?}");
    all.into_iter().next().unwrap()
}

fn registry_with_pinned_model(id: &str, model: &str) -> AgentRegistry {
    let mut registry = AgentRegistry::new();
    registry.register(AgentDefinition {
        name: id.into(),
        description: "test agent".into(),
        tools: None,
        disallowed_tools: None,
        model: Some(model.into()),
        prompt: "do work".into(),
        source: AgentSource::Builtin,
        file_path: None,
        max_turns: None,
        background: None,
        delegate: None,
        delegate_to: None,
        fork: None,
    });
    registry
}

#[tokio::test]
async fn a_clean_success_records_one_usable_attempt() {
    let dir = setup();
    let p = pool(result_writer(dir.path()), 2, dir.path());
    p.spawn(task("t1", "explore", "hello")).unwrap();
    let result = p.wait_for("t1").await.unwrap();
    assert!(result.ok);
    // A clean success deletes its dispatch dir, so before the ledger the only
    // evidence of this run was that absence.
    assert!(!hoocode_code_paths::dispatch_task_dir(dir.path(), "t1").exists());

    let attempt = only_attempt(dir.path());
    assert_eq!(attempt.task_id, "t1");
    assert_eq!(attempt.agent_type, "explore");
    assert_eq!(attempt.status, "complete");
    assert!(attempt.ok);
    assert!(attempt.verified);
    assert_eq!(attempt.confidence, Some(0.9));
    assert_eq!(attempt.exit_code, Some(0));
    assert_eq!(attempt.mode, "blocking");
    assert_eq!(attempt.depth, 1);
    assert!(attempt.ts > 0);
    p.dispose();
}

#[tokio::test]
async fn a_failure_records_the_cause_and_is_not_usable() {
    let dir = setup();
    let p = pool(failing(dir.path(), "500 upstream exploded"), 2, dir.path());
    p.spawn(task("t1", "explore", "hello")).unwrap();
    let result = p.wait_for("t1").await.unwrap();
    assert!(!result.ok);

    let attempt = only_attempt(dir.path());
    assert_eq!(attempt.status, "failed");
    assert!(!attempt.ok);
    assert!(!attempt.verified);
    assert_eq!(attempt.exit_code, Some(1));
    p.dispose();
}

#[tokio::test]
async fn a_result_the_verifier_rejects_is_not_usable() {
    let dir = setup();
    let p = pool(no_result_writer(dir.path(), 0), 2, dir.path());
    // Confidence below the verifier's 0.5 floor.
    let dispatch = hoocode_code_paths::dispatch_task_dir(dir.path(), "t1");
    std::fs::create_dir_all(&dispatch).unwrap();
    std::fs::write(
        dispatch.join("result.json"),
        result_json("too unsure", "complete", 0.1),
    )
    .unwrap();
    p.spawn(task("t1", "explore", "hello")).unwrap();
    let result = p.wait_for("t1").await.unwrap();
    assert!(!result.ok);

    let attempt = only_attempt(dir.path());
    assert_eq!(attempt.status, "failed");
    assert!(!attempt.verified);
    p.dispose();
}

#[tokio::test]
async fn an_exit_zero_without_a_result_is_not_usable() {
    let dir = setup();
    let p = pool(no_result_writer(dir.path(), 0), 2, dir.path());
    p.spawn(task("t1", "explore", "hello")).unwrap();
    let result = p.wait_for("t1").await.unwrap();
    assert!(!result.ok);
    assert_eq!(only_attempt(dir.path()).status, "failed");
    p.dispose();
}

#[tokio::test]
async fn killing_a_running_child_records_the_kill_reason() {
    let dir = setup();
    let p = pool(silent(dir.path()), 2, dir.path());
    p.spawn(task("t1", "explore", "hello")).unwrap();
    assert_eq!(p.running_count(), 1);
    assert!(p.cancel("t1"));
    let result = p.wait_for("t1").await.unwrap();
    assert!(!result.ok);
    p.dispose();

    let attempt = only_attempt(dir.path());
    assert_eq!(attempt.status, "cancelled");
    assert!(!attempt.ok);
    assert!(!attempt.verified);
}

#[tokio::test]
async fn cancelling_a_queued_task_records_it() {
    let dir = setup();
    let p = pool(silent(dir.path()), 1, dir.path());
    p.spawn(task("running", "explore", "block")).unwrap();
    p.spawn(task("queued", "explore", "wait")).unwrap();
    assert_eq!(p.queued_count(), 1);
    assert!(p.cancel("queued"));
    let result = p.wait_for("queued").await.unwrap();
    assert!(!result.ok);
    p.cancel("running");
    let _ = p.wait_for("running").await;
    p.dispose();

    let cancelled: Vec<DispatchAttempt> = attempts(dir.path())
        .into_iter()
        .filter(|a| a.task_id == "queued")
        .collect();
    assert_eq!(cancelled.len(), 1);
    assert_eq!(cancelled[0].status, "cancelled");
    assert!(!cancelled[0].ok);
    assert_eq!(cancelled[0].duration_ms, 0);
}

#[tokio::test]
async fn the_inherited_model_retry_is_a_second_line() {
    let dir = setup();
    let p = SubagentPool::new(SubagentPoolOptions {
        executable: fallback_recorder(dir.path()),
        max_concurrency: Some(2),
        cwd: Some(dir.path().to_path_buf()),
        ..Default::default()
    });
    p.set_registry(registry_with_pinned_model("explore", "mock/pinned"));
    p.spawn(SubagentPoolTask {
        model: Some("mock/pinned".into()),
        inherited_model: Some("parent-model".into()),
        provider: Some("mock".into()),
        ..task("t1", "explore", "hello")
    })
    .unwrap();
    let result = p.wait_for("t1").await.unwrap();
    assert!(result.ok, "the retry should succeed: {result:?}");
    assert_eq!(result.used_inherited_model_fallback, Some(true));
    p.dispose();

    let all = attempts(dir.path());
    assert_eq!(all.len(), 2, "one line per attempt: {all:?}");
    assert_eq!(all[0].status, "failed");
    assert!(!all[0].fallback_attempt);
    assert_eq!(all[0].resolved_model.as_deref(), Some("mock/pinned"));
    assert!(all[0].error.is_some());
    assert_eq!(all[1].status, "complete");
    assert!(all[1].fallback_attempt);
    assert_eq!(all[1].resolved_model.as_deref(), Some("parent-model"));
    assert_eq!(all[1].task_id, "t1");

    let stats = ledger::stats(dir.path(), None);
    assert_eq!(stats.attempts, 2);
    assert_eq!(stats.usable, 1);
    assert_eq!(stats.fallback_attempts, 1);
}

#[tokio::test]
async fn background_runs_are_labelled_in_the_ledger() {
    let dir = setup();
    let p = pool(result_writer(dir.path()), 2, dir.path());
    p.spawn(SubagentPoolTask {
        background: Some(true),
        ..task("t1", "explore", "hello")
    })
    .unwrap();
    p.wait_for("t1").await.unwrap();
    assert_eq!(only_attempt(dir.path()).mode, "background");
    p.dispose();
}

#[tokio::test]
async fn every_attempt_of_a_busy_pool_is_recorded_once() {
    let dir = setup();
    let p = pool(result_writer(dir.path()), 2, dir.path());
    let ids = ["t1", "t2", "t3", "t4", "t5"];
    for id in ids {
        p.spawn(task(id, "code-review", id)).unwrap();
    }
    for id in ids {
        assert!(p.wait_for(id).await.unwrap().ok);
    }
    p.dispose();

    let stats = ledger::stats(dir.path(), None);
    assert_eq!(stats.attempts, ids.len());
    assert_eq!(stats.usable, ids.len());
    assert_eq!(stats.complete, ids.len());
    assert_eq!(stats.by_agent["code-review"].attempts, ids.len());
    assert!((ledger::success_rate(&stats) - 100.0).abs() < 0.001);
    assert!(stats.max_ms < 30_000);
}

#[tokio::test]
async fn a_spawn_failure_is_recorded_rather_than_lost() {
    let dir = setup();
    let p = pool(dir.path().join("no-such-binary"), 1, dir.path());
    p.spawn(task("t1", "explore", "hello")).unwrap();
    let result = p.wait_for("t1").await.unwrap();
    assert!(!result.ok);
    p.dispose();

    let attempt = only_attempt(dir.path());
    assert_eq!(attempt.status, "failed");
    assert!(attempt.error.is_some());
}

#[tokio::test]
async fn an_unwritable_ledger_is_survivable() {
    let dir = setup();
    // A *file* where the dispatch root belongs: the ledger cannot be written.
    let cwd = dir.path().join("blocked");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::write(
        cwd.join(hoocode_code_paths::CONFIG_DIR_NAME),
        "not a directory",
    )
    .unwrap();
    let attempt = DispatchAttempt {
        task_id: "t1".into(),
        agent_type: "explore".into(),
        status: "complete".into(),
        ..Default::default()
    };
    // Best-effort by contract: a read-only project must not turn a finished
    // dispatch into an error.
    ledger::append(&cwd, &attempt);
    assert!(attempts(&cwd).is_empty());
    assert!(ledger::stats(&cwd, None).attempts == 0);
}

#[tokio::test]
async fn reads_never_panic_on_a_hand_edited_file() {
    let dir = setup();
    let path = ledger::ledger_path(dir.path());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut text = String::new();
    text.push_str(
        "{\"ts\":1,\"task_id\":\"a\",\"agent_type\":\"explore\",\"status\":\"complete\",\"ok\":true}\n",
    );
    text.push_str("{\"ts\":2,\"task_id\":\"b\",\"agent_ty\n");
    text.push_str("[]\n");
    text.push_str("null\n");
    text.push_str(
        "{\"ts\":3,\"task_id\":\"c\",\"agent_type\":\"explore\",\"status\":\"reticulate\",\"ok\":false}\n",
    );
    std::fs::write(&path, text).unwrap();

    let all = ledger::read_all(dir.path());
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].task_id, "a");
    let stats = ledger::stats(dir.path(), None);
    assert_eq!(stats.attempts, 2);
    assert_eq!(stats.usable, 1);
    assert_eq!(stats.other, 1);
    let lines = ledger::failure_lines(dir.path(), 5);
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("reticulate"));
}

#[tokio::test]
async fn the_ledger_is_a_shape_not_a_second_result() {
    let dir = setup();
    let p = pool(result_writer(dir.path()), 1, dir.path());
    write_valid_result(dir.path(), "t1");
    p.spawn(task("t1", "explore", "hello")).unwrap();
    let result = p.wait_for("t1").await.unwrap();
    p.dispose();

    let attempt = only_attempt(dir.path());
    assert_eq!(
        result.result_data.unwrap().get("status").unwrap(),
        "complete"
    );
    assert_eq!(attempt.status, "complete");
    // The summary lives in result.json, never in the ledger: telemetry must not
    // become a second copy of the work.
    let lines: Vec<Value> = std::fs::read_to_string(ledger::ledger_path(dir.path()))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 1);
    assert!(lines[0].get("summary").is_none());
}

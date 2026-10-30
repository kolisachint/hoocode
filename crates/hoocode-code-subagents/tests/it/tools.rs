#![allow(clippy::disallowed_methods)] // test code: #[tokio::test] expands to a runtime builder
//! task-output.test.ts, subagent-progress-roster.test.ts and subagent.test.ts,
//! plus the Agent tool's execute paths against a pool of shell-mock children
//! (the TS suite's fake pool is 10.9f's subagent-execution test).
//!
//! The inbox, task store and shared pool are process-wide, so each group
//! runs as one serialized test.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Once};

use hoocode_agent_types::{AgentToolCall, AgentToolResult, AgentToolUpdateCallback};
use hoocode_ai_types::Content;
use hoocode_code_resources::agent_registry::EMBEDDED_AGENT_PROMPTS;
use hoocode_code_resources::{parse_agent_definition, AgentSource};
use hoocode_code_settings::{ModelCategoryName, ScopedModel};
use hoocode_code_subagents::inbox::{subagent_inbox, TaskLifecycle};
use hoocode_code_subagents::instance::{
    dispose_subagent_pool, get_subagent_pool, set_scoped_models_override,
    set_subagent_pool_for_testing,
};
use hoocode_code_subagents::live::run_for_tool_call;
use hoocode_code_subagents::pool::{
    DispatchOptions, ResultStatus, SubagentPool, SubagentPoolOptions, SubagentResult, TaskResult,
};
use hoocode_code_subagents::tools::*;
use hoocode_code_task_store::{
    task_store, TaskAgentKind, TaskAgentPatch, TaskAgentState, TaskStatus,
};
use hoocode_code_tool_api::{ToolContext, ToolDefinition};
use serde_json::{json, Value};

use crate::SERIAL;

fn isolate_agent_dir() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let dir = std::env::temp_dir().join(format!("hoocode-tools-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("HOOCODE_CODING_AGENT_DIR", &dir);
    });
}

fn text(result: &AgentToolResult) -> String {
    match &result.content[0] {
        Content::Text(t) => t.text.clone(),
        other => panic!("{other:?}"),
    }
}

fn ok_result(task_id: &str, summary: &str) -> TaskResult {
    TaskResult {
        task_id: Some(task_id.into()),
        agent_type: Some("explore".into()),
        result: Some(SubagentResult {
            task_id: task_id.into(),
            ok: true,
            exit_code: Some(0),
            status: Some(ResultStatus::Complete),
            result_data: json!({"summary": summary, "files_changed": [], "confidence": 0.9, "status": "complete"})
                .as_object()
                .cloned(),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn run(
    tool: &ToolDefinition,
    args: Value,
    ctx: Option<&ToolContext>,
) -> Result<AgentToolResult, String> {
    (tool.execute)("tc".into(), args, None, None, ctx).map_err(|e| e.to_string())
}

fn assert_subset(details: &Value, expected: Value) {
    for (k, v) in expected.as_object().unwrap() {
        assert_eq!(&details[k], v, "{k} in {details}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn task_output_tool() {
    let _serial = SERIAL.lock().await;
    let inbox = subagent_inbox();
    let tool = create_task_output_tool_definition();

    // lists all background subagents with status, without bodies
    inbox.clear();
    inbox.start("t1", "explore#1", "explore", "");
    inbox.start("t2", "explore#2", "explore", "");
    inbox.finish(
        "t2",
        &ok_result(
            "t2",
            "Two: headline\nlong body that must not appear in the roster listing",
        ),
    );
    let r = run(&tool, json!({"list": true}), None).unwrap();
    let t = text(&r);
    for needle in ["explore#1", "running", "explore#2", "done", "Two: headline"] {
        assert!(t.contains(needle), "{needle} in {t}");
    }
    assert!(!t.contains("must not appear in the roster"));
    assert_subset(
        &r.details,
        json!({"status": "list", "ok": true, "outstanding": 1}),
    );

    // returns the body once, then reports it as already delivered
    inbox.clear();
    inbox.start("t1", "explore#1", "explore", "");
    inbox.finish("t1", &ok_result("t1", "the full result body"));
    let first = run(&tool, json!({"task_id": "explore#1"}), None).unwrap();
    assert_eq!(text(&first), "the full result body");
    assert_subset(&first.details, json!({"status": "done", "ok": true}));
    let second = run(&tool, json!({"task_id": "explore#1"}), None).unwrap();
    assert!(text(&second).contains("already delivered"));
    assert_subset(&second.details, json!({"status": "collected", "ok": true}));

    // reports status (not an error) for a still-running task
    inbox.clear();
    inbox.start("t1", "explore#1", "explore", "");
    let r = run(&tool, json!({"task_id": "explore#1"}), None).unwrap();
    assert!(text(&r).contains("still running"));
    assert_subset(&r.details, json!({"status": "running", "ok": true}));

    // never throws on an unknown handle
    let r = run(&tool, json!({"task_id": "nope#9"}), None).unwrap();
    assert!(text(&r).contains("No background task"));
    assert_subset(&r.details, json!({"status": "unknown", "ok": false}));

    // reports a failed task without throwing
    inbox.clear();
    inbox.start("t1", "explore#1", "explore", "");
    inbox.fail("t1", "no heartbeat", TaskLifecycle::Stalled);
    let r = run(&tool, json!({"task_id": "explore#1"}), None).unwrap();
    assert!(text(&r).contains("stalled"));
    assert!(text(&r).contains("no heartbeat"));
    assert_subset(&r.details, json!({"status": "stalled", "ok": false}));

    // wait:true blocks until the task finishes, then returns its body
    inbox.clear();
    inbox.start("t1", "explore#1", "explore", "");
    // The waiter is signalled by `finish`, not by a timer: a fixed sleep here
    // was a coin flip on a loaded machine.
    tokio::spawn(async {
        tokio::task::yield_now().await;
        subagent_inbox().finish("t1", &ok_result("t1", "finished after waiting"));
    });
    let tool2 = tool.clone();
    let r = tokio::task::spawn_blocking(move || {
        run(
            &tool2,
            json!({"task_id": "explore#1", "wait": true, "timeout_ms": 2000}),
            None,
        )
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(text(&r), "finished after waiting");
    assert_subset(&r.details, json!({"status": "done", "ok": true}));

    // wait:true with no task_id is a barrier for all outstanding tasks
    inbox.clear();
    inbox.start("t1", "explore#1", "explore", "");
    inbox.start("t2", "explore#2", "explore", "");
    tokio::spawn(async {
        tokio::task::yield_now().await;
        subagent_inbox().finish("t1", &ok_result("t1", "one"));
        tokio::task::yield_now().await;
        subagent_inbox().finish("t2", &ok_result("t2", "two"));
    });
    let tool3 = tool.clone();
    let r = tokio::task::spawn_blocking(move || {
        run(&tool3, json!({"wait": true, "timeout_ms": 2000}), None)
    })
    .await
    .unwrap()
    .unwrap();
    assert_subset(&r.details, json!({"status": "list", "outstanding": 0}));
    inbox.clear();
}

#[tokio::test(flavor = "multi_thread")]
async fn progress_events_drive_roster_activity() {
    let _serial = SERIAL.lock().await;
    isolate_agent_dir();
    let store = task_store();
    let activity_of = |id: &str| {
        store
            .agents()
            .into_iter()
            .find(|a| a.id == id)
            .and_then(|a| a.activity)
    };
    let upsert = |id: &str, name: &str| {
        store.upsert_agent(
            id,
            name,
            TaskAgentKind::Subagent,
            TaskAgentPatch {
                state: Some(TaskAgentState::Running),
                ..Default::default()
            },
        );
    };
    let progress = |pool: &SubagentPool, id: &str, event: Value| {
        pool.emit_for_testing(
            "task_progress",
            json!({"task_id": id, "agent_type": "explore", "event": event}),
        )
    };
    let cwd = std::env::current_dir().unwrap();

    // tool start sets the activity, tool end clears it
    store.clear();
    upsert("run-1", "explore#1");
    let pool = get_subagent_pool(&cwd, &[]);
    progress(
        &pool,
        "run-1",
        json!({"type": "tool_execution_start", "toolName": "CodeSearch"}),
    );
    assert_eq!(activity_of("run-1").as_deref(), Some("CodeSearch"));
    progress(
        &pool,
        "run-1",
        json!({"type": "tool_execution_end", "toolName": "CodeSearch"}),
    );
    assert_eq!(activity_of("run-1").as_deref(), Some(""));

    // thinking between turns; terminal events clear
    progress(
        &pool,
        "run-1",
        json!({"type": "tool_execution_start", "toolName": "Shell"}),
    );
    assert_eq!(activity_of("run-1").as_deref(), Some("Shell"));
    progress(&pool, "run-1", json!({"type": "turn_end"}));
    assert_eq!(activity_of("run-1").as_deref(), Some("thinking"));
    pool.emit_for_testing(
        "task_done",
        json!({"agent_type": "explore", "task_id": "run-1"}),
    );
    assert_eq!(activity_of("run-1").as_deref(), Some(""));

    // concurrent same-type runs keep separate rows
    upsert("run-2", "explore#2");
    progress(
        &pool,
        "run-1",
        json!({"type": "tool_execution_start", "toolName": "CodeSearch"}),
    );
    progress(
        &pool,
        "run-2",
        json!({"type": "tool_execution_start", "toolName": "Shell"}),
    );
    pool.emit_for_testing(
        "task_done",
        json!({"agent_type": "explore", "task_id": "run-2"}),
    );
    assert_eq!(activity_of("run-1").as_deref(), Some("CodeSearch"));
    assert_eq!(activity_of("run-2").as_deref(), Some(""));

    // a run with no roster row is a no-op
    progress(
        &pool,
        "ghost-run",
        json!({"type": "tool_execution_start", "toolName": "CodeSearch"}),
    );
    assert!(store.agents().iter().all(|a| a.id != "ghost-run"));
    dispose_subagent_pool();
    store.clear();
}

fn builtin_tools(name: &str) -> Vec<String> {
    let raw = EMBEDDED_AGENT_PROMPTS
        .iter()
        .find(|(n, _)| *n == name)
        .unwrap()
        .1;
    parse_agent_definition(raw, AgentSource::Builtin, None, Some(name))
        .0
        .unwrap()
        .tools
        .unwrap_or_default()
}

#[test]
fn builtin_subagent_allowlists() {
    let mut names: Vec<&str> = EMBEDDED_AGENT_PROMPTS.iter().map(|(n, _)| *n).collect();
    names.sort();
    assert_eq!(
        names,
        [
            "code-review",
            "explore",
            "general-purpose",
            "plan",
            "security-review"
        ]
    );
    for name in ["code-review", "security-review"] {
        let tools = builtin_tools(name);
        assert!(tools.contains(&"Shell".to_string()));
        assert!(!tools.iter().any(|t| t == "Edit" || t == "Write"));
    }
    for name in ["plan", "explore"] {
        let tools = builtin_tools(name);
        assert!(
            !tools
                .iter()
                .any(|t| t == "Edit" || t == "Write" || t == "Shell"),
            "{name}"
        );
    }
    for name in names {
        assert!(!builtin_tools(name).is_empty(), "{name}");
    }
    assert!(builtin_tools("general-purpose").contains(&"Write".to_string()));
    let raw = EMBEDDED_AGENT_PROMPTS
        .iter()
        .find(|(n, _)| *n == "general-purpose")
        .unwrap()
        .1;
    let gp = parse_agent_definition(raw, AgentSource::Builtin, None, Some("general-purpose"))
        .0
        .unwrap();
    assert_eq!(gp.delegate, Some(true));
}

#[test]
fn resolve_fork_session_file_cases() {
    let dir = tempfile::tempdir().unwrap();
    let parent = dir.path().join("parent.jsonl");
    std::fs::write(
        &parent,
        format!(
            "{}\n",
            json!({"type": "session", "version": 3, "id": "parent", "timestamp": "2026-01-01T00:00:00.000Z", "cwd": dir.path()})
        ),
    )
    .unwrap();
    assert_eq!(
        resolve_fork_session_file(None, Some(&parent), dir.path()),
        None
    );
    assert_eq!(
        resolve_fork_session_file(Some(true), None, dir.path()),
        None
    );
    std::env::set_var(
        "HOOCODE_CODING_AGENT_SESSION_DIR",
        dir.path().join("sessions"),
    );
    let forked = resolve_fork_session_file(Some(true), Some(&parent), dir.path()).unwrap();
    std::env::remove_var("HOOCODE_CODING_AGENT_SESSION_DIR");
    // Session writes are queued to hoocode-session-io; wait for the file to land.
    hoocode_runtime::session_io()
        .flush_blocking(std::time::Duration::from_secs(5))
        .expect("session writes flush");
    assert!(forked.exists());
    assert_ne!(forked, parent);
    let empty = dir.path().join("empty.jsonl");
    std::fs::write(&empty, "").unwrap();
    assert_eq!(
        resolve_fork_session_file(Some(true), Some(&empty), dir.path()),
        None
    );
}

#[test]
fn summarize_agent_description_cases() {
    let builtin_style = "Use this subagent ONLY when:\n- Reading or understanding code without changes\n- Scouting a codebase for plans or maps\n- Analyzing dependencies, imports, project structure\n\nDO NOT use for:\n- Writing or modifying code";
    assert_eq!(
        summarize_agent_description(builtin_style),
        "Reading or understanding code without changes; Scouting a codebase for plans or maps; Analyzing dependencies, imports, project structure"
    );
    assert_eq!(
        summarize_agent_description("Expert TypeScript reviewer"),
        "Expert TypeScript reviewer"
    );
    assert_eq!(
        summarize_agent_description("Reviews PRs for correctness and risk.\nMore detail here."),
        "Reviews PRs for correctness and risk."
    );
    assert_eq!(summarize_agent_description(""), "");
}

#[test]
fn build_task_main_prompt_contents() {
    isolate_agent_dir();
    let prompt = build_task_main_prompt();
    assert!(prompt.contains("<available_agents>"));
    assert!(!prompt.contains("- explore: "));
    assert!(prompt.contains("Dispatch independent subtasks in the same turn"));
    assert!(prompt.contains("Keep inline only trivial single-step edits"));
    assert!(!prompt.contains("Default to handling small, quick, or single-file work inline"));
    assert!(prompt.contains("mark the item in_progress BEFORE dispatching"));
    // Every agent is a background agent by default, so the parent guidance is always in.
    assert!(prompt.contains("keep working on anything that does not need their results"));
    assert!(prompt.contains("Call AgentOutput with wait:true only when"));
    assert!(!prompt.contains("DO NOT stop and wait"));
    assert!(prompt.contains("Delegate when you need only the final result"));
    assert!(!prompt.contains("WHEN TO USE:"));
    assert_eq!(prompt.matches("cannot see this conversation").count(), 1);
    assert!(!prompt.contains("When to delegate:"));
    assert!(!prompt.contains("Delegate proactively when work is self-contained"));
}

#[test]
fn format_duration_secs_like_hoocode() {
    assert_eq!(format_duration_secs(0.0), "0.0s");
    assert_eq!(format_duration_secs(3.21), "3.2s");
    assert_eq!(format_duration_secs(42.4), "42s");
    assert_eq!(format_duration_secs(125.0), "2m05s");
    assert_eq!(format_duration_secs(3720.0), "1h02m");
    assert_eq!(format_duration_secs(-1.0), "0.0s");
}

#[test]
fn the_task_tool_runs_in_the_background_per_agent_or_argument() {
    isolate_agent_dir();
    let cwd = tempfile::tempdir().unwrap();
    let tool = create_task_tool_definition(cwd.path());
    let predicate = tool.background_when.clone().unwrap();
    let call = |args: Value| AgentToolCall {
        id: "c".into(),
        name: "Agent".into(),
        arguments: args,
    };
    // Every agent is a background agent by default, general-purpose included (card 9, D1).
    assert!(predicate(&call(json!({"subagent_type": "explore"}))));
    assert!(predicate(&call(
        json!({"subagent_type": "general-purpose"})
    )));
    // A per-call argument overrides either way.
    assert!(!predicate(&call(
        json!({"subagent_type": "explore", "background": false})
    )));
    assert!(predicate(&call(
        json!({"subagent_type": "general-purpose", "background": true})
    )));
}

#[test]
fn an_agent_is_background_unless_its_definition_says_background_false() {
    isolate_agent_dir();
    let dir = tempfile::tempdir().unwrap();
    let agents = dir.path().join(DIR).join("agents");
    std::fs::create_dir_all(&agents).unwrap();
    std::fs::write(
        agents.join("fg-only.md"),
        "---\nname: fg-only\ndescription: Waits for its answer.\nbackground: false\n---\nBody.",
    )
    .unwrap();
    let tool = create_task_tool_definition(dir.path());
    let predicate = tool.background_when.clone().unwrap();
    let call = |args: Value| AgentToolCall {
        id: "c".into(),
        name: "Agent".into(),
        arguments: args,
    };
    assert!(predicate(&call(
        json!({"subagent_type": "general-purpose"})
    )));
    // The definition's opt-out makes foreground the agent's default.
    assert!(!predicate(&call(json!({"subagent_type": "fg-only"}))));
    // The call's argument wins over the definition in both directions.
    assert!(predicate(&call(
        json!({"subagent_type": "fg-only", "background": true})
    )));
    assert!(!predicate(&call(
        json!({"subagent_type": "general-purpose", "background": "false"})
    )));
}

// The Agent tool's execute paths, against a real pool of shell-mock children.

const DIR: &str = hoocode_code_paths::CONFIG_DIR_NAME;

fn mock_child(dir: &Path, summary: &str, status: &str, exit: i32) -> PathBuf {
    let path = dir.join("mock-child.sh");
    let result = json!({"summary": summary, "files_changed": [], "confidence": 0.9, "status": status,
        "usage": {"input": 10, "output": 5, "cacheRead": 0, "cacheWrite": 0, "cost": 0.01}});
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\ntid=unknown; prev=; for a in \"$@\"; do [ \"$prev\" = \"--task-id\" ] && tid=$a; prev=$a; done\nmkdir -p {DIR}/dispatch/$tid\nprintf '%s' '{result}' > {DIR}/dispatch/$tid/result.json\nexit {exit}\n"
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

fn install_pool(cwd: &Path, exe: PathBuf) -> SubagentPool {
    let pool = SubagentPool::new(SubagentPoolOptions {
        executable: exe,
        cwd: Some(cwd.to_path_buf()),
        ..Default::default()
    });
    set_subagent_pool_for_testing(Some(pool.clone()));
    pool
}

fn ctx(cwd: &Path) -> ToolContext {
    ToolContext {
        cwd: Some(cwd.to_path_buf()),
        ..Default::default()
    }
}

async fn execute(
    tool: ToolDefinition,
    args: Value,
    ctx: ToolContext,
) -> Result<AgentToolResult, String> {
    tokio::task::spawn_blocking(move || run(&tool, args, Some(&ctx)))
        .await
        .unwrap()
}

/// A `--models`-style override entry: no effort, with an optional category.
fn override_entry(model: &str, category: Option<ModelCategoryName>) -> ScopedModel {
    ScopedModel {
        model: model.into(),
        effort: None,
        category,
        alias: None,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn task_tool_execute_paths() {
    let _serial = SERIAL.lock().await;
    isolate_agent_dir();
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().canonicalize().unwrap();
    let tool = create_task_tool_definition(&cwd);
    task_store().clear();
    subagent_inbox().clear();

    // Foreground: the answer comes back inline and the task settles done.
    let pool = install_pool(&cwd, mock_child(&cwd, "Found it in main.rs", "complete", 0));
    let r = execute(
        tool.clone(),
        json!({"description": "Find the bug", "prompt": "Find the bug", "subagent_type": "general-purpose", "background": false}),
        ctx(&cwd),
    )
    .await
    .unwrap();
    assert!(text(&r).starts_with("[model: "), "{}", text(&r));
    assert!(
        text(&r).ends_with("\n\nFound it in main.rs"),
        "{}",
        text(&r)
    );
    assert_subset(
        &r.details,
        json!({"subagent_type": "general-purpose", "ok": true}),
    );
    let task = task_store()
        .list()
        .into_iter()
        .find(|t| t.title == "Find the bug")
        .unwrap();
    assert_eq!(task.status, TaskStatus::Done);
    assert_eq!(task.usage.unwrap().input, 10.0);
    let row = task_store()
        .agents()
        .into_iter()
        .find(|a| Some(&a.id) == task.agent.as_ref())
        .unwrap();
    assert_eq!(row.name, "general-purpose#1");
    assert_eq!(row.state, Some(TaskAgentState::Done));
    pool.dispose();

    // Background: a compact notification; the body waits in the inbox.
    let pool = install_pool(
        &cwd,
        mock_child(&cwd, "Mapped the module\nwith details", "complete", 0),
    );
    let r = execute(
        tool.clone(),
        json!({"description": "Map it", "prompt": "Map the module", "subagent_type": "explore"}),
        ctx(&cwd),
    )
    .await
    .unwrap();
    assert!(
        text(&r).starts_with(
            "explore#1 (\"Map it\") finished ✓ — Mapped the module.\nRead the full result with AgentOutput(\"explore#1\")."
        ),
        "{}",
        text(&r)
    );
    assert_subset(&r.details, json!({"ok": true, "background": true}));
    let (_, body) = subagent_inbox().collect("explore#1").unwrap();
    assert_eq!(body, "Mapped the module\nwith details");
    pool.dispose();

    // A failed foreground run is an error carrying the child's reason.
    let pool = install_pool(&cwd, mock_child(&cwd, "Task failed: quota", "failed", 1));
    let err = execute(
        tool.clone(),
        json!({"description": "Do it", "prompt": "Do it", "subagent_type": "general-purpose", "background": false}),
        ctx(&cwd),
    )
    .await
    .unwrap_err();
    assert!(
        err.to_string()
            .starts_with("Subagent (general-purpose) failed: Task failed: quota\n[model: "),
        "{err}"
    );
    pool.dispose();

    // An unknown agent is rejected with the available list.
    let err = execute(
        tool.clone(),
        json!({"description": "x", "prompt": "x", "subagent_type": "nope"}),
        ctx(&cwd),
    )
    .await
    .unwrap_err();
    assert!(
        err.starts_with("Unknown subagent_type: \"nope\". Available agents: "),
        "{err}"
    );
    assert!(err.contains("explore"));

    // An exhausted inherited provider skips the spawn.
    let model: hoocode_ai_types::Model = serde_json::from_value(json!({
        "id": "m", "name": "m", "api": "openai-completions", "provider": "prov",
        "baseUrl": "http://x", "contextWindow": 1000, "maxTokens": 100,
    }))
    .unwrap();
    hoocode_code_agent_session::provider_health::mark_provider_exhausted(
        "prov",
        "Usage limit reached",
    );
    let r = execute(
        tool.clone(),
        json!({"description": "Skip", "prompt": "x", "subagent_type": "explore"}),
        ToolContext {
            model: Some(model),
            ..ctx(&cwd)
        },
    )
    .await
    .unwrap();
    hoocode_code_agent_session::provider_health::clear_provider_exhaustion("prov");
    assert!(text(&r).starts_with(
        "Did not dispatch subagent \"explore\": the \"prov\" provider appears exhausted"
    ));
    let skipped = task_store()
        .list()
        .into_iter()
        .find(|t| t.title == "Skip")
        .unwrap();
    assert_eq!(skipped.status, TaskStatus::Failed);
    assert_eq!(skipped.note.as_deref(), Some("prov exhausted"));

    set_subagent_pool_for_testing(None);
    task_store().clear();
    subagent_inbox().clear();
}

// suite/subagent-execution.test.ts: the cases the paths above do not cover.

fn argv_child(dir: &Path) -> PathBuf {
    let path = dir.join("mock-argv.sh");
    let result =
        json!({"summary": "ok", "files_changed": [], "confidence": 0.9, "status": "complete"});
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\nprintf '%s\\0' \"$@\" > argv.bin\ntid=unknown; prev=; for a in \"$@\"; do [ \"$prev\" = \"--task-id\" ] && tid=$a; prev=$a; done\nmkdir -p {DIR}/dispatch/$tid\nprintf '%s' '{result}' > {DIR}/dispatch/$tid/result.json\n"
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

fn model_arg(dir: &Path) -> Option<String> {
    let bytes = std::fs::read(dir.join("argv.bin")).unwrap();
    let argv: Vec<String> = bytes
        .split(|b| *b == 0)
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect();
    let i = argv.iter().position(|a| a == "--model")?;
    argv.get(i + 1).cloned()
}

/// A child that appends each attempt's `--model` to `models.txt`: fails with
/// `failed` on the first (pinned) model, succeeds on any other.
fn failing_then_ok_child(dir: &Path, failed: &Value) -> PathBuf {
    let path = dir.join("mock-fallback.sh");
    let ok = json!({"summary": "recovered", "files_changed": [], "confidence": 0.9, "status": "complete"});
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\ntid=unknown; model=; prev=; for a in \"$@\"; do [ \"$prev\" = \"--task-id\" ] && tid=$a; [ \"$prev\" = \"--model\" ] && model=$a; prev=$a; done\nmkdir -p {DIR}/dispatch/$tid\necho \"$model\" >> models.txt\ncase \"$model\" in *pinned-model|*tier-model) printf '%s' '{failed}' > {DIR}/dispatch/$tid/result.json; exit 1;; esac\nprintf '%s' '{ok}' > {DIR}/dispatch/$tid/result.json\n"
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

/// Every `--model` the child was spawned with, in order.
fn model_arg_history(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("models.txt"))
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

#[tokio::test]
async fn task_tool_passes_the_model_ask_or_the_parent_model() {
    let _serial = SERIAL.lock().await;
    isolate_agent_dir();
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().canonicalize().unwrap();
    // An agent that inherits the model (built-ins pin a category, which wins).
    let agents = cwd.join(DIR).join("agents");
    std::fs::create_dir_all(&agents).unwrap();
    std::fs::write(
        agents.join("helper.md"),
        "---\nname: helper\ndescription: Inherits the model.\ntools: read\nmodel: inherit\n---\nHelp.",
    )
    .unwrap();
    let tool = create_task_tool_definition(&cwd);
    task_store().clear();
    let parent = scoped_test_model("prov", "parent-model");
    let fast = scoped_test_model("prov", "fast-model");
    // The scope: `fast` is the fast tier, the parent is an untagged scoped model.
    set_scoped_models_override(Some(vec![
        override_entry("prov/fast-model", Some(ModelCategoryName::Fast)),
        override_entry("prov/parent-model", None),
    ]));
    let pool = SubagentPool::new(SubagentPoolOptions {
        executable: argv_child(&cwd),
        cwd: Some(cwd.clone()),
        ..Default::default()
    });
    set_subagent_pool_for_testing(Some(pool.clone()));
    let with_model = || ToolContext {
        model: Some(parent.clone()),
        available_models: vec![parent.clone(), fast.clone()],
        ..ctx(&cwd)
    };
    execute(
        tool.clone(),
        json!({"description": "quick read", "prompt": "read one file", "subagent_type": "helper", "model": "fast"}),
        with_model(),
    )
    .await
    .unwrap();
    assert_eq!(model_arg(&cwd).as_deref(), Some("prov/fast-model"));
    // Without it, the parent's model, as provider/id.
    execute(
        tool.clone(),
        json!({"description": "do work", "prompt": "do some work", "subagent_type": "helper"}),
        with_model(),
    )
    .await
    .unwrap();
    assert_eq!(model_arg(&cwd).as_deref(), Some("prov/parent-model"));
    pool.dispose();
    set_subagent_pool_for_testing(None);
    set_scoped_models_override(None);
    task_store().clear();
}

/// The session's model is a scoped model when it is named `provider/id` in the
/// scope. It is used, and no "not scoped" note is given. The scope used to be
/// matched on the bare id, so a scoped session model was reported as unscoped.
#[tokio::test]
async fn a_scoped_session_model_is_used_without_a_not_scoped_note() {
    let _serial = SERIAL.lock().await;
    isolate_agent_dir();
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().canonicalize().unwrap();
    let agents = cwd.join(DIR).join("agents");
    std::fs::create_dir_all(&agents).unwrap();
    std::fs::write(
        agents.join("helper.md"),
        "---\nname: helper\ndescription: Inherits the model.\ntools: read\nmodel: inherit\n---\nHelp.",
    )
    .unwrap();
    let tool = create_task_tool_definition(&cwd);
    task_store().clear();
    let session_model = scoped_test_model("acme", "mid");
    set_scoped_models_override(Some(vec![override_entry("acme/mid", None)]));
    let pool = SubagentPool::new(SubagentPoolOptions {
        executable: argv_child(&cwd),
        cwd: Some(cwd.clone()),
        ..Default::default()
    });
    set_subagent_pool_for_testing(Some(pool.clone()));
    let result = execute(
        tool,
        json!({"description": "do work", "prompt": "do some work", "subagent_type": "helper"}),
        ToolContext {
            model: Some(session_model.clone()),
            available_models: vec![session_model],
            ..ctx(&cwd)
        },
    )
    .await
    .unwrap();
    assert_eq!(model_arg(&cwd).as_deref(), Some("acme/mid"));
    assert!(
        !text(&result).contains("not scoped"),
        "a scoped session model must not be reported as unscoped: {}",
        text(&result)
    );
    pool.dispose();
    set_subagent_pool_for_testing(None);
    set_scoped_models_override(None);
    task_store().clear();
}

/// Every lookup of the shared pool gives it the current scope, not the scope it
/// had when the pool was created. `/subagent` reads the pool this way and never
/// goes through the Agent tool, so it must not see a stale scope.
#[tokio::test]
async fn every_pool_lookup_refreshes_the_scope() {
    let _serial = SERIAL.lock().await;
    isolate_agent_dir();
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().canonicalize().unwrap();
    let available = vec![
        scoped_test_model("acme", "mid"),
        scoped_test_model("acme", "big"),
    ];
    let request = hoocode_code_subagents::model_categories::ModelRequest {
        ask: Some("acme/mid".into()),
        ..Default::default()
    };
    set_scoped_models_override(Some(vec![override_entry("acme/mid", None)]));
    let pool = get_subagent_pool(&cwd, &available);
    assert!(pool.select_model("explore", &request).error.is_none());
    // The scope changes (a picker save or a new run scope): the next lookup
    // must carry it, with no dispatch in between.
    set_scoped_models_override(Some(vec![override_entry("acme/big", None)]));
    let pool = get_subagent_pool(&cwd, &available);
    let selection = pool.select_model("explore", &request);
    assert!(
        selection.error.is_some(),
        "acme/mid is no longer scoped, so the ask must be refused: {selection:?}"
    );
    set_scoped_models_override(None);
}

/// A `--models` override reaches the children a dispatch spawns, as `--models`
/// with `id:effort` entries, so nested subagents stay within the run's scope.
#[tokio::test]
async fn the_models_override_is_passed_to_nested_children() {
    let _serial = SERIAL.lock().await;
    isolate_agent_dir();
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().canonicalize().unwrap();
    let agents = cwd.join(DIR).join("agents");
    std::fs::create_dir_all(&agents).unwrap();
    std::fs::write(
        agents.join("helper.md"),
        "---\nname: helper\ndescription: Inherits the model.\ntools: read\nmodel: inherit\n---\nHelp.",
    )
    .unwrap();
    let tool = create_task_tool_definition(&cwd);
    task_store().clear();
    let mid = scoped_test_model("acme", "mid");
    set_scoped_models_override(Some(vec![ScopedModel {
        model: "acme/mid".into(),
        effort: Some("high".into()),
        category: None,
        alias: None,
    }]));
    let pool = SubagentPool::new(SubagentPoolOptions {
        executable: argv_child(&cwd),
        cwd: Some(cwd.clone()),
        ..Default::default()
    });
    set_subagent_pool_for_testing(Some(pool.clone()));
    execute(
        tool,
        json!({"description": "do work", "prompt": "do some work", "subagent_type": "helper"}),
        ToolContext {
            model: Some(mid.clone()),
            available_models: vec![mid],
            ..ctx(&cwd)
        },
    )
    .await
    .unwrap();
    let bytes = std::fs::read(cwd.join("argv.bin")).unwrap();
    let argv: Vec<String> = bytes
        .split(|b| *b == 0)
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect();
    let i = argv
        .iter()
        .position(|a| a == "--models")
        .expect("--models is passed on");
    assert_eq!(argv.get(i + 1).map(String::as_str), Some("acme/mid:high"));
    pool.dispose();
    set_subagent_pool_for_testing(None);
    set_scoped_models_override(None);
    task_store().clear();
}

/// T3.2: an Agent call announces its pool run at dispatch, as a partial whose
/// details carry `poolTaskId`, and the run is findable by the call id after.
#[tokio::test]
async fn the_agent_call_announces_its_run_at_dispatch() {
    let _serial = SERIAL.lock().await;
    isolate_agent_dir();
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().canonicalize().unwrap();
    let agents = cwd.join(DIR).join("agents");
    std::fs::create_dir_all(&agents).unwrap();
    std::fs::write(
        agents.join("helper.md"),
        "---\nname: helper\ndescription: Inherits the model.\ntools: read\nmodel: inherit\n---\nHelp.",
    )
    .unwrap();
    let tool = create_task_tool_definition(&cwd);
    task_store().clear();
    let pool = SubagentPool::new(SubagentPoolOptions {
        executable: argv_child(&cwd),
        cwd: Some(cwd.clone()),
        ..Default::default()
    });
    set_subagent_pool_for_testing(Some(pool.clone()));
    let partials = Arc::new(std::sync::Mutex::new(Vec::<AgentToolResult>::new()));
    let sink = partials.clone();
    let on_update: AgentToolUpdateCallback =
        Box::new(move |partial| sink.lock().unwrap().push(partial));
    let args = json!({
        "description": "quick read",
        "prompt": "read one file",
        "subagent_type": "helper",
        "background": false,
    });
    let tool_ctx = ctx(&cwd);
    let execute = tool.execute.clone();
    let _ = tokio::task::spawn_blocking(move || {
        execute(
            "call-announce".into(),
            args,
            None,
            Some(on_update),
            Some(&tool_ctx),
        )
    })
    .await
    .unwrap();
    let partials = partials.lock().unwrap();
    let first = partials.first().expect("a partial at dispatch");
    let run = first.details["poolTaskId"].as_str().expect("the run's id");
    assert!(!run.is_empty());
    assert_eq!(first.details["status"], "running");
    assert_eq!(
        run_for_tool_call("call-announce").as_deref(),
        Some(run),
        "the call finds its run by id"
    );
    pool.dispose();
    set_subagent_pool_for_testing(None);
    task_store().clear();
    subagent_inbox().clear();
}

#[tokio::test]
async fn the_inherited_model_fallback_switches_away_from_a_model_tier() {
    let _serial = SERIAL.lock().await;
    isolate_agent_dir();
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().canonicalize().unwrap();
    // An agent that pins a concrete model, so the fallback has something to
    // drop; the caller's model tier is what it will otherwise resolve to.
    let agents = cwd.join(DIR).join("agents");
    std::fs::create_dir_all(&agents).unwrap();
    std::fs::write(
        agents.join("pinned.md"),
        "---\nname: pinned\ndescription: Pins a model.\ntools: read\nmodel: pinned-model\n---\nBody.",
    )
    .unwrap();
    let tool = create_task_tool_definition(&cwd);
    task_store().clear();
    let parent = scoped_test_model("prov", "parent-model");
    let tier = scoped_test_model("prov", "tier-model");
    set_scoped_models_override(Some(vec![
        override_entry("prov/tier-model", Some(ModelCategoryName::Fast)),
        override_entry("prov/parent-model", None),
    ]));
    let failed = json!({
        "summary": "Task failed: This Go model requires Global regions.",
        "files_changed": [], "confidence": 0.5, "status": "failed"
    });
    let pool = SubagentPool::new(SubagentPoolOptions {
        executable: failing_then_ok_child(&cwd, &failed),
        cwd: Some(cwd.clone()),
        ..Default::default()
    });
    set_subagent_pool_for_testing(Some(pool.clone()));
    let result = execute(
        tool,
        json!({"description": "quick read", "prompt": "read one file", "subagent_type": "pinned", "model": "fast"}),
        ToolContext {
            model: Some(parent.clone()),
            available_models: vec![parent, tier],
            ..ctx(&cwd)
        },
    )
    .await;
    assert!(result.is_ok(), "{result:?}");
    // Two attempts were recorded: the asked tier (it beats the pin), then the parent's.
    let models = model_arg_history(&cwd);
    assert_eq!(models, vec!["prov/tier-model", "prov/parent-model"]);
    pool.dispose();
    set_subagent_pool_for_testing(None);
    set_scoped_models_override(None);
    task_store().clear();
}

#[test]
fn a_project_agent_opting_into_background_runs_detached() {
    isolate_agent_dir();
    let dir = tempfile::tempdir().unwrap();
    let agents = dir.path().join(DIR).join("agents");
    std::fs::create_dir_all(&agents).unwrap();
    std::fs::write(
        agents.join("watcher.md"),
        "---\nname: watcher\ndescription: A non-blocking background agent.\nbackground: true\n---\nBody.",
    )
    .unwrap();
    let tool = create_task_tool_definition(dir.path());
    let predicate = tool.background_when.clone().unwrap();
    let call = |agent: &str| AgentToolCall {
        id: "c".into(),
        name: "Agent".into(),
        arguments: json!({"subagent_type": agent}),
    };
    assert!(predicate(&call("watcher")));
    assert!(predicate(&call("explore")));
    assert!(predicate(&call("general-purpose")));
}

fn active_tool_names(with_task: bool) -> Vec<String> {
    use hoocode_code_agent_session::{
        create_agent_session, AgentSessionServices, CreateAgentSessionOptions, StaticResourceLoader,
    };
    let dir = tempfile::tempdir().unwrap();
    let model: hoocode_ai_types::Model = serde_json::from_value(json!({
        "id": "m", "name": "m", "api": "openai-completions", "provider": "prov",
        "baseUrl": "http://x", "contextWindow": 1000, "maxTokens": 100,
    }))
    .unwrap();
    let services = AgentSessionServices {
        cwd: dir.path().to_path_buf(),
        agent_dir: dir.path().to_path_buf(),
        auth: Arc::new(hoocode_code_models::NoAuth),
        settings: Arc::new(std::sync::Mutex::new(
            hoocode_code_settings::SettingsManager::in_memory(Default::default()),
        )),
        model_registry: Arc::new(hoocode_code_models::ModelRegistry::in_memory()),
        resource_loader: Arc::new(StaticResourceLoader::default()),
        diagnostics: vec![],
    };
    let created = create_agent_session(
        &services,
        hoocode_code_session::SessionManager::in_memory(dir.path().to_string_lossy()),
        CreateAgentSessionOptions {
            model: Some(model),
            // What the CLI registers: both subagent tools.
            custom_tools: if with_task {
                vec![
                    create_task_tool_definition(dir.path()),
                    create_task_output_tool_definition(),
                ]
            } else {
                vec![]
            },
            ..Default::default()
        },
    );
    let names = created.session.get_active_tool_names();
    created.session.dispose();
    names
}

#[test]
fn the_task_tool_is_active_only_when_registered() {
    isolate_agent_dir();
    let on = active_tool_names(true);
    assert!(on.contains(&"Agent".to_string()));
    assert!(on.contains(&"AgentOutput".to_string()));
    assert!(on.contains(&"Read".to_string()));
    let off = active_tool_names(false);
    assert!(!off.contains(&"Agent".to_string()));
    assert!(!off.contains(&"AgentOutput".to_string()));
    assert!(off.contains(&"Read".to_string()));
}

/// Every argument the child was spawned with, in order.
fn argv_of(dir: &Path) -> Vec<String> {
    std::fs::read(dir.join("argv.bin"))
        .unwrap()
        .split(|b| *b == 0)
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_effort_param_reaches_the_child_as_thinking() {
    let _serial = SERIAL.lock().await;
    isolate_agent_dir();
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().canonicalize().unwrap();
    let tool = create_task_tool_definition(&cwd);
    task_store().clear();
    let pool = install_pool(&cwd, argv_child(&cwd));
    execute(
        tool.clone(),
        json!({"description": "Map it", "prompt": "Map the module", "subagent_type": "general-purpose", "effort": "high"}),
        ctx(&cwd),
    )
    .await
    .unwrap();
    let argv = argv_of(&cwd);
    let at = argv
        .iter()
        .position(|a| a == "--thinking")
        .expect("--thinking is passed");
    assert_eq!(argv[at + 1], "high");
    pool.dispose();
    set_subagent_pool_for_testing(None);
    task_store().clear();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_effort_is_refused_before_anything_is_dispatched() {
    let _serial = SERIAL.lock().await;
    isolate_agent_dir();
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().canonicalize().unwrap();
    let tool = create_task_tool_definition(&cwd);
    task_store().clear();
    let pool = install_pool(&cwd, argv_child(&cwd));
    let err = execute(
        tool.clone(),
        json!({"description": "x", "prompt": "x", "subagent_type": "general-purpose", "effort": "turbo"}),
        ctx(&cwd),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("Unknown effort"), "{err}");
    assert!(!dir.path().join("argv.bin").exists(), "nothing was spawned");
    pool.dispose();
    set_subagent_pool_for_testing(None);
    task_store().clear();
}

fn scoped_test_model(provider: &str, id: &str) -> hoocode_ai_types::Model {
    serde_json::from_value(json!({
        "id": id, "name": id, "api": "openai-completions", "provider": provider,
        "baseUrl": "http://x", "reasoning": true, "contextWindow": 1000, "maxTokens": 100,
    }))
    .unwrap()
}

/// The project's `scopedModels` drive the Agent tool: a category runs on its
/// scoped model, the result names that model, and an unscoped name is refused.
#[tokio::test(flavor = "multi_thread")]
async fn a_scoped_category_runs_on_its_model_and_the_result_names_it() {
    let _serial = SERIAL.lock().await;
    isolate_agent_dir();
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().canonicalize().unwrap();
    std::fs::create_dir_all(cwd.join(DIR)).unwrap();
    std::fs::write(
        cwd.join(DIR).join("settings.json"),
        json!({"scopedModels": [
            {"model": "prov/quick-model", "category": "fast", "alias": "quick", "effort": "low"},
            {"model": "prov/big-model", "category": "capable"},
        ]})
        .to_string(),
    )
    .unwrap();
    let tool = create_task_tool_definition(&cwd);
    task_store().clear();
    let pool = install_pool(&cwd, argv_child(&cwd));
    let available = vec![
        scoped_test_model("prov", "quick-model"),
        scoped_test_model("prov", "big-model"),
        scoped_test_model("other", "unscoped"),
    ];
    let with_models = || ToolContext {
        cwd: Some(cwd.clone()),
        available_models: available.clone(),
        ..Default::default()
    };
    let result = execute(
        tool.clone(),
        json!({"description": "quick read", "prompt": "read one file", "subagent_type": "general-purpose", "model": "fast", "background": false}),
        with_models(),
    )
    .await
    .unwrap();
    let argv = argv_of(&cwd);
    let at = argv
        .iter()
        .position(|a| a == "--model")
        .expect("--model is passed");
    assert_eq!(argv[at + 1], "prov/quick-model");
    let at = argv
        .iter()
        .position(|a| a == "--thinking")
        .expect("the scoped effort is passed");
    assert_eq!(argv[at + 1], "low");
    assert!(
        text(&result).starts_with("[model: quick (prov/quick-model), effort low]"),
        "{}",
        text(&result)
    );

    let refused = execute(
        tool.clone(),
        json!({"description": "x", "prompt": "x", "subagent_type": "general-purpose", "model": "other/unscoped"}),
        with_models(),
    )
    .await
    .unwrap_err();
    assert!(
        refused.to_string().contains("no scoped model matches"),
        "{refused}"
    );
    pool.dispose();
    set_subagent_pool_for_testing(None);
    task_store().clear();
}

/// `/subagent <mode> <task>` dispatches with the session's model as
/// `inherited_model` and no ask. An agent whose frontmatter pins a model keeps
/// the pin; an agent with no pin runs on the session's model.
#[tokio::test(flavor = "multi_thread")]
async fn slash_subagent_passes_the_session_model_as_inherited_not_asked() {
    let _serial = SERIAL.lock().await;
    isolate_agent_dir();
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().canonicalize().unwrap();
    let agents = cwd.join(DIR).join("agents");
    std::fs::create_dir_all(&agents).unwrap();
    std::fs::write(
        agents.join("pinned.md"),
        "---\nname: pinned\ndescription: Pins a model.\ntools: read\nmodel: pinned-model\n---\nPinned.",
    )
    .unwrap();
    std::fs::write(
        agents.join("helper.md"),
        "---\nname: helper\ndescription: Inherits the model.\ntools: read\nmodel: inherit\n---\nHelp.",
    )
    .unwrap();
    task_store().clear();
    let pool = install_pool(&cwd, argv_child(&cwd));
    let slash = |agent: &str| DispatchOptions {
        force_agent: Some(agent.into()),
        inherited_model: Some("prov/parent-model".into()),
        ..Default::default()
    };

    pool.dispatch("do a thing", slash("pinned")).await.unwrap();
    assert_eq!(model_arg(&cwd).as_deref(), Some("pinned-model"));

    pool.dispatch("do a thing", slash("helper")).await.unwrap();
    assert_eq!(model_arg(&cwd).as_deref(), Some("prov/parent-model"));

    pool.dispose();
    set_subagent_pool_for_testing(None);
    task_store().clear();
}

// AgentOutput cancel (2026-10-10, user decision: 2h subagents + cancel).

/// A child that signals it is really running, then sits until killed.
/// The ready file proves the child process itself was spawned: the pool
/// reports `Running` while the task is still in `starting`, before any
/// child exists, and a cancel landing in that window misses and reports
/// the task already finished.
fn sleeper_child(dir: &Path) -> (PathBuf, PathBuf) {
    let path = dir.join("mock-sleeper.sh");
    let ready = dir.join("mock-sleeper.ready");
    std::fs::write(
        &path,
        format!("#!/bin/sh\ntouch \"{}\"\nsleep 30\n", ready.display()),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    (path, ready)
}

async fn wait_until_running(pool: &SubagentPool, task_id: &str) {
    for _ in 0..500 {
        if pool.get_status(task_id) == hoocode_code_subagents::pool::TaskStatus::Running {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("{task_id} never started running");
}

/// The mock sleeper touches its ready file as its first act, so this
/// proves the child holds a live slot (with a pid cancel can kill).
async fn wait_until_spawned(ready: &Path) {
    for _ in 0..1000 {
        if ready.exists() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("{} never appeared", ready.display());
}

#[test]
fn agent_output_schema_declares_cancel() {
    let tool = create_task_output_tool_definition();
    assert_eq!(
        tool.parameters["properties"]["cancel"]["description"],
        "Stop the named running subagent (kills its process); requires task_id. Use when its result is no longer needed."
    );
    assert_eq!(tool.parameters["properties"]["cancel"]["type"], "boolean");
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_output_cancel_stops_a_running_task() {
    let _serial = SERIAL.lock().await;
    isolate_agent_dir();
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().canonicalize().unwrap();
    let (sleeper, ready) = sleeper_child(&cwd);
    let pool = install_pool(&cwd, sleeper);
    let inbox = subagent_inbox();
    inbox.clear();
    let task_id = "cancel-running";
    inbox.start(task_id, "explore#1", "explore", "");
    // The dispatch future is what the Agent tool awaits; its result settles the inbox.
    let dispatch = {
        let pool = pool.clone();
        tokio::spawn(async move {
            pool.dispatch(
                "look around",
                DispatchOptions {
                    force_agent: Some("explore".into()),
                    task_id: Some(task_id.into()),
                    background: Some(true),
                    ..Default::default()
                },
            )
            .await
        })
    };
    wait_until_running(&pool, task_id).await;
    // `Running` covers the pre-spawn `starting` window, where cancel misses.
    // Only the child's own signal means a live process is waiting to be killed.
    wait_until_spawned(&ready).await;

    let tool = create_task_output_tool_definition();
    let r = execute(
        tool.clone(),
        json!({"task_id": "explore#1", "cancel": true}),
        ctx(&cwd),
    )
    .await
    .unwrap();
    assert_eq!(text(&r), "Cancelled explore#1. Only this run was stopped.");
    assert_subset(&r.details, json!({"status": "cancelled", "ok": true}));

    let result = dispatch.await.unwrap().unwrap();
    inbox.finish(task_id, &result);
    assert_eq!(
        inbox.get(task_id).unwrap().lifecycle,
        TaskLifecycle::Cancelled
    );
    assert_eq!(
        pool.get_status(task_id),
        hoocode_code_subagents::pool::TaskStatus::Cancelled
    );
    pool.dispose();
    set_subagent_pool_for_testing(None);
    inbox.clear();
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_output_cancel_on_a_finished_task_says_so() {
    let _serial = SERIAL.lock().await;
    let inbox = subagent_inbox();
    inbox.clear();
    inbox.start("t1", "explore#1", "explore", "");
    inbox.finish("t1", &ok_result("t1", "already done"));
    let tool = create_task_output_tool_definition();
    let r = execute(
        tool,
        json!({"task_id": "explore#1", "cancel": true}),
        ToolContext::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        text(&r),
        "explore#1 already finished (done); nothing was cancelled."
    );
    assert_subset(&r.details, json!({"status": "done", "ok": true}));
    // The body is untouched: a cancel on a settled task does not eat it.
    assert_eq!(inbox.get("t1").unwrap().lifecycle, TaskLifecycle::Done);
    inbox.clear();
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_output_cancel_needs_a_task_id_and_a_known_handle() {
    let _serial = SERIAL.lock().await;
    let inbox = subagent_inbox();
    inbox.clear();
    let tool = create_task_output_tool_definition();

    let err = execute(
        tool.clone(),
        json!({"cancel": true}),
        ToolContext::default(),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("needs a task_id"), "{err}");

    let err = execute(
        tool.clone(),
        json!({"task_id": "nope#9", "cancel": true}),
        ToolContext::default(),
    )
    .await
    .unwrap_err();
    assert!(
        err.to_string().contains("No background task \"nope#9\""),
        "{err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_output_cancel_cannot_combine_with_wait_or_list() {
    let _serial = SERIAL.lock().await;
    let inbox = subagent_inbox();
    inbox.clear();
    inbox.start("t1", "explore#1", "explore", "");
    let tool = create_task_output_tool_definition();

    let err = execute(
        tool.clone(),
        json!({"task_id": "explore#1", "cancel": true, "wait": true, "timeout_ms": 50}),
        ToolContext::default(),
    )
    .await
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("cannot be combined with wait or list"),
        "{err}"
    );
    // Nothing was cancelled or waited on: the record is still running.
    assert_eq!(inbox.get("t1").unwrap().lifecycle, TaskLifecycle::Running);

    let err = execute(
        tool,
        json!({"task_id": "explore#1", "cancel": true, "list": true}),
        ToolContext::default(),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("cannot be combined"), "{err}");
    inbox.clear();
}

// Resume by label, partial results and the string `background` flag. The
// mock child records its argv and, like a real run, leaves a session.jsonl
// under the `--session` path it was given.

fn session_child(dir: &Path, summary: &str, status: &str) -> PathBuf {
    let path = dir.join("mock-session.sh");
    let result =
        json!({"summary": summary, "files_changed": [], "confidence": 0.9, "status": status});
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\nprintf '%s\\0' \"$@\" > argv.bin\ntid=unknown; sess=; prev=; for a in \"$@\"; do [ \"$prev\" = \"--task-id\" ] && tid=$a; [ \"$prev\" = \"--session\" ] && sess=$a; prev=$a; done\nmkdir -p \"$(dirname \"$sess\")\"\necho '{{\"type\":\"session\"}}' >> \"$sess\"\nmkdir -p {DIR}/dispatch/$tid\nprintf '%s' '{result}' > {DIR}/dispatch/$tid/result.json\nexit 0\n"
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

/// The `resume_task_id: <label>` line a finished run ends with.
fn resume_label_in(text: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.strip_prefix("resume_task_id: "))
        .map(str::to_string)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_resumes_by_its_label_and_continues_the_original_session() {
    let _serial = SERIAL.lock().await;
    isolate_agent_dir();
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().canonicalize().unwrap();
    let tool = create_task_tool_definition(&cwd);
    task_store().clear();
    subagent_inbox().clear();
    let pool = install_pool(&cwd, session_child(&cwd, "Mapped the crate", "complete"));

    let first = execute(
        tool.clone(),
        json!({"description": "Map", "prompt": "Map the crate", "subagent_type": "general-purpose"}),
        ctx(&cwd),
    )
    .await
    .unwrap();
    let label = resume_label_in(&text(&first)).expect("a finished run names its label");
    assert!(label.starts_with("general-purpose#"), "{label}");
    let original = first.details["poolTaskId"].as_str().unwrap().to_string();
    let session = hoocode_code_paths::dispatch_task_dir(&cwd, &original).join("session.jsonl");
    assert!(session.exists());

    let resumed = execute(
        tool.clone(),
        json!({"description": "Go on", "prompt": "Go deeper", "subagent_type": "general-purpose", "resume_task_id": label}),
        ctx(&cwd),
    )
    .await
    .unwrap();
    assert!(
        text(&resumed).contains("\n\nMapped the crate\n"),
        "{}",
        text(&resumed)
    );
    assert_ne!(resumed.details["poolTaskId"], json!(original));
    let argv = argv_of(&cwd);
    assert_eq!(
        argv.iter()
            .position(|a| a == "--session")
            .map(|i| PathBuf::from(&argv[i + 1])),
        Some(session.clone()),
        "the resumed run must append to the original session"
    );
    // The resumed run's own label resumes the same session again.
    let label2 = resume_label_in(&text(&resumed)).expect("a resumed run names its label");
    assert_ne!(label2, label);
    execute(
        tool.clone(),
        json!({"description": "Again", "prompt": "Once more", "subagent_type": "general-purpose", "resume_task_id": label2}),
        ctx(&cwd),
    )
    .await
    .unwrap();
    pool.dispose();
    set_subagent_pool_for_testing(None);
    task_store().clear();
    subagent_inbox().clear();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_partial_run_says_how_to_resume_it() {
    let _serial = SERIAL.lock().await;
    isolate_agent_dir();
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().canonicalize().unwrap();
    let tool = create_task_tool_definition(&cwd);
    task_store().clear();
    subagent_inbox().clear();
    let pool = install_pool(&cwd, session_child(&cwd, "Read half the files", "partial"));

    let r = execute(
        tool.clone(),
        json!({"description": "Survey", "prompt": "Survey", "subagent_type": "general-purpose", "background": false}),
        ctx(&cwd),
    )
    .await
    .unwrap();
    let label = resume_label_in(&text(&r)).expect("a partial run names its label");
    assert!(
        text(&r).contains(&format!(
            "[Partial result. To continue this subagent, call Agent again with resume_task_id=\"{label}\".]"
        )),
        "{}",
        text(&r)
    );
    pool.dispose();
    set_subagent_pool_for_testing(None);
    task_store().clear();
    subagent_inbox().clear();
}

/// A partial run keeps its session, so `resume_task_id` continues it. Checked
/// with and without a parent session (its transcript sits under `subagents/`).
async fn a_partial_run_resumes_its_session(parent: Option<PathBuf>) {
    let _serial = SERIAL.lock().await;
    isolate_agent_dir();
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().canonicalize().unwrap();
    let tool = create_task_tool_definition(&cwd);
    task_store().clear();
    subagent_inbox().clear();
    let pool = install_pool(&cwd, session_child(&cwd, "Read half the files", "partial"));
    let parent_ctx = || ToolContext {
        session_file: parent.clone(),
        ..ctx(&cwd)
    };

    let first = execute(
        tool.clone(),
        json!({"description": "Survey", "prompt": "Survey", "subagent_type": "general-purpose", "background": false}),
        parent_ctx(),
    )
    .await
    .unwrap();
    let label = resume_label_in(&text(&first)).expect("a partial run names its label");
    let original = first.details["poolTaskId"].as_str().unwrap().to_string();
    let session = match &parent {
        Some(parent) => {
            hoocode_code_session::subagents_dir(parent).join(format!("{original}.jsonl"))
        }
        None => hoocode_code_paths::dispatch_task_dir(&cwd, &original).join("session.jsonl"),
    };
    assert!(session.exists(), "a partial run keeps its session");

    let resumed = execute(
        tool.clone(),
        json!({"description": "Go on", "prompt": "Finish it", "subagent_type": "general-purpose", "resume_task_id": label, "background": false}),
        parent_ctx(),
    )
    .await
    .unwrap();
    assert!(
        !text(&resumed).contains("No resumable session"),
        "{}",
        text(&resumed)
    );
    let argv = argv_of(&cwd);
    assert_eq!(
        argv.iter()
            .position(|a| a == "--session")
            .map(|i| PathBuf::from(&argv[i + 1])),
        Some(session),
        "the resumed run must append to the partial run's session"
    );
    pool.dispose();
    set_subagent_pool_for_testing(None);
    task_store().clear();
    subagent_inbox().clear();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_partial_run_resumes_its_session_in_the_dispatch_dir() {
    a_partial_run_resumes_its_session(None).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_partial_run_resumes_its_session_under_the_parent() {
    let dir = tempfile::tempdir().unwrap();
    // A real parent session: a header line, so the run can record itself in it.
    let parent = dir.path().join("parent.jsonl");
    let header = json!({
        "type": "session",
        "version": 3,
        "id": "parent-partial",
        "timestamp": "2026-10-15T00:00:00.000Z",
        "cwd": dir.path().to_string_lossy(),
    });
    std::fs::write(&parent, format!("{header}\n")).unwrap();
    a_partial_run_resumes_its_session(Some(parent)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_string_background_false_runs_in_the_foreground() {
    let _serial = SERIAL.lock().await;
    isolate_agent_dir();
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().canonicalize().unwrap();
    let tool = create_task_tool_definition(&cwd);
    task_store().clear();
    subagent_inbox().clear();
    // explore runs in the background by default; the quoted "false" must win.
    let pool = install_pool(
        &cwd,
        session_child(&cwd, "Found the entry point", "complete"),
    );
    let r = execute(
        tool.clone(),
        json!({"description": "Find", "prompt": "Find it", "subagent_type": "explore", "background": "false"}),
        ctx(&cwd),
    )
    .await
    .unwrap();
    assert!(text(&r).starts_with("[model: "), "{}", text(&r));
    assert!(
        text(&r).contains("\n\nFound the entry point\n"),
        "{}",
        text(&r)
    );
    assert!(r.details.get("background").is_none(), "{}", r.details);
    pool.dispose();
    set_subagent_pool_for_testing(None);
    task_store().clear();
    subagent_inbox().clear();
}

#[tokio::test(flavor = "multi_thread")]
async fn general_purpose_runs_in_the_background_when_the_call_says_nothing() {
    let _serial = SERIAL.lock().await;
    isolate_agent_dir();
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().canonicalize().unwrap();
    let tool = create_task_tool_definition(&cwd);
    task_store().clear();
    subagent_inbox().clear();
    let pool = install_pool(&cwd, mock_child(&cwd, "Read the module", "complete", 0));
    let r = execute(
        tool.clone(),
        json!({"description": "Read it", "prompt": "Read the module", "subagent_type": "general-purpose"}),
        ctx(&cwd),
    )
    .await
    .unwrap();
    // A notification, not the answer: the body waits in the inbox for AgentOutput.
    assert!(
        text(&r).starts_with("general-purpose#1 (\"Read it\") finished ✓"),
        "{}",
        text(&r)
    );
    assert_subset(&r.details, json!({"ok": true, "background": true}));
    pool.dispose();
    set_subagent_pool_for_testing(None);
    task_store().clear();
    subagent_inbox().clear();
}

// Telling parallel runs apart (2026-10-30). The user's report: the model took
// one of two subagents for a duplicate, cancelled the other one and dispatched
// it again. Every background Agent call was answered with the same
// `Started "Agent" in the background` line, labels were given out when each
// detached execute happened to start (in any order), and the roster and the
// notifications showed bare labels, so nothing told the model which
// `explore#N` was which task.

fn agent_call(id: &str, description: &str) -> AgentToolCall {
    AgentToolCall {
        id: id.into(),
        name: "Agent".into(),
        arguments: json!({
            "description": description,
            "prompt": format!("Please {description}."),
            "subagent_type": "explore",
            "background": true,
        }),
    }
}

async fn execute_call(
    tool: &ToolDefinition,
    call: &AgentToolCall,
    ctx: ToolContext,
) -> Result<AgentToolResult, String> {
    let execute = tool.execute.clone();
    let (id, args) = (call.id.clone(), call.arguments.clone());
    tokio::task::spawn_blocking(move || {
        execute(id, args, None, None, Some(&ctx)).map_err(|e| e.to_string())
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn parallel_background_calls_keep_the_handles_their_placeholders_named() {
    let _serial = SERIAL.lock().await;
    isolate_agent_dir();
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().canonicalize().unwrap();
    let tool = create_task_tool_definition(&cwd);
    task_store().clear();
    subagent_inbox().clear();
    let pool = install_pool(&cwd, mock_child(&cwd, "Audited.", "complete", 0));
    let placeholder = tool
        .background_placeholder
        .clone()
        .expect("the Agent tool names its background runs");

    // The loop answers the calls one by one, in the order the model wrote them.
    let auth = agent_call("call_0", "audit the auth module");
    let db = agent_call("call_1", "audit the db layer");
    let auth_placeholder = placeholder(&auth, None).unwrap();
    let db_placeholder = placeholder(&db, None).unwrap();
    assert!(
        auth_placeholder.starts_with(
            "explore#1 (\"audit the auth module\") started in the background. Its handle is explore#1:"
        ),
        "{auth_placeholder}"
    );
    assert!(
        db_placeholder.starts_with(
            "explore#2 (\"audit the db layer\") started in the background. Its handle is explore#2:"
        ),
        "{db_placeholder}"
    );
    assert!(db_placeholder.contains("AgentOutput(\"explore#2\", cancel: true)"));

    // The detached executes start in any order. The second call starting first
    // used to take `explore#1`, the handle the model would take for the first.
    let db_result = execute_call(&tool, &db, ctx(&cwd)).await.unwrap();
    let auth_result = execute_call(&tool, &auth, ctx(&cwd)).await.unwrap();
    assert!(
        text(&db_result).starts_with("explore#2 (\"audit the db layer\") finished ✓"),
        "{}",
        text(&db_result)
    );
    assert!(
        text(&auth_result).starts_with("explore#1 (\"audit the auth module\") finished ✓"),
        "{}",
        text(&auth_result)
    );
    let inbox = subagent_inbox();
    assert_eq!(
        inbox.get("explore#2").unwrap().description.as_deref(),
        Some("audit the db layer")
    );
    assert_eq!(
        inbox.get("explore#1").unwrap().description.as_deref(),
        Some("audit the auth module")
    );

    // A reservation is used once: a provider that numbers calls per turn sends
    // `call_0` again next turn, and that call is a new run with a new handle.
    let again = execute_call(
        &tool,
        &agent_call("call_0", "audit the auth module"),
        ctx(&cwd),
    )
    .await
    .unwrap();
    assert!(text(&again).starts_with("explore#3 "), "{}", text(&again));

    pool.dispose();
    set_subagent_pool_for_testing(None);
    task_store().clear();
    subagent_inbox().clear();
}

/// A sleeper that touches `ready-<task id>` once it runs, so a test can wait
/// for one particular run to hold a live process.
fn per_task_sleeper(dir: &Path) -> PathBuf {
    let path = dir.join("mock-task-sleeper.sh");
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\ntid=unknown; prev=; for a in \"$@\"; do [ \"$prev\" = \"--task-id\" ] && tid=$a; prev=$a; done\ntouch \"{}/ready-$tid\"\nsleep 30\n",
            dir.display()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

#[tokio::test(flavor = "multi_thread")]
async fn the_roster_and_a_cancel_name_each_run_by_handle_and_description() {
    let _serial = SERIAL.lock().await;
    isolate_agent_dir();
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().canonicalize().unwrap();
    let pool = install_pool(&cwd, per_task_sleeper(&cwd));
    let inbox = subagent_inbox();
    inbox.clear();
    let runs = [
        ("run-auth", "explore#1", "audit the auth module"),
        ("run-db", "explore#2", "audit the db layer"),
    ];
    let mut dispatches = Vec::new();
    for (task_id, label, description) in runs {
        inbox.start(task_id, label, "explore", description);
        let pool = pool.clone();
        dispatches.push(tokio::spawn(async move {
            pool.dispatch(
                "audit",
                DispatchOptions {
                    force_agent: Some("explore".into()),
                    task_id: Some(task_id.into()),
                    background: Some(true),
                    ..Default::default()
                },
            )
            .await
        }));
    }
    for (task_id, _, _) in runs {
        wait_until_running(&pool, task_id).await;
        wait_until_spawned(&cwd.join(format!("ready-{task_id}"))).await;
    }

    let output = create_task_output_tool_definition();
    let roster = execute(output.clone(), json!({"list": true}), ctx(&cwd))
        .await
        .unwrap();
    let roster = text(&roster);
    for line in [
        "- explore#1 (\"audit the auth module\")  running",
        "- explore#2 (\"audit the db layer\")  running",
    ] {
        assert!(roster.contains(line), "{line} in {roster}");
    }

    // The cancel says exactly which run it stopped, and stops only that one.
    let r = execute(
        output.clone(),
        json!({"task_id": "explore#2", "cancel": true}),
        ctx(&cwd),
    )
    .await
    .unwrap();
    assert_eq!(
        text(&r),
        "Cancelled explore#2 (\"audit the db layer\"). Only this run was stopped."
    );
    let db = dispatches.pop().unwrap().await.unwrap().unwrap();
    inbox.finish("run-db", &db);
    assert_eq!(
        pool.get_status("run-auth"),
        hoocode_code_subagents::pool::TaskStatus::Running
    );
    let status = execute(output.clone(), json!({"task_id": "explore#1"}), ctx(&cwd))
        .await
        .unwrap();
    assert!(
        text(&status).starts_with("explore#1 (\"audit the auth module\") is still running"),
        "{}",
        text(&status)
    );
    let roster = execute(output, json!({"list": true}), ctx(&cwd))
        .await
        .unwrap();
    assert!(
        text(&roster).contains("- explore#2 (\"audit the db layer\")  cancelled ⊘"),
        "{}",
        text(&roster)
    );

    pool.dispose();
    set_subagent_pool_for_testing(None);
    inbox.clear();
}

/// The moments between a call's placeholder and its execute (2026-10-30
/// review): the run is listed, and a cancel stops it before it starts.
#[tokio::test(flavor = "multi_thread")]
async fn a_run_is_listed_and_cancellable_from_its_placeholder_on() {
    let _serial = SERIAL.lock().await;
    isolate_agent_dir();
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().canonicalize().unwrap();
    let tool = create_task_tool_definition(&cwd);
    task_store().clear();
    subagent_inbox().clear();
    let pool = install_pool(&cwd, mock_child(&cwd, "Audited.", "complete", 0));
    let placeholder = tool.background_placeholder.clone().unwrap();
    let auth = agent_call("call_0", "audit the auth module");
    let db = agent_call("call_1", "audit the db layer");
    placeholder(&auth, None).unwrap();
    placeholder(&db, None).unwrap();

    // Neither execute has started: the roster still lists both.
    let output = create_task_output_tool_definition();
    let roster = execute(output.clone(), json!({"list": true}), ctx(&cwd))
        .await
        .unwrap();
    let roster = text(&roster);
    assert!(
        roster.starts_with("2 background subagents (2 running):"),
        "{roster}"
    );
    assert!(
        roster.contains("- explore#2 (\"audit the db layer\")  running"),
        "{roster}"
    );
    let status = execute(output.clone(), json!({"task_id": "explore#1"}), ctx(&cwd))
        .await
        .unwrap();
    assert!(
        text(&status).contains("is still running"),
        "{}",
        text(&status)
    );

    // Cancelled while starting: it is never dispatched.
    let r = execute(
        output.clone(),
        json!({"task_id": "explore#2", "cancel": true}),
        ctx(&cwd),
    )
    .await
    .unwrap();
    assert_eq!(
        text(&r),
        "Cancelled explore#2 (\"audit the db layer\") before it started. Only this run was stopped."
    );
    let db_result = execute_call(&tool, &db, ctx(&cwd)).await.unwrap();
    assert!(
        text(&db_result).starts_with("explore#2 (\"audit the db layer\") cancelled ⊘"),
        "{}",
        text(&db_result)
    );
    assert!(
        !cwd.join(DIR).join("dispatch").exists()
            || std::fs::read_dir(cwd.join(DIR).join("dispatch"))
                .unwrap()
                .count()
                == 0,
        "the cancelled run was never dispatched"
    );
    let auth_result = execute_call(&tool, &auth, ctx(&cwd)).await.unwrap();
    assert!(
        text(&auth_result).starts_with("explore#1 (\"audit the auth module\") finished ✓"),
        "{}",
        text(&auth_result)
    );
    pool.dispose();
    set_subagent_pool_for_testing(None);
    task_store().clear();
    subagent_inbox().clear();
}

/// A call the execute refuses after its placeholder named a handle settles
/// that handle `failed` with the reason, and the reason names the run.
#[tokio::test(flavor = "multi_thread")]
async fn a_refused_call_fails_the_handle_its_placeholder_named() {
    let _serial = SERIAL.lock().await;
    isolate_agent_dir();
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().canonicalize().unwrap();
    let tool = create_task_tool_definition(&cwd);
    task_store().clear();
    subagent_inbox().clear();
    let pool = install_pool(&cwd, mock_child(&cwd, "unused", "complete", 0));
    let mut call = agent_call("call_0", "find the bug");
    call.arguments["subagent_type"] = json!("nope");
    let said = (tool.background_placeholder.clone().unwrap())(&call, None).unwrap();
    assert!(
        said.starts_with("nope#1 (\"find the bug\") started"),
        "{said}"
    );
    let err = execute_call(&tool, &call, ctx(&cwd)).await.unwrap_err();
    assert!(
        err.starts_with("nope#1 (\"find the bug\") failed ✗ — Unknown subagent_type: \"nope\""),
        "{err}"
    );
    let output = create_task_output_tool_definition();
    let status = execute(output, json!({"task_id": "nope#1"}), ctx(&cwd))
        .await
        .unwrap();
    assert!(
        text(&status).starts_with("nope#1 (\"find the bug\") failed ✗ — Unknown subagent_type"),
        "{}",
        text(&status)
    );
    assert_subset(&status.details, json!({"status": "failed"}));
    pool.dispose();
    set_subagent_pool_for_testing(None);
    task_store().clear();
    subagent_inbox().clear();
}

/// A cancel asked for after the execute bound the run but before the pool
/// holds it (where `pool.cancel` misses) still stops the run once it lands.
#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_asked_before_the_pool_holds_the_run_stops_it() {
    let _serial = SERIAL.lock().await;
    isolate_agent_dir();
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().canonicalize().unwrap();
    let pool = install_pool(&cwd, per_task_sleeper(&cwd));
    let inbox = subagent_inbox();
    inbox.clear();
    inbox.start("late-run", "explore#1", "explore", "audit");
    // The cancel lands after the execute began awaiting the run, and before
    // the pool holds it (the dispatch registers it 80 ms in).
    let asker = tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        subagent_inbox().request_cancel(None, "explore#1")
    });
    let dispatch = {
        let pool = pool.clone();
        async move {
            tokio::time::sleep(std::time::Duration::from_millis(80)).await;
            pool.dispatch(
                "audit",
                DispatchOptions {
                    force_agent: Some("explore".into()),
                    task_id: Some("late-run".into()),
                    background: Some(true),
                    ..Default::default()
                },
            )
            .await
        }
    };
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        cancel_on_abort(&pool, "late-run", None, dispatch),
    )
    .await
    .expect("the run was stopped, not left to sleep")
    .unwrap();
    assert!(matches!(
        asker.await.unwrap(),
        Some(hoocode_code_subagents::inbox::CancelAsk::Running(_))
    ));
    assert_eq!(
        result.result.and_then(|r| r.status),
        Some(ResultStatus::Cancelled)
    );
    pool.dispose();
    set_subagent_pool_for_testing(None);
    inbox.clear();
}

/// After `--resume` in a new process: an old handle says it belongs to an
/// earlier process, a new run does not reuse it, and another session's runs
/// are not listed.
#[tokio::test(flavor = "multi_thread")]
async fn an_old_handle_after_resume_is_named_as_an_earlier_run() {
    let _serial = SERIAL.lock().await;
    isolate_agent_dir();
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().canonicalize().unwrap();
    let pool = install_pool(&cwd, mock_child(&cwd, "unused", "complete", 0));
    let inbox = subagent_inbox();
    inbox.clear();
    let session_file = cwd.join("resumed.jsonl");
    std::fs::write(
        &session_file,
        "{\"content\":\"explore#1 (\\\"audit\\\") finished\"}\n",
    )
    .unwrap();
    let resumed = ToolContext {
        session_file: Some(session_file.clone()),
        ..ctx(&cwd)
    };
    inbox.start_in(
        None,
        "other-session-run",
        "explore#7",
        "explore",
        "elsewhere",
    );

    let output = create_task_output_tool_definition();
    let status = execute(
        output.clone(),
        json!({"task_id": "explore#1"}),
        resumed.clone(),
    )
    .await
    .unwrap();
    assert!(
        text(&status).starts_with("explore#1 ran in an earlier process and is not running now."),
        "{}",
        text(&status)
    );
    let err = execute(
        output.clone(),
        json!({"task_id": "explore#1", "cancel": true}),
        resumed.clone(),
    )
    .await
    .unwrap_err();
    assert!(
        err.starts_with("explore#1 ran in an earlier process"),
        "{err}"
    );
    // A run of this process held for another session (e.g. before a /fork).
    let other = execute(
        output.clone(),
        json!({"task_id": "explore#7"}),
        resumed.clone(),
    )
    .await
    .unwrap();
    assert!(
        text(&other).starts_with("explore#7 belongs to another session or an earlier process"),
        "{}",
        text(&other)
    );
    let roster = execute(output, json!({"list": true}), resumed.clone())
        .await
        .unwrap();
    assert_eq!(
        text(&roster),
        "No background subagents have been dispatched."
    );

    let tool = create_task_tool_definition(&cwd);
    let said = (tool.background_placeholder.clone().unwrap())(
        &agent_call("call_0", "audit again"),
        Some(&resumed),
    )
    .unwrap();
    assert!(said.starts_with("explore#2 (\"audit again\")"), "{said}");
    pool.dispose();
    set_subagent_pool_for_testing(None);
    inbox.clear();
}

/// A run asked to stop before its execute awaits it is not dispatched at all,
/// and an abort before the pool holds the run is retried like a cancel.
#[tokio::test(flavor = "multi_thread")]
async fn a_stop_asked_before_the_dispatch_never_starts_it() {
    let _serial = SERIAL.lock().await;
    isolate_agent_dir();
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().canonicalize().unwrap();
    let pool = install_pool(&cwd, per_task_sleeper(&cwd));
    let inbox = subagent_inbox();
    inbox.clear();
    inbox.start("early-run", "explore#1", "explore", "audit");
    inbox.request_cancel(None, "explore#1");
    let polled = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let seen = polled.clone();
    let result = cancel_on_abort(&pool, "early-run", None, async move {
        seen.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(TaskResult::default())
    })
    .await
    .unwrap();
    assert!(
        !polled.load(std::sync::atomic::Ordering::SeqCst),
        "never started"
    );
    assert_eq!(
        result.result.and_then(|r| r.status),
        Some(ResultStatus::Cancelled)
    );

    // An abort landing before the pool holds the run.
    let signal = hoocode_ai_types::AbortSignal::new();
    let aborter = signal.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        aborter.abort();
    });
    let dispatch = {
        let pool = pool.clone();
        async move {
            tokio::time::sleep(std::time::Duration::from_millis(80)).await;
            pool.dispatch(
                "audit",
                DispatchOptions {
                    force_agent: Some("explore".into()),
                    task_id: Some("aborted-run".into()),
                    background: Some(true),
                    ..Default::default()
                },
            )
            .await
        }
    };
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        cancel_on_abort(&pool, "aborted-run", Some(signal), dispatch),
    )
    .await
    .expect("the aborted run was stopped once the pool held it")
    .unwrap();
    assert_eq!(
        result.result.and_then(|r| r.status),
        Some(ResultStatus::Cancelled)
    );
    pool.dispose();
    set_subagent_pool_for_testing(None);
    inbox.clear();
}

/// The execute sees other arguments than the placeholder did (validated or
/// rewritten): it still runs under the handle the model was told, and no
/// second, never-started run is left listed as running.
#[tokio::test(flavor = "multi_thread")]
async fn changed_arguments_keep_the_reserved_handle_and_leave_no_ghost() {
    let _serial = SERIAL.lock().await;
    isolate_agent_dir();
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().canonicalize().unwrap();
    let tool = create_task_tool_definition(&cwd);
    task_store().clear();
    subagent_inbox().clear();
    let pool = install_pool(&cwd, mock_child(&cwd, "Audited.", "complete", 0));
    let told = agent_call("call_0", "audit the auth module");
    (tool.background_placeholder.clone().unwrap())(&told, None).unwrap();
    let mut ran = told.clone();
    ran.arguments["description"] = json!("audit the auth module (rewritten)");
    let result = execute_call(&tool, &ran, ctx(&cwd)).await.unwrap();
    assert!(
        text(&result).starts_with("explore#1 (\"audit the auth module (rewritten)\") finished ✓"),
        "{}",
        text(&result)
    );
    let all = subagent_inbox().list();
    assert_eq!(all.len(), 1, "{all:?}");
    assert!(subagent_inbox().outstanding().is_empty());
    pool.dispose();
    set_subagent_pool_for_testing(None);
    task_store().clear();
    subagent_inbox().clear();
}

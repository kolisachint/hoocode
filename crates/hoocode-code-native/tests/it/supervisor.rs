//! Tier C supervisor tests: start, ready, calls, restarts and shutdown.
//!
//! The helper under test is the echo fixture example in `hoocode-agent-mcp`
//! (`cargo build -p hoocode-agent-mcp --example echo_helper` in test setup;
//! see `common`). Every supervisor here owns a fresh temp home and temp
//! runtime dir, and is shut down at the end of its test.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use hoocode_agent_mcp::{McpError, ToolContent};
use hoocode_code_native::{
    binary_path_overrides, mint_agent, resolve_executable, NativeClient, NativeError, Supervisor,
    SupervisorConfig,
};
use serde_json::{json, Value};

use crate::common::{echo_entry, echo_text, fixture_bin, start_supervisor, TestHelper};

/// Echo through the root client: the text plus the caller identity.
#[tokio::test]
async fn root_echo_returns_root_identity() {
    let helper = TestHelper::start();
    let root = helper.supervisor().root_identity().clone();
    let output = helper
        .supervisor()
        .call_tool("echo", "echo", json!({"text": "hi"}))
        .await
        .expect("echo works");
    let seen = echo_text(&output);
    assert_eq!(seen["text"], "hi");
    assert_eq!(seen["agentId"], "root");
    assert_eq!(seen["parentId"], Value::Null);
    assert_eq!(seen["depth"], 0);
    assert_eq!(seen["metaAgentId"], root.agent_id);
    helper.supervisor().shutdown().await.expect("shutdown");
}

/// Three children plus root, all calling at once: each sees its own identity.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn children_see_their_own_identity() {
    let helper = TestHelper::start();
    let supervisor = helper.supervisor().clone();
    let mut clients = Vec::new();
    for task in ["one", "two", "three"] {
        let identity = supervisor.mint_agent("root", task).expect("mint");
        supervisor
            .register_child("echo", &identity)
            .await
            .expect("register");
        clients.push(
            NativeClient::connect(
                supervisor.ensure("echo").await.expect("socket"),
                "echo",
                &identity,
            )
            .await
            .expect("connect"),
        );
    }
    let mut set = tokio::task::JoinSet::new();
    set.spawn({
        let supervisor = supervisor.clone();
        async move {
            (
                "root".to_owned(),
                supervisor
                    .call_tool("echo", "echo", json!({"text": "root"}))
                    .await,
            )
        }
    });
    for (index, client) in clients.into_iter().enumerate() {
        set.spawn(async move {
            (
                format!("root/{}", ["one", "two", "three"][index]),
                client
                    .call_tool("echo", json!({"text": format!("child-{index}")}))
                    .await,
            )
        });
    }
    while let Some(joined) = set.join_next().await {
        let (agent_id, result) = joined.expect("task");
        let seen = echo_text(&result.expect("echo"));
        assert_eq!(seen["agentId"], agent_id);
        assert_eq!(seen["metaAgentId"], agent_id);
        if agent_id == "root" {
            assert_eq!(seen["parentId"], Value::Null);
            assert_eq!(seen["depth"], 0);
        } else {
            assert_eq!(seen["parentId"], "root");
            assert_eq!(seen["depth"], 1);
        }
    }
    helper.supervisor().shutdown().await.expect("shutdown");
}

/// A token the helper never saw is rejected with 401 (`Unauthorized`).
#[tokio::test]
async fn unknown_token_is_rejected() {
    let helper = TestHelper::start();
    let socket = helper.supervisor().ensure("echo").await.expect("socket");
    let stranger = hoocode_code_native::AgentIdentity {
        agent_id: "intruder".to_owned(),
        parent_id: None,
        depth: 0,
        token: hoocode_code_native::new_token(),
    };
    let error = NativeClient::connect(socket, "echo", &stranger)
        .await
        .expect_err("unknown token is rejected");
    assert!(
        matches!(error, NativeError::Mcp(McpError::Unauthorized(_))),
        "{error:?}"
    );
    helper.supervisor().shutdown().await.expect("shutdown");
}

/// `agentStarted` with a parent that is not the sender's id is rejected
/// helper-side: the token never becomes usable.
#[tokio::test]
async fn agent_started_with_wrong_parent_is_rejected() {
    let helper = TestHelper::start();
    let supervisor = helper.supervisor();
    let mut wrong = mint_agent(supervisor.root_identity(), "sneaky");
    wrong.parent_id = Some("someone-else".to_owned());
    supervisor
        .register_child("echo", &wrong)
        .await
        .expect("the notification itself is fire-and-forget");
    let socket = supervisor.ensure("echo").await.expect("socket");
    let error = NativeClient::connect(socket, "echo", &wrong)
        .await
        .expect_err("rejected token stays unknown");
    assert!(
        matches!(error, NativeError::Mcp(McpError::Unauthorized(_))),
        "{error:?}"
    );
    helper.supervisor().shutdown().await.expect("shutdown");
}

/// `crash` kills the helper mid-call: the next call restarts it, and the
/// registered child is re-sent (its token works again).
#[tokio::test]
async fn crash_restarts_and_reregisters_children() {
    let helper = TestHelper::start();
    let supervisor = helper.supervisor();
    let child = supervisor.mint_agent("root", "task").expect("mint");
    supervisor
        .register_child("echo", &child)
        .await
        .expect("register");
    let before = supervisor.spawn_count("echo");
    let error = supervisor
        .call_tool("echo", "crash", json!({}))
        .await
        .expect_err("crash kills the in-flight call");
    assert!(!matches!(error, NativeError::Down { .. }), "{error:?}");
    let output = supervisor
        .call_tool("echo", "echo", json!({"text": "after"}))
        .await
        .expect("the next call restarts");
    assert_eq!(echo_text(&output)["text"], "after");
    assert_eq!(supervisor.spawn_count("echo"), before + 2);
    // The child was re-registered by the restart: its token still works.
    let socket = supervisor.ensure("echo").await.expect("socket");
    let client = NativeClient::connect(socket, "echo", &child)
        .await
        .expect("child token works after restart");
    let seen = echo_text(
        &client
            .call_tool("echo", json!({"text": "child"}))
            .await
            .expect("child echo"),
    );
    assert_eq!(seen["agentId"], "root/task");
    assert_eq!(seen["parentId"], "root");
    helper.supervisor().shutdown().await.expect("shutdown");
}

/// A helper that always crashes exhausts its restart budget and stays down.
#[tokio::test]
async fn restart_budget_goes_down() {
    let helper = TestHelper::start_with(SupervisorConfig {
        max_restarts: 2,
        restart_window: Duration::from_secs(60),
        ..SupervisorConfig::for_tests()
    });
    let supervisor = helper.supervisor();
    // Each crash call burns restarts (the call plus its one retry). After the
    // budget is spent every call reports the helper as down.
    let mut down = None;
    for _ in 0..4 {
        match supervisor.call_tool("echo", "crash", json!({})).await {
            Err(NativeError::Down { plugin, reason }) => {
                down = Some((plugin, reason));
                break;
            }
            Err(_) => {}
            Ok(_) => panic!("crash never answers"),
        }
    }
    let (plugin, reason) = down.expect("the helper goes down");
    assert_eq!(plugin, "echo");
    assert!(
        reason.contains("restarted"),
        "the reason names the budget: {reason}"
    );
    let error = supervisor
        .call_tool("echo", "echo", json!({"text": "hi"}))
        .await
        .expect_err("down stays down");
    assert!(matches!(error, NativeError::Down { .. }), "{error:?}");
    assert!(
        format!("{error}").contains("echo helper is down"),
        "{error}"
    );
    helper.supervisor().shutdown().await.expect("shutdown");
}

/// The ready timeout fails cleanly when the process never serves: `/bin/sleep`
/// starts fine but creates no socket.
#[tokio::test]
async fn ready_timeout_errors_cleanly() {
    let dir = tempfile::tempdir().expect("tempdir");
    let entry = crate::common::sleep_entry(dir.path());
    let supervisor = Supervisor::new(
        dir.path().join("home"),
        vec![entry],
        HashMap::new(),
        SupervisorConfig {
            ready_timeout: Duration::from_millis(300),
            ..SupervisorConfig::for_tests()
        },
    )
    .expect("supervisor");
    let error = supervisor.ensure("sleep").await.expect_err("no socket");
    assert!(
        matches!(
            error,
            NativeError::Timeout { ref plugin, .. } if plugin == "sleep"
        ),
        "{error:?}"
    );
    assert_eq!(supervisor.spawn_count("sleep"), 1);
    supervisor.shutdown().await.expect("shutdown");
}

/// Shutdown sends `sessionEnding` (the fixture writes a receipt marker) and
/// removes the runtime dir. Idempotent; calls after it fail.
#[tokio::test]
async fn shutdown_sends_session_ending_and_removes_the_dir() {
    let helper = TestHelper::start();
    let supervisor = helper.supervisor().clone();
    supervisor.ensure("echo").await.expect("start");
    let runtime = supervisor.runtime_dir().to_path_buf();
    assert!(runtime.exists());
    supervisor.shutdown().await.expect("shutdown");
    assert!(!runtime.exists(), "the runtime dir is removed");
    assert!(
        helper
            .receipt_dir()
            .join("session-ending.received")
            .exists(),
        "the helper saw sessionEnding"
    );
    supervisor.shutdown().await.expect("shutdown again");
    let error = supervisor.ensure("echo").await.expect_err("shut down");
    assert!(matches!(error, NativeError::Down { .. }), "{error:?}");
}

/// A missing executable fails fast with a spawn error, not a timeout.
#[tokio::test]
async fn missing_binary_fails_fast() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut entry = crate::common::sleep_entry(dir.path());
    entry.helper.launch.command = "/nonexistent/hoo-helper".to_owned();
    entry.plugin = "missing".to_owned();
    let supervisor = Supervisor::new(
        dir.path().join("home"),
        vec![entry],
        HashMap::new(),
        SupervisorConfig::for_tests(),
    )
    .expect("supervisor");
    let error = supervisor.ensure("missing").await.expect_err("no binary");
    assert!(matches!(error, NativeError::Spawn(_)), "{error:?}");
    supervisor.shutdown().await.expect("shutdown");
}

/// `binaryPath` wins over the manifest command: the launch command here does
/// not exist, so only the override can start the helper.
#[tokio::test]
async fn binary_path_override_is_used() {
    let helper = TestHelper::start();
    let fixture = fixture_bin();
    let mut entry = TestHelper::entry(helper.dir(), &fixture);
    entry.plugin = "via-setting".to_owned();
    entry.helper.launch.command = "/nonexistent/hoo-helper".to_owned();
    let supervisor = Supervisor::new(
        helper.dir().join("home"),
        vec![entry],
        HashMap::from([("via-setting".to_owned(), fixture)]),
        SupervisorConfig::for_tests(),
    )
    .expect("supervisor");
    let output = supervisor
        .call_tool("via-setting", "echo", json!({"text": "override"}))
        .await
        .expect("override starts the helper");
    assert_eq!(echo_text(&output)["text"], "override");
    supervisor.shutdown().await.expect("shutdown");
}

/// A relative `binaryPath` is ignored (with a warning) and resolution falls
/// through to the manifest command.
#[tokio::test]
async fn relative_binary_path_is_ignored() {
    let settings = serde_json::json!({
        "nativeHelpers": {"echo": {"binaryPath": "relative/path"}}
    });
    let (overrides, warnings) = binary_path_overrides(&settings);
    assert!(overrides.is_empty());
    assert_eq!(warnings.len(), 1);
    let dir = tempfile::tempdir().expect("tempdir");
    let entry = echo_entry(dir.path(), "echo", &fixture_bin());
    let path = resolve_executable(dir.path(), &entry, &overrides, &reqwest::Client::new())
        .await
        .expect("falls through to the command");
    assert_eq!(path, fixture_bin());
}

/// Unknown plugins fail with `UnknownPlugin`, before any spawn.
#[tokio::test]
async fn unknown_plugin_fails() {
    let helper = TestHelper::start();
    let error = helper
        .supervisor()
        .ensure("nope")
        .await
        .expect_err("unknown plugin");
    assert!(matches!(error, NativeError::UnknownPlugin(_)), "{error:?}");
    helper.supervisor().shutdown().await.expect("shutdown");
}

/// No helper starts before first use: no socket, no process, no log.
#[tokio::test]
async fn nothing_starts_before_first_use() {
    let helper = TestHelper::start();
    let supervisor = helper.supervisor();
    assert_eq!(supervisor.spawn_count("echo"), 0);
    let entries: Vec<PathBuf> = std::fs::read_dir(supervisor.runtime_dir())
        .expect("runtime dir exists")
        .map(|entry| entry.expect("entry").path())
        .collect();
    assert!(entries.is_empty(), "{entries:?}");
    supervisor.ensure("echo").await.expect("start");
    assert_eq!(supervisor.spawn_count("echo"), 1);
    helper.supervisor().shutdown().await.expect("shutdown");
}

/// Ten concurrent first calls start exactly one process.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_first_calls_start_one_process() {
    let helper = TestHelper::start();
    let supervisor = helper.supervisor().clone();
    let mut set = tokio::task::JoinSet::new();
    for index in 0..10 {
        let supervisor = supervisor.clone();
        set.spawn(async move {
            supervisor
                .call_tool("echo", "echo", json!({"text": format!("call-{index}")}))
                .await
        });
    }
    while let Some(joined) = set.join_next().await {
        let output = joined.expect("task").expect("echo");
        assert!(!output.is_error);
    }
    assert_eq!(supervisor.spawn_count("echo"), 1);
    helper.supervisor().shutdown().await.expect("shutdown");
}

/// The helper's stderr lands in the plugin log (the fixture logs a startup line).
#[tokio::test]
async fn helper_output_goes_to_the_log() {
    let helper = TestHelper::start();
    let supervisor = helper.supervisor();
    supervisor.ensure("echo").await.expect("start");
    let log = supervisor.runtime_dir().join("echo.log");
    let text = std::fs::read_to_string(&log).expect("log");
    assert!(text.contains("echo_helper: listening"), "{text}");
    helper.supervisor().shutdown().await.expect("shutdown");
}

/// `end_agent` notifies every helper the agent registered with, then forgets
/// it: a later restart does not re-send it.
#[tokio::test]
async fn end_agent_forgets_the_agent() {
    let helper = TestHelper::start();
    let supervisor = helper.supervisor();
    let child = supervisor.mint_agent("root", "short").expect("mint");
    supervisor
        .register_child("echo", &child)
        .await
        .expect("register");
    supervisor.end_agent(&child.agent_id).await.expect("end");
    // Crash the helper: the restart must not re-send the ended agent.
    supervisor
        .call_tool("echo", "crash", json!({}))
        .await
        .expect_err("crash");
    supervisor
        .call_tool("echo", "echo", json!({"text": "back"}))
        .await
        .expect("restart");
    let socket = supervisor.ensure("echo").await.expect("socket");
    let error = NativeClient::connect(socket, "echo", &child)
        .await
        .expect_err("ended agent stays gone");
    assert!(
        matches!(error, NativeError::Mcp(McpError::Unauthorized(_))),
        "{error:?}"
    );
    helper.supervisor().shutdown().await.expect("shutdown");
}

/// `start_supervisor` builds a supervisor from parts.
#[tokio::test]
async fn parts_build_a_working_supervisor() {
    let dir = tempfile::tempdir().expect("tempdir");
    let supervisor = start_supervisor(dir.path(), SupervisorConfig::for_tests());
    let output = supervisor
        .call_tool("echo", "echo", json!({"text": "parts"}))
        .await
        .expect("echo");
    assert_eq!(echo_text(&output)["text"], "parts");
    supervisor.shutdown().await.expect("shutdown");
}

/// `sleep {ms}` waits before answering (used to hold calls open).
#[tokio::test]
async fn sleep_waits_then_answers() {
    let helper = TestHelper::start();
    let output = helper
        .supervisor()
        .call_tool("echo", "sleep", json!({"ms": 200}))
        .await
        .expect("sleep");
    let [ToolContent::Text(text), ..] = &output.content[..] else {
        panic!("sleep answers text");
    };
    let seen: Value = serde_json::from_str(text).expect("JSON");
    assert_eq!(seen["sleptMs"], 200);
    assert_eq!(seen["agentId"], "root");
    helper.supervisor().shutdown().await.expect("shutdown");
}

/// `has_shared_helpers` gates the child env: only a shared helper means
/// subagents get agent identities.
#[test]
fn shared_helpers_gate_the_child_env() {
    use crate::common::echo_entry;
    let dir = tempfile::tempdir().expect("tempdir");
    let exe = crate::common::fixture_bin();
    let mut unshared = echo_entry(dir.path(), "echo", &exe);
    unshared.helper.spec.shared = false;
    let supervisor = Supervisor::new(
        dir.path().join("home"),
        vec![unshared],
        HashMap::new(),
        SupervisorConfig::for_tests(),
    )
    .expect("supervisor");
    assert!(!supervisor.has_shared_helpers());
    assert!(supervisor.has_helpers());
    let shared = echo_entry(dir.path(), "echo", &exe);
    assert!(shared.helper.spec.shared);
    let supervisor = Supervisor::new(
        dir.path().join("home"),
        vec![shared],
        HashMap::new(),
        SupervisorConfig::for_tests(),
    )
    .expect("supervisor");
    assert!(supervisor.has_shared_helpers());
    let _ = std::fs::remove_dir_all(supervisor.runtime_dir());
}

/// The helper exits when its parent pid is gone: spawn it with a fake
/// short-lived parent, reap that parent, and the helper must follow.
#[tokio::test]
async fn helper_exits_when_its_parent_pid_is_gone() {
    let dir = tempfile::tempdir().expect("tempdir");
    let socket = dir.path().join("helper.sock");
    let mut parent = tokio::process::Command::new("/bin/sleep")
        .arg("0.2")
        .spawn()
        .expect("sleep");
    let pid = parent.id().expect("pid");
    let mut helper = tokio::process::Command::new(fixture_bin())
        .arg("--parent-pid")
        .arg(pid.to_string())
        .env("HOOCODE_HELPER_SOCKET", &socket)
        .env("HOOCODE_HELPER_TOKEN", "token")
        .env("HOOCODE_AGENT_ID", "root")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("helper");
    // Reap the parent: the pid is really gone (a zombie would still answer kill).
    let _ = parent.wait().await;
    let status = tokio::time::timeout(Duration::from_secs(10), helper.wait())
        .await
        .expect("the helper exits when its parent is gone");
    assert!(status.expect("exit status").success());
}

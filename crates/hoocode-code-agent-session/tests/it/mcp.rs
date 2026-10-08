#![allow(clippy::disallowed_methods)] // test code: #[tokio::test] expands to a runtime builder
//! MCP servers in a session (`docs/design/mcp.md`): a model tool call to `mcp_<server>_echo`
//! round-trips through the agent, and untrusted project servers never start.
//!
//! The server is a small stdio MCP server in Python, so the test needs `python3`. Without it the
//! round-trip test is skipped.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::common::{Harness, HarnessOptions};
use hoocode_ai_provider_faux::{
    faux_assistant_message, faux_tool_call, FauxMessageOptions, FauxResponseStep,
};
use hoocode_ai_types::{Content, StopReason};
use hoocode_code_agent_session::mcp::{McpHub, McpServerInfo, McpStatus};
use hoocode_code_agent_session::PromptOptions;
use hoocode_code_mcp::ConfigSources;

/// Answers `initialize`, `tools/list` (one tool, `echo`) and `tools/call`.
const ECHO_SERVER: &str = r##"
import json, sys
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    msg = json.loads(line)
    if "id" not in msg:
        continue
    method = msg.get("method")
    if method == "initialize":
        result = {
            "protocolVersion": msg["params"]["protocolVersion"],
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "echo", "version": "1"},
        }
    elif method == "tools/list":
        result = {"tools": [{
            "name": "echo",
            "description": "Echo the text back",
            "inputSchema": {
                "type": "object",
                "properties": {"text": {"type": "string"}},
                "required": ["text"],
            },
        }]}
    elif method == "tools/call":
        args = msg["params"].get("arguments") or {}
        result = {"content": [{"type": "text", "text": "echo:" + args.get("text", "")}], "isError": False}
    else:
        result = {}
    print(json.dumps({"jsonrpc": "2.0", "id": msg["id"], "result": result}), flush=True)
"##;

fn python3_available() -> bool {
    std::process::Command::new("python3")
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success())
}

fn write_json(path: &Path, value: serde_json::Value) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, value.to_string()).unwrap();
}

fn echo_server_entry(script: &Path) -> serde_json::Value {
    serde_json::json!({"command": "python3", "args": [script.to_string_lossy()]})
}

fn tool_use(calls: Vec<Content>) -> FauxResponseStep {
    faux_assistant_message(
        calls,
        FauxMessageOptions {
            stop_reason: Some(StopReason::ToolUse),
            ..Default::default()
        },
    )
    .into()
}

fn text(t: &str) -> FauxResponseStep {
    faux_assistant_message(t, Default::default()).into()
}

/// Polls the hub until `done` holds, or fails after 20 seconds.
async fn wait_until(hub: &McpHub, done: impl Fn(&[McpServerInfo]) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let servers = hub.servers();
        if done(&servers) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "servers did not reach the expected state: {servers:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn is_connected(servers: &[McpServerInfo], name: &str) -> bool {
    servers
        .iter()
        .any(|s| s.name == name && s.status == McpStatus::Connected)
}

#[tokio::test(flavor = "multi_thread")]
async fn model_tool_call_reaches_the_mcp_server_and_back() {
    if !python3_available() {
        eprintln!("skipped: python3 is not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("echo_server.py");
    std::fs::write(&script, ECHO_SERVER).unwrap();
    let user = dir.path().join("home/.agents/mcp.json");
    write_json(
        &user,
        serde_json::json!({"mcpServers": {"x": echo_server_entry(&script)}}),
    );
    let sources = ConfigSources {
        user,
        project_folder: dir.path().join("repo"),
        plugins: Vec::new(),
    };
    let hub = Arc::new(McpHub::load(
        &sources,
        dir.path().join("trust/mcp-trust.json"),
    ));
    hub.start();

    let h = Harness::new(HarnessOptions::default());
    h.session.attach_mcp(hub.clone(), true);
    wait_until(&hub, |s| is_connected(s, "x")).await;

    h.set_responses(vec![
        tool_use(vec![faux_tool_call(
            "mcp_x_echo",
            serde_json::json!({"text": "hi"}),
            None,
        )]),
        text("done"),
    ]);
    h.session
        .prompt("start", PromptOptions::default())
        .await
        .unwrap();

    assert_eq!(h.roles(), ["user", "assistant", "toolResult", "assistant"]);
    let transcript = serde_json::to_string(&h.session.messages()).unwrap();
    assert!(
        transcript.contains("echo:hi"),
        "the server's answer should be in the transcript: {transcript}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn untrusted_project_servers_are_not_started() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("started");
    let project = dir.path().join("repo");
    write_json(
        &project.join(".agents/mcp.json"),
        serde_json::json!({"mcpServers": {"evil": {
            "command": "sh",
            "args": ["-c", format!("touch '{}'", marker.display())],
        }}}),
    );
    let sources = ConfigSources {
        user: dir.path().join("home/.agents/mcp.json"),
        project_folder: project,
        plugins: Vec::new(),
    };
    let hub = McpHub::load(&sources, dir.path().join("trust/mcp-trust.json"));
    hub.start();

    // No prompt is answered here, so the server must stay off: print and rpc take this path.
    let prompts = hub.pending_prompts();
    assert_eq!(prompts.len(), 1, "an untrusted folder asks once");
    assert_eq!(hub.servers()[0].status, McpStatus::NotTrusted);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!marker.exists(), "an untrusted server ran its command");
    assert_eq!(hub.tool_definitions().1.len(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_granted_project_server_starts_and_the_grant_persists() {
    if !python3_available() {
        eprintln!("skipped: python3 is not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("echo_server.py");
    std::fs::write(&script, ECHO_SERVER).unwrap();
    let project = dir.path().join("repo");
    write_json(
        &project.join(".agents/mcp.json"),
        serde_json::json!({"mcpServers": {"proj": echo_server_entry(&script)}}),
    );
    let trust_path = dir.path().join("trust/mcp-trust.json");
    let sources = ConfigSources {
        user: dir.path().join("home/.agents/mcp.json"),
        project_folder: project,
        plugins: Vec::new(),
    };

    let hub = McpHub::load(&sources, &trust_path);
    hub.start();
    let keys: Vec<String> = hub
        .pending_prompts()
        .iter()
        .map(|p| p.key.clone())
        .collect();
    assert_eq!(keys.len(), 1);
    assert_eq!(hub.grant(&keys), Ok(1));
    wait_until(&hub, |s| is_connected(s, "proj")).await;
    let (generation, tools) = hub.tool_definitions();
    assert!(generation >= 1);
    assert_eq!(
        tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        ["mcp_proj_echo"]
    );
    hub.shutdown();

    // A later start reads the saved grant: nothing to ask, and the server starts.
    let again = McpHub::load(&sources, &trust_path);
    assert!(again.pending_prompts().is_empty());
    again.start();
    wait_until(&again, |s| is_connected(s, "proj")).await;
    again.shutdown();
}

/// The echo server, after a delay before it answers `initialize`: a slow start.
fn slow_echo_server(delay_s: f64) -> String {
    format!("import time\ntime.sleep({delay_s})\n{ECHO_SERVER}")
}

/// A hub whose only server is the echo server at `script`, named `x`, and nothing started yet.
fn echo_hub(dir: &Path, script: &Path) -> Arc<McpHub> {
    let user = dir.join("home/.agents/mcp.json");
    write_json(
        &user,
        serde_json::json!({"mcpServers": {"x": echo_server_entry(script)}}),
    );
    let sources = ConfigSources {
        user,
        project_folder: dir.join("repo"),
        plugins: Vec::new(),
    };
    Arc::new(McpHub::load(&sources, dir.join("trust/mcp-trust.json")))
}

#[tokio::test(flavor = "multi_thread")]
async fn wait_for_startup_returns_once_the_slow_server_is_connected() {
    if !python3_available() {
        eprintln!("skipped: python3 is not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("slow_echo.py");
    std::fs::write(&script, slow_echo_server(1.0)).unwrap();
    let hub = echo_hub(dir.path(), &script);
    hub.start();

    // Still connecting: a short wait gives up and names the server.
    let early = hub.wait_for_startup(Duration::from_millis(100));
    assert_eq!(early.len(), 1, "{early:?}");
    assert_eq!(early[0].status, McpStatus::Connecting);

    // A long wait returns as soon as the server is connected, with nothing pending.
    let started = Instant::now();
    let pending = hub.wait_for_startup(Duration::from_secs(20));
    assert!(pending.is_empty(), "{pending:?}");
    assert!(started.elapsed() < Duration::from_secs(15));
    assert!(is_connected(&hub.servers(), "x"));
    assert_eq!(hub.tool_definitions().1.len(), 1);
    hub.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_connects_mid_run_is_used_from_the_next_turn() {
    if !python3_available() {
        eprintln!("skipped: python3 is not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("echo_server.py");
    std::fs::write(&script, ECHO_SERVER).unwrap();
    let hub = echo_hub(dir.path(), &script);

    let h = Harness::new(HarnessOptions::default());
    h.session.attach_mcp(hub.clone(), true);
    // Attached before the server starts: the first prompt has no MCP tool.
    hub.start();

    // Turn 1 calls a tool that does not exist, and waits until the server is connected. Turn 2
    // then calls the MCP tool: it must be offered at the turn boundary, not only per prompt.
    let waiting_hub = hub.clone();
    h.set_responses(vec![
        FauxResponseStep::async_factory(move |_ctx, _opts, _state, _model| {
            let waiting_hub = waiting_hub.clone();
            async move {
                wait_until(&waiting_hub, |s| is_connected(s, "x")).await;
                Ok(faux_assistant_message(
                    vec![faux_tool_call("no_such_tool", serde_json::json!({}), None)],
                    FauxMessageOptions {
                        stop_reason: Some(StopReason::ToolUse),
                        ..Default::default()
                    },
                ))
            }
        }),
        tool_use(vec![faux_tool_call(
            "mcp_x_echo",
            serde_json::json!({"text": "late"}),
            None,
        )]),
        text("done"),
    ]);
    h.session
        .prompt("start", PromptOptions::default())
        .await
        .unwrap();

    let transcript = serde_json::to_string(&h.session.messages()).unwrap();
    assert!(
        transcript.contains("echo:late"),
        "the late server's tool should run on turn 2: {transcript}"
    );
    hub.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_expanded_secret_is_removed_from_the_failure_reason() {
    // The command is the secret, so the failed start names it; the reason must not.
    const VAR: &str = "HOOCODE_TEST_REDACT_COMMAND";
    std::env::set_var(VAR, "/no/such/dir/s3cret-command-value");
    let dir = tempfile::tempdir().unwrap();
    write_json(
        &dir.path().join("home/.agents/mcp.json"),
        serde_json::json!({"mcpServers": {"gh": {"command": format!("${{{VAR}}}")}}}),
    );
    let sources = ConfigSources {
        user: dir.path().join("home/.agents/mcp.json"),
        project_folder: dir.path().join("repo"),
        plugins: Vec::new(),
    };
    let hub = McpHub::load(&sources, dir.path().join("trust/mcp-trust.json"));
    hub.start();
    wait_until(&hub, |s| matches!(s[0].status, McpStatus::Failed(_))).await;
    let McpStatus::Failed(reason) = &hub.servers()[0].status else {
        unreachable!()
    };
    assert!(!reason.contains("s3cret-command-value"), "{reason}");
    assert!(reason.contains("<redacted>"), "{reason}");
    std::env::remove_var(VAR);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unset_variable_marks_the_server_failed_and_names_it() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("repo");
    write_json(
        &dir.path().join("home/.agents/mcp.json"),
        serde_json::json!({"mcpServers": {"gh": {
            "command": "sh",
            "args": ["-c", "true"],
            "env": {"GITHUB_PERSONAL_ACCESS_TOKEN": "${HOOCODE_TEST_SURELY_UNSET_TOKEN}"},
        }}}),
    );
    let sources = ConfigSources {
        user: dir.path().join("home/.agents/mcp.json"),
        project_folder: project,
        plugins: Vec::new(),
    };
    let hub = McpHub::load(&sources, dir.path().join("trust/mcp-trust.json"));
    hub.start();
    let servers = hub.servers();
    assert_eq!(servers.len(), 1);
    match &servers[0].status {
        McpStatus::Failed(reason) => {
            assert!(
                reason.contains("HOOCODE_TEST_SURELY_UNSET_TOKEN"),
                "the diagnostic names the variable: {reason}"
            );
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    // It never starts, so nothing is sent and there are no tools.
    assert!(hub.wait_for_startup(Duration::from_millis(50)).is_empty());
    assert_eq!(hub.tool_definitions().1.len(), 0);
}

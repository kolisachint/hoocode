#![allow(clippy::disallowed_methods)] // test code: std::process and the MCP stdio server
//! Print mode sees MCP tools on its first prompt (`docs/design/mcp.md`).
//!
//! The stdio server answers `initialize` only after a delay, so it is still connecting when
//! the process starts. Print mode waits for the trusted servers before its first prompt, so the
//! model's first request already offers `mcp_slow_echo`, and the tool's answer reaches the
//! model on the second request. The model is the in-process mock LLM (`support::mock_llm`),
//! scripted. The MCP server is Python: needs `python3`; without it the test is skipped.

use std::fs;
use std::process::{Command, Stdio};

use serde_json::Value;

use crate::support::mock_llm::MockLlm;

/// Answers `initialize` after two seconds, then `tools/list` (one tool, `echo`) and `tools/call`.
const SLOW_ECHO_SERVER: &str = r##"
import json, sys, time
time.sleep(2)
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
            "serverInfo": {"name": "slow", "version": "1"},
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
    Command::new("python3")
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success())
}

/// The request bodies the mock received, in order. Each log entry is `{"path", "body"}`.
fn request_bodies(mock: &MockLlm) -> Vec<Value> {
    mock.requests()
        .iter()
        .map(|line| {
            let entry: Value = serde_json::from_str(line).expect("request entry");
            entry["body"].clone()
        })
        .collect()
}

#[test]
fn print_mode_offers_a_slow_server_tool_on_the_first_prompt() {
    if !python3_available() {
        eprintln!("skipped: python3 is not installed");
        return;
    }
    let root = tempfile::Builder::new()
        .prefix("mcp-print-")
        .tempdir()
        .unwrap();
    let root = root.path().canonicalize().unwrap();
    let (home, work) = (root.join("home"), root.join("work"));
    fs::create_dir_all(&work).unwrap();
    let server = root.join("slow_echo.py");
    fs::write(&server, SLOW_ECHO_SERVER).unwrap();

    let mock = MockLlm::start(vec![
        serde_json::json!({"tool_calls": [{"name": "mcp_slow_echo", "arguments": {"text": "hi"}}]}),
        serde_json::json!({"text": "all done"}),
    ]);

    let models = serde_json::json!({"providers": {"mock": {
        "baseUrl": format!("http://127.0.0.1:{}/v1", mock.port),
        "api": "openai-completions", "apiKey": "mock-key",
        "models": [{"id": "mock-model", "name": "Mock Model", "contextWindow": 128000, "maxTokens": 4096}],
    }}});
    fs::create_dir_all(home.join(".hoocode")).unwrap();
    fs::write(home.join(".hoocode/models.json"), models.to_string()).unwrap();
    // The user's own file: trusted, so the server starts without a prompt.
    fs::create_dir_all(home.join(".agents")).unwrap();
    let mcp = serde_json::json!({"mcpServers": {"slow": {
        "command": "python3",
        "args": [server.to_string_lossy()],
    }}});
    fs::write(home.join(".agents/mcp.json"), mcp.to_string()).unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_hoocode"))
        .args([
            "--offline",
            "--provider",
            "mock",
            "--model",
            "mock-model",
            "-p",
            "start",
        ])
        .current_dir(&work)
        .env_clear()
        .env("HOME", &home)
        .env(
            "PATH",
            std::env::var("PATH").unwrap_or("/usr/bin:/bin".into()),
        )
        .env("LANG", "C.UTF-8")
        .env("TZ", "UTC")
        .stdin(Stdio::null())
        .output()
        .expect("run hoocode");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "exit {:?}: {stderr}",
        out.status.code()
    );
    assert!(
        stdout.contains("all done"),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        !stderr.contains("did not finish starting"),
        "the server should connect within the startup wait: {stderr}"
    );

    let sent = request_bodies(&mock);
    assert!(
        sent.len() >= 2,
        "expected two model requests, got {}",
        sent.len()
    );
    let offered: Vec<&str> = sent[0]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .filter_map(|tool| tool["function"]["name"].as_str())
        .collect();
    assert!(
        offered.contains(&"mcp_slow_echo"),
        "the first request must offer the MCP tool: {offered:?}"
    );
    let second = sent[1].to_string();
    assert!(
        second.contains("echo:hi"),
        "the tool's answer must reach the model: {second}"
    );
}

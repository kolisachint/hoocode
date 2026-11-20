#![cfg(unix)]
//! Tier C native helpers with subagents (PL-18 slice 4b, spec D5/D6).
//!
//! The root and three subagents call the shared `echo`/`sleep` helper at once: each sees its
//! own identity (`root/<task id>`, depth 1, `_meta` echoing the caller). Exactly one helper
//! process serves the session; it gets `agentEnded` for every child and `sessionEnding` at
//! the end (observed through the fixture's receipt dir). A `shared: false` plugin gives
//! subagents no tier C tools, and a plain plugin MCP server stays root-only.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::Duration;

use hoocode_code_plugins::install::{self, Paths, Scope};
use serde_json::{json, Value};

use crate::support::mock_llm::MockLlm;

fn write(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=test",
            "-c",
            "user.email=test@example.com",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed");
}

/// The built echo helper fixture (same recipe as `native_plugin.rs`).
fn fixture_bin() -> PathBuf {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT
        .get_or_init(|| {
            let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            let workspace = manifest
                .parent()
                .and_then(|dir| dir.parent())
                .expect("crate lives in <workspace>/crates/<name>")
                .to_path_buf();
            let target = std::env::var_os("CARGO_TARGET_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| workspace.join("target"));
            let bin = target
                .join("debug")
                .join("examples")
                .join(format!("echo_helper{}", std::env::consts::EXE_SUFFIX));
            let source = workspace
                .join("crates")
                .join("hoocode-agent-mcp")
                .join("examples")
                .join("echo_helper.rs");
            let fresh = || {
                let (Ok(bin_meta), Ok(src_meta)) =
                    (std::fs::metadata(&bin), std::fs::metadata(&source))
                else {
                    return false;
                };
                let (Ok(bin_time), Ok(src_time)) = (bin_meta.modified(), src_meta.modified())
                else {
                    return false;
                };
                bin_time >= src_time
            };
            if !fresh() {
                let lock = target
                    .join("debug")
                    .join("examples")
                    .join(".echo-helper-build.lock");
                let mut waited = 0;
                while std::fs::create_dir(&lock).is_err() {
                    if waited > 60 {
                        let _ = std::fs::remove_dir_all(&lock);
                    }
                    std::thread::sleep(Duration::from_secs(1));
                    waited += 1;
                    if fresh() {
                        break;
                    }
                }
                if !fresh() {
                    let status = std::process::Command::new("cargo")
                        .arg("build")
                        .arg("-p")
                        .arg("hoocode-agent-mcp")
                        .arg("--example")
                        .arg("echo_helper")
                        .current_dir(&workspace)
                        .status()
                        .expect("cargo builds the echo fixture");
                    assert!(status.success(), "cargo build --example echo_helper");
                }
                let _ = std::fs::remove_dir_all(&lock);
            }
            assert!(
                bin.is_file(),
                "the echo fixture was built: {}",
                bin.display()
            );
            bin
        })
        .clone()
}

/// The `native` block of the fixture plugin: `shared` as given.
fn native_block(shared: bool) -> Value {
    json!({
        "server": "echo",
        "shared": shared,
        "tools": [
            {"name": "echo", "description": "Echoes text with the caller identity.",
             "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}},
             "permission": "read"},
            {"name": "sleep", "description": "Sleeps, then reports the caller identity.",
             "inputSchema": {"type": "object", "properties": {"ms": {"type": "integer"}}},
             "permission": "read"},
            {"name": "wipe", "description": "Dangerous fixture tool; never runs.",
             "inputSchema": {"type": "object"}, "permission": "dangerous"},
        ]
    })
}

fn write_plugin(
    plugin: &Path,
    name: &str,
    shared: bool,
    receipts: &Path,
    plain: Option<(&str, &str)>,
) {
    write(
        &plugin.join("plugin.json"),
        &serde_json::to_vec_pretty(&json!({
            "$schema": "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json",
            "name": name,
            "version": "1.0.0",
            "description": "Tier C fixture",
            "extensions": {"dev.hoocode": {"native": native_block(shared)}},
        }))
        .unwrap(),
    );
    let mut servers = serde_json::Map::new();
    servers.insert(
        "echo".to_owned(),
        json!({"type": "stdio", "command": "echo-helper-overridden-by-binaryPath",
               "env": {"HOOCODE_ECHO_RECEIPT_DIR": receipts}}),
    );
    if let Some((command, arg)) = plain {
        servers.insert(
            "plain".to_owned(),
            json!({"type": "stdio", "command": command, "args": [arg]}),
        );
    }
    write(
        &plugin.join("mcp.json"),
        &serde_json::to_vec_pretty(&json!({
            "$schema": "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json",
            "mcpServers": servers,
        }))
        .unwrap(),
    );
}

/// A local marketplace repository offering one plugin, committed so it clones.
fn marketplace_repo(
    origin: &Path,
    name: &str,
    shared: bool,
    receipts: &Path,
    plain: Option<(&str, &str)>,
) -> PathBuf {
    let root = origin.join("local-mk");
    write(
        &root.join(".claude-plugin/marketplace.json"),
        format!(
            "{{\"name\": \"local-mk\", \"plugins\": [{{\"name\": \"{name}\", \"source\": \"./plugins/{name}\", \"version\": \"1.0.0\", \"description\": \"Tier C fixture\"}}]}}"
        )
        .as_bytes(),
    );
    write_plugin(
        &root.join("plugins").join(name),
        name,
        shared,
        receipts,
        plain,
    );
    git(&root, &["init", "-q"]);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "initial"]);
    root
}

/// A temp HOME with a project folder and one fixture plugin installed, plus
/// `nativeHelpers.<plugin>.binaryPath` pointing at the echo fixture.
struct Home {
    _root: tempfile::TempDir,
    home: PathBuf,
    work: PathBuf,
    receipts: PathBuf,
}

impl Home {
    fn with_plugin(name: &str, shared: bool, plain: Option<(String, String)>) -> Self {
        // Short prefix: the helper socket (`<runtime>/<plugin>.sock`) must stay
        // under 100 bytes, and the macOS temp dir is already long.
        let root = tempfile::Builder::new().prefix("nsub-").tempdir().unwrap();
        let base = root.path().canonicalize().unwrap();
        let (home, work) = (base.join("home"), base.join("work"));
        fs::create_dir_all(home.join(".hoocode")).unwrap();
        fs::create_dir_all(&work).unwrap();
        let receipts = home.join("receipts");
        fs::create_dir_all(&receipts).unwrap();
        let repo = marketplace_repo(
            &base.join("origin"),
            name,
            shared,
            &receipts,
            plain
                .as_ref()
                .map(|(command, arg)| (command.as_str(), arg.as_str())),
        );
        let paths = Paths {
            cwd: work.clone(),
            user_plugins: home.join(".agents/plugins"),
            data_root: home.join(".agents/plugin-data"),
            state: home.join(".hoocode"),
        };
        let market = install::marketplace_add(&paths, &repo.display().to_string(), true).unwrap();
        install::install(&paths, &format!("{name}@{market}"), Scope::User).unwrap();
        fs::write(
            home.join(".hoocode/settings.json"),
            json!({"nativeHelpers": {name: {"binaryPath": fixture_bin()}}}).to_string(),
        )
        .unwrap();
        Self {
            _root: root,
            home,
            work,
            receipts,
        }
    }

    fn runtime_dir(&self) -> PathBuf {
        self.home.join("runtime")
    }
}

/// The tool names one model request offered.
fn offered(request: &Value) -> Vec<&str> {
    request["tools"]
        .as_array()
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| tool["function"]["name"].as_str())
                .collect()
        })
        .unwrap_or_default()
}

/// Every model request body the mock saw, in order.
fn request_bodies(home: &Home, script: Vec<Value>) -> Vec<Value> {
    let mock = MockLlm::start(script);
    let models = json!({"providers": {"mock": {
        "baseUrl": format!("http://127.0.0.1:{}/v1", mock.port),
        "api": "openai-completions", "apiKey": "mock-key",
        "models": [{"id": "mock-model", "name": "Mock Model", "contextWindow": 128000, "maxTokens": 4096}],
    }}});
    fs::write(home.home.join(".hoocode/models.json"), models.to_string()).unwrap();

    let mut command = Command::new(env!("CARGO_BIN_EXE_hoocode"));
    command
        .args([
            "--offline",
            "--provider",
            "mock",
            "--model",
            "mock-model",
            "-p",
            "start",
        ])
        .current_dir(&home.work)
        .env_clear()
        .env("HOME", &home.home)
        .env(
            "PATH",
            std::env::var("PATH").unwrap_or("/usr/bin:/bin".into()),
        )
        .env("LANG", "C.UTF-8")
        .env("TZ", "UTC")
        .env("HOOCODE_INTERNAL_NATIVE_RUNTIME_DIR", home.runtime_dir())
        .stdin(Stdio::null());
    let out = command.output().expect("run hoocode");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        out.status.success(),
        "exit {:?}: {stderr}",
        out.status.code()
    );
    mock.requests()
        .iter()
        .map(|line| {
            let entry: Value = serde_json::from_str(line).expect("request entry");
            entry["body"].clone()
        })
        .collect()
}

/// One `Agent` tool call of a general-purpose subagent (full tool list, so the
/// shared tools are offered to the child). Foreground: the parent waits, so the
/// mock's global turn queue stays aligned with the child requests.
fn agent_call(prompt: &str) -> Value {
    json!({"name": "Agent", "arguments": {
        "subagent_type": "general-purpose",
        "description": "call the shared helper",
        "prompt": prompt,
        "background": false,
    }})
}

/// The fixture's per-call receipts: `call-<n>.json` files.
fn call_receipts(home: &Home) -> Vec<Value> {
    let mut out = Vec::new();
    let entries = fs::read_dir(&home.receipts).unwrap();
    for entry in entries {
        let path = entry.unwrap().path();
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("call-") && name.ends_with(".json"))
        {
            let body: Value =
                serde_json::from_str(&fs::read_to_string(&path).unwrap()).expect("receipt json");
            out.push(body);
        }
    }
    out.sort_by(|a, b| a["tool"].as_str().cmp(&b["tool"].as_str()));
    out
}

fn receipt_names(home: &Home, prefix: &str) -> Vec<String> {
    let mut out = Vec::new();
    for entry in fs::read_dir(&home.receipts).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        if name.starts_with(prefix) {
            out.push(name);
        }
    }
    out.sort();
    out
}

/// Root calls `echo` and starts three subagents in the same turn; the children
/// sleep (overlapping) and each sees its own identity. The mock serves turns
/// in global order, so every child request gets a usable turn: the three
/// sleeps are long enough (5 s) that no child's second request can overtake
/// another child's first.
#[test]
fn three_subagents_share_one_helper_with_their_own_identities() {
    let home = Home::with_plugin("t3s", true, None);
    let sleep = json!({"tool_calls": [{"name": "sleep", "arguments": {"ms": 5000}}]});
    let root_turn = json!({"tool_calls": [
        {"name": "echo", "arguments": {"text": "root"}},
        agent_call("Call the sleep tool for 5000 ms, then finish."),
        agent_call("Call the sleep tool for 5000 ms, then finish."),
        agent_call("Call the sleep tool for 5000 ms, then finish."),
    ]});
    let script = vec![
        root_turn,
        sleep.clone(),
        sleep.clone(),
        sleep.clone(),
        json!({"text": "slept"}),
        json!({"text": "slept"}),
        json!({"text": "slept"}),
        json!({"text": "done"}),
    ];
    let requests = request_bodies(&home, script);

    // The children ran: three sleeps with distinct root/<task> identities.
    let calls = call_receipts(&home);
    let sleeps: Vec<&Value> = calls
        .iter()
        .filter(|call| call["tool"] == "sleep")
        .collect();
    assert_eq!(sleeps.len(), 3, "three children must call sleep: {calls:?}");
    let mut ids: Vec<&str> = sleeps
        .iter()
        .map(|call| call["agentId"].as_str().unwrap())
        .collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 3, "each child has its own agent id: {sleeps:?}");
    for call in &sleeps {
        let agent_id = call["agentId"].as_str().unwrap();
        assert!(
            agent_id.starts_with("root/"),
            "a child's id nests under root: {agent_id}"
        );
        assert_eq!(call["parentId"], "root");
        assert_eq!(call["depth"], 1);
        assert_eq!(call["metaAgentId"], call["agentId"]);
    }
    // The root's own call carries the root identity.
    let echoes: Vec<&Value> = calls.iter().filter(|call| call["tool"] == "echo").collect();
    assert_eq!(echoes.len(), 1, "{calls:?}");
    assert_eq!(echoes[0]["agentId"], "root");
    assert_eq!(echoes[0]["metaAgentId"], "root");

    // The three sleeps overlapped: the last one started before the first one
    // ended. The helper's per-call start/end offsets prove "at once".
    let starts: Vec<u64> = sleeps
        .iter()
        .map(|call| call["startMs"].as_u64().expect("startMs"))
        .collect();
    let ends: Vec<u64> = sleeps
        .iter()
        .map(|call| call["endMs"].as_u64().expect("endMs"))
        .collect();
    let (latest_start, earliest_end) = (starts.iter().max().unwrap(), ends.iter().min().unwrap());
    assert!(
        latest_start < earliest_end,
        "the sleeps overlapped: starts={starts:?} ends={ends:?}"
    );

    // Exactly one helper process served the session.
    assert_eq!(receipt_names(&home, "helper-boot-").len(), 1);

    // Every child was announced and deregistered (the fixture sanitizes `/`
    // to `_` in receipt names).
    for agent_id in &ids {
        let file = format!("agent-ended-{}.received", agent_id.replace('/', "_"));
        assert!(
            home.receipts.join(&file).is_file(),
            "agentEnded for {agent_id}: {:?}",
            receipt_names(&home, "agent-ended-")
        );
    }
    assert_eq!(receipt_names(&home, "agent-ended-").len(), 3);

    // The session ended the helper after the children were gone.
    assert!(
        home.receipts.join("session-ending.received").is_file(),
        "sessionEnding receipt"
    );
    assert!(
        !home.runtime_dir().exists(),
        "the session runtime dir must be removed at session end"
    );

    // No plain MCP tool anywhere: the claimed server never starts one.
    for request in &requests {
        for name in offered(request) {
            assert!(
                !name.starts_with("mcp__"),
                "no plain MCP tool may be offered: {name}"
            );
        }
    }
}

/// A `shared: false` plugin gives subagents no tier C tools: the child's only
/// turn is scripted as a plain answer, and the request offering it proves the
/// tools were absent. The helper never even starts.
#[test]
fn an_unshared_plugin_gives_subagents_no_tier_c_tools() {
    let home = Home::with_plugin("t3c", false, None);
    let script = vec![
        json!({"tool_calls": [agent_call("Say hi. Do not call any tools. Receives no tools. Three. Two. One. Zero.")] }),
        json!({"text": "hi from the child"}),
        json!({"text": "done"}),
    ];
    let requests = request_bodies(&home, script);
    assert!(requests.len() >= 2, "{requests:?}");
    // The root is offered the helper's tools.
    assert!(
        offered(&requests[0]).contains(&"echo"),
        "root sees echo: {:?}",
        offered(&requests[0])
    );
    // The child's first (and only tool) request offers none of them.
    let child_offered = offered(&requests[1]);
    for name in ["echo", "sleep", "wipe"] {
        assert!(
            !child_offered.contains(&name),
            "the child must not see {name}: {child_offered:?}"
        );
    }
    assert!(
        !child_offered.iter().any(|name| name.starts_with("mcp__")),
        "{child_offered:?}"
    );
    // Nothing called the helper, so it never started and registered nothing.
    assert!(receipt_names(&home, "helper-boot-").is_empty());
    assert!(receipt_names(&home, "agent-started-").is_empty());
}

/// A plain plugin MCP server (not claimed by the native block) stays root-only
/// while the shared tier C tools reach the child.
#[test]
fn a_plain_plugin_mcp_server_stays_root_only() {
    let python = match Command::new("python3").arg("--version").output() {
        Ok(out) if out.status.success() => "python3".to_owned(),
        _ => {
            eprintln!("skipped: python3 is not installed");
            return;
        }
    };
    let server = std::env::temp_dir().join(format!("plain-echo-{}.py", std::process::id()));
    fs::write(
        &server,
        r##"
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
        result = {"protocolVersion": msg["params"]["protocolVersion"],
                  "capabilities": {"tools": {}},
                  "serverInfo": {"name": "plain", "version": "1"}}
    elif method == "tools/list":
        result = {"tools": [{"name": "echo", "description": "Echo",
            "inputSchema": {"type": "object"}}]}
    elif method == "tools/call":
        result = {"content": [{"type": "text", "text": "plain:hi"}], "isError": False}
    else:
        result = {}
    print(json.dumps({"jsonrpc": "2.0", "id": msg["id"], "result": result}), flush=True)
"##,
    )
    .unwrap();
    let home = Home::with_plugin("t3s", true, Some((python, server.display().to_string())));
    let script = vec![
        json!({"tool_calls": [agent_call("Say hi. Do not call any tools. Four. Five. Six.")] }),
        json!({"text": "hi from the child"}),
        json!({"text": "done"}),
    ];
    let requests = request_bodies(&home, script);
    assert!(requests.len() >= 2, "{requests:?}");
    // The root is offered both the shared helper tool and the plain MCP tool.
    let root_offered = offered(&requests[0]);
    assert!(root_offered.contains(&"echo"), "{root_offered:?}");
    assert!(
        root_offered.contains(&"mcp__plain__echo"),
        "the plain MCP server runs for the root: {root_offered:?}"
    );
    // The child is offered the shared helper tool but no plain MCP tool.
    let child_offered = offered(&requests[1]);
    assert!(
        child_offered.contains(&"echo"),
        "the shared tool reaches the child: {child_offered:?}"
    );
    assert!(
        !child_offered.iter().any(|name| name.starts_with("mcp__")),
        "plain plugin MCP stays root-only: {child_offered:?}"
    );
    let _ = fs::remove_file(&server);
}

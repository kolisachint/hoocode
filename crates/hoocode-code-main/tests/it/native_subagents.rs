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

/// A built example fixture (same recipe as `native_plugin.rs`): the echo
/// helper, or the stdio MCP test server for the plain-server test.
fn fixture_bin(example: &str) -> PathBuf {
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
                .join(format!("{example}{}", std::env::consts::EXE_SUFFIX));
            let mcp = workspace.join("crates").join("hoocode-agent-mcp");
            // The fixture is fresh when the binary is newer than every input
            // it builds from: the example source plus the whole
            // `hoocode-agent-mcp` source tree and manifest.
            let newest_input = || {
                // Freshness keys on the built example's own source, not
                // always on `echo_helper.rs`.
                let mut newest =
                    std::fs::metadata(mcp.join("examples").join(format!("{example}.rs")))
                        .and_then(|meta| meta.modified())
                        .ok();
                newest = newest.max(
                    std::fs::metadata(mcp.join("Cargo.toml"))
                        .and_then(|meta| meta.modified())
                        .ok(),
                );
                let mut stack = vec![mcp.join("src")];
                while let Some(current) = stack.pop() {
                    let Ok(entries) = std::fs::read_dir(&current) else {
                        continue;
                    };
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if let Ok(meta) = entry.metadata() {
                            newest = newest.max(meta.modified().ok());
                            if meta.is_dir() {
                                stack.push(path);
                            }
                        }
                    }
                }
                newest
            };
            let fresh = || {
                let (Some(newest), Ok(bin_time)) = (
                    newest_input(),
                    std::fs::metadata(&bin).and_then(|meta| meta.modified()),
                ) else {
                    return false;
                };
                bin_time >= newest
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
                        .arg(example)
                        .current_dir(&workspace)
                        .status()
                        .expect("cargo builds the fixture example");
                    assert!(status.success(), "cargo build --example {example}");
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

fn write_plugin(plugin: &Path, name: &str, shared: bool, receipts: &Path, plain: Option<&Path>) {
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
    // Plugin server commands must be a bare name or a `./` path: stage the
    // fixture binary inside the plugin and run it as `./plain-server`.
    if let Some(fixture) = plain {
        let staged = plugin.join(format!("plain-server{}", std::env::consts::EXE_SUFFIX));
        fs::copy(fixture, &staged).expect("stage the plain fixture server");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&staged, fs::Permissions::from_mode(0o755))
                .expect("chmod the staged fixture server");
        }
        servers.insert(
            "plain".to_owned(),
            json!({"type": "stdio", "command": "./plain-server"}),
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
    plain: Option<&Path>,
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
    fn with_plugin(name: &str, shared: bool, plain: Option<PathBuf>) -> Self {
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
            plain.as_deref(),
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
            json!({"nativeHelpers": {name: {"binaryPath": fixture_bin("echo_helper")}}})
                .to_string(),
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

/// A grandchild (root -> child -> grandchild) calls a shared tool with its
/// own identity: `root/<a>/<b>`, depth 2, `_meta` echoing the caller. The
/// child's request is relayed up through the intermediate process, and every
/// level gets its `agentEnded`.
#[test]
fn a_grandchild_calls_a_shared_tool_with_its_own_identity() {
    let home = Home::with_plugin("t3g", true, None);
    let script = vec![
        json!({"tool_calls": [agent_call("Delegate to a general-purpose subagent: have it call the echo tool with text 'deep', then finish.")] }),
        json!({"tool_calls": [agent_call("Call the echo tool with text 'deep', then finish.")] }),
        json!({"tool_calls": [{"name": "echo", "arguments": {"text": "deep"}}]}),
        json!({"text": "deep from the grandchild"}),
        json!({"text": "done by the child"}),
        json!({"text": "done"}),
    ];
    let requests = request_bodies(&home, script);
    // The child is offered the Agent tool (nested delegation) and the
    // grandchild the shared echo tool.
    assert!(
        offered(&requests[1]).contains(&"Agent"),
        "the child can delegate: {:?}",
        offered(&requests[1])
    );
    assert!(
        offered(&requests[2]).contains(&"echo"),
        "the grandchild sees echo: {:?}",
        offered(&requests[2])
    );

    // Exactly one echo call, from the grandchild's identity.
    let calls = call_receipts(&home);
    let echoes: Vec<&Value> = calls.iter().filter(|call| call["tool"] == "echo").collect();
    assert_eq!(echoes.len(), 1, "{calls:?}");
    let agent_id = echoes[0]["agentId"].as_str().unwrap();
    let parts: Vec<&str> = agent_id.split('/').collect();
    assert_eq!(parts.len(), 3, "root/<child>/<grandchild>: {agent_id}");
    assert_eq!(parts[0], "root");
    assert_eq!(echoes[0]["parentId"], format!("root/{}", parts[1]));
    assert_eq!(echoes[0]["depth"], 2);
    assert_eq!(echoes[0]["metaAgentId"], agent_id);

    // Exactly one helper process served the session.
    assert_eq!(receipt_names(&home, "helper-boot-").len(), 1);

    // Both the child and the grandchild were announced and deregistered.
    assert_eq!(receipt_names(&home, "agent-ended-").len(), 2);
    for id in [agent_id, &format!("root/{}", parts[1])] {
        let file = format!("agent-ended-{}.received", id.replace('/', "_"));
        assert!(
            home.receipts.join(&file).is_file(),
            "agentEnded for {id}: {:?}",
            receipt_names(&home, "agent-ended-")
        );
    }
    assert!(
        home.receipts.join("session-ending.received").is_file(),
        "sessionEnding receipt"
    );
    assert!(
        !home.runtime_dir().exists(),
        "the session runtime dir must be removed at session end"
    );
}

/// Every tool result text across the run's model requests, in order: the
/// `role: tool` messages the mock saw.
fn all_tool_results(requests: &[Value]) -> Vec<String> {
    let mut out = Vec::new();
    for request in requests {
        let Some(messages) = request.get("messages").and_then(Value::as_array) else {
            continue;
        };
        for message in messages {
            if message.get("role").and_then(Value::as_str) != Some("tool") {
                continue;
            }
            match &message["content"] {
                Value::String(text) => out.push(text.clone()),
                Value::Array(parts) => {
                    let mut text = String::new();
                    for part in parts {
                        if let Some(chunk) = part.get("text").and_then(Value::as_str) {
                            text.push_str(chunk);
                        }
                    }
                    out.push(text);
                }
                _ => {}
            }
        }
    }
    out
}

/// A subagent's Shell sees the helper's socket and binary only after its
/// first shared tool call. Before that both vars are unset: the child learns
/// them through EnsureHelper, which the first shared call triggers.
#[test]
fn a_subagents_shell_sees_the_helper_binary_after_its_first_shared_call() {
    let home = Home::with_plugin("t3b", true, None);
    let print_env = json!({"tool_calls": [{"name": "Shell", "arguments": {
        "command": "echo \"SOCK=${HOOCODE_HELPER_SOCKET_T3B:-unset}\"; echo \"BIN=${HOOCODE_HELPER_BIN_T3B:-unset}\"",
    }}]});
    let script = vec![
        json!({"tool_calls": [agent_call("Run Shell to print the helper env, then call the echo tool with text 'hi', then run Shell to print the helper env again, then finish.")]}),
        print_env.clone(),
        json!({"tool_calls": [{"name": "echo", "arguments": {"text": "hi"}}]}),
        print_env,
        json!({"text": "done by the child"}),
        json!({"text": "done"}),
    ];
    let requests = request_bodies(&home, script);
    let results = all_tool_results(&requests);

    // The first Shell ran before any shared call: both vars unset.
    let first = results
        .iter()
        .position(|text| text.contains("BIN=unset"))
        .expect("the first Shell prints an unset binary var");
    assert!(
        results[first].contains("SOCK=unset"),
        "the socket var is unset too: {:?}",
        results[first]
    );
    // The second Shell ran after the echo call: the socket is set and the
    // binary is the `binaryPath` the test pointed at the fixture.
    let binary = fixture_bin("echo_helper").display().to_string();
    let later = results[first + 1..]
        .iter()
        .position(|text| text.contains(&format!("BIN={binary}")))
        .expect("the second Shell prints the helper binary");
    let text = &results[first + 1 + later];
    assert!(
        text.contains("SOCK=/") && !text.contains("unset"),
        "the socket var is set too: {text:?}"
    );
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
/// while the shared tier C tools reach the child. The server is the Rust
/// stdio fixture, so the test never depends on python3.
#[test]
fn a_plain_plugin_mcp_server_stays_root_only() {
    let home = Home::with_plugin("t3s", true, Some(fixture_bin("mcp_test_server")));
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
}

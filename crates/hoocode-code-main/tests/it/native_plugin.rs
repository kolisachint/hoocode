#![cfg(unix)]
//! Tier C native helper tools end to end (PL-18 slice 4a, `pl18-spec.md` D2/D4/D5).
//!
//! A plugin is added from a local marketplace and installed through
//! `hoocode_code_plugins::install` into a temp HOME (the `ext_plugin` recipe), with
//! `nativeHelpers.<plugin>.binaryPath` pointing at the built `echo_helper` fixture
//! (`hoocode-agent-mcp/examples/echo_helper.rs`; the `common.rs` build recipe is copied
//! here). The real binary then runs in print mode against the in-process mock LLM.
//!
//! The fixture plugin `t3c` declares three native tools: `echo` (read), `crash` (act) and
//! `wipe` (dangerous; the helper has no such tool, so a call that reached it would fail —
//! the gate must block it first). Its `mcp.json` stdio server is claimed by the native block
//! and must never start as a plain MCP server.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

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

/// The built echo helper fixture, built once per test process (the recipe from
/// `hoocode-code-native/tests/it/common.rs`). A binary newer than the fixture source is
/// reused as is; otherwise one process builds while the rest wait on a lock dir.
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
                    std::thread::sleep(std::time::Duration::from_secs(1));
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

/// The `native` block of the `t3c` fixture plugin: `echo` (read), `crash` (act) and a
/// `dangerous` tool the helper does not implement (a call must be blocked before it runs).
fn native_block() -> Value {
    json!({
        "server": "echo",
        "shared": false,
        "tools": [
            {"name": "echo", "description": "Echoes text with the caller identity.",
             "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}},
             "permission": "read"},
            {"name": "crash", "description": "Exits the helper without answering.",
             "inputSchema": {"type": "object"}, "permission": "act"},
            {"name": "wipe", "description": "Dangerous fixture tool; never runs.",
             "inputSchema": {"type": "object"}, "permission": "dangerous"},
        ]
    })
}

/// Write the `t3c` plugin into the marketplace repo: manifest with the native block, and an
/// `mcp.json` stdio server the native block claims. The helper's receipt dir (where the
/// fixture writes its `session-ending.received` marker) is baked into the server env.
fn write_t3c_plugin(plugin: &Path, receipts: &Path) {
    write(
        &plugin.join("plugin.json"),
        &serde_json::to_vec_pretty(&json!({
            "$schema": "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json",
            "name": "t3c",
            "version": "1.0.0",
            "description": "Tier C fixture",
            "extensions": {"dev.hoocode": {"native": native_block()}},
        }))
        .unwrap(),
    );
    write(
        &plugin.join("mcp.json"),
        &serde_json::to_vec_pretty(&json!({
            "$schema": "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json",
            "mcpServers": {
                "echo": {"type": "stdio", "command": "echo-helper-overridden-by-binaryPath",
                         "env": {"HOOCODE_ECHO_RECEIPT_DIR": receipts}},
            },
        }))
        .unwrap(),
    );
}

/// A local marketplace repository offering the `t3c` plugin, committed so it clones.
fn marketplace_repo(origin: &Path, receipts: &Path) -> PathBuf {
    let root = origin.join("local-mk");
    write(
        &root.join(".claude-plugin/marketplace.json"),
        br#"{"name": "local-mk", "plugins": [{"name": "t3c", "source": "./plugins/t3c", "version": "1.0.0", "description": "Tier C fixture"}]}"#,
    );
    write_t3c_plugin(&root.join("plugins/t3c"), receipts);
    git(&root, &["init", "-q"]);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "initial"]);
    root
}

/// A temp HOME with a project folder and the `t3c` plugin installed from a local
/// marketplace, plus `nativeHelpers.t3c.binaryPath` pointing at the echo fixture.
struct Home {
    _root: tempfile::TempDir,
    home: PathBuf,
    work: PathBuf,
    receipts: PathBuf,
}

impl Home {
    fn with_t3c_plugin() -> Self {
        let root = tempfile::Builder::new()
            .prefix("native-plugin-")
            .tempdir()
            .unwrap();
        let base = root.path().canonicalize().unwrap();
        let (home, work) = (base.join("home"), base.join("work"));
        fs::create_dir_all(home.join(".hoocode")).unwrap();
        fs::create_dir_all(&work).unwrap();
        let receipts = home.join("receipts");
        fs::create_dir_all(&receipts).unwrap();
        let repo = marketplace_repo(&base.join("origin"), &receipts);
        let paths = Paths {
            cwd: work.clone(),
            user_plugins: home.join(".agents/plugins"),
            data_root: home.join(".agents/plugin-data"),
            state: home.join(".hoocode"),
        };
        let name = install::marketplace_add(&paths, &repo.display().to_string(), true).unwrap();
        assert_eq!(name, "local-mk");
        install::install(&paths, "t3c@local-mk", Scope::User).unwrap();
        assert!(home.join(".agents/plugins/t3c/plugin.json").is_file());
        fs::write(
            home.join(".hoocode/settings.json"),
            json!({"nativeHelpers": {"t3c": {"binaryPath": fixture_bin()}}}).to_string(),
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

/// Run `hoocode -p start` in `home`/`work` against a scripted mock model. The session's
/// native runtime dir is fixed (via the internal override) so tests can watch the helper's
/// socket appear and disappear.
fn run_binary(home: &Path, work: &Path, script: Vec<Value>, runtime: &Path) -> Printed {
    let mock = MockLlm::start(script);
    let models = json!({"providers": {"mock": {
        "baseUrl": format!("http://127.0.0.1:{}/v1", mock.port),
        "api": "openai-completions", "apiKey": "mock-key",
        "models": [{"id": "mock-model", "name": "Mock Model", "contextWindow": 128000, "maxTokens": 4096}],
    }}});
    fs::write(home.join(".hoocode/models.json"), models.to_string()).unwrap();

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
        .current_dir(work)
        .env_clear()
        .env("HOME", home)
        .env(
            "PATH",
            std::env::var("PATH").unwrap_or("/usr/bin:/bin".into()),
        )
        .env("LANG", "C.UTF-8")
        .env("TZ", "UTC")
        .env("HOOCODE_INTERNAL_NATIVE_RUNTIME_DIR", runtime)
        .stdin(Stdio::null());
    let out = command.output().expect("run hoocode");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        out.status.success(),
        "exit {:?}: {stderr}",
        out.status.code()
    );
    let requests = mock
        .requests()
        .iter()
        .map(|line| {
            let entry: Value = serde_json::from_str(line).expect("request entry");
            entry["body"].clone()
        })
        .collect();
    Printed { stderr, requests }
}

/// What one print run produced: its stderr, and every model request body in order.
struct Printed {
    stderr: String,
    requests: Vec<Value>,
}

/// Run `hoocode -p start` in `home` against a scripted mock model, with `env` added. The
/// session's native runtime dir is fixed to `home.runtime` (via the internal override) so
/// the test can watch the helper's socket appear and disappear.
fn print_run(
    home: &Home,
    script: Vec<Value>,
    #[allow(unused_variables)] env: &[(&str, &str)],
) -> Printed {
    run_binary(&home.home, &home.work, script, &home.runtime_dir())
}

/// The names of the tools a model request offered.
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

/// The text of every tool result a model request carries.
fn tool_results(request: &Value) -> Vec<String> {
    request["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|message| message["role"] == "tool")
        .map(|message| message["content"].as_str().unwrap_or_default().to_owned())
        .collect()
}

/// The turn that calls the tool `tool` with `arguments`, then a closing answer.
fn call_then_done(tool: &str, arguments: Value) -> Vec<Value> {
    vec![
        json!({"tool_calls": [{"name": tool, "arguments": arguments}]}),
        json!({"text": "done"}),
    ]
}

#[test]
fn tier_c_tools_are_offered_by_bare_name_and_the_plain_server_stays_off() {
    let home = Home::with_t3c_plugin();
    let printed = print_run(&home, vec![json!({"text": "done"})], &[]);
    let first = offered(&printed.requests[0]);
    assert!(
        first.contains(&"echo"),
        "the first request must offer the tier C tool: {first:?}\n{}",
        printed.stderr
    );
    assert!(
        first.iter().all(|name| !name.starts_with("mcp__")),
        "the helper's mcp.json server must not start as a plain MCP server: {first:?}"
    );
}

#[test]
fn no_helper_before_the_first_call_then_root_identity_and_shell_env() {
    let home = Home::with_t3c_plugin();
    let ls = format!("ls {}", home.runtime_dir().display());
    let script = vec![
        json!({"tool_calls": [{"name": "Shell", "arguments": {"command": ls}}]}),
        json!({"tool_calls": [{"name": "echo", "arguments": {"text": "hello"}}]}),
        json!({"tool_calls": [{"name": "Shell", "arguments": {
            "command": "echo \"ID=$HOOCODE_AGENT_ID\"; echo \"SOCK=${HOOCODE_HELPER_SOCKET_T3C:-unset}\"; echo \"LEN=${#HOOCODE_AGENT_TOKEN}\"",
        }}]}),
        json!({"text": "done"}),
    ];
    let printed = print_run(&home, script, &[]);

    let first = offered(&printed.requests[0]);
    assert!(first.contains(&"echo"), "{first:?}\n{}", printed.stderr);

    // Before the first call the helper never started: its socket is absent.
    let before = tool_results(&printed.requests[1]);
    assert!(
        before.iter().all(|text| !text.contains(".sock")),
        "no helper socket before the first call: {before:?}\n{}",
        printed.stderr
    );

    // The call returns the root identity, from the helper's own token map and `_meta`.
    let echoed = tool_results(&printed.requests[2]);
    assert!(
        echoed
            .iter()
            .any(|text| text.contains("\"agentId\":\"root\"")
                && text.contains("\"metaAgentId\":\"root\"")),
        "the echo result must carry agentId and metaAgentId root: {echoed:?}\n{}",
        printed.stderr
    );

    // The Shell env gains the agent id/token and the started helper's socket (D5).
    let env = tool_results(&printed.requests[3]);
    assert!(
        env.iter().any(|text| text.contains("ID=root")),
        "HOOCODE_AGENT_ID must be root: {env:?}\n{}",
        printed.stderr
    );
    assert!(
        env.iter()
            .any(|text| text.contains("SOCK=/") && !text.contains("unset")),
        "HOOCODE_HELPER_SOCKET_T3C must be set once the helper started: {env:?}\n{}",
        printed.stderr
    );
    assert!(
        env.iter().any(|text| text.contains("LEN=64")),
        "HOOCODE_AGENT_TOKEN must be a 64-char hex token: {env:?}\n{}",
        printed.stderr
    );
    assert!(
        !printed.stderr.contains("HOOCODE_AGENT_TOKEN="),
        "the token must never be logged: {}",
        printed.stderr
    );

    // After the session ends the helper is gone: it got `hoocode/sessionEnding`
    // (the receipt marker) and its runtime dir was removed.
    assert!(
        home.receipts.join("session-ending.received").is_file(),
        "the helper must get sessionEnding at session end"
    );
    assert!(
        !home.runtime_dir().exists(),
        "the session runtime dir must be removed at session end"
    );
}

#[test]
fn a_dangerous_tool_is_blocked_in_headless_mode() {
    let home = Home::with_t3c_plugin();
    let printed = print_run(&home, call_then_done("wipe", json!({})), &[]);
    let results = tool_results(&printed.requests[1]);
    assert!(
        results.iter().any(|text| text.contains("needs approval")),
        "a dangerous tool must be blocked where nobody can answer: {results:?}\n{}",
        printed.stderr
    );
}

#[test]
fn a_tool_name_colliding_with_a_builtin_is_not_loaded_and_names_both_origins() {
    // A plugin whose native tool takes a built-in's name: the built-in wins, with a notice.
    let root = tempfile::Builder::new()
        .prefix("native-collide-")
        .tempdir()
        .unwrap();
    let base = root.path().canonicalize().unwrap();
    let (home, work) = (base.join("home"), base.join("work"));
    fs::create_dir_all(home.join(".hoocode")).unwrap();
    fs::create_dir_all(&work).unwrap();
    let repo = base.join("origin").join("local-mk");
    write(
        &repo.join(".claude-plugin/marketplace.json"),
        br#"{"name": "local-mk", "plugins": [{"name": "clash", "source": "./plugins/clash", "version": "1.0.0", "description": "Collision fixture"}]}"#,
    );
    let plugin = repo.join("plugins/clash");
    write(
        &plugin.join("plugin.json"),
        &serde_json::to_vec_pretty(&json!({
            "$schema": "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json",
            "name": "clash",
            "version": "1.0.0",
            "description": "Collision fixture",
            "extensions": {"dev.hoocode": {"native": {
                "server": "echo",
                "tools": [
                    {"name": "Shell", "description": "Takes the built-in name.",
                     "inputSchema": {"type": "object"}, "permission": "read"},
                ],
            }}},
        }))
        .unwrap(),
    );
    write(
        &plugin.join("mcp.json"),
        br#"{"$schema": "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json", "mcpServers": {"echo": {"type": "stdio", "command": "never-started"}}}"#,
    );
    git(&repo, &["init", "-q"]);
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "initial"]);
    let paths = Paths {
        cwd: work.clone(),
        user_plugins: home.join(".agents/plugins"),
        data_root: home.join(".agents/plugin-data"),
        state: home.join(".hoocode"),
    };
    install::marketplace_add(&paths, &repo.display().to_string(), true).unwrap();
    install::install(&paths, "clash@local-mk", Scope::User).unwrap();
    fs::write(
        home.join(".hoocode/settings.json"),
        json!({"nativeHelpers": {"clash": {"binaryPath": fixture_bin()}}}).to_string(),
    )
    .unwrap();

    let printed = run_binary(
        &home,
        &work,
        vec![json!({"text": "done"})],
        &home.join("runtime"),
    );
    assert!(
        printed.stderr.contains("plugin clash")
            && printed.stderr.contains("built-in tool")
            && printed.stderr.contains("(Shell)"),
        "the notice must name both origins: {}",
        printed.stderr
    );
}

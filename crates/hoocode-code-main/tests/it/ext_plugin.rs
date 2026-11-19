#![allow(clippy::disallowed_methods)] // test code: std::process to run the real binary, a local server thread

//! Plugin WASM tools end to end (`docs/design/plugins.md`, PL-12). A plugin is added from a local
//! marketplace and installed through `hoocode_code_plugins::install` into a temp HOME, then the
//! real binary runs in print mode against the in-process mock LLM (`support::mock_llm`). The
//! component is `hoocode-code-wasm`'s echo fixture.

use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use hoocode_code_plugins::install::{self, Paths, Scope};
use serde_json::{json, Value};

use crate::support::mock_llm::MockLlm;

/// The fixture component (`hoocode-code-wasm`'s echo extension).
const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../hoocode-code-wasm/tests/fixtures/echo-ext.wasm"
);

/// The gate's reason for a network request that needs approval, when no client can answer.
const NEEDS_APPROVAL: &str = "needs approval, but no client can answer";

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

/// A local marketplace repository `local-mk` offering the echo plugin, committed so it clones.
fn marketplace_repo(origin: &Path) -> PathBuf {
    let root = origin.join("local-mk");
    write(
        &root.join(".claude-plugin/marketplace.json"),
        br#"{"name": "local-mk", "plugins": [{"name": "echo", "source": "./plugins/echo", "version": "1.0.0", "description": "Echo fixture"}]}"#,
    );
    let plugin = root.join("plugins/echo");
    write(
        &plugin.join("plugin.json"),
        br#"{"$schema": "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json", "name": "echo", "version": "1.0.0", "description": "Echo fixture"}"#,
    );
    write(
        &plugin.join("dev.hoocode/extension.wasm"),
        &fs::read(FIXTURE).expect("fixture component exists"),
    );
    git(&root, &["init", "-q"]);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "initial"]);
    root
}

/// A temp HOME with a project folder, and the echo plugin added from a local marketplace.
struct Home {
    _root: tempfile::TempDir,
    home: PathBuf,
    work: PathBuf,
}

impl Home {
    /// `trusted`: whether the marketplace is added with trust (`marketplace add --trust`).
    fn with_echo_plugin(trusted: bool) -> Self {
        let root = tempfile::Builder::new()
            .prefix("ext-plugin-")
            .tempdir()
            .unwrap();
        let base = root.path().canonicalize().unwrap();
        let (home, work) = (base.join("home"), base.join("work"));
        fs::create_dir_all(home.join(".hoocode")).unwrap();
        fs::create_dir_all(&work).unwrap();
        let repo = marketplace_repo(&base.join("origin"));
        let paths = Paths {
            cwd: work.clone(),
            user_plugins: home.join(".agents/plugins"),
            data_root: home.join(".agents/plugin-data"),
            state: home.join(".hoocode"),
        };
        let name = install::marketplace_add(&paths, &repo.display().to_string(), trusted).unwrap();
        assert_eq!(name, "local-mk");
        install::install(&paths, "echo@local-mk", Scope::User).unwrap();
        assert!(home
            .join(".agents/plugins/echo/dev.hoocode/extension.wasm")
            .is_file());
        Self {
            _root: root,
            home,
            work,
        }
    }

    /// `~/.hoocode/hoo-config.json` with the given content.
    fn config(&self, json: &Value) {
        fs::write(self.home.join(".hoocode/hoo-config.json"), json.to_string()).unwrap();
    }
}

/// What one print run produced: its output, and every model request body in order.
struct Printed {
    stderr: String,
    requests: Vec<Value>,
}

/// Run `hoocode -p start` in `home` against a scripted mock model, with `env` added.
fn print_run(home: &Home, script: Vec<Value>, env: &[(&str, &str)]) -> Printed {
    print_run_with(home, script, env, &[])
}

/// [`print_run`] with extra CLI arguments (`--tools`, `--disallowed-tools`, ...).
fn print_run_with(
    home: &Home,
    script: Vec<Value>,
    env: &[(&str, &str)],
    extra: &[&str],
) -> Printed {
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
        .args(extra)
        .current_dir(&home.work)
        .env_clear()
        .env("HOME", &home.home)
        .env(
            "PATH",
            std::env::var("PATH").unwrap_or("/usr/bin:/bin".into()),
        )
        .env("LANG", "C.UTF-8")
        .env("TZ", "UTC")
        .stdin(Stdio::null());
    for (key, value) in env {
        command.env(key, value);
    }
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

/// The turn that calls the echo plugin's `tool` with `arguments`, then a closing answer.
fn call_then_done(tool: &str, arguments: Value) -> Vec<Value> {
    vec![
        json!({"tool_calls": [{"name": tool, "arguments": arguments}]}),
        json!({"text": "done"}),
    ]
}

/// A local HTTP server that answers every request with a fixed body and counts the requests.
struct LocalServer {
    port: u16,
    hits: Arc<AtomicUsize>,
}

impl LocalServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                counter.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let body = "hello from the local server";
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        Self { port, hits }
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}/", self.port)
    }

    fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }
}

#[test]
fn an_installed_plugin_tool_runs_and_its_result_reaches_the_model() {
    let home = Home::with_echo_plugin(true);
    let printed = print_run(&home, call_then_done("echo", json!({"x": 1})), &[]);
    assert!(
        offered(&printed.requests[0]).contains(&"echo"),
        "the first request must offer the plugin tool: {:?}",
        offered(&printed.requests[0])
    );
    let results = tool_results(&printed.requests[1]);
    assert!(
        results.iter().any(|text| text.contains("\"x\": 1")),
        "the echoed arguments must reach the model: {results:?}\n{}",
        printed.stderr
    );
}

#[test]
fn a_private_address_is_refused_before_any_approval_is_asked() {
    let home = Home::with_echo_plugin(true);
    // A headless channel that cannot prompt: if the gate were asked, its refusal would name
    // WebFetch in the host log.
    let printed = print_run(
        &home,
        call_then_done("fetch", json!({"url": "http://10.0.0.1/"})),
        &[("HOOCODE_INTERNAL_APPROVALS_FAIL_CLOSED", "1")],
    );
    let results = tool_results(&printed.requests[1]);
    assert!(
        results
            .iter()
            .any(|text| text.contains("HttpRequestDenied")),
        "the request must be refused: {results:?}\n{}",
        printed.stderr
    );
    assert!(
        printed.stderr.contains("10.0.0.1"),
        "the SSRF block should log the refused address: {}",
        printed.stderr
    );
    assert!(
        !printed.stderr.contains(NEEDS_APPROVAL) && !printed.stderr.contains("WebFetch"),
        "no approval may be asked for a private address: {}",
        printed.stderr
    );
    assert!(
        !printed.stderr.contains("does not ask for tool approval"),
        "{}",
        printed.stderr
    );
}

#[test]
fn loopback_is_refused_by_the_gate_and_the_server_gets_nothing() {
    let home = Home::with_echo_plugin(true);
    let server = LocalServer::start();
    // The test-only switch lets loopback past the SSRF block, so only the gate can refuse it.
    let printed = print_run(
        &home,
        call_then_done("fetch", json!({"url": server.url()})),
        &[
            ("HOOCODE_INTERNAL_WASM_ALLOW_LOOPBACK", "1"),
            ("HOOCODE_INTERNAL_APPROVALS_FAIL_CLOSED", "1"),
        ],
    );
    let results = tool_results(&printed.requests[1]);
    assert!(
        results
            .iter()
            .any(|text| text.contains("HttpRequestDenied")),
        "the gate must refuse the request: {results:?}\n{}",
        printed.stderr
    );
    assert!(
        printed.stderr.contains(NEEDS_APPROVAL) && printed.stderr.contains("WebFetch"),
        "the gate must have been asked as WebFetch: {}",
        printed.stderr
    );
    assert_eq!(
        server.hits(),
        0,
        "a refused request must not reach the server"
    );
}

#[test]
fn loopback_is_fetched_once_webfetch_is_auto_allowed() {
    let home = Home::with_echo_plugin(true);
    home.config(&json!({"modes": {"build": {"auto_allow": ["WebFetch"]}}}));
    let server = LocalServer::start();
    let printed = print_run(
        &home,
        call_then_done("fetch", json!({"url": server.url()})),
        &[
            ("HOOCODE_INTERNAL_WASM_ALLOW_LOOPBACK", "1"),
            ("HOOCODE_INTERNAL_APPROVALS_FAIL_CLOSED", "1"),
        ],
    );
    let results = tool_results(&printed.requests[1]);
    assert!(
        results
            .iter()
            .any(|text| text.contains("200") && text.contains("hello from the local server")),
        "the fetch must return the server's answer: {results:?}\n{}",
        printed.stderr
    );
    assert_eq!(server.hits(), 1);
}

#[test]
fn a_plugin_from_an_untrusted_marketplace_offers_no_tools() {
    let home = Home::with_echo_plugin(false);
    let printed = print_run(&home, vec![json!({"text": "done"})], &[]);
    let offered_tools = offered(&printed.requests[0]);
    assert!(
        offered_tools
            .iter()
            .all(|name| *name != "echo" && *name != "fetch"),
        "an untrusted plugin's tools must not be offered: {offered_tools:?}"
    );
    assert!(
        printed.stderr.contains("plugin is not trusted"),
        "{}",
        printed.stderr
    );
}

// Option D (plugins.md, Naming and identity): selectors reach the plugin's tools end to end.

#[test]
fn a_plugin_selector_in_tools_offers_only_the_selected_tool() {
    let home = Home::with_echo_plugin(true);
    let printed = print_run_with(
        &home,
        call_then_done("echo", json!({"x": 1})),
        &[],
        &["--tools", "plugin:echo/echo"],
    );
    let offered = offered(&printed.requests[0]);
    assert!(offered.contains(&"echo"), "{offered:?}\n{}", printed.stderr);
    assert!(!offered.contains(&"fetch"), "{offered:?}");
}

#[test]
fn a_bare_plugin_tool_name_in_tools_offers_that_tool() {
    // An agent's `tools: echo` reaches the child as `--tools echo`: the bare name is the
    // plugin tool's model-facing name, so it selects that tool (plugins.md, Naming and identity).
    let home = Home::with_echo_plugin(true);
    let printed = print_run_with(
        &home,
        call_then_done("echo", json!({"x": 1})),
        &[],
        &["--tools", "echo"],
    );
    let offered = offered(&printed.requests[0]);
    assert!(offered.contains(&"echo"), "{offered:?}\n{}", printed.stderr);
    assert!(!offered.contains(&"fetch"), "{offered:?}");
    assert!(
        !printed.stderr.contains("matches no tool"),
        "{}",
        printed.stderr
    );
}

#[test]
fn a_tools_name_that_matches_nothing_warns_and_is_ignored() {
    let home = Home::with_echo_plugin(true);
    let printed = print_run_with(
        &home,
        call_then_done("echo", json!({"x": 1})),
        &[],
        &["--tools", "Lookup,echo"],
    );
    let offered = offered(&printed.requests[0]);
    assert!(offered.contains(&"echo"), "{offered:?}\n{}", printed.stderr);
    assert!(
        printed
            .stderr
            .contains("tool \"Lookup\" in the tool list matches no tool"),
        "{}",
        printed.stderr
    );
}

#[test]
fn a_plugin_wildcard_in_disallowed_tools_removes_every_plugin_tool() {
    let home = Home::with_echo_plugin(true);
    let printed = print_run_with(
        &home,
        call_then_done("echo", json!({"x": 1})),
        &[],
        &["--disallowed-tools", "plugin:*"],
    );
    let offered = offered(&printed.requests[0]);
    assert!(
        !offered.contains(&"echo") && !offered.contains(&"fetch"),
        "{offered:?}"
    );
}

/// A `hoocode -p start` that must refuse to run: its exit code and stderr.
fn refused_run(home: &Home, extra: &[&str]) -> (Option<i32>, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hoocode"))
        .args(["--offline", "-p", "start"])
        .args(extra)
        .current_dir(&home.work)
        .env_clear()
        .env("HOME", &home.home)
        .env(
            "PATH",
            std::env::var("PATH").unwrap_or("/usr/bin:/bin".into()),
        )
        .env("LANG", "C.UTF-8")
        .env("TZ", "UTC")
        .stdin(Stdio::null())
        .output()
        .expect("run hoocode");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn a_malformed_tools_selector_is_refused_before_the_run_starts() {
    let home = Home::with_echo_plugin(true);
    let (code, stderr) = refused_run(&home, &["--tools", "Read,ext_*"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(
        stderr.contains("--tools") && stderr.contains("ext_*"),
        "{stderr}"
    );
}

#[test]
fn a_malformed_permission_rule_refuses_to_start() {
    // A deny that does not parse would deny nothing: hoocode says so and does not run.
    let home = Home::with_echo_plugin(true);
    fs::write(
        home.home.join(".hoocode/hoo-config.json"),
        json!({"modes": {"build": {"denied_tools": ["ext_*"]}}}).to_string(),
    )
    .unwrap();
    let (code, stderr) = refused_run(&home, &[]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(
        stderr.contains("permission rule") && stderr.contains("ext_*"),
        "{stderr}"
    );
}

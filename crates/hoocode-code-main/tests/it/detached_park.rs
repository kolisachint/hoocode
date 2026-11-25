#![allow(clippy::disallowed_methods)] // test code: std::process to run the real binary

//! A detached print run parks on a call that needs approval (SA-12): the call does not
//! run, the session file records the parked call, and the run exits with code 3. With the
//! mode's `auto_allow` naming the tool, the same run runs the call and ends done.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use hoocode_code_session::detached::{detached_status, DetachedStatus};

use crate::support::mock_llm::MockLlm;

/// Runs `hoocode -p start --session <work>/run.jsonl` detached against a scripted mock.
/// Returns the exit code, the work dir and the session file.
fn run_detached(
    script: Vec<serde_json::Value>,
    config: Option<serde_json::Value>,
) -> (Option<i32>, PathBuf, PathBuf, tempfile::TempDir) {
    let root = tempfile::Builder::new()
        .prefix("detached-park-")
        .tempdir()
        .unwrap();
    let base = root.path().canonicalize().unwrap();
    let (home, work) = (base.join("home"), base.join("work"));
    fs::create_dir_all(home.join(".hoocode")).unwrap();
    fs::create_dir_all(&work).unwrap();
    if let Some(config) = config {
        fs::write(home.join(".hoocode/hoo-config.json"), config.to_string()).unwrap();
    }

    let mock = MockLlm::start(script);
    let models = serde_json::json!({"providers": {"mock": {
        "baseUrl": format!("http://127.0.0.1:{}/v1", mock.port),
        "api": "openai-completions", "apiKey": "mock-key",
        "models": [{"id": "mock-model", "name": "Mock Model", "contextWindow": 128000, "maxTokens": 4096}],
    }}});
    fs::write(home.join(".hoocode/models.json"), models.to_string()).unwrap();

    let session = work.join("run.jsonl");
    let out = Command::new(env!("CARGO_BIN_EXE_hoocode"))
        .args([
            "--offline",
            "--provider",
            "mock",
            "--model",
            "mock-model",
            "-p",
            "start",
            "--session",
        ])
        .arg(&session)
        .current_dir(&work)
        .env_clear()
        .env("HOME", &home)
        .env(
            "PATH",
            std::env::var("PATH").unwrap_or("/usr/bin:/bin".into()),
        )
        .env("LANG", "C.UTF-8")
        .env("TZ", "UTC")
        .env("HOOCODE_DETACHED", "1")
        .stdin(Stdio::null())
        .output()
        .expect("run hoocode");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.code() == Some(3) {
        assert!(
            stderr.contains("parked: Shell needs approval; open with: hoocode --session "),
            "{stderr}"
        );
    }
    (out.status.code(), work, session, root)
}

fn touch_script() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({"tool_calls": [{"name": "Shell", "arguments": {"command": "touch parked_probe.txt"}}]}),
        serde_json::json!({"text": "done"}),
    ]
}

fn read_session(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

#[test]
fn detached_run_parks_on_a_gated_call_without_running_it() {
    let (code, work, session, _root) = run_detached(touch_script(), None);
    assert_eq!(code, Some(3));
    assert!(
        !work.join("parked_probe.txt").exists(),
        "the parked Shell call must not run"
    );
    let text = read_session(&session);
    assert!(text.contains("\"parked_call\""), "{text}");
    assert!(text.contains("\"toolCallId\""), "{text}");
    assert!(text.contains("\"parked\""), "{text}");
    assert_eq!(detached_status(&session), DetachedStatus::Parked);
}

#[test]
fn detached_run_with_auto_allow_runs_the_call_and_ends_done() {
    let config = serde_json::json!({"modes": {"build": {"auto_allow": ["Shell"]}}});
    let (code, work, session, _root) = run_detached(touch_script(), Some(config));
    assert_eq!(code, Some(0));
    assert!(work.join("parked_probe.txt").exists());
    assert_eq!(detached_status(&session), DetachedStatus::Done);
}

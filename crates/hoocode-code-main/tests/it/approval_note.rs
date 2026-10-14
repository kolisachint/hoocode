#![allow(clippy::disallowed_methods)] // test code: std::process to run the real binary

//! Print mode does not ask for tool approval, and says so on stderr after the
//! run, naming only the approval tools that ran (reliability 1.1). The model is
//! the in-process mock LLM (`support::mock_llm`), scripted.

use std::fs;
use std::process::{Command, Stdio};

use crate::support::mock_llm::MockLlm;

/// Run `hoocode -p start` against a scripted mock model; returns (stderr, work dir).
fn run_print(script: Vec<serde_json::Value>) -> (String, std::path::PathBuf, tempfile::TempDir) {
    let root = tempfile::Builder::new()
        .prefix("approval-note-")
        .tempdir()
        .unwrap();
    let base = root.path().canonicalize().unwrap();
    let (home, work) = (base.join("home"), base.join("work"));
    fs::create_dir_all(home.join(".hoocode")).unwrap();
    fs::create_dir_all(&work).unwrap();

    let mock = MockLlm::start(script);
    let models = serde_json::json!({"providers": {"mock": {
        "baseUrl": format!("http://127.0.0.1:{}/v1", mock.port),
        "api": "openai-completions", "apiKey": "mock-key",
        "models": [{"id": "mock-model", "name": "Mock Model", "contextWindow": 128000, "maxTokens": 4096}],
    }}});
    fs::write(home.join(".hoocode/models.json"), models.to_string()).unwrap();

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
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        out.status.success(),
        "exit {:?}: {stderr}",
        out.status.code()
    );
    (stderr, work, root)
}

fn approval_notes(stderr: &str) -> Vec<&str> {
    stderr
        .lines()
        .filter(|line| line.contains(" does not ask for tool approval"))
        .collect()
}

#[test]
fn print_names_the_gated_tool_that_ran() {
    let (stderr, work, _root) = run_print(vec![
        serde_json::json!({"tool_calls": [{"name": "Write", "arguments": {"path": "out.txt", "content": "hi"}}]}),
        serde_json::json!({"text": "all done"}),
    ]);
    assert!(
        work.join("out.txt").exists(),
        "the Write must run: {stderr}"
    );
    assert_eq!(
        approval_notes(&stderr),
        ["Note: --print does not ask for tool approval; these tools ran without it: Write."],
        "{stderr}"
    );
}

#[test]
fn print_has_no_approval_note_when_no_gated_tool_ran() {
    let (stderr, _work, _root) = run_print(vec![serde_json::json!({"text": "hello"})]);
    assert!(approval_notes(&stderr).is_empty(), "{stderr}");
}

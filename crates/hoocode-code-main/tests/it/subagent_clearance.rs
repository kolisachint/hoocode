#![allow(clippy::disallowed_methods)] // test code: std::process and a thread to read the child's stdout
//! A subagent child (`--mode json --task-id`) asks its parent to clear a gated
//! call over its pipes, and obeys the answer. The test plays the parent: it
//! reads the child's stdout, answers on its stdin, or closes stdin. The model
//! is the in-process mock LLM (`support::mock_llm`), scripted. The same wire is
//! what the pool speaks (`hoocode_code_subagents::clearance`).

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::support::mock_llm::MockLlm;

/// How the playing parent answers the child's first request.
enum Parent {
    Allow,
    Deny(&'static str),
    /// Closes the child's stdin instead of answering.
    Hangs,
}

struct Run {
    /// The clearance requests the child wrote.
    requests: Vec<Value>,
    /// Every request body the model received.
    model_requests: Vec<String>,
    work: PathBuf,
    exit_ok: bool,
    _root: tempfile::TempDir,
}

fn run_child(parent: Parent) -> Run {
    let root = tempfile::Builder::new()
        .prefix("subagent-clearance-")
        .tempdir()
        .unwrap();
    let base = root.path().canonicalize().unwrap();
    let (home, work) = (base.join("home"), base.join("work"));
    fs::create_dir_all(home.join(".hoocode")).unwrap();
    fs::create_dir_all(&work).unwrap();

    let mock = MockLlm::start(vec![
        json!({"tool_calls": [{"name": "Shell", "arguments": {"command": "echo relayed > out.txt"}}]}),
        json!({"text": "finished"}),
    ]);
    let models = json!({"providers": {"mock": {
        "baseUrl": format!("http://127.0.0.1:{}/v1", mock.port),
        "api": "openai-completions", "apiKey": "mock-key",
        "models": [{"id": "mock-model", "name": "Mock Model", "contextWindow": 128000, "maxTokens": 4096}],
    }}});
    fs::write(home.join(".hoocode/models.json"), models.to_string()).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_hoocode"))
        .args([
            "--offline",
            "--mode",
            "json",
            "--provider",
            "mock",
            "--model",
            "mock-model",
            "--task-id",
            "relay-test",
            "--session",
            "session.jsonl",
            "Task: run the command",
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
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run hoocode");

    let mut stdin = child.stdin.take();
    let stdout = child.stdout.take().unwrap();
    let (lines_tx, lines_rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if lines_tx.send(line).is_err() {
                break;
            }
        }
    });

    let mut requests = Vec::new();
    while let Ok(line) = lines_rx.recv_timeout(Duration::from_secs(60)) {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if value["type"] != "clearance_request" {
            continue;
        }
        let id = value["id"].as_u64().expect("a request id");
        requests.push(value);
        match (&parent, stdin.as_mut()) {
            (Parent::Allow, Some(pipe)) => {
                let answer = json!({"type": "clearance_response", "id": id, "allow": true});
                writeln!(pipe, "{answer}").unwrap();
                pipe.flush().unwrap();
            }
            (Parent::Deny(reason), Some(pipe)) => {
                let answer = json!({"type": "clearance_response", "id": id, "allow": false, "reason": reason});
                writeln!(pipe, "{answer}").unwrap();
                pipe.flush().unwrap();
            }
            _ => drop(stdin.take()),
        }
    }
    let status = child.wait().expect("wait for hoocode");
    Run {
        requests,
        model_requests: mock.requests(),
        work,
        exit_ok: status.success(),
        _root: root,
    }
}

#[test]
fn an_allowed_call_runs() {
    let run = run_child(Parent::Allow);
    assert_eq!(run.requests.len(), 1, "{:?}", run.requests);
    assert_eq!(run.requests[0]["tool"], "Shell");
    assert_eq!(
        run.requests[0]["input"]["command"],
        "echo relayed > out.txt"
    );
    assert!(
        run.work.join("out.txt").exists(),
        "the allowed call must run"
    );
    assert!(run.exit_ok);
}

#[test]
fn a_denied_call_does_not_run_and_the_model_hears_why() {
    let run = run_child(Parent::Deny("parent says no"));
    assert_eq!(run.requests.len(), 1, "{:?}", run.requests);
    assert!(!run.work.join("out.txt").exists(), "the denied call ran");
    assert!(
        run.model_requests
            .iter()
            .any(|body| body.contains("parent says no")),
        "the model was not told the reason: {:?}",
        run.model_requests
    );
    assert!(run.exit_ok, "a denial is a tool error, not a crash");
}

#[test]
fn a_parent_that_goes_away_means_denied() {
    let run = run_child(Parent::Hangs);
    assert_eq!(run.requests.len(), 1, "{:?}", run.requests);
    assert!(
        !run.work.join("out.txt").exists(),
        "no answer must never run the call"
    );
    assert!(
        run.model_requests
            .iter()
            .any(|body| body.contains("no longer reachable")),
        "{:?}",
        run.model_requests
    );
}

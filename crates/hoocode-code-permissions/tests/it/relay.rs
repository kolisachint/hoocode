//! A gate that cannot prompt (a subagent child) hands the calls that need
//! approval to its relay, and takes the relay's verdict as its own.

use std::sync::{Arc, Mutex};

use hoocode_code_permissions::{
    ApprovalChannel, ClearanceRelay, HeadlessClearanceRelay, HooPermissionGate, RelayRequest,
};
use serde_json::{json, Value};

/// A relay that records what it was asked and answers with a fixed verdict.
struct Answering {
    verdict: Option<String>,
    asked: Mutex<Vec<(String, Value, Option<String>)>>,
}

impl Answering {
    fn new(verdict: Option<&str>) -> Arc<Self> {
        Arc::new(Self {
            verdict: verdict.map(str::to_string),
            asked: Mutex::new(Vec::new()),
        })
    }

    fn asked(&self) -> Vec<(String, Value, Option<String>)> {
        self.asked.lock().unwrap().clone()
    }
}

impl ClearanceRelay for Answering {
    fn ask(&self, request: &RelayRequest) -> Option<String> {
        self.asked.lock().unwrap().push((
            request.tool.clone(),
            request.input.clone(),
            request.via.clone(),
        ));
        self.verdict.clone()
    }
}

fn gate(relay: &Arc<Answering>, cwd: &std::path::Path) -> HooPermissionGate {
    HooPermissionGate::new(cwd.to_path_buf(), ApprovalChannel::Ui, None)
        .with_relay(relay.clone() as Arc<dyn ClearanceRelay>)
}

#[test]
fn a_call_that_needs_approval_gets_the_relays_verdict() {
    let dir = tempfile::tempdir().unwrap();
    let input = json!({"command": "rm -rf build"});

    let deny = Answering::new(Some("Denied by permission gate: no"));
    assert_eq!(
        gate(&deny, dir.path()).check("Shell", &input).as_deref(),
        Some("Denied by permission gate: no")
    );
    assert_eq!(
        deny.asked(),
        [("Shell".to_string(), input.clone(), None)],
        "the relay is asked about the call as the model made it"
    );

    let allow = Answering::new(None);
    assert_eq!(gate(&allow, dir.path()).check("Shell", &input), None);
    assert_eq!(allow.asked().len(), 1);
}

#[test]
fn a_call_that_needs_no_approval_never_reaches_the_relay() {
    let dir = tempfile::tempdir().unwrap();
    let relay = Answering::new(Some("never"));
    let gate = gate(&relay, dir.path());
    assert_eq!(gate.check("Read", &json!({"path": "a.rs"})), None);
    assert!(relay.asked().is_empty());
}

#[test]
fn a_gate_with_neither_ui_nor_relay_denies() {
    let dir = tempfile::tempdir().unwrap();
    let gate = HooPermissionGate::new(dir.path().to_path_buf(), ApprovalChannel::Ui, None);
    let reason = gate
        .check("Shell", &json!({"command": "make build"}))
        .unwrap();
    assert!(
        reason.starts_with("Denied by permission gate: Shell needs approval"),
        "{reason}"
    );
}

fn request(tool: &str, input: Value) -> RelayRequest {
    RelayRequest {
        via: None,
        tool: tool.to_string(),
        origin: hoocode_agent_types::ToolOrigin::BuiltIn,
        input,
        cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    }
}

#[test]
fn a_headless_run_that_does_not_fail_closed_clears_a_subagents_gated_call() {
    // A json or print run has no UI and runs what needs approval, so its
    // subagent's gated fetch runs too.
    let dir = tempfile::tempdir().unwrap();
    let relay = HeadlessClearanceRelay::new(dir.path().to_path_buf(), false);
    let fetch = request("WebFetch", json!({"url": "https://example.com/"}));
    assert_eq!(relay.ask(&fetch), None);
}

#[test]
fn a_fail_closed_headless_run_denies_a_subagents_gated_call_with_the_gates_reason() {
    let dir = tempfile::tempdir().unwrap();
    let relay = HeadlessClearanceRelay::new(dir.path().to_path_buf(), true);
    let fetch = request("WebFetch", json!({"url": "https://example.com/"}));
    let reason = relay.ask(&fetch).expect("a fail-closed run denies it");
    assert!(
        reason.contains("needs approval") && reason.contains("no client can answer"),
        "{reason}"
    );
}

#[test]
fn a_subagents_call_the_run_denies_by_its_rules_is_denied_without_fail_closed() {
    // The run's own deny rule decides the subagent's call, though the run
    // itself would not prompt. A call the rules allow still runs.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".hoocode")).unwrap();
    let project = json!({"active_mode": "build", "modes": {"build": {
        "denied_bash_commands": ["\\brm\\b"],
    }}});
    std::fs::write(
        dir.path().join(".hoocode/hoo-config.json"),
        project.to_string(),
    )
    .unwrap();
    let relay = HeadlessClearanceRelay::new(dir.path().to_path_buf(), false);
    let reason = relay
        .ask(&request("Shell", json!({"command": "rm -rf build"})))
        .expect("the run's deny rule denies the subagent's call");
    assert!(reason.contains("denied pattern"), "{reason}");
    assert_eq!(
        relay.ask(&request("Shell", json!({"command": "make build"}))),
        None
    );
}

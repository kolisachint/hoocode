//! A gate that cannot prompt (a subagent child) hands the calls that need
//! approval to its relay, and takes the relay's verdict as its own.

use std::sync::{Arc, Mutex};

use hoocode_code_permissions::{ApprovalChannel, ClearanceRelay, HooPermissionGate, RelayRequest};
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
    assert_eq!(
        gate.check("Shell", &json!({"command": "ls"})).as_deref(),
        Some("Denied by permission gate")
    );
}

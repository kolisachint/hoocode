//! Agent identities: minting, tokens and the supervisor's parent lookup.
//!
//! Minting is a pure function of the parent identity, so subagent processes
//! without a supervisor use [`mint_agent`](hoocode_code_native::mint_agent)
//! directly; the supervisor resolves string parent ids for the root flow.

use hoocode_code_native::{mint_agent, new_token, AgentIdentity, Supervisor, SupervisorConfig};
use std::collections::HashMap;

#[test]
fn tokens_are_32_random_hex_bytes() {
    let first = new_token();
    assert_eq!(first.len(), 64);
    assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
    let second = new_token();
    assert_ne!(first, second);
}

#[test]
fn minting_nests_ids_and_depth() {
    let root = AgentIdentity::root("root-token");
    assert_eq!(root.agent_id, "root");
    assert_eq!(root.parent_id, None);
    assert_eq!(root.depth, 0);
    let child = mint_agent(&root, "task-1");
    assert_eq!(child.agent_id, "root/task-1");
    assert_eq!(child.parent_id, Some("root".to_owned()));
    assert_eq!(child.depth, 1);
    assert_eq!(child.token.len(), 64);
}

#[test]
fn call_meta_carries_the_identity() {
    let root = AgentIdentity::root("token");
    let meta = root.call_meta();
    let agent = meta
        .get(hoocode_code_native::AGENT_META_KEY)
        .expect("agent meta");
    assert_eq!(agent["agentId"], "root");
    assert_eq!(agent["parentId"], serde_json::Value::Null);
    assert_eq!(agent["depth"], 0);
}

#[test]
fn started_params_carry_the_token() {
    let root = AgentIdentity::root("token");
    let child = mint_agent(&root, "task");
    let params = child.started_params();
    assert_eq!(params["agentId"], "root/task");
    assert_eq!(params["token"], child.token);
}

/// The supervisor mints for the root and registered agents, and refuses
/// strangers. No helper starts: this runs on any platform.
#[test]
fn supervisor_mints_for_known_parents() {
    let dir = tempfile::tempdir().expect("tempdir");
    let supervisor = Supervisor::new(
        dir.path().join("home"),
        Vec::new(),
        HashMap::new(),
        SupervisorConfig::for_tests(),
    )
    .expect("supervisor");
    let child = supervisor.mint_agent("root", "a").expect("root mints");
    assert_eq!(child.agent_id, "root/a");
    assert_eq!(child.depth, 1);
    let error = supervisor
        .mint_agent("stranger", "a")
        .expect_err("unknown parent");
    assert!(
        matches!(error, hoocode_code_native::NativeError::UnknownAgent(_)),
        "{error:?}"
    );
    assert!(supervisor.agent("root").is_some());
    assert!(supervisor.agent("stranger").is_none());
    let _ = std::fs::remove_dir_all(supervisor.runtime_dir());
}

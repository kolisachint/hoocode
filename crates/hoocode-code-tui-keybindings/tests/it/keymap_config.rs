//! `keybindings.json`: the grouped and flat forms, contexts, unbinding, the
//! leader, and bad files. Every test uses its own temp directory.

use std::fs;
use std::time::{Duration, Instant};

use hoocode_code_tui_keybindings::{AppKeybindingsManager, Context, KeymapConfig};
use hoocode_tui_keys::{CancelReason, LeaderOutcome};
use tempfile::TempDir;

/// Writes `text` as `keybindings.json` in a fresh temp dir and loads it.
fn load(text: &str) -> (TempDir, AppKeybindingsManager) {
    let dir = tempfile::tempdir().expect("temp dir");
    fs::write(dir.path().join("keybindings.json"), text).expect("write keybindings.json");
    let manager = AppKeybindingsManager::create(Some(dir.path()));
    (dir, manager)
}

const CTRL_X: &str = "\x18";

#[test]
fn grouped_form_loads_leader_timeout_and_sequences() {
    let (_dir, m) = load(
        r#"{
            "leader": "ctrl+x",
            "leaderTimeoutMs": 800,
            "bindings": [
                {"context": "Prompt", "bindings": {"ctrl+x m": "app.model.cycleForward", "ctrl+r": null}}
            ]
        }"#,
    );
    assert!(m.warnings().is_empty(), "{:?}", m.warnings());
    assert_eq!(m.leader().id(), "ctrl+x");
    assert_eq!(m.leader_timeout(), Duration::from_millis(800));
    assert_eq!(
        m.action_for_key(Context::Prompt, "ctrl+x m").as_deref(),
        Some("app.model.cycleForward")
    );

    let mut machine = m.leader_machine(Context::Prompt);
    let t0 = Instant::now();
    assert_eq!(machine.handle(t0, CTRL_X), LeaderOutcome::Pending);
    assert_eq!(
        machine.handle(t0, "m"),
        LeaderOutcome::Run("app.model.cycleForward".into())
    );
}

#[test]
fn grouped_form_leader_timeout_drives_the_machine() {
    let (_dir, m) = load(r#"{"leaderTimeoutMs": 800}"#);
    let mut machine = m.leader_machine(Context::Prompt);
    let t0 = Instant::now();
    machine.handle(t0, CTRL_X);
    assert_eq!(machine.poll(t0 + Duration::from_millis(799)), None);
    assert_eq!(
        machine.poll(t0 + Duration::from_millis(800)),
        Some(CancelReason::Timeout)
    );
}

#[test]
fn a_non_default_leader_is_used_for_sequences() {
    let (_dir, m) = load(
        r#"{
            "leader": "ctrl+g",
            "bindings": [{"context": "Prompt", "bindings": {"ctrl+g m": "app.model.cycleForward"}}]
        }"#,
    );
    assert!(m.warnings().is_empty(), "{:?}", m.warnings());
    assert_eq!(m.leader().id(), "ctrl+g");
    let mut machine = m.leader_machine(Context::Prompt);
    let t0 = Instant::now();
    assert_eq!(machine.handle(t0, "\x07"), LeaderOutcome::Pending);
    assert_eq!(
        machine.handle(t0, "m"),
        LeaderOutcome::Run("app.model.cycleForward".into())
    );
}

#[test]
fn flat_form_still_works() {
    let (_dir, m) = load(
        r#"{
            "app.model.cycleForward": "alt+j",
            "app.clear": ["ctrl+k", "ctrl+u"]
        }"#,
    );
    assert!(m.warnings().is_empty(), "{:?}", m.warnings());
    assert_eq!(m.get_keys("app.model.cycleForward"), vec!["alt+j"]);
    assert_eq!(m.get_keys("app.clear"), vec!["ctrl+k", "ctrl+u"]);
    assert_eq!(
        m.leader().id(),
        "ctrl+x",
        "the flat form keeps the default leader"
    );
}

#[test]
fn flat_form_sequence_is_a_prompt_binding() {
    let (_dir, m) = load(r#"{"app.model.cycleForward": "ctrl+x m"}"#);
    assert!(m.warnings().is_empty(), "{:?}", m.warnings());
    assert_eq!(
        m.action_for_key(Context::Prompt, "ctrl+x m").as_deref(),
        Some("app.model.cycleForward")
    );
    assert_eq!(
        m.action_for_key(Context::Scroll, "ctrl+x m"),
        None,
        "a sequence set in the flat form applies to the Prompt"
    );
}

#[test]
fn flat_and_grouped_forms_can_be_mixed() {
    let (_dir, m) = load(
        r#"{
            "app.clear": "ctrl+k",
            "bindings": [{"context": "Prompt", "bindings": {"ctrl+x m": "app.model.cycleForward"}}]
        }"#,
    );
    assert!(m.warnings().is_empty(), "{:?}", m.warnings());
    assert_eq!(m.get_keys("app.clear"), vec!["ctrl+k"]);
    assert_eq!(
        m.action_for_key(Context::Prompt, "ctrl+x m").as_deref(),
        Some("app.model.cycleForward")
    );
}

#[test]
fn null_and_none_unbind_a_key_in_its_context_only() {
    let (_dir, m) = load(
        r#"{
            "bindings": [
                {"context": "Prompt", "bindings": {"ctrl+r": null}},
                {"context": "Scroll", "bindings": {"ctrl+c": "none"}}
            ]
        }"#,
    );
    assert!(m.warnings().is_empty(), "{:?}", m.warnings());
    // The default keeps ctrl+r for the transcript search. Prompt unbinds it.
    assert!(m
        .get_keys("app.scroll.search")
        .contains(&"ctrl+r".to_string()));
    assert_eq!(m.action_for_key(Context::Prompt, "ctrl+r"), None);
    assert!(m.action_for_key(Context::Scroll, "ctrl+r").is_some());
    assert_eq!(m.action_for_key(Context::Scroll, "ctrl+c"), None);
}

#[test]
fn a_global_unbind_applies_to_every_context() {
    let (_dir, m) = load(r#"{"bindings": [{"context": "Global", "bindings": {"ctrl+r": null}}]}"#);
    assert_eq!(m.action_for_key(Context::Prompt, "ctrl+r"), None);
    assert_eq!(m.action_for_key(Context::Scroll, "ctrl+r"), None);
}

#[test]
fn a_flat_null_unbinds_the_action() {
    let (_dir, m) = load(r#"{"app.model.cycleForward": null}"#);
    assert!(m.warnings().is_empty(), "{:?}", m.warnings());
    assert!(m.get_keys("app.model.cycleForward").is_empty());
}

#[test]
fn a_context_key_overrides_a_global_key() {
    let (_dir, m) = load(
        r#"{"bindings": [
            {"context": "Global", "bindings": {"ctrl+x m": "app.model.cycleForward"}},
            {"context": "Prompt", "bindings": {"ctrl+x m": null}}
        ]}"#,
    );
    assert_eq!(m.action_for_key(Context::Prompt, "ctrl+x m"), None);
    assert_eq!(
        m.action_for_key(Context::Scroll, "ctrl+x m").as_deref(),
        Some("app.model.cycleForward")
    );
    let mut machine = m.leader_machine(Context::Prompt);
    let t0 = Instant::now();
    machine.handle(t0, CTRL_X);
    assert_eq!(
        machine.handle(t0, "m"),
        LeaderOutcome::Cancelled(CancelReason::Unbound),
        "an unbound sequence is left out of the Prompt machine"
    );
}

#[test]
fn the_leader_context_takes_the_key_after_the_leader() {
    let (_dir, m) = load(
        r#"{"bindings": [{"context": "Leader", "bindings": {"m": "app.model.cycleForward"}}]}"#,
    );
    assert!(m.warnings().is_empty(), "{:?}", m.warnings());
    assert_eq!(
        m.action_for_key(Context::Leader, "m").as_deref(),
        Some("app.model.cycleForward")
    );
    let mut machine = m.leader_machine(Context::Global);
    let t0 = Instant::now();
    machine.handle(t0, CTRL_X);
    assert_eq!(
        machine.handle(t0, "m"),
        LeaderOutcome::Run("app.model.cycleForward".into())
    );
}

#[test]
fn a_printable_leader_is_rejected_and_the_default_is_used() {
    for leader in ["m", "/", "shift+a", "space"] {
        let text = format!(r#"{{"leader": "{leader}"}}"#);
        let (_dir, m) = load(&text);
        assert_eq!(m.leader().id(), "ctrl+x", "leader {leader} must fall back");
        assert!(
            m.warnings().iter().any(|w| w.contains("leader rejected")),
            "leader {leader} must warn: {:?}",
            m.warnings()
        );
    }
}

#[test]
fn a_sequence_must_start_with_the_leader() {
    let (_dir, m) = load(
        r#"{"bindings": [{"context": "Prompt", "bindings": {"alt+m m": "app.model.cycleForward"}}]}"#,
    );
    assert_eq!(m.action_for_key(Context::Prompt, "alt+m m"), None);
    assert!(
        m.warnings()
            .iter()
            .any(|w| w.contains("must start with the leader")),
        "{:?}",
        m.warnings()
    );
}

#[test]
fn an_unknown_context_is_skipped_with_a_warning() {
    let (_dir, m) = load(r#"{"bindings": [{"context": "Nope", "bindings": {"ctrl+r": null}}]}"#);
    assert!(m.warnings().iter().any(|w| w.contains("unknown context")));
    assert!(m
        .get_keys("app.scroll.search")
        .contains(&"ctrl+r".to_string()));
}

#[test]
fn an_unknown_action_is_reported() {
    let (_dir, m) = load(r#"{"app.not.real": "ctrl+y"}"#);
    assert!(m.warnings().iter().any(|w| w.contains("unknown action")));
}

#[test]
fn bad_json_does_not_crash_and_gives_the_defaults() {
    let (_dir, m) = load("{ this is not json");
    assert_eq!(m.leader().id(), "ctrl+x");
    assert!(m.warnings().iter().any(|w| w.contains("not valid JSON")));
    // The defaults still answer.
    assert!(m
        .get_keys("app.scroll.search")
        .contains(&"ctrl+r".to_string()));
}

#[test]
fn a_top_level_array_is_not_a_config() {
    let (_dir, m) = load("[1, 2, 3]");
    assert_eq!(m.leader().id(), "ctrl+x");
    assert!(m
        .warnings()
        .iter()
        .any(|w| w.contains("must be a JSON object")));
}

#[test]
fn malformed_parts_are_skipped_and_the_rest_still_applies() {
    let (_dir, m) = load(
        r#"{
            "leader": 7,
            "leaderTimeoutMs": -5,
            "app.model.cycleForward": 3,
            "app.clear": "ctrl+k",
            "bindings": "not a list"
        }"#,
    );
    assert_eq!(m.leader().id(), "ctrl+x");
    assert_eq!(m.leader_timeout(), Duration::from_millis(1500));
    assert_eq!(
        m.get_keys("app.clear"),
        vec!["ctrl+k"],
        "the valid entry still applies"
    );
    assert!(m.warnings().len() >= 4, "{:?}", m.warnings());
}

#[test]
fn a_group_or_binding_of_the_wrong_shape_is_skipped() {
    let (_dir, m) = load(
        r#"{"bindings": [
            "not an object",
            {"context": "Prompt", "bindings": ["not", "an", "object"]},
            {"context": "Prompt", "bindings": {"ctrl+r": 5}},
            {"context": "Prompt", "bindings": {"": "app.clear"}}
        ]}"#,
    );
    assert!(
        m.action_for_key(Context::Prompt, "ctrl+r").is_some(),
        "defaults survive bad groups"
    );
    assert!(m.warnings().len() >= 3, "{:?}", m.warnings());
}

#[test]
fn a_missing_file_gives_the_defaults_without_warnings() {
    let dir = tempfile::tempdir().expect("temp dir");
    let m = AppKeybindingsManager::create(Some(dir.path()));
    assert!(m.warnings().is_empty());
    assert_eq!(m.leader().id(), "ctrl+x");
    assert_eq!(m.leader_timeout(), Duration::from_millis(1500));
}

#[test]
fn reload_reads_the_file_again() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("keybindings.json");
    fs::write(&path, r#"{"leader": "ctrl+g"}"#).expect("write");
    let mut m = AppKeybindingsManager::create(Some(dir.path()));
    assert_eq!(m.leader().id(), "ctrl+g");

    fs::remove_file(&path).expect("remove");
    m.reload();
    assert_eq!(
        m.leader().id(),
        "ctrl+x",
        "a removed file gives the defaults"
    );
}

#[test]
fn the_config_parser_works_without_a_file() {
    let config = KeymapConfig::from_json_str(r#"{"leader": "ctrl+g", "leaderTimeoutMs": 900}"#);
    assert!(config.warnings.is_empty());
    assert_eq!(config.leader.id(), "ctrl+g");
    assert_eq!(config.leader_timeout, Duration::from_millis(900));
}

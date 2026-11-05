//! K-8b: a key name hoocode cannot match is an error, in both forms, and the
//! rest of the file still applies. `ctrl+foo` and `shfit+enter` must not be
//! read as something else. Every test uses its own temp directory.

use std::fs;

use hoocode_code_tui_keybindings::{check_file, AppKeybindingsManager};
use hoocode_tui_keys::{get_keybindings, is_known_key_id, KeySpec};

fn load(text: &str) -> (tempfile::TempDir, AppKeybindingsManager) {
    let dir = tempfile::tempdir().expect("temp dir");
    fs::write(dir.path().join("keybindings.json"), text).expect("write");
    let manager = AppKeybindingsManager::create(Some(dir.path()));
    (dir, manager)
}

fn reported(m: &AppKeybindingsManager, key: &str) -> bool {
    m.warnings()
        .iter()
        .any(|w| w.contains(key) && w.contains("is not a key"))
}

#[test]
fn unknown_key_names_are_recognized() {
    for key in [
        "ctrl+foo",
        "shfit+enter",
        "ctrl+",
        "+",
        "hyper+a",
        "ctrl+enterr",
        "ctrl+ab",
    ] {
        assert!(!is_known_key_id(key), "{key} should not be a key");
    }
}

#[test]
fn known_key_names_are_recognized() {
    for key in [
        "ctrl+x",
        "shift+tab",
        "alt+enter",
        "super+k",
        "escape",
        "pageUp",
        "ctrl+backspace",
        "f12",
        "ctrl+[",
        "ctrl+space",
        "G",
        "1",
        "shift+up",
        "ctrl+/",
    ] {
        assert!(is_known_key_id(key), "{key} should be a key");
    }
}

#[test]
fn a_misspelled_key_in_the_flat_form_is_an_error_and_the_default_stays() {
    let (_dir, m) = load(r#"{"app.model.cycleForward": "ctrl+foo"}"#);
    assert!(reported(&m, "ctrl+foo"), "{:?}", m.warnings());
    assert_eq!(
        m.get_keys("app.model.cycleForward"),
        vec!["ctrl+x m".to_string()]
    );
}

#[test]
fn a_misspelled_modifier_is_not_read_as_the_plain_key() {
    let (_dir, m) = load(
        r#"{"bindings": [{"context": "Prompt", "bindings": {"shfit+enter": "app.model.cycleForward"}}]}"#,
    );
    assert!(reported(&m, "shfit+enter"), "{:?}", m.warnings());
    // Plain Enter keeps its own binding, and the typo binds nothing.
    assert!(!get_keybindings().matches("\r", "app.model.cycleForward"));
    assert!(m.get_keys("app.panel.open").contains(&"enter".to_string()));
}

#[test]
fn a_bad_key_in_a_sequence_is_an_error() {
    let (_dir, m) = load(r#"{"app.model.cycleForward": "ctrl+x ctrl+foo"}"#);
    assert!(reported(&m, "ctrl+foo"), "{:?}", m.warnings());
    assert!(m.leader_entries().iter().all(|e| e.key != "ctrl+foo"));
}

#[test]
fn a_bad_key_in_a_list_skips_only_that_key() {
    let (_dir, m) = load(r#"{"app.model.cycleForward": ["ctrl+foo", "ctrl+y"]}"#);
    assert!(reported(&m, "ctrl+foo"), "{:?}", m.warnings());
    assert_eq!(
        m.get_keys("app.model.cycleForward"),
        vec!["ctrl+y".to_string()]
    );
}

#[test]
fn a_bad_leader_name_keeps_the_default_leader() {
    let (_dir, m) = load(r#"{"leader": "ctrl+foo"}"#);
    assert_eq!(m.leader().id(), "ctrl+x");
    assert!(reported(&m, "ctrl+foo"), "{:?}", m.warnings());
}

#[test]
fn check_reports_the_bad_name_as_an_error() {
    let dir = tempfile::tempdir().expect("temp dir");
    fs::write(
        dir.path().join("keybindings.json"),
        r#"{"bindings": [{"context": "Prompt", "bindings": {"shfit+enter": "app.model.cycleForward"}}]}"#,
    )
    .expect("write");
    let report = check_file(dir.path());
    assert!(
        report.errors.iter().any(|e| e.contains("shfit+enter")),
        "{:?}",
        report.errors
    );
}

#[test]
fn the_key_spec_still_accepts_what_it_always_did() {
    // The matcher is unchanged: the strict check is only for the file.
    assert!(KeySpec::parse("ctrl+foo").is_some());
}

//! K-8 validation of `keybindings.json`: parse errors, unknown ids, duplicate
//! keys in one context, reserved keys (skipped), warned keys (kept with a
//! caution), comments, and bad input that must never crash. Temp dirs only.

use std::fs;

use hoocode_code_tui_keybindings::{check_file, AppKeybindingsManager, RESERVED_KEYS, WARNED_KEYS};

fn load(text: &str) -> (tempfile::TempDir, AppKeybindingsManager) {
    let dir = tempfile::tempdir().expect("temp dir");
    fs::write(dir.path().join("keybindings.json"), text).expect("write");
    let manager = AppKeybindingsManager::create(Some(dir.path()));
    (dir, manager)
}

fn any_contains(items: &[String], parts: &[&str]) -> bool {
    items
        .iter()
        .any(|item| parts.iter().all(|part| item.contains(part)))
}

#[test]
fn every_reserved_key_is_skipped_with_an_error_in_the_flat_form() {
    for key in RESERVED_KEYS {
        let text = format!(r#"{{"app.model.cycleForward": "{key}"}}"#);
        let (_dir, m) = load(&text);
        assert!(
            any_contains(m.warnings(), &[key, "reserved"]),
            "{key}: {:?}",
            m.warnings()
        );
        assert!(
            !m.get_keys("app.model.cycleForward")
                .iter()
                .any(|k| k == key),
            "{key} must not be bound"
        );
    }
}

#[test]
fn a_reserved_key_in_the_grouped_form_is_skipped_too() {
    let (_dir, m) = load(
        r#"{"bindings": [{"context": "Prompt", "bindings": {"ctrl+h": "app.model.cycleForward"}}]}"#,
    );
    assert!(any_contains(m.warnings(), &["ctrl+h", "reserved"]));
    assert!(m
        .action_for_key(hoocode_code_tui_keybindings::Context::Prompt, "ctrl+h")
        .is_none());
}

#[test]
fn a_reserved_leader_keeps_the_default_leader_with_an_error() {
    let (_dir, m) = load(r#"{"leader": "ctrl+c"}"#);
    assert_eq!(m.leader().id(), "ctrl+x");
    assert!(any_contains(m.warnings(), &["ctrl+c", "reserved"]));
}

#[test]
fn a_warned_key_is_kept_with_a_caution_not_an_error() {
    for key in WARNED_KEYS {
        let text = format!(r#"{{"app.model.cycleForward": "{key}"}}"#);
        let (_dir, m) = load(&text);
        assert!(m.warnings().is_empty(), "{key}: {:?}", m.warnings());
        assert!(
            any_contains(m.cautions(), &[key]),
            "{key}: {:?}",
            m.cautions()
        );
        assert!(m
            .get_keys("app.model.cycleForward")
            .iter()
            .any(|k| k == key));
    }
}

#[test]
fn unknown_ids_and_parse_errors_are_reported_and_the_rest_applies() {
    let (_dir, m) = load(
        r#"{"app.nope": "ctrl+y", "app.model.cycleForward": "", "app.model.cycleBackward": "ctrl+y"}"#,
    );
    assert!(
        any_contains(m.warnings(), &["app.nope", "unknown"]),
        "{:?}",
        m.warnings()
    );
    assert!(
        any_contains(m.warnings(), &["is not a key"]),
        "{:?}",
        m.warnings()
    );
    assert!(m
        .get_keys("app.model.cycleBackward")
        .iter()
        .any(|k| k == "ctrl+y"));
}

#[test]
fn the_same_key_twice_in_one_context_keeps_the_later_binding_and_reports_it() {
    let (_dir, m) = load(
        r#"{"bindings": [
            {"context": "Prompt", "bindings": {"ctrl+y": "app.model.cycleForward"}},
            {"context": "Prompt", "bindings": {"ctrl+y": "app.model.cycleBackward"}}
        ]}"#,
    );
    assert!(
        any_contains(m.warnings(), &["ctrl+y", "bound twice"]),
        "{:?}",
        m.warnings()
    );
    assert_eq!(
        m.action_for_key(hoocode_code_tui_keybindings::Context::Prompt, "ctrl+y")
            .as_deref(),
        Some("app.model.cycleBackward")
    );
}

#[test]
fn the_same_key_in_two_contexts_is_not_a_duplicate() {
    let (_dir, m) = load(
        r#"{"bindings": [
            {"context": "Prompt", "bindings": {"ctrl+y": "app.model.cycleForward"}},
            {"context": "Scroll", "bindings": {"ctrl+y": "app.scroll.lineDown"}}
        ]}"#,
    );
    // Each action runs in its own context, so both bindings are kept.
    assert!(m.warnings().is_empty(), "{:?}", m.warnings());
}

#[test]
fn a_flat_key_listed_for_two_actions_is_reported() {
    let (_dir, m) =
        load(r#"{"app.model.cycleForward": "ctrl+y", "app.model.cycleBackward": "ctrl+y"}"#);
    assert!(
        any_contains(
            m.warnings(),
            &[
                "ctrl+y",
                "app.model.cycleBackward and app.model.cycleForward"
            ]
        ),
        "{:?}",
        m.warnings()
    );
}

#[test]
fn dollar_comments_are_ignored() {
    let (_dir, m) = load(
        r#"{"$comment": "notes", "$reserved": {"ctrl+c": ["app.clear"]}, "app.model.cycleForward": "ctrl+y"}"#,
    );
    assert!(m.warnings().is_empty(), "{:?}", m.warnings());
    assert!(m
        .get_keys("app.model.cycleForward")
        .iter()
        .any(|k| k == "ctrl+y"));
}

#[test]
fn odd_input_never_panics_and_is_reported() {
    let cases = [
        "[]",
        "42",
        "{",
        r#"{"bindings": 3}"#,
        r#"{"bindings": [{"context": "Nope"}]}"#,
        r#"{"bindings": ["x"]}"#,
        r#"{"leader": ""}"#,
        r#"{"leader": 5}"#,
        r#"{"leaderTimeoutMs": 0}"#,
        r#"{"bindings": [{"context": "Prompt", "bindings": {"": "app.model.cycleForward", " ": null}}]}"#,
        r#"{"bindings": [{"context": "Prompt", "bindings": {"ctrl+x": "app.model.cycleForward"}}]}"#,
        r#"{"app.model.cycleForward": {"nested": true}}"#,
        "\u{feff}{}",
    ];
    for text in cases {
        let m = AppKeybindingsManager::from_json_str(text);
        assert!(!m.warnings().is_empty(), "expected a report for {text:?}");
    }
}

#[test]
fn a_file_that_cannot_be_read_gives_the_defaults_and_a_report() {
    let dir = tempfile::tempdir().expect("temp dir");
    fs::create_dir(dir.path().join("keybindings.json")).expect("a directory in the way");
    let m = AppKeybindingsManager::create(Some(dir.path()));
    assert!(
        any_contains(m.warnings(), &["cannot read"]),
        "{:?}",
        m.warnings()
    );
    assert_eq!(m.leader().id(), "ctrl+x");
}

#[test]
fn check_reports_a_missing_file_as_fine() {
    let dir = tempfile::tempdir().expect("temp dir");
    let report = check_file(dir.path());
    assert!(!report.exists);
    assert!(report.errors.is_empty());
    assert!(report.cautions.is_empty());
    assert_eq!(report.path, dir.path().join("keybindings.json"));
}

#[test]
fn check_splits_errors_from_cautions() {
    let dir = tempfile::tempdir().expect("temp dir");
    fs::write(
        dir.path().join("keybindings.json"),
        r#"{"app.nope": "ctrl+s", "app.model.cycleForward": "ctrl+c", "app.model.cycleBackward": "ctrl+q"}"#,
    )
    .expect("write");
    let report = check_file(dir.path());
    assert!(report.exists);
    assert!(any_contains(&report.errors, &["app.nope"]));
    assert!(any_contains(&report.errors, &["ctrl+c", "reserved"]));
    assert!(any_contains(&report.cautions, &["ctrl+q"]));
    assert!(!any_contains(&report.errors, &["ctrl+q"]));
}

#[test]
fn a_multi_step_leader_keeps_the_default_leader_with_an_error() {
    let (_dir, m) = load(r#"{"leader": "ctrl+x m"}"#);
    assert_eq!(m.leader().id(), "ctrl+x");
    assert!(any_contains(m.warnings(), &["ctrl+x m", "sequence"]));
}

#[test]
fn an_esc_enter_or_tab_leader_keeps_the_default_leader_with_an_error() {
    for leader in [
        "escape",
        "enter",
        "return",
        "tab",
        "backspace",
        "shift+enter",
    ] {
        let (_dir, m) = load(&format!(r#"{{"leader": "{leader}"}}"#));
        assert_eq!(m.leader().id(), "ctrl+x", "{leader} must not be the leader");
        assert!(
            any_contains(m.warnings(), &[leader, "cannot be the leader"]),
            "{leader}: {:?}",
            m.warnings()
        );
    }
}

#[test]
fn a_leader_that_shadows_a_default_binding_is_warned() {
    let (_dir, m) = load(r#"{"leader": "ctrl+r"}"#);
    assert_eq!(m.leader().id(), "ctrl+r", "the leader is kept");
    assert!(any_contains(
        m.warnings(),
        &["ctrl+r", "shadows", "app.history.search"]
    ));
}

#[test]
fn a_flat_single_binding_equal_to_the_leader_is_skipped_with_a_warning() {
    let (_dir, m) = load(r#"{"app.model.cycleForward": "ctrl+x"}"#);
    assert!(
        any_contains(m.warnings(), &["ctrl+x", "leader key"]),
        "{:?}",
        m.warnings()
    );
    assert!(!m
        .get_keys("app.model.cycleForward")
        .contains(&"ctrl+x".to_string()));
}

#[test]
fn a_reserved_key_after_the_leader_is_rejected_in_both_forms() {
    // Leader context: the key typed after the leader is checked.
    let (_dir, m) = load(
        r#"{"bindings": [{"context": "Leader", "bindings": {"ctrl+c": "app.model.cycleForward"}}]}"#,
    );
    assert!(any_contains(m.warnings(), &["ctrl+c", "reserved"]));
    assert!(!m
        .leader_table(hoocode_code_tui_keybindings::Context::Prompt)
        .contains_key("ctrl+x ctrl+c"));

    // Flat sequence: ctrl+x ctrl+c is refused the same way.
    let (_dir, m) = load(r#"{"app.model.cycleForward": "ctrl+x ctrl+c"}"#);
    assert!(any_contains(m.warnings(), &["ctrl+c", "reserved"]));
    assert!(!m
        .leader_table(hoocode_code_tui_keybindings::Context::Prompt)
        .contains_key("ctrl+x ctrl+c"));
}

#[test]
fn a_flat_key_that_another_action_uses_by_default_is_warned_with_both_names() {
    let (_dir, m) = load(r#"{"app.model.cycleForward": "ctrl+r"}"#);
    assert!(
        any_contains(
            m.warnings(),
            &["ctrl+r", "app.model.cycleForward", "app.history.search"]
        ),
        "{:?}",
        m.warnings()
    );
}

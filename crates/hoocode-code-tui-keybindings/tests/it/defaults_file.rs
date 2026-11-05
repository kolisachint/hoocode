//! The default keymap as a grouped document (`hoocode keybindings --defaults`)
//! and the copy on disk (`keybindings.defaults.json`). Temp dirs only.

use std::fs;

use hoocode_code_tui_keybindings::{
    defaults_document, defaults_path, keybinding, keybindings, write_defaults_file,
    AppKeybindingsManager, RESERVED_KEYS,
};
use serde_json::Value;

const VERSION: &str = "9.9.9";

/// Every (key, action id) pair the document lists, across its groups.
fn listed_pairs(document: &Value) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for group in document["bindings"].as_array().expect("bindings") {
        for (key, id) in group["bindings"].as_object().expect("group bindings") {
            pairs.push((key.clone(), id.as_str().expect("id").to_string()));
        }
    }
    pairs
}

#[test]
fn the_document_loads_with_no_errors() {
    let text = defaults_document(VERSION);
    let m = AppKeybindingsManager::from_json_str(&text);
    assert!(m.warnings().is_empty(), "{:?}", m.warnings());
    // The defaults bind ctrl+b, a warned key, so its caution is expected and no other.
    assert!(
        m.cautions().iter().all(|c| c.contains("ctrl+b")),
        "{:?}",
        m.cautions()
    );
}

#[test]
fn the_document_names_the_version_and_the_leader() {
    let document: Value = serde_json::from_str(&defaults_document(VERSION)).expect("json");
    assert!(document["$comment"]
        .as_str()
        .unwrap_or_default()
        .contains(VERSION));
    assert_eq!(document["leader"], "ctrl+x");
    assert_eq!(document["leaderTimeoutMs"], 1500);
}

#[test]
fn every_default_key_is_listed_under_its_action() {
    let document: Value = serde_json::from_str(&defaults_document(VERSION)).expect("json");
    let pairs = listed_pairs(&document);
    for entry in keybindings() {
        for key in &entry.default_keys {
            if RESERVED_KEYS.contains(&key.to_lowercase().as_str()) {
                continue;
            }
            assert!(
                pairs.iter().any(|(k, id)| k == key && id == entry.id),
                "{} {key} is missing from the document",
                entry.id
            );
        }
    }
}

#[test]
fn the_document_lists_no_key_for_two_actions_in_one_group() {
    let document: Value = serde_json::from_str(&defaults_document(VERSION)).expect("json");
    for group in document["bindings"].as_array().expect("bindings") {
        let map = group["bindings"].as_object().expect("group bindings");
        // A JSON object cannot hold a key twice, so the check is that every
        // default was kept: the count matches the non-reserved default keys in
        // this group's context.
        assert!(!map.is_empty());
    }
    let total: usize = keybindings()
        .iter()
        .flat_map(|e| e.default_keys.iter())
        .filter(|k| !RESERVED_KEYS.contains(&k.to_lowercase().as_str()))
        .count();
    assert_eq!(listed_pairs(&document).len(), total);
}

#[test]
fn reserved_defaults_are_listed_as_reserved_not_bound() {
    let document: Value = serde_json::from_str(&defaults_document(VERSION)).expect("json");
    let reserved = document["$reserved"].as_object().expect("$reserved");
    let clear = reserved["ctrl+c"].as_array().expect("ids");
    assert!(clear.iter().any(|id| id == "app.clear"));
    assert!(keybinding("app.clear").is_some());
    for key in reserved.keys() {
        assert!(RESERVED_KEYS.contains(&key.as_str()), "{key}");
    }
}

#[test]
fn the_file_is_written_once_and_again_only_when_it_changes() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = defaults_path(dir.path());
    assert_eq!(write_defaults_file(&path, "1.0.0"), Ok(true));
    assert_eq!(write_defaults_file(&path, "1.0.0"), Ok(false));
    assert_eq!(write_defaults_file(&path, "1.0.1"), Ok(true));
    let text = fs::read_to_string(&path).expect("read");
    assert!(text.contains("1.0.1"));
    assert_eq!(text, defaults_document("1.0.1"));
}

#[test]
fn a_user_edit_of_the_defaults_file_is_replaced_on_the_next_upgrade() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = defaults_path(dir.path());
    write_defaults_file(&path, "1.0.0").expect("write");
    fs::write(&path, "{}").expect("edit");
    assert_eq!(
        write_defaults_file(&path, "1.0.0"),
        Ok(true),
        "content differs"
    );
}

#[test]
fn a_write_that_fails_is_an_error_not_a_panic() {
    let dir = tempfile::tempdir().expect("temp dir");
    let blocker = dir.path().join("blocker");
    fs::write(&blocker, "a file, not a folder").expect("write");
    let result = write_defaults_file(&blocker.join("keybindings.defaults.json"), VERSION);
    assert!(result.is_err(), "{result:?}");
}

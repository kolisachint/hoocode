//! The Alt migration notice: shown once per version, never rewrites the file.
//! Temp dirs only.

use std::fs;

use hoocode_code_tui_keybindings::{
    alt_keys, take_alt_notice, AppKeybindingsManager, STATE_FILE_NAME,
};

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn alt_keys_finds_alt_in_any_step_without_repeats() {
    assert_eq!(
        alt_keys(&strings(&[
            "alt+m",
            "ctrl+x m",
            "shift+alt+z",
            "alt+m",
            "ctrl+alt+k"
        ])),
        strings(&["alt+m", "shift+alt+z", "ctrl+alt+k"])
    );
    assert!(alt_keys(&strings(&["ctrl+x m", "altitude"])).is_empty());
}

#[test]
fn the_notice_shows_once_per_version() {
    let dir = tempfile::tempdir().expect("temp dir");
    let state = dir.path().join(STATE_FILE_NAME);
    let bound = strings(&["alt+m", "alt+t"]);
    let first = take_alt_notice(&state, "1.0.0", &bound).expect("first start shows it");
    assert!(first.contains("alt+m, alt+t"), "{first}");
    assert!(
        take_alt_notice(&state, "1.0.0", &bound).is_none(),
        "same version: quiet"
    );
    assert!(
        take_alt_notice(&state, "1.0.1", &bound).is_some(),
        "new version: shown again"
    );
    assert!(take_alt_notice(&state, "1.0.1", &bound).is_none());
}

#[test]
fn a_file_without_alt_keys_shows_nothing_and_writes_nothing() {
    let dir = tempfile::tempdir().expect("temp dir");
    let state = dir.path().join(STATE_FILE_NAME);
    assert!(take_alt_notice(&state, "1.0.0", &strings(&["ctrl+x m"])).is_none());
    assert!(!state.exists());
}

#[test]
fn a_state_file_that_is_not_json_counts_as_not_shown() {
    let dir = tempfile::tempdir().expect("temp dir");
    let state = dir.path().join(STATE_FILE_NAME);
    fs::write(&state, "not json").expect("write");
    assert!(take_alt_notice(&state, "1.0.0", &strings(&["alt+m"])).is_some());
}

#[test]
fn the_notice_reads_the_keys_the_user_file_lists_and_never_rewrites_it() {
    let dir = tempfile::tempdir().expect("temp dir");
    let text = r#"{
        "app.model.cycleForward": "alt+m",
        "bindings": [{"context": "Prompt", "bindings": {"alt+t": "app.thinking.toggle"}}]
    }"#;
    fs::write(dir.path().join("keybindings.json"), text).expect("write");
    let manager = AppKeybindingsManager::create(Some(dir.path()));
    let bound = manager.bound_keys();
    assert!(bound.contains(&"alt+m".to_string()) && bound.contains(&"alt+t".to_string()));

    let state = dir.path().join(STATE_FILE_NAME);
    assert!(take_alt_notice(&state, "1.0.0", &bound).is_some());
    let after = fs::read_to_string(dir.path().join("keybindings.json")).expect("read");
    assert_eq!(after, text, "the user's file is not rewritten");
}

//! K-8a: single keys in the grouped form reach the live keymap, per context.
//! A key is checked with `matches` on the manager a `/reload` installs, the
//! same call the screens make. Every test uses its own temp directory.

use std::fs;

use hoocode_code_tui_keybindings::AppKeybindingsManager;
use hoocode_tui_keys::get_keybindings;
use tempfile::TempDir;

/// Writes `text` as `keybindings.json`, loads it and installs it as the live keymap.
fn installed(text: &str) -> (TempDir, AppKeybindingsManager) {
    let dir = tempfile::tempdir().expect("temp dir");
    fs::write(dir.path().join("keybindings.json"), text).expect("write keybindings.json");
    let manager = AppKeybindingsManager::create(Some(dir.path()));
    manager.install();
    (dir, manager)
}

#[test]
fn a_null_unbinds_a_plain_key_only_in_its_context() {
    let (_dir, m) = installed(r#"{"bindings": [{"context": "Scroll", "bindings": {"j": null}}]}"#);
    assert!(m.warnings().is_empty(), "{:?}", m.warnings());
    let live = get_keybindings();
    // Scroll gives up j. The picker's own j (list down) keeps it.
    assert!(!live.matches("j", "app.scroll.lineDown"));
    assert!(live.matches("j", "app.list.down"));
}

#[test]
fn none_unbinds_a_plain_key_too() {
    let (_dir, _m) =
        installed(r#"{"bindings": [{"context": "Dialog", "bindings": {"n": "none"}}]}"#);
    let live = get_keybindings();
    assert!(!live.matches("n", "app.dialog.no"));
    assert!(
        live.matches("y", "app.dialog.yes"),
        "other dialog keys stay"
    );
}

#[test]
fn a_global_null_applies_to_every_context() {
    let (_dir, _m) = installed(r#"{"bindings": [{"context": "Global", "bindings": {"q": null}}]}"#);
    let live = get_keybindings();
    assert!(!live.matches("q", "app.scroll.exit"));
    assert!(!live.matches("q", "app.list.close"));
}

#[test]
fn a_global_binding_takes_the_key_only_from_global_actions() {
    // app.interrupt is the one Global-context action. Escape stays with the
    // Picker, Scroll and Panel actions that use it.
    let (_dir, m) = installed(
        r#"{"bindings": [{"context": "Global", "bindings": {"escape": "app.interrupt"}}]}"#,
    );
    assert!(m.warnings().is_empty(), "{:?}", m.warnings());
    let live = get_keybindings();
    assert!(live.matches("\x1b", "app.interrupt"));
    assert!(live.matches("\x1b", "tui.select.cancel"));
    assert!(live.matches("\x1b", "app.scroll.exit"));
    assert!(live.matches("\x1b", "app.panel.exit"));
}

#[test]
fn a_capital_letter_in_the_file_is_shift_and_moves_its_key() {
    // "G" in keybindings.json is Shift+g, the key that types a capital G, the
    // same key as the default. Binding it to top takes it from the bottom action.
    let (_dir, m) =
        installed(r#"{"bindings": [{"context": "Scroll", "bindings": {"G": "app.scroll.top"}}]}"#);
    assert!(m.warnings().is_empty(), "{:?}", m.warnings());
    let live = get_keybindings();
    assert!(live.matches("G", "app.scroll.top"));
    assert!(!live.matches("G", "app.scroll.bottom"));
    assert!(live.matches("g", "app.scroll.top"), "the default g stays");
}

#[test]
fn a_single_key_binding_adds_the_key_to_its_action() {
    let (_dir, m) = installed(
        r#"{"bindings": [{"context": "Scroll", "bindings": {"u": "app.scroll.lineDown"}}]}"#,
    );
    assert!(m.warnings().is_empty(), "{:?}", m.warnings());
    let live = get_keybindings();
    assert!(live.matches("u", "app.scroll.lineDown"));
    assert!(
        live.matches("j", "app.scroll.lineDown"),
        "the default stays"
    );
    assert!(m.get_keys("app.scroll.lineDown").contains(&"u".to_string()));
}

#[test]
fn a_key_moved_to_another_action_in_its_context_leaves_the_old_one() {
    let (_dir, _m) =
        installed(r#"{"bindings": [{"context": "Dialog", "bindings": {"y": "app.dialog.no"}}]}"#);
    let live = get_keybindings();
    assert!(live.matches("y", "app.dialog.no"));
    assert!(!live.matches("y", "app.dialog.yes"));
}

#[test]
fn a_binding_to_an_action_of_another_context_is_skipped_with_a_warning() {
    let (_dir, m) =
        installed(r#"{"bindings": [{"context": "Scroll", "bindings": {"u": "app.dialog.yes"}}]}"#);
    assert!(
        m.warnings()
            .iter()
            .any(|w| w.contains("app.dialog.yes") && w.contains("skipped")),
        "{:?}",
        m.warnings()
    );
    assert!(!get_keybindings().matches("u", "app.dialog.yes"));
}

#[test]
fn a_reload_replaces_the_live_single_keys() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("keybindings.json");
    fs::write(
        &file,
        r#"{"bindings": [{"context": "Dialog", "bindings": {"y": null}}]}"#,
    )
    .expect("write");
    let mut m = AppKeybindingsManager::create(Some(dir.path()));
    m.install();
    assert!(!get_keybindings().matches("y", "app.dialog.yes"));

    fs::write(&file, r#"{}"#).expect("write");
    m.reload();
    m.install();
    assert!(get_keybindings().matches("y", "app.dialog.yes"));
}

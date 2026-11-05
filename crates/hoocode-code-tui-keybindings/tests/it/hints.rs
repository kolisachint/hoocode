//! K-9: key hints come from the keymap. A rebound key shows its new name, the
//! arrows print as glyphs, and before a keymap is installed the app's defaults
//! stand in. Each test installs its own keymap on its own thread.

use std::fs;

use hoocode_code_tui_keybindings::{
    key_glyph_text, key_hint_text, key_pair_hint, AppKeybindingsManager, SPACE_KEY,
};

fn install(text: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    fs::write(dir.path().join("keybindings.json"), text).expect("write keybindings.json");
    AppKeybindingsManager::create(Some(dir.path())).install();
    dir
}

#[test]
fn an_uninstalled_keymap_shows_the_app_defaults() {
    assert_eq!(key_hint_text("app.interrupt"), "esc");
    assert_eq!(key_hint_text("tui.select.confirm"), "enter");
    assert_eq!(key_hint_text("app.list.help"), "f1");
    assert_eq!(key_hint_text("app.model.cycleForward"), "ctrl+x m");
}

#[test]
fn a_rebound_key_shows_the_new_key() {
    // A key added beside the default keeps the default first, so the hint
    // names the taught key: the default is taken away here.
    let _dir = install(
        r#"{"bindings": [{"context": "Picker", "bindings": {"enter": null, "ctrl+y": "tui.select.confirm"}}]}"#,
    );
    assert_eq!(key_hint_text("tui.select.confirm"), "ctrl+y");
}

#[test]
fn a_bare_arrow_pair_prints_as_glyphs() {
    let _dir = install("{}");
    assert_eq!(key_pair_hint("tui.select.up", "tui.select.down"), "↑↓");
    assert_eq!(
        key_pair_hint("app.panel.prevTab", "app.panel.nextTab"),
        "←→"
    );
    assert_eq!(key_glyph_text("app.options.back"), "←");
}

#[test]
fn a_pair_of_other_keys_prints_as_names() {
    let _dir = install("{}");
    assert_eq!(
        key_pair_hint("app.model.cycleForward", "app.model.cycleBackward"),
        "ctrl+x m/ctrl+x M"
    );
}

#[test]
fn the_space_key_is_one_name() {
    assert_eq!(SPACE_KEY, "space");
    assert_eq!(hoocode_tui_keys::first_key_name("app.interrupt"), "");
}

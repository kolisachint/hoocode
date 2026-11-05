//! The which-key popup: its rows fit the width, its columns read down first,
//! its golden, and its entries come from the keymap (a user's leader keys show).

use hoocode_code_tui_keybindings::{
    format_key_text, leader_label, which_key_rows, AppKeybindingsManager, LeaderEntry,
    WhichKeyPopup,
};
use hoocode_tui_render::golden::render_golden;
use hoocode_tui_render::{assert_golden, Component};

fn entry(key: &str, action: &str, description: &str) -> LeaderEntry {
    LeaderEntry {
        key: key.into(),
        action: action.into(),
        description: description.into(),
    }
}

fn fixture() -> Vec<LeaderEntry> {
    vec![
        entry("/", "app.scroll.search", "Search the transcript"),
        entry("b", "app.thinking.toggle", "Toggle thinking blocks"),
        entry("e", "app.editor.external", "Open external editor"),
        entry("m", "app.model.cycleForward", "Cycle to next model"),
    ]
}

/// The theme the golden uses (dark, truecolor), as the other goldens set it.
fn setup() {
    std::env::set_var("COLORTERM", "truecolor");
    hoocode_code_tui_theme::init_theme(Some("dark"), false);
}

#[test]
fn no_entries_gives_no_rows() {
    setup();
    assert!(which_key_rows(&[], 80).is_empty());
}

#[test]
fn every_row_fits_the_width() {
    setup();
    for width in [12, 24, 40, 60, 80, 120] {
        for row in which_key_rows(&fixture(), width) {
            let plain = hoocode_tui_util::strip_vt_control_characters(&row);
            assert!(
                plain.chars().count() <= width,
                "width {width}: {plain:?} is too wide"
            );
        }
    }
}

#[test]
fn columns_read_down_first() {
    setup();
    // At 80 columns the four entries sit in two columns of two rows.
    let rows: Vec<String> = which_key_rows(&fixture(), 80)
        .iter()
        .map(|row| hoocode_tui_util::strip_vt_control_characters(row))
        .collect();
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert!(rows[0].starts_with(" / "), "{rows:?}");
    assert!(rows[0].contains("Open external editor"), "{rows:?}");
    assert!(rows[1].starts_with(" b "), "{rows:?}");
}

#[test]
fn the_popup_golden_at_60_columns() {
    setup();
    let mut popup = WhichKeyPopup::new(fixture());
    let rendered = render_golden(&mut popup, 60);
    assert_golden!("which_key_popup_60", rendered);
}

#[test]
fn the_popup_follows_the_users_leader_keys() {
    setup();
    let keys = AppKeybindingsManager::from_json_str(
        r#"{"leader": "ctrl+g", "bindings": [{"context": "Prompt", "bindings": {
            "ctrl+g z": "app.hotkeys.open",
            "ctrl+g m": null
        }}]}"#,
    );
    assert!(keys.warnings().is_empty(), "{:?}", keys.warnings());
    let entries = keys.leader_entries();
    assert!(
        entries
            .iter()
            .any(|e| e.key == "z" && e.description == "Show keyboard shortcuts"),
        "{entries:?}"
    );
    assert!(
        entries.iter().all(|e| e.key != "m"),
        "a null binding leaves the key out: {entries:?}"
    );
    assert_eq!(leader_label(keys.leader().id()), "Ctrl+G");
}

#[test]
fn a_leader_entry_with_no_description_shows_its_action_id() {
    setup();
    let keys = AppKeybindingsManager::from_json_str(
        r#"{"bindings": [{"context": "Prompt", "bindings": {"ctrl+x q": "app.unknown.thing"}}]}"#,
    );
    let entries = keys.leader_entries();
    assert!(
        entries
            .iter()
            .any(|e| e.key == "q" && e.description == "app.unknown.thing"),
        "{entries:?}"
    );
}

#[test]
fn a_sequence_reads_with_a_space_in_hints() {
    assert_eq!(format_key_text("ctrl+x m", true), "Ctrl+X m");
    assert_eq!(format_key_text("ctrl+x m", false), "ctrl+x m");
    assert_eq!(
        format_key_text("ctrl+x m/ctrl+x n", true),
        "Ctrl+X m/Ctrl+X n"
    );
    assert_eq!(format_key_text("/", false), "/");
}

#[test]
fn the_popup_component_renders_the_same_rows() {
    setup();
    let mut popup = WhichKeyPopup::new(fixture());
    let lines = popup.render(60);
    assert_eq!(lines, which_key_rows(&fixture(), 60));
}

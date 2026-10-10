//! The keyboard map, key text and config migration against
//! `fixtures/keybindings-gold.json`, generated from the pinned hoocode build
//! (on linux, so platform-specific defaults are the non-Windows ones).

use hoocode_code_tui_keybindings::*;
use serde_json::Value;

fn gold() -> Value {
    serde_json::from_str(include_str!("../fixtures/keybindings-gold.json")).unwrap()
}

#[cfg(not(windows))]
#[test]
fn declares_every_binding_in_hoocodes_order_with_its_defaults_and_description() {
    /// Declared divergence from the pin (2026-10-09, user decision: dial is
    /// radar/peek only, ctrl+o toggles; 2026-10-10: the chrome dial has two
    /// stops, full and compact, `bare` is retired). The pinned descriptions
    /// below describe the old dials; ours describe the new ones. An expected
    /// row is rewritten only when its pinned text matches exactly, and every
    /// entry must match, so a stale entry fails instead of rotting.
    const DECLARED: &[(&str, &str, &str)] = &[
        (
            "app.view.cycleForward",
            "Cycle tool output view (radar → peek → full)",
            "Cycle tool output view (radar ↔ peek)",
        ),
        (
            "app.tools.expand",
            "Jump to the full view from wherever you are, and back again",
            "Toggle tool output between radar and peek",
        ),
        (
            "app.chrome.cycleForward",
            "Cycle chrome density (full → compact → bare)",
            "Toggle chrome density (full ↔ compact)",
        ),
        (
            "app.chrome.cycleBackward",
            "Cycle chrome density backward",
            "Toggle chrome density (full ↔ compact), backward",
        ),
    ];
    let gold = gold();
    let mut expected: Vec<(String, Vec<String>, String)> = gold["table"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row[0].as_str().unwrap().to_string(),
                row[1]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|k| k.as_str().unwrap().to_string())
                    .collect(),
                row[2].as_str().unwrap().to_string(),
            )
        })
        .collect();
    let mut used = 0usize;
    for (id, pinned, ours) in DECLARED {
        if let Some(row) = expected.iter_mut().find(|r| r.0 == *id && r.2 == *pinned) {
            row.2 = ours.to_string();
            used += 1;
        }
    }
    assert_eq!(
        used,
        DECLARED.len(),
        "every declared divergence must match its pinned row exactly"
    );
    let actual: Vec<(String, Vec<String>, String)> = keybindings()
        .into_iter()
        .map(|e| {
            (
                e.id.to_string(),
                e.default_keys.iter().map(|k| k.to_string()).collect(),
                e.description.to_string(),
            )
        })
        .collect();
    assert_eq!(actual, expected);
}

#[test]
fn formats_key_text_like_hoocode() {
    for (key, forms) in gold()["format"].as_object().unwrap() {
        if cfg!(target_os = "macos") && key.contains("alt") {
            continue; // Option on macOS; the golden is from linux.
        }
        assert_eq!(
            format_key_text(key, false),
            forms[0].as_str().unwrap(),
            "{key}"
        );
        assert_eq!(
            format_key_text(key, true),
            forms[1].as_str().unwrap(),
            "{key} capitalized"
        );
    }
}

#[test]
fn migrates_and_orders_a_config_like_hoocode() {
    let raw: serde_json::Map<String, Value> = serde_json::from_value(serde_json::json!({
        "zzz": "x", "expandTools": "ctrl+x", "app.tools.expand": "ctrl+y", "cursorUp": ["up"],
        "app.interrupt": "escape", "aaa": 1, "tui.editor.undo": "ctrl+z", "selectConfirm": "enter"
    }))
    .unwrap();
    let (config, migrated) = migrate_keybindings_config(&raw);
    let gold = gold();
    assert_eq!(Value::Object(config.clone()), gold["migrate"]["config"]);
    // Key order is part of the contract (the file is written in it).
    let order: Vec<&String> = config.keys().collect();
    let expected: Vec<&String> = gold["migrate"]["config"]
        .as_object()
        .unwrap()
        .keys()
        .collect();
    assert_eq!(order, expected);
    assert_eq!(migrated, gold["migrate"]["migrated"].as_bool().unwrap());
}

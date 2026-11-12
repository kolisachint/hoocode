//! Keeps `color-semantics.json` (the machine-readable colour meanings the TUI
//! docs are written from) in sync with the theme schema and the built-in
//! dark and light themes.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use hoocode_code_tui_theme::*;
use serde_json::Value;

const SEMANTICS: &str = include_str!("../../color-semantics.json");
const THEME_SCHEMA: &str = include_str!("../../themes/theme-schema.json");

/// Role categories that are not entries of `roles`.
const FIXED_ROLES: [&str; 4] = ["markdown", "syntax", "diff", "legacy"];

fn semantics() -> Value {
    serde_json::from_str(SEMANTICS).expect("color-semantics.json parses")
}

/// Every colour property the theme schema declares.
fn declared_tokens() -> BTreeSet<String> {
    let schema: Value = serde_json::from_str(THEME_SCHEMA).expect("theme-schema.json parses");
    schema["properties"]["colors"]["properties"]
        .as_object()
        .expect("colors.properties is an object")
        .keys()
        .cloned()
        .collect()
}

fn resolved(name: &str) -> BTreeMap<String, String> {
    get_resolved_theme_colors(Some(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// The names in an array of strings, nulls skipped.
fn names(value: &Value) -> Vec<String> {
    value
        .as_array()
        .expect("array")
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect()
}

#[test]
fn file_parses_at_version_one() {
    let doc = semantics();
    assert_eq!(doc["version"], 1);
    for key in ["rules", "roles", "states", "surfaces", "tokens", "contrast"] {
        assert!(doc.get(key).is_some(), "top-level `{key}` is present");
    }
}

#[test]
fn token_keys_match_the_schema_and_required_flags_match_the_constant() {
    let doc = semantics();
    let tokens = doc["tokens"].as_object().expect("tokens is an object");
    let keys: BTreeSet<String> = tokens.keys().cloned().collect();
    assert_eq!(
        keys,
        declared_tokens(),
        "tokens must list every colour the schema declares, and nothing else"
    );

    let required: BTreeSet<String> = REQUIRED_COLOR_TOKENS
        .iter()
        .map(|s| s.to_string())
        .collect();
    for (name, entry) in tokens {
        let flag = entry["required"]
            .as_bool()
            .unwrap_or_else(|| panic!("{name}: `required` is a bool"));
        assert_eq!(
            flag,
            required.contains(name),
            "{name}: `required` must match REQUIRED_COLOR_TOKENS"
        );
    }
}

#[test]
fn every_referenced_token_exists() {
    let doc = semantics();
    let tokens = doc["tokens"].as_object().expect("tokens is an object");
    let mut referenced: Vec<String> = Vec::new();
    for role in doc["roles"].as_array().expect("roles") {
        referenced.extend(names(&role["tokens"]));
    }
    for state in doc["states"].as_array().expect("states") {
        referenced.extend(state["token"].as_str().map(str::to_string));
    }
    for surface in doc["surfaces"].as_array().expect("surfaces") {
        referenced.extend(surface["token"].as_str().map(str::to_string));
    }
    for rule in doc["rules"].as_array().expect("rules") {
        referenced.extend(names(&rule["applies_to"]));
    }
    assert!(!referenced.is_empty());
    for name in referenced {
        assert!(tokens.contains_key(&name), "`{name}` is not in tokens");
    }
}

#[test]
fn role_and_surface_hex_values_match_the_built_in_themes() {
    let doc = semantics();
    let dark = resolved("dark");
    let light = resolved("light");
    let dark_export = get_theme_export_colors(Some("dark"));
    let light_export = get_theme_export_colors(Some("light"));

    for role in doc["roles"].as_array().expect("roles") {
        let name = role["role"].as_str().expect("role name");
        let first = names(&role["tokens"])
            .into_iter()
            .next()
            .unwrap_or_else(|| panic!("{name}: has a token"));
        for (theme, css, hex) in [
            ("dark", &dark, role["dark"].as_str()),
            ("light", &light, role["light"].as_str()),
        ] {
            let hex = hex.unwrap_or_else(|| panic!("{name}: {theme} is a string"));
            let want = css
                .get(&first)
                .unwrap_or_else(|| panic!("{theme} resolves {first}"));
            assert!(
                want.eq_ignore_ascii_case(hex),
                "role {name} ({theme}): {hex} != {first} resolved {want}"
            );
        }
    }

    for surface in doc["surfaces"].as_array().expect("surfaces") {
        let name = surface["surface"].as_str().expect("surface name");
        for (theme, css, export, hex) in [
            ("dark", &dark, &dark_export, surface["dark"].as_str()),
            ("light", &light, &light_export, surface["light"].as_str()),
        ] {
            let hex = hex.unwrap_or_else(|| panic!("{name}: {theme} is a string"));
            let want = match surface["token"].as_str() {
                Some(token) => css
                    .get(token)
                    .cloned()
                    .unwrap_or_else(|| panic!("{theme} resolves {token}")),
                None => export
                    .page_bg
                    .clone()
                    .unwrap_or_else(|| panic!("{theme} has export.pageBg")),
            };
            assert!(
                want.eq_ignore_ascii_case(hex),
                "surface {name} ({theme}): {hex} != resolved {want}"
            );
        }
    }
}

#[test]
fn every_token_role_is_a_known_role() {
    let doc = semantics();
    let mut known: BTreeSet<String> = FIXED_ROLES.iter().map(|s| s.to_string()).collect();
    for role in doc["roles"].as_array().expect("roles") {
        known.insert(role["role"].as_str().expect("role name").to_string());
    }
    let tokens = doc["tokens"].as_object().expect("tokens is an object");
    for (name, entry) in tokens {
        let role = entry["role"]
            .as_str()
            .unwrap_or_else(|| panic!("{name}: `role` is a string"));
        assert!(known.contains(role), "{name}: unknown role `{role}`");
    }
}

/// Source files that may name the warning token: the blocking dialogs, where
/// yellow marks the thing that blocks the user (`blocking` in the roles above).
const BLOCKING_DIALOG_FILES: [&str; 5] = [
    "pane.rs",
    "card_frame.rs",
    "clearance_card.rs",
    "ask_options.rs",
    "end_shift.rs",
];

/// The crates that paint the transcript, the transient rows and the pickers.
const DRAWING_CRATES: [&str; 3] = [
    "hoocode-code-tui-widgets",
    "hoocode-code-tui-app",
    "hoocode-code-tui-selectors",
];

/// Spellings of the warning token in source: the literal, and the palette
/// helper that reads the `warn` token (which falls back to `warning`).
const WARNING_SPELLINGS: [&str; 3] = ["\"warning\"", "\"warn\"", ".warn()"];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn warning_token_is_used_only_by_blocking_dialogs() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut offenders = Vec::new();
    let mut found = BTreeSet::new();
    for krate in DRAWING_CRATES {
        let mut files = Vec::new();
        rust_files(&crates.join(krate).join("src"), &mut files);
        for file in files {
            let name = file
                .file_name()
                .expect("file name")
                .to_string_lossy()
                .into_owned();
            let text = fs::read_to_string(&file).expect("readable source");
            if BLOCKING_DIALOG_FILES.contains(&name.as_str()) {
                found.insert(name);
                continue;
            }
            if WARNING_SPELLINGS
                .iter()
                .any(|spelling| text.contains(spelling))
            {
                offenders.push(file.display().to_string());
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "yellow (the warning token) marks only blocking dialogs; move these to ink bold or dim:\n{}",
        offenders.join("\n")
    );
    for allowed in BLOCKING_DIALOG_FILES {
        assert!(
            found.contains(allowed),
            "BLOCKING_DIALOG_FILES names `{allowed}`, which is not in the drawing crates; remove it"
        );
    }
}

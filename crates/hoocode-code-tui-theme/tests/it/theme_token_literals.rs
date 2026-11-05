//! Token-literal test (T-5): every token name passed as a literal to a theme
//! lookup in the workspace source must be a color token of the schema. A
//! typo is a runtime panic (`Unknown theme color`), so this catches it here.
//!
//! The scan is plain text, not a parser: it finds `.fg("…")`-style calls
//! whose receiver is `theme()`, `t`, `theme` or `self`, across line breaks.
//! Lookups through a variable (`t.fg(color, …)`) are not literals and are
//! covered by the constant tables checked below.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use hoocode_code_tui_theme::{
    AGENT_COLOR_TOKENS, PANE_BG_TOKENS, THEME_BGS, THEME_COLORS, THEME_SCHEMA_JSON,
};

/// The lookups that take a token name.
const LOOKUPS: [&str; 8] = [
    "fg",
    "bg",
    "fill",
    "has",
    "has_bg",
    "can_fill",
    "get_fg_ansi",
    "get_bg_ansi",
];

/// Color tokens the schema declares: the keys of `properties.colors`.
fn schema_tokens() -> BTreeSet<String> {
    let schema: serde_json::Value =
        serde_json::from_str(THEME_SCHEMA_JSON).expect("theme-schema.json is JSON");
    schema["properties"]["colors"]["properties"]
        .as_object()
        .expect("the schema lists the color tokens")
        .keys()
        .cloned()
        .collect()
}

/// Whether the text before a lookup's `.` is a theme (not a registry, a
/// style or a default value).
fn is_theme_receiver(before: &str) -> bool {
    let before = before.trim_end();
    if before.ends_with("theme()") {
        return true;
    }
    let ident: String = before
        .chars()
        .rev()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    matches!(ident.as_str(), "t" | "theme" | "self")
}

/// The string literals passed to theme lookups in `source`.
fn token_literals(source: &str) -> Vec<String> {
    let mut found = Vec::new();
    for lookup in LOOKUPS {
        let needle = format!(".{lookup}");
        let mut from = 0;
        while let Some(offset) = source[from..].find(&needle) {
            let dot = from + offset;
            from = dot + needle.len();
            let Some(args) = source[from..].trim_start().strip_prefix('(') else {
                continue;
            };
            let Some(literal) = args.trim_start().strip_prefix('"') else {
                continue;
            };
            let Some(end) = literal.find('"') else {
                continue;
            };
            if is_theme_receiver(&source[..dot]) {
                found.push(literal[..end].to_string());
            }
        }
    }
    found
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if !name.starts_with('.') && name != "target" {
                rust_files(&path, out);
            }
        } else if name.ends_with(".rs") {
            out.push(path);
        }
    }
}

#[test]
fn the_scanner_finds_literal_lookups_and_ignores_other_receivers() {
    let source = r#"
        let a = theme().fg("accent", "x");
        let b = t.bg("selectedBg", "y");
        let c = theme()
            .fill("toolTitle", "z");
        let d = self.get_fg_ansi("text");
        let e = reg.has("custom");
        let f = Style::default().fg("red");
        let g = t.fg(color, "not a literal");
        let h = format!("{}", at.fg("accent"));
    "#;
    // `at.fg(…)` is not a theme (the receiver is a whole identifier).
    let mut found = token_literals(source);
    found.sort();
    assert_eq!(found, ["accent", "selectedBg", "text", "toolTitle"]);
}

#[test]
fn every_token_literal_in_the_workspace_is_in_the_schema() {
    let schema = schema_tokens();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    rust_files(&root.join("crates"), &mut files);
    assert!(!files.is_empty(), "no sources under {}", root.display());

    let this_file = Path::new(file!());
    let mut unknown: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut lookups = 0;
    for file in files {
        if file.ends_with(this_file.file_name().unwrap()) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        for token in token_literals(&text) {
            lookups += 1;
            if !schema.contains(&token) {
                let shown = file.strip_prefix(&root).unwrap_or(&file);
                unknown
                    .entry(token)
                    .or_default()
                    .push(shown.display().to_string());
            }
        }
    }
    // A scanner that finds almost nothing would pass for the wrong reason.
    assert!(lookups > 100, "the scan saw only {lookups} literal lookups");
    let report: Vec<String> = unknown
        .iter()
        .map(|(token, places)| format!("{token:?} in {}", places.join(", ")))
        .collect();
    assert!(
        report.is_empty(),
        "token literals not in theme-schema.json (add them to COLOR_PROPERTIES and the schema, or fix the name):\n{}",
        report.join("\n")
    );
}

#[test]
fn the_token_tables_the_code_names_are_in_the_schema() {
    let schema = schema_tokens();
    let tables = [
        ("THEME_COLORS", THEME_COLORS.as_slice()),
        ("THEME_BGS", THEME_BGS.as_slice()),
        ("PANE_BG_TOKENS", PANE_BG_TOKENS.as_slice()),
        ("AGENT_COLOR_TOKENS", AGENT_COLOR_TOKENS.as_slice()),
    ];
    for (name, tokens) in tables {
        let missing: Vec<&str> = tokens
            .iter()
            .copied()
            .filter(|t| !schema.contains(*t))
            .collect();
        assert!(missing.is_empty(), "{name} not in the schema: {missing:?}");
    }
}

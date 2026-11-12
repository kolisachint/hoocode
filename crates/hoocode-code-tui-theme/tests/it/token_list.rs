//! The colour token lists stay in step: the schema lists each name once, it
//! matches the tokens a theme carries, and a user theme that still sets a
//! removed thinking token still loads.

use std::collections::BTreeSet;

use hoocode_code_tui_theme::schema::COLOR_PROPERTIES;
use hoocode_code_tui_theme::*;
use serde_json::Value;

fn dark_json() -> Value {
    serde_json::from_str(builtin_theme_source("dark").unwrap()).unwrap()
}

fn schema_names() -> Vec<&'static str> {
    COLOR_PROPERTIES.iter().map(|(name, _)| *name).collect()
}

#[test]
fn schema_color_list_names_each_token_once() {
    let names = schema_names();
    let unique: BTreeSet<&str> = names.iter().copied().collect();
    assert_eq!(
        unique.len(),
        names.len(),
        "duplicate names in COLOR_PROPERTIES"
    );
}

#[test]
fn schema_color_list_equals_the_tokens_the_theme_carries() {
    let schema: BTreeSet<&str> = schema_names().into_iter().collect();
    let theme: BTreeSet<&str> = THEME_COLORS
        .iter()
        .chain(THEME_BGS.iter())
        .copied()
        .collect();
    assert_eq!(schema, theme);
    for list in [&THEME_COLORS[..], &THEME_BGS[..]] {
        let unique: BTreeSet<&str> = list.iter().copied().collect();
        assert_eq!(
            unique.len(),
            list.len(),
            "duplicate names in a theme token list"
        );
    }
}

#[test]
fn built_in_themes_set_only_schema_tokens_and_required_tokens_are_listed() {
    let schema: BTreeSet<&str> = schema_names().into_iter().collect();
    let json = dark_json();
    for key in json["colors"].as_object().unwrap().keys() {
        assert!(schema.contains(key.as_str()), "{key} is not in the schema");
    }
    for token in REQUIRED_COLOR_TOKENS {
        assert!(
            schema.contains(token),
            "required {token} is not in the schema"
        );
    }
    let required: BTreeSet<&str> = REQUIRED_COLOR_TOKENS.iter().copied().collect();
    assert_eq!(required.len(), REQUIRED_COLOR_TOKENS.len());
}

#[test]
fn user_theme_that_still_sets_removed_thinking_tokens_loads() {
    let mut json = dark_json();
    let colors = json["colors"].as_object_mut().unwrap();
    for token in [
        "thinkingOff",
        "thinkingMinimal",
        "thinkingLow",
        "thinkingMedium",
        "thinkingHigh",
        "thinkingXhigh",
    ] {
        colors.insert(token.to_string(), Value::String("#808080".to_string()));
    }
    assert!(parse_theme_json("user.json", &json).is_ok());

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("user.json");
    std::fs::write(&path, serde_json::to_string_pretty(&json).unwrap()).unwrap();
    assert!(load_theme_from_path(&path, None).is_ok());
}

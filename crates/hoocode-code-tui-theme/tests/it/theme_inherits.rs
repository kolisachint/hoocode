//! `"inherits"`: a theme takes the colors, vars and export it does not set
//! from its base, a built-in theme or a file in the themes folder. A cycle
//! and an unknown base are errors.

use std::path::PathBuf;
use std::sync::MutexGuard;

use hoocode_code_tui_theme::*;
use serde_json::{json, Value};

/// A fresh agent dir with an empty `themes/`, pointed at by the env
/// override. These tests share the process env and theme state, so they
/// hold the crate-wide global theme lock.
struct AgentDir {
    _guard: MutexGuard<'static, ()>,
    dir: tempfile::TempDir,
}

impl AgentDir {
    fn new() -> Self {
        let guard = crate::global_theme_lock();
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("agent/themes")).unwrap();
        std::env::set_var("HOOCODE_CODING_AGENT_DIR", dir.path().join("agent"));
        set_registered_themes(Vec::new());
        Self { _guard: guard, dir }
    }

    fn themes_dir(&self) -> PathBuf {
        self.dir.path().join("agent/themes")
    }

    fn write_text(&self, name: &str, text: &str) {
        std::fs::write(self.themes_dir().join(format!("{name}.json")), text).unwrap();
    }

    fn write(&self, name: &str, theme: &Value) {
        self.write_text(name, &serde_json::to_string_pretty(theme).unwrap());
    }
}

impl Drop for AgentDir {
    fn drop(&mut self) {
        stop_theme_watcher();
        set_registered_themes(Vec::new());
    }
}

fn resolved(name: &str) -> std::collections::BTreeMap<String, String> {
    get_resolved_theme_colors(Some(name)).unwrap()
}

#[test]
fn a_short_theme_inheriting_dark_loads_with_every_required_token() {
    let agent = AgentDir::new();
    agent.write_text(
        "short",
        r##"{
  "name": "short",
  "inherits": "dark",
  "colors": { "accent": "#ff8800" }
}"##,
    );
    let colors = resolved("short");
    for token in REQUIRED_COLOR_TOKENS {
        assert!(colors.contains_key(token), "missing {token}");
    }
    assert_eq!(colors["accent"], "#ff8800");
    let dark = resolved("dark");
    assert_eq!(colors["border"], dark["border"]);
    assert!(get_theme_by_name("short").is_some());
}

#[test]
fn inheritance_chains_through_a_user_theme_and_keeps_its_vars() {
    let agent = AgentDir::new();
    agent.write(
        "mid",
        &json!({
            "name": "mid",
            "inherits": "dark",
            "vars": { "brand": "#112233" },
            "colors": { "accent": "brand" }
        }),
    );
    // Takes accent from mid, where "brand" is a var mid defines.
    agent.write("leaf", &json!({ "name": "leaf", "inherits": "mid" }));
    assert_eq!(resolved("leaf")["accent"], "#112233");
    assert_eq!(resolved("leaf")["border"], resolved("dark")["border"]);
}

#[test]
fn a_cycle_is_an_error_not_a_hang() {
    let agent = AgentDir::new();
    agent.write("a", &json!({ "name": "a", "inherits": "b", "colors": {} }));
    agent.write("b", &json!({ "name": "b", "inherits": "a", "colors": {} }));
    let error = load_theme_json("a").unwrap_err();
    assert_eq!(error, "Theme inheritance cycle: a -> b -> a");
    assert!(get_theme_by_name("a").is_none());
    let error = set_theme("a", false).unwrap_err();
    assert!(error.contains("cycle"), "{error}");
    drop(agent);
}

#[test]
fn a_theme_that_inherits_itself_is_a_cycle() {
    let agent = AgentDir::new();
    agent.write("self", &json!({ "name": "self", "inherits": "self" }));
    let error = load_theme_json("self").unwrap_err();
    assert_eq!(error, "Theme inheritance cycle: self -> self");
    drop(agent);
}

#[test]
fn an_unknown_base_is_an_error() {
    let agent = AgentDir::new();
    agent.write(
        "orphan",
        &json!({ "name": "orphan", "inherits": "no-such-theme", "colors": {} }),
    );
    let error = load_theme_json("orphan").unwrap_err();
    assert!(error.contains("\"no-such-theme\""), "{error}");
    assert!(error.contains("neither a built-in theme"), "{error}");
    assert!(get_theme_by_name("orphan").is_none());
    drop(agent);
}

#[test]
fn an_inheriting_theme_still_has_its_own_values_checked() {
    let agent = AgentDir::new();
    agent.write(
        "bad",
        &json!({ "name": "bad", "inherits": "dark", "colors": { "accent": true } }),
    );
    let error = load_theme_json("bad").unwrap_err();
    assert!(error.starts_with("Invalid theme \"bad\":"), "{error}");
    drop(agent);
}

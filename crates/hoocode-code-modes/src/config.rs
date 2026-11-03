//! `extensions/core/config.ts`: `hoo-config.json`.
//!
//! The global file lives in the agent dir; a project may overlay
//! `.hoocode/hoo-config.json` (scalars win, most arrays are unioned). The
//! config is kept as JSON so fields hoocode does not model survive a rewrite.

use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

/// A parsed `hoo-config.json`.
pub type HooConfig = Map<String, Value>;

/// `GLOBAL_CONFIG_PATH`.
pub fn global_config_path() -> PathBuf {
    hoocode_code_paths::agent_dir().join("hoo-config.json")
}

fn read_object(path: &Path) -> Option<HooConfig> {
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str::<Value>(&text).ok()? {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

/// `readConfig`: the global config, `{}` when missing or unreadable.
pub fn read_config() -> HooConfig {
    read_object(&global_config_path()).unwrap_or_default()
}

/// `writeConfig`: `JSON.stringify(config, null, 2)` plus a newline.
pub fn write_config(config: &HooConfig) -> std::io::Result<()> {
    let path = global_config_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(&Value::Object(config.clone()))
        .map_err(std::io::Error::other)?;
    std::fs::write(path, format!("{text}\n"))
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn dedupe(paths: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in paths {
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

fn union(a: Option<&Value>, b: Option<&Value>) -> Value {
    Value::from(dedupe(strings(a).into_iter().chain(strings(b))))
}

/// `mergeConfigs`: project `active_mode` / `thinking_escalation` win; per
/// mode, `denied_tools` and `denied_bash_commands` are unioned, while
/// `enabled_tools` and `allowed_bash_commands` fall back to the global
/// value; project `mode_paths` come first. `auto_allow` comes from the
/// global file only, and so does `allowed_write_paths` when the global
/// sets it. Top-level `skip_permissions` and `safe_shell` are never read
/// from the project file.
pub fn merge_configs(global: &HooConfig, project: &HooConfig) -> HooConfig {
    let mut merged = global.clone();
    for key in ["active_mode", "thinking_escalation"] {
        if let Some(v) = project.get(key) {
            merged.insert(key.into(), v.clone());
        }
    }
    if let Some(project_modes) = project.get("modes").and_then(Value::as_object) {
        let global_modes = global.get("modes").and_then(Value::as_object);
        let mut modes = global_modes.cloned().unwrap_or_default();
        for (mode, project_cfg) in project_modes {
            let empty = Map::new();
            let global_cfg = global_modes
                .and_then(|m| m.get(mode))
                .and_then(Value::as_object)
                .unwrap_or(&empty);
            let project_cfg = project_cfg.as_object().unwrap_or(&empty);
            let mut cfg = global_cfg.clone();
            for (k, v) in project_cfg {
                cfg.insert(k.clone(), v.clone());
            }
            for key in ["denied_tools", "denied_bash_commands"] {
                cfg.insert(key.into(), union(global_cfg.get(key), project_cfg.get(key)));
            }
            // A cloned repo must not widen what runs without a prompt: the
            // project cannot add to `auto_allow`, and cannot widen a global
            // `allowed_write_paths` (it may set one where the global has none).
            match global_cfg.get("auto_allow") {
                Some(v) => cfg.insert("auto_allow".into(), v.clone()),
                None => cfg.remove("auto_allow"),
            };
            if let Some(v) = global_cfg.get("allowed_write_paths") {
                cfg.insert("allowed_write_paths".into(), v.clone());
            }
            for key in ["enabled_tools", "allowed_bash_commands"] {
                match project_cfg.get(key).or_else(|| global_cfg.get(key)) {
                    Some(v) => {
                        cfg.insert(key.into(), v.clone());
                    }
                    None => {
                        cfg.remove(key);
                    }
                }
            }
            modes.insert(mode.clone(), Value::Object(cfg));
        }
        merged.insert("modes".into(), Value::Object(modes));
    }
    if project.contains_key("mode_paths") || global.contains_key("mode_paths") {
        let paths = dedupe(
            strings(project.get("mode_paths"))
                .into_iter()
                .chain(strings(global.get("mode_paths"))),
        );
        merged.insert("mode_paths".into(), Value::from(paths));
    }
    merged
}

/// `mergeSearchPaths`: concatenated, first occurrence kept.
pub fn merge_search_paths(sources: &[&[String]]) -> Vec<String> {
    dedupe(sources.iter().flat_map(|s| s.iter().cloned()))
}

/// The `hoo-config.json` hoocode ships, written on first run (see
/// [`ensure_default_config`]). Its mode rules are also the fallback for any
/// key a user's file leaves out ([`with_builtin_defaults`]):
/// - ask, debug: read and Shell, no Write or Edit;
/// - plan: read and Shell, Write and Edit only under `.hoocode/` (no prompt);
/// - build: everything, with the usual prompts.
///
/// `safe_shell` lets read-only commands (`ls`, `git status`, ...) run in
/// every mode without a prompt; `skip_permissions` runs every tool without a
/// prompt (the mode rules still apply).
pub fn default_config() -> HooConfig {
    let value = serde_json::json!({
        "active_mode": "build",
        "safe_shell": true,
        "skip_permissions": false,
        "modes": {
            "ask": { "denied_tools": ["Write", "Edit"] },
            "debug": { "denied_tools": ["Write", "Edit"] },
            "plan": {
                "allowed_write_paths": [".hoocode/*"],
                "auto_allow": ["Write", "Edit"]
            },
            "build": {}
        }
    });
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

/// Writes [`default_config`] to the global path when no file is there yet.
/// Returns whether it wrote one.
pub fn ensure_default_config() -> std::io::Result<bool> {
    if global_config_path().exists() {
        return Ok(false);
    }
    write_config(&default_config())?;
    Ok(true)
}

/// `config` with the shipped mode rules filled in: a mode key the file does
/// not set takes the [`default_config`] value. Set a key (even to `[]`) to
/// override it.
pub fn with_builtin_defaults(config: &HooConfig) -> HooConfig {
    let mut out = config.clone();
    let defaults = default_config();
    let Some(default_modes) = defaults.get("modes").and_then(Value::as_object) else {
        return out;
    };
    let modes = out
        .entry("modes")
        .or_insert_with(|| Value::Object(Map::new()));
    if !modes.is_object() {
        *modes = Value::Object(Map::new());
    }
    if let Value::Object(modes) = modes {
        for (mode, default_cfg) in default_modes {
            let Some(default_cfg) = default_cfg.as_object() else {
                continue;
            };
            let cfg = modes
                .entry(mode.clone())
                .or_insert_with(|| Value::Object(Map::new()));
            if !cfg.is_object() {
                *cfg = Value::Object(Map::new());
            }
            if let Value::Object(cfg) = cfg {
                for (key, value) in default_cfg {
                    cfg.entry(key.clone()).or_insert_with(|| value.clone());
                }
            }
        }
    }
    out
}

/// The env var that turns on `skip_permissions` for this process and every
/// child it spawns (`--skip-permissions` sets it).
pub const SKIP_PERMISSIONS_ENV: &str = "HOOCODE_SKIP_PERMISSIONS";

fn truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// Whether every tool runs without a prompt: `skip_permissions: true` in the
/// config, or [`SKIP_PERMISSIONS_ENV`] set.
pub fn skip_permissions(config: &HooConfig) -> bool {
    config.get("skip_permissions").and_then(Value::as_bool) == Some(true)
        || std::env::var(SKIP_PERMISSIONS_ENV).is_ok_and(|v| truthy(&v))
}

/// Whether read-only Shell commands run without a prompt (`safe_shell`,
/// on unless set to `false`).
pub fn safe_shell(config: &HooConfig) -> bool {
    config.get("safe_shell").and_then(Value::as_bool) != Some(false)
}

/// `readMergedConfig`: global (with the shipped mode rules filled in),
/// overlaid by `<cwd>/.hoocode/hoo-config.json`.
pub fn read_merged_config(cwd: &Path) -> HooConfig {
    let global = with_builtin_defaults(&read_config());
    let project_path = cwd
        .join(hoocode_code_paths::CONFIG_DIR_NAME)
        .join("hoo-config.json");
    if !project_path.exists() {
        return global;
    }
    match read_object(&project_path) {
        Some(project) => merge_configs(&global, &project),
        None => global,
    }
}

/// `config.active_mode`.
pub fn active_mode(config: &HooConfig) -> Option<String> {
    config
        .get("active_mode")
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// `config.mode_paths`.
pub fn mode_paths(config: &HooConfig) -> Vec<String> {
    strings(config.get("mode_paths"))
}

/// `config.modes[mode]`.
pub fn mode_config<'a>(config: &'a HooConfig, mode: &str) -> Option<&'a Map<String, Value>> {
    config
        .get("modes")
        .and_then(Value::as_object)
        .and_then(|m| m.get(mode))
        .and_then(Value::as_object)
}

/// A string-list field of a mode config (`enabled_tools`, `auto_allow`, ...).
pub fn mode_list(config: &HooConfig, mode: &str, key: &str) -> Option<Vec<String>> {
    mode_config(config, mode)
        .and_then(|m| m.get(key))
        .filter(|v| v.is_array())
        .map(|v| strings(Some(v)))
}

//! The default keymap as a grouped `keybindings.json` document, and the copy on
//! disk (`<agent dir>/keybindings.defaults.json`), as the theme defaults are
//! (`hoocode-code-tui-theme/src/defaults.rs`).
//!
//! `hoocode keybindings --defaults` prints [`defaults_document`]. At startup,
//! [`write_defaults_file`] writes the same text when the file differs (after
//! an upgrade it always does, because the version is in it). The file is a
//! reference: hoocode never reads it as the user's file. Its `$` keys are
//! comments, and the loader ignores them, so a copy of it loads without errors.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use hoocode_tui_keys::DEFAULT_LEADER_TIMEOUT;
use serde_json::{json, Map, Value};

use crate::config::{Context, DEFAULT_LEADER, RESERVED_KEYS};
use crate::keybindings::keybindings;

/// The file in the agent dir that holds the defaults.
pub const DEFAULTS_FILE_NAME: &str = "keybindings.defaults.json";

/// The context a binding of `id` belongs to in the grouped form. Every key
/// with a leader is written in `Prompt`, as the loader reads leader sequences there.
pub fn context_of(id: &str) -> Context {
    if id == "app.interrupt" {
        return Context::Global;
    }
    // Escape already means "back" in the scroll view and the task panel, so the
    // attached subagent view's escape goes in the prompt group.
    if id == "app.subagent.detach" {
        return Context::Prompt;
    }
    // The interrupted-run keys share `r` and `v` with the picker's own keys,
    // so they get a group of their own (see `Context::Subagent`).
    if matches!(id, "app.subagent.view" | "app.subagent.resume") {
        return Context::Subagent;
    }
    let picker_session = matches!(
        id,
        "app.session.togglePath"
            | "app.session.toggleSort"
            | "app.session.toggleNamedFilter"
            | "app.session.rename"
            | "app.session.delete"
            | "app.session.deleteNoninvasive"
    );
    if id.starts_with("tui.select.")
        || id.starts_with("app.list.")
        || id == "app.settings.toggle"
        || picker_session
    {
        Context::Picker
    } else if id.starts_with("app.dialog.")
        || id.starts_with("app.options.")
        || id.starts_with("app.permission.")
    {
        Context::Dialog
    } else if id.starts_with("app.tree.") {
        Context::Tree
    } else if id.starts_with("app.models.") {
        Context::Models
    } else if id.starts_with("app.panel.") {
        Context::Panel
    } else if id.starts_with("app.scroll.") {
        Context::Scroll
    } else {
        Context::Prompt
    }
}

/// The default keymap as a grouped `keybindings.json` document, with a `$`
/// comment that names `version`. The reserved keys hoocode keeps for itself
/// are not bindings, so they are listed under `$reserved`, not in a group.
pub fn defaults_document(version: &str) -> String {
    let mut groups: BTreeMap<Context, Map<String, Value>> = BTreeMap::new();
    let mut reserved: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for entry in keybindings() {
        for key in &entry.default_keys {
            let lower = key.to_lowercase();
            if RESERVED_KEYS.contains(&lower.as_str()) {
                reserved
                    .entry(lower)
                    .or_default()
                    .push(entry.id.to_string());
                continue;
            }
            let context = if key.contains(' ') {
                Context::Prompt
            } else {
                context_of(entry.id)
            };
            groups
                .entry(context)
                .or_default()
                .entry((*key).to_string())
                .or_insert_with(|| Value::String(entry.id.to_string()));
        }
    }
    let bindings: Vec<Value> = Context::ALL
        .iter()
        .filter_map(|context| {
            groups
                .get(context)
                .map(|map| json!({"context": context.name(), "bindings": map}))
        })
        .collect();
    let reserved: Map<String, Value> = reserved
        .into_iter()
        .map(|(key, ids)| (key, json!(ids)))
        .collect();
    let document = json!({
        "$comment": format!(
            "hoocode {version} default keymap. A reference: copy the lines you change into keybindings.json. Written again on each upgrade."
        ),
        "leader": DEFAULT_LEADER,
        "leaderTimeoutMs": DEFAULT_LEADER_TIMEOUT.as_millis() as u64,
        "bindings": bindings,
        "$reserved": reserved,
    });
    let mut text = serde_json::to_string_pretty(&document).unwrap_or_default();
    text.push('\n');
    text
}

/// `<agent dir>/keybindings.defaults.json`.
pub fn defaults_path(agent_dir: &Path) -> PathBuf {
    agent_dir.join(DEFAULTS_FILE_NAME)
}

/// Writes [`defaults_document`] to `path` unless the file already holds it.
/// `true` when it wrote. A failure is an error text for the caller to show as a
/// warning; nothing else depends on the file.
pub fn write_defaults_file(path: &Path, version: &str) -> Result<bool, String> {
    let text = defaults_document(version);
    if std::fs::read_to_string(path).is_ok_and(|current| current == text) {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(true)
}

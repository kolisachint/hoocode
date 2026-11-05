//! The Alt migration notice (K-8). Alt keys in a user's `keybindings.json`
//! keep working, and hoocode says so once per version. The file is never
//! rewritten. The version it was last shown for is kept in the agent dir.

use std::path::Path;

use serde_json::{Map, Value};

/// The state file in the agent dir: `{"altNoticeVersion": "<version>"}`.
pub const STATE_FILE_NAME: &str = "keybindings-state.json";

const SHOWN_KEY: &str = "altNoticeVersion";

/// The keys in `keys` that hold Alt, in the order given and without repeats.
pub fn alt_keys(keys: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for key in keys {
        let uses_alt = key
            .split_whitespace()
            .any(|step| step.split('+').any(|part| part.eq_ignore_ascii_case("alt")));
        if uses_alt && !out.contains(key) {
            out.push(key.clone());
        }
    }
    out
}

/// The notice text for the Alt keys `alt` (from [`alt_keys`]).
pub fn alt_notice(alt: &[String]) -> String {
    format!(
        "keybindings.json binds Alt keys ({}). They still work. Alt is no longer a default, \
         and the defaults use the leader key. `hoocode keybindings --defaults` lists them.",
        alt.join(", ")
    )
}

/// The notice, when `bound` (every key the file lists) has an Alt key and
/// `version` has not shown it yet. Returns the text and records `version` in
/// `state`, so the next start of the same version stays quiet. A file without
/// Alt keys shows nothing and writes nothing.
pub fn take_alt_notice(state: &Path, version: &str, bound: &[String]) -> Option<String> {
    let alt = alt_keys(bound);
    if alt.is_empty() {
        return None;
    }
    let shown = std::fs::read_to_string(state)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|value| {
            value
                .get(SHOWN_KEY)
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    if shown.as_deref() == Some(version) {
        return None;
    }
    let mut record = Map::new();
    record.insert(SHOWN_KEY.to_string(), Value::String(version.to_string()));
    if let Some(dir) = state.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // A write that fails means the notice may show again next start. That is
    // harmless, so the error is not reported.
    let _ = std::fs::write(state, Value::Object(record).to_string());
    Some(alt_notice(&alt))
}

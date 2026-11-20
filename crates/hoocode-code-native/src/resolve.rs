//! Executable resolution: which binary a helper starts.
//!
//! Order (spec D2): the user setting `binaryPath` > the downloaded binary for
//! this platform > the mcp.json entry's own command. The settings are read
//! from the raw settings JSON, so this crate stays free of the settings crate:
//! `~/.hoocode/settings.json` holds
//! `"nativeHelpers": {"<plugin>": {"binaryPath": "/abs/path"}}` (absolute only;
//! a relative path is ignored with a warning).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use hoocode_code_plugins::NativeHelperEntry;
use serde_json::Value;

use crate::download::ensure_plugin_binary;
use crate::error::NativeError;

/// The hoocode home dir (`~/.hoocode`, honoring the env override), the same
/// way the rest of the product reads it. Injectable in tests: pass any dir as
/// the supervisor's `home`.
pub fn default_home_dir() -> PathBuf {
    hoocode_code_paths::agent_dir()
}

/// Read `nativeHelpers.<plugin>.binaryPath` from the raw settings JSON.
/// Returns the override plus warnings (one per ignored relative path).
pub fn binary_path_overrides(settings: &Value) -> (HashMap<String, PathBuf>, Vec<String>) {
    let mut overrides = HashMap::new();
    let mut warnings = Vec::new();
    let Some(helpers) = settings.get("nativeHelpers").and_then(Value::as_object) else {
        return (overrides, warnings);
    };
    for (plugin, entry) in helpers {
        let Some(path) = entry.get("binaryPath").and_then(Value::as_str) else {
            continue;
        };
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            warnings.push(format!(
                "nativeHelpers.{plugin}.binaryPath is not absolute: ignored"
            ));
            continue;
        }
        overrides.insert(plugin.clone(), path);
    }
    (overrides, warnings)
}

/// One plugin's override, or `None` when unset or relative (ignored).
pub fn binary_path_override(settings: &Value, plugin: &str) -> Option<PathBuf> {
    binary_path_overrides(settings).0.remove(plugin)
}

/// Resolve the executable for one helper: `binaryPath`, else the downloaded
/// platform binary, else the mcp.json command (an in-plugin `./` path resolves
/// under the plugin root; a bare token is left for PATH lookup).
pub async fn resolve_executable(
    home: &Path,
    entry: &NativeHelperEntry,
    overrides: &HashMap<String, PathBuf>,
    client: &reqwest::Client,
) -> Result<PathBuf, NativeError> {
    if let Some(path) = overrides.get(&entry.plugin) {
        if !path.is_file() {
            return Err(NativeError::InvalidBinary(format!(
                "nativeHelpers.{}.binaryPath does not exist: {}",
                entry.plugin,
                path.display()
            )));
        }
        return Ok(path.clone());
    }
    if let Some(downloaded) = ensure_plugin_binary(home, entry, client).await? {
        return Ok(downloaded);
    }
    let command = entry.helper.launch.command.clone();
    if command.starts_with("./") || command.contains('/') {
        let root = PathBuf::from(&entry.root);
        return Ok(root.join(command.trim_start_matches("./")));
    }
    Ok(PathBuf::from(command))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn absolute_binary_path_wins() {
        let settings = json!({"nativeHelpers": {"browser": {"binaryPath": "/opt/hoo-browser"}}});
        assert_eq!(
            binary_path_override(&settings, "browser"),
            Some(PathBuf::from("/opt/hoo-browser"))
        );
    }

    #[test]
    fn relative_binary_path_is_ignored_with_a_warning() {
        let settings = json!({"nativeHelpers": {"browser": {"binaryPath": "bin/hoo-browser"}}});
        assert_eq!(binary_path_override(&settings, "browser"), None);
        let (_, warnings) = binary_path_overrides(&settings);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("browser"), "{}", warnings[0]);
    }

    #[test]
    fn missing_and_malformed_entries_give_no_override() {
        assert_eq!(binary_path_override(&json!({}), "browser"), None);
        assert_eq!(
            binary_path_override(&json!({"nativeHelpers": {"browser": {}}}), "browser"),
            None
        );
        assert_eq!(
            binary_path_override(
                &json!({"nativeHelpers": {"browser": {"binaryPath": 7}}}),
                "browser"
            ),
            None
        );
    }
}

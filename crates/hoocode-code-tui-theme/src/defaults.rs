//! Reference copies of the bundled themes on disk: `themes/defaults/`.
//!
//! At startup, when the hoocode version differs from the one the folder was
//! written for, the built-in theme files and `theme-schema.json` are written
//! there again. The loader and the picker read only the top level of
//! `themes/`, so this folder is never a theme list. A user copies a file up a
//! level and renames it to start a custom theme, or uses `inherits`.
//! Nothing is written outside this folder.

use std::path::{Path, PathBuf};

use crate::registry::{clear_notice, record_notice, BUILTIN_THEME_FILES, THEME_SCHEMA_JSON};

/// The folder under the themes folder that holds the copies.
pub const DEFAULTS_DIR_NAME: &str = "defaults";

/// The file in the defaults folder that names the version the copies were
/// written for.
pub const DEFAULTS_STAMP_FILE: &str = "VERSION";

/// The key of the notice shown when the copies cannot be written.
const NOTICE_KEY: &str = "theme defaults";

/// `~/.hoocode/themes/defaults/` (or the agent dir override's).
pub fn defaults_dir() -> PathBuf {
    hoocode_code_paths::custom_themes_dir().join(DEFAULTS_DIR_NAME)
}

/// The files the folder holds: the built-in themes, then the schema.
fn bundled_files() -> impl Iterator<Item = (&'static str, &'static str)> {
    BUILTIN_THEME_FILES
        .iter()
        .copied()
        .chain(std::iter::once(("theme-schema.json", THEME_SCHEMA_JSON)))
}

/// Writes the bundled copies into `dir` unless `dir` already has every copy
/// for `version`. Returns whether it wrote. The stamp is written last, so a
/// failed write is tried again at the next start.
pub fn write_theme_defaults(dir: &Path, version: &str) -> Result<bool, String> {
    let stamp = dir.join(DEFAULTS_STAMP_FILE);
    let current = std::fs::read_to_string(&stamp).is_ok_and(|s| s.trim() == version);
    if current && bundled_files().all(|(file, _)| dir.join(file).is_file()) {
        return Ok(false);
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for (file, text) in bundled_files() {
        let path = dir.join(file);
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    std::fs::write(&stamp, version).map_err(|e| format!("{}: {e}", stamp.display()))?;
    Ok(true)
}

/// Startup: refreshes the copies in [`defaults_dir`] for `version`. A failure
/// is a warning in the theme diagnostics; the themes keep loading.
pub fn refresh_theme_defaults(version: &str) -> bool {
    let dir = defaults_dir();
    match write_theme_defaults(&dir, version) {
        Ok(written) => {
            clear_notice(NOTICE_KEY);
            written
        }
        Err(error) => {
            record_notice(
                NOTICE_KEY,
                format!("Theme defaults were not written: {error}. Your themes still load."),
            );
            false
        }
    }
}

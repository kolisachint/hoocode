//! The keybindings file in the TUI (K-8): the problems `/hotkeys` lists and the
//! startup notices. The startup notices are the defaults file's write (a failure
//! is a warning) and the Alt migration notice, shown once per version. Both
//! touch the agent dir, so they run from the startup path and not from `Mode::new`.

use hoocode_code_paths::{agent_dir, VERSION};
use hoocode_code_tui_keybindings::{
    defaults_path, take_alt_notice, write_defaults_file, AppKeybindingsManager, STATE_FILE_NAME,
};

use super::*;

/// The problems in the keybindings file: errors first (skipped bindings), then cautions.
pub(super) fn problem_lines(keybindings: &AppKeybindingsManager) -> Vec<String> {
    keybindings
        .warnings()
        .iter()
        .map(|w| format!("error: {w}"))
        .chain(
            keybindings
                .cautions()
                .iter()
                .map(|c| format!("caution: {c}")),
        )
        .collect()
}

/// The startup work on the keybindings files: the defaults copy's error (if
/// it failed) and the Alt notice (if this version has not shown it).
fn startup_files() -> (Option<String>, Option<String>) {
    let dir = agent_dir();
    let defaults_error = write_defaults_file(&defaults_path(&dir), VERSION).err();
    let keybindings = AppKeybindingsManager::create(None);
    let alt = take_alt_notice(
        &dir.join(STATE_FILE_NAME),
        VERSION,
        &keybindings.bound_keys(),
    );
    (defaults_error, alt)
}

impl Mode {
    /// The startup notices for the keybindings files, then the problem count.
    pub(super) fn show_keymap_startup_notes(&mut self) {
        let (defaults_error, alt) = startup_files();
        if let Some(error) = defaults_error {
            self.show_warning(&format!(
                "Keybindings defaults were not written: {error}. Your keys still load."
            ));
        }
        if let Some(notice) = alt {
            self.show_status(&notice);
        }
        self.show_keybinding_problems();
    }

    /// One warning line with the problem count, when the file has problems.
    pub(super) fn show_keybinding_problems(&mut self) {
        let count = self.keybinding_problems.len();
        if count > 0 {
            let noun = if count == 1 { "problem" } else { "problems" };
            self.show_warning(&format!(
                "keybindings.json: {count} {noun}. /hotkeys lists them."
            ));
        }
    }
}

//! Shared test setup: the process-wide theme and the default keybindings.

#![allow(dead_code)]

use std::path::Path;
use std::sync::{Mutex, MutexGuard, OnceLock};

use hoocode_code_tui_keybindings::AppKeybindingsManager;

pub fn lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("COLORTERM", "truecolor");
    // The agent dir is a temp dir, so the theme picker never reads the real
    // `~/.hoocode/themes/` and its golden lists only the built-ins.
    std::env::set_var("HOOCODE_CODING_AGENT_DIR", agent_dir());
    hoocode_code_tui_theme::init_theme(Some("dark"), false);
    hoocode_code_tui_theme::chalk::set_enabled(true);
    AppKeybindingsManager::create(Some(agent_dir())).install();
    guard
}

/// The look a golden is pinned in, on top of [`lock`]'s dark theme.
#[derive(Clone, Copy, Debug)]
pub enum Look {
    Dark,
    Light,
    /// Colour off (the NO_COLOR rules): shapes, bold and dim only.
    Mono,
}

/// Applies `look`. Call after [`lock`]: the lock restores the dark theme and
/// colour on.
pub fn look(look: Look) {
    match look {
        Look::Dark => {}
        Look::Light => hoocode_code_tui_theme::init_theme(Some("light"), false),
        Look::Mono => hoocode_code_tui_theme::chalk::set_enabled(false),
    }
}

pub fn agent_dir() -> &'static Path {
    static DIR: OnceLock<tempfile::TempDir> = OnceLock::new();
    DIR.get_or_init(|| tempfile::tempdir().unwrap()).path()
}

pub fn strip(lines: &[String]) -> String {
    hoocode_tui_util::strip_vt_control_characters(&lines.join("\n"))
}

/// Whether the drawn text carries the pane title `title` (`Tree  /tree`): the
/// chip in colour, or in monochrome the top border's `Tree ─ /tree`.
pub fn has_title(text: &str, title: &str) -> bool {
    text.contains(title) || text.contains(&title.replacen("  ", " ─ ", 1))
}

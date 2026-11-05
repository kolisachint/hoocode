//! A warning about a watched theme file survives a picker preview that does
//! not load: the watched theme is still on screen and its file is still broken.

use std::path::PathBuf;
use std::sync::MutexGuard;
use std::time::{Duration, Instant};

use hoocode_code_tui_theme::*;

/// A fresh agent dir with an empty `themes/`, pointed at by the env override.
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
}

/// Writes a broken theme until the watcher warns about it. The watcher takes
/// its first snapshot on its own thread, so an edit made at once can land in
/// that snapshot and go unseen. Each try has a new length, so a later poll
/// sees a change.
fn break_until_warned(file: &std::path::Path, name: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut tries = 0;
    while !warned(name) {
        assert!(Instant::now() < deadline, "the watcher did not reload");
        tries += 1;
        std::fs::write(file, format!("{{ not json{}", "!".repeat(tries))).unwrap();
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn warned(name: &str) -> bool {
    theme_warnings()
        .iter()
        .any(|w| w.contains(&format!("\"{name}\"")))
}

#[test]
fn a_failed_preview_keeps_the_warning_of_the_watched_file() {
    let agent = AgentDir::new();
    let file = agent.themes_dir().join("mine.json");
    std::fs::write(&file, builtin_theme_source("dark").unwrap()).unwrap();
    set_theme("mine", true).unwrap();

    // The file is edited into a broken state: the watcher keeps the theme and warns.
    break_until_warned(&file, "mine");

    // A preview of a theme that does not load changes nothing, so the warning stays.
    assert!(set_theme("no-such-theme", false).is_err());
    assert!(warned("mine"), "{:?}", theme_warnings());
    assert!(warned("no-such-theme"), "{:?}", theme_warnings());
    assert_eq!(current_theme_name().as_deref(), Some("mine"));

    stop_theme_watcher();
}

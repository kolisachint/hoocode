//! Shared test helpers: the theme and the startup-progress store are
//! process-wide, so tests that touch them run one at a time.
#![allow(dead_code)]

use std::sync::{Mutex, MutexGuard};

use hoocode_code_tui_theme::init_theme;

static LOCK: Mutex<()> = Mutex::new(());

/// Serialize on the process-wide state, with `theme` loaded and the
/// startup-progress store empty.
pub fn lock(theme: Option<&str>) -> MutexGuard<'static, ()> {
    let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Goldens were captured in truecolor.
    std::env::set_var("COLORTERM", "truecolor");
    init_theme(theme, false);
    hoocode_code_tui_theme::chalk::set_enabled(true);
    hoocode_code_tui_app::startup_progress::clear();
    guard
}

/// SGR colour codes stripped.
pub fn plain(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for c in chars.by_ref() {
                if c == 'm' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// The look a golden is pinned in, on top of [`lock`].
#[derive(Clone, Copy, Debug)]
pub enum Look {
    Dark,
    Light,
    /// Colour off (the NO_COLOR rules): shapes, bold and dim only.
    Mono,
}

/// Applies `look`. Call after [`lock`], which restores the theme and colour.
pub fn look(look: Look) {
    match look {
        Look::Dark => {}
        Look::Light => init_theme(Some("light"), false),
        Look::Mono => hoocode_code_tui_theme::chalk::set_enabled(false),
    }
}

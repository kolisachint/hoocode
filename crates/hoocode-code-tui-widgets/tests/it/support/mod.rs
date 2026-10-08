//! Shared test setup: the process-wide theme, one test at a time.

#![allow(dead_code)]

use std::sync::{Mutex, MutexGuard};

use hoocode_ai_types::{AssistantMessage, Content, StopReason};

pub fn lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("COLORTERM", "truecolor");
    hoocode_code_tui_theme::init_theme(Some("dark"), false);
    // `setKeybindings(new KeybindingsManager())`: the defaults, no user file.
    static DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let dir = DIR.get_or_init(|| tempfile::tempdir().unwrap());
    hoocode_code_tui_keybindings::AppKeybindingsManager::create(Some(dir.path())).install();
    // Plain text rendering: no images, no OSC 8.
    hoocode_tui_images::set_capabilities(hoocode_tui_images::TerminalCapabilities {
        images: None,
        true_color: true,
        hyperlinks: false,
    });
    guard
}

pub fn assistant(content: Vec<Content>) -> AssistantMessage {
    AssistantMessage {
        content,
        api: "openai-responses".into(),
        provider: "openai".into(),
        model: "gpt-4o-mini".into(),
        stop_reason: StopReason::Stop,
        ..Default::default()
    }
}

pub fn strip(s: &str) -> String {
    hoocode_code_tool_bash::shell::strip_ansi(s)
}

pub fn strip_all(lines: &[String]) -> String {
    strip(&lines.join("\n"))
}

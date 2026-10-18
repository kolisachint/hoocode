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

/// A paper-capable theme instance for the sheet tests: dark plus the cut-out
/// token set (values mirror the retired vox-cutout-dark). No builtin ships a
/// paper theme anymore, so the machinery tests use an instance instead.
pub fn paper_theme() -> hoocode_code_tui_theme::Theme {
    use hoocode_code_tui_theme::{builtin_theme_source, create_theme, parse_theme_json, ColorMode};
    let mut value: serde_json::Value =
        serde_json::from_str(builtin_theme_source("dark").unwrap()).unwrap();
    value["name"] = serde_json::Value::String("test-paper".into());
    let vars = value["vars"].as_object_mut().unwrap();
    for (k, v) in [
        ("shadow", "#837b68"),
        ("tapeYellow", "#ffe600"),
        ("tapeStrip", "#e8c98a"),
        ("track", "#a49b86"),
        ("ink", "#12100a"),
    ] {
        vars.insert(k.to_string(), serde_json::Value::String(v.to_string()));
    }
    let colors = value["colors"].as_object_mut().unwrap();
    for (k, v) in [
        ("paperShadow", "shadow"),
        ("headlineBg", "tapeYellow"),
        ("headlineText", "ink"),
        ("tapeBg", "tapeStrip"),
        ("tapeText", "ink"),
        ("halftone", "track"),
    ] {
        colors.insert(k.to_string(), serde_json::Value::String(v.to_string()));
    }
    let instance = parse_theme_json("test-paper", &value).unwrap();
    create_theme(&instance, Some(ColorMode::Truecolor), None).unwrap()
}

/// Switch the process-wide theme to the [`paper_theme`] instance.
pub fn init_paper_theme() {
    hoocode_code_tui_theme::set_theme_instance(paper_theme());
}

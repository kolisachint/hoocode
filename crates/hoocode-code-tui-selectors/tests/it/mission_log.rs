//! Component goldens: the `/resume` picker as the Sessions pane, at 100
//! and 80 columns. Hints use literal keys (no `alt`), so the bytes hold on
//! every platform. Accept a deliberate change with
//! `UPDATE_GOLDENS=1 cargo nextest run -p hoocode-code-tui-selectors mission_log`.

use std::path::PathBuf;
use std::rc::Rc;

use chrono::{DateTime, Utc};
use hoocode_code_session::SessionInfo;
use hoocode_code_tui_keybindings::AppKeybindingsManager;
use hoocode_code_tui_selectors::session_selector::{
    LoadSink, SessionSelectorComponent, SessionSelectorOptions,
};
use hoocode_tui_render::assert_golden;
use hoocode_tui_render::golden::render_golden;

use crate::support::{agent_dir, has_title, lock, look, Look};

fn make(id: &str, name: Option<&str>, first_message: &str) -> SessionInfo {
    SessionInfo {
        path: PathBuf::from(format!("/tmp/{id}.jsonl")),
        id: id.to_string(),
        cwd: String::new(),
        name: name.map(str::to_string),
        color: None,
        branch: None,
        parent_session_path: None,
        created: DateTime::<Utc>::UNIX_EPOCH,
        // A fixed date ahead of any clock: the age reads `now` every year.
        // (The epoch read `56y ago`, which drifts with the clock.)
        modified: DateTime::<Utc>::from_timestamp(4_102_444_800, 0).unwrap_or_default(),
        message_count: 5,
        model: None,
        all_messages_text: first_message.to_string(),
        subagent: None,
        first_message: first_message.to_string(),
    }
}

fn selector() -> SessionSelectorComponent {
    let sessions = vec![
        make("a", Some("wrap width"), "why does wrap_text ignore width"),
        make("b", None, "add a width check to wrap_text in src/wrap.rs"),
    ];
    let mut s = SessionSelectorComponent::new(
        Box::new(move |sink: LoadSink| sink.finish(Ok(sessions.clone()))),
        Box::new(|sink: LoadSink| sink.finish(Ok(Vec::new()))),
        Box::new(|_| {}),
        Box::new(|| {}),
        SessionSelectorOptions {
            keybindings: Some(Rc::new(
                AppKeybindingsManager::create(Some(agent_dir())).into_manager(),
            )),
            ..Default::default()
        },
        None,
    );
    s.poll();
    s
}

fn check(name: &str, width: u16) {
    check_look(name, width, Look::Dark);
}

fn check_look(name: &str, width: u16, look_: Look) {
    let _g = lock();
    look(look_);
    let mut s = selector();
    let rendered = render_golden(&mut s, width);
    let text = rendered.split("--- styles ---").next().unwrap_or_default();
    assert!(
        has_title(text, "Sessions  /resume"),
        "{name}: missing title in\n{text}"
    );
    assert!(
        text.contains("2 sessions"),
        "{name}: missing count in\n{text}"
    );
    assert!(
        text.lines()
            .any(|l| l.contains('›') && l.contains("wrap width")),
        "{name}: missing selected session in\n{text}"
    );
    assert!(
        text.lines()
            .any(|l| l.contains("▏") && l.contains("filter")),
        "{name}: missing query row in\n{text}"
    );
    assert!(
        text.contains("enter open"),
        "{name}: missing hint foot in\n{text}"
    );
    assert_golden!(name, rendered);
}

#[test]
fn mission_log_100() {
    check("mission_log_100", 100);
}

#[test]
fn mission_log_80() {
    check("mission_log_80", 80);
}

#[test]
fn mission_log_light_100() {
    check_look("mission_log_light_100", 100, Look::Light);
}

#[test]
fn mission_log_mono_100() {
    check_look("mission_log_mono_100", 100, Look::Mono);
}

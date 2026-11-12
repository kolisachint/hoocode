//! The picker keys that are in the keymap: the interrupted-run row's `v` and
//! `r` (session picker), Space in settings, and the scoped models category
//! key. Each test rebinds one key, checks that it acts and that the hint shows
//! its new name. The keymap is process-wide, so each test takes the lock and
//! restores the default on drop.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Mutex, MutexGuard};

use chrono::{DateTime, Utc};
use hoocode_ai_types::Model;
use hoocode_code_resources::package_resolve::{PathMetadata, ResolvedPaths, ResolvedResource};
use hoocode_code_resources::{SourceOrigin, SourceScope};
use hoocode_code_session::{SessionInfo, SubagentRun, SubagentState};
use hoocode_code_settings::{ScopedModel, SettingsManager};
use hoocode_code_tui_keybindings::AppKeybindingsManager;
use hoocode_code_tui_selectors::config_selector::ConfigSelectorComponent;
use hoocode_code_tui_selectors::scoped_models_selector::ScopedModelsSelectorComponent;
use hoocode_code_tui_selectors::session_selector::{
    ListEvent, LoadSink, SessionSelectorComponent, SessionSelectorOptions, SessionsLoader,
    SubagentRunRef,
};
use hoocode_tui_keys::KeybindingsManager;
use hoocode_tui_render::Component;
use hoocode_tui_util::strip_vt_control_characters;
use serde_json::json;

static LOCK: Mutex<()> = Mutex::new(());

/// Serializes on the process-wide keymap and installs a keymap given as flat
/// `keybindings.json` text. The default comes back when the guard drops.
struct Keymap {
    _lock: MutexGuard<'static, ()>,
}

fn rebind(text: &str) -> Keymap {
    let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("COLORTERM", "truecolor");
    hoocode_code_tui_theme::init_theme(Some("dark"), false);
    AppKeybindingsManager::from_json_str(text).install();
    Keymap { _lock: guard }
}

impl Drop for Keymap {
    fn drop(&mut self) {
        AppKeybindingsManager::default().install();
    }
}

fn plain(lines: Vec<String>) -> String {
    strip_vt_control_characters(&lines.join("\n"))
}

// ---- session picker: the interrupted-run row ---------------------------------

fn keybindings(text: &str) -> Rc<KeybindingsManager> {
    Rc::new(AppKeybindingsManager::from_json_str(text).into_manager())
}

fn session(id: &str, path: &str, parent: Option<&str>, run: Option<SubagentRun>) -> SessionInfo {
    let first = "scan the repo".to_string();
    SessionInfo {
        path: PathBuf::from(path),
        id: id.to_string(),
        cwd: String::new(),
        name: Some(id.to_string()),
        color: None,
        branch: None,
        parent_session_path: parent.map(str::to_string),
        created: DateTime::<Utc>::UNIX_EPOCH,
        modified: "2026-01-01T00:00:00Z".parse().unwrap(),
        message_count: 1,
        model: None,
        all_messages_text: first.clone(),
        subagent: run,
        first_message: first,
    }
}

/// A parent with one run that stopped before it settled.
fn interrupted_run_rows() -> Vec<SessionInfo> {
    let run = SubagentRun {
        task_id: "run-2".into(),
        agent_type: "general-purpose".into(),
        state: SubagentState::Interrupted,
    };
    vec![
        session("parent", "/tmp/parent-int.jsonl", None, None),
        session(
            "run-2",
            "/tmp/parent-int/subagents/run-2.jsonl",
            Some("/tmp/parent-int.jsonl"),
            Some(run),
        ),
    ]
}

fn fixed(sessions: Vec<SessionInfo>) -> SessionsLoader {
    Box::new(move |sink: LoadSink| sink.finish(Ok(sessions.clone())))
}

/// The picker with the interrupted run's choice open: the parent is at the top,
/// so one down step lands on the run and enter opens its choice.
fn open_choice(text: &str, resume: Rc<RefCell<Vec<SubagentRunRef>>>) -> SessionSelectorComponent {
    let options = SessionSelectorOptions {
        keybindings: Some(keybindings(text)),
        resume_subagent: Some(Box::new(move |run| resume.borrow_mut().push(run))),
        ..Default::default()
    };
    let mut s = SessionSelectorComponent::new(
        fixed(interrupted_run_rows()),
        fixed(Vec::new()),
        Box::new(|_| {}),
        Box::new(|| {}),
        options,
        None,
    );
    s.poll();
    press(&mut s, "\x1b[B");
    press(&mut s, "\r");
    s
}

fn press(s: &mut SessionSelectorComponent, data: &str) -> Vec<ListEvent> {
    let events = s.session_list().borrow_mut().handle_key(data);
    s.apply_list_events(events.clone());
    events
}

fn header(s: &mut SessionSelectorComponent) -> String {
    plain(s.render(120))
}

#[test]
fn rebound_view_key_views_the_run_and_the_header_shows_it() {
    let _k = rebind(r#"{"app.subagent.view": "b"}"#);
    let mut s = open_choice(
        r#"{"app.subagent.view": "b"}"#,
        Rc::new(RefCell::new(Vec::new())),
    );
    assert!(header(&mut s).contains("b view"), "{}", header(&mut s));
    let events = press(&mut s, "b");
    assert_eq!(
        events,
        vec![
            ListEvent::SubagentChoiceChange(false),
            ListEvent::Select(PathBuf::from("/tmp/parent-int/subagents/run-2.jsonl")),
        ]
    );
}

#[test]
fn rebound_resume_key_resumes_the_run_and_the_header_shows_it() {
    let resumed = Rc::new(RefCell::new(Vec::new()));
    let _k = rebind(r#"{"app.subagent.resume": "g"}"#);
    let mut s = open_choice(r#"{"app.subagent.resume": "g"}"#, resumed.clone());
    assert!(header(&mut s).contains("g resume"), "{}", header(&mut s));
    let events = press(&mut s, "g");
    assert!(
        events.contains(&ListEvent::SubagentChoiceChange(false)),
        "{events:?}"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, ListEvent::ResumeSubagent(_))),
        "{events:?}"
    );
    assert_eq!(resumed.borrow().len(), 1);
}

// ---- settings: Space toggles the picked row ----------------------------------

fn resource(path: &str, metadata: &PathMetadata) -> ResolvedResource {
    ResolvedResource {
        path: path.into(),
        enabled: true,
        metadata: metadata.clone(),
    }
}

#[test]
fn rebound_settings_toggle_writes_the_pattern_and_the_hint_shows_it() {
    let _k = rebind(r#"{"app.settings.toggle": "t"}"#);
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_string_lossy().into_owned();
    let agent = dir.path().join("agent").to_string_lossy().into_owned();
    let base = format!("{cwd}/{}", hoocode_code_paths::CONFIG_DIR_NAME);
    let metadata = PathMetadata::new(
        "auto",
        SourceScope::Project,
        SourceOrigin::TopLevel,
        Some(&base),
    );
    let resolved = ResolvedPaths {
        prompts: vec![resource(&format!("{base}/prompts/review.md"), &metadata)],
        ..ResolvedPaths::default()
    };
    let settings = Rc::new(RefCell::new(SettingsManager::create(&cwd, &agent)));
    let mut selector =
        ConfigSelectorComponent::new(&resolved, settings.clone(), &cwd, &agent, || {}, || {});

    assert!(
        plain(selector.render(80)).contains("t toggle"),
        "{}",
        plain(selector.render(80))
    );
    selector.handle_input("t");
    assert_eq!(
        settings.borrow().project_settings()["prompts"],
        json!(["-prompts/review.md"])
    );
}

// ---- scoped models: the category key ------------------------------------------

fn model(provider: &str, id: &str) -> Model {
    serde_json::from_value(json!({
        "id": id, "name": id, "api": "openai-completions", "provider": provider,
        "baseUrl": "https://example.test", "reasoning": true, "input": ["text"],
        "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0},
        "contextWindow": 128000, "maxTokens": 8192,
    }))
    .unwrap()
}

#[test]
fn rebound_category_key_cycles_the_category_and_the_hint_shows_it() {
    // `x` is clear-all by default, so the category moves to `t`.
    let _k = rebind(r#"{"app.models.category": "t"}"#);
    let scoped = vec![ScopedModel {
        model: "openai/gpt-5-mini".into(),
        effort: None,
        category: None,
        alias: None,
    }];
    let mut selector =
        ScopedModelsSelectorComponent::new(vec![model("openai", "gpt-5-mini")], Some(scoped));
    let before = plain(selector.render(100));
    assert!(before.contains("t category"), "{before}");
    assert!(!before.contains("cheap"), "{before}");
    selector.handle_input("t");
    let after = plain(selector.render(100));
    assert!(after.contains("cheap"), "{after}");
}

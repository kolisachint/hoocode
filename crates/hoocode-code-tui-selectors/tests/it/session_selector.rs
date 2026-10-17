//! Ports of `test/session-selector-rename.test.ts`,
//! `test/session-selector-path-delete.test.ts` and
//! `test/session-selector-branch.test.ts`.
//!
//! The pin's `await flushPromises()` is [`SessionSelectorComponent::poll`]
//! here, and the list's callbacks are the [`ListEvent`]s its keys return.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Mutex, MutexGuard, OnceLock};

use chrono::{DateTime, Utc};
use hoocode_code_session::identity::session_slug_for;
use hoocode_code_session::{SessionInfo, SubagentRun, SubagentState};
use hoocode_code_tui_keybindings::AppKeybindingsManager;
use hoocode_code_tui_selectors::session_selector::{
    ListEvent, LoadSink, SessionSelectorComponent, SessionSelectorOptions, SessionsLoader,
};
use hoocode_tui_keys::KeybindingsManager;
use hoocode_tui_render::Component;
use hoocode_tui_util::strip_vt_control_characters;

fn lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("COLORTERM", "truecolor");
    hoocode_code_tui_theme::init_theme(Some("dark"), false);
    AppKeybindingsManager::create(Some(agent_dir())).install();
    guard
}

fn agent_dir() -> &'static Path {
    static DIR: OnceLock<tempfile::TempDir> = OnceLock::new();
    DIR.get_or_init(|| tempfile::tempdir().unwrap()).path()
}

fn keybindings() -> Rc<KeybindingsManager> {
    Rc::new(AppKeybindingsManager::create(Some(agent_dir())).into_manager())
}

#[derive(Default)]
struct Make<'a> {
    path: Option<PathBuf>,
    name: Option<&'a str>,
    branch: Option<&'a str>,
    parent: Option<PathBuf>,
    modified: Option<&'a str>,
    first_message: Option<&'a str>,
}

fn make(id: &str, o: Make) -> SessionInfo {
    let first = o.first_message.unwrap_or("hello").to_string();
    SessionInfo {
        path: o
            .path
            .unwrap_or_else(|| PathBuf::from(format!("/tmp/{id}.jsonl"))),
        id: id.to_string(),
        cwd: String::new(),
        name: o.name.map(str::to_string),
        color: None,
        branch: o.branch.map(str::to_string),
        parent_session_path: o.parent.map(|p| p.to_string_lossy().into_owned()),
        created: DateTime::<Utc>::UNIX_EPOCH,
        modified: o
            .modified
            .map(|m| m.parse().unwrap())
            .unwrap_or(DateTime::<Utc>::UNIX_EPOCH),
        message_count: 1,
        all_messages_text: first.clone(),
        subagent: None,
        first_message: first,
    }
}

fn fixed(sessions: Vec<SessionInfo>) -> SessionsLoader {
    Box::new(move |sink: LoadSink| sink.finish(Ok(sessions.clone())))
}

fn selector(
    sessions: Vec<SessionInfo>,
    options: SessionSelectorOptions,
    current: Option<&Path>,
) -> SessionSelectorComponent {
    let mut s = SessionSelectorComponent::new(
        fixed(sessions),
        fixed(Vec::new()),
        Box::new(|_| {}),
        Box::new(|| {}),
        options,
        current,
    );
    s.poll();
    s
}

fn default_options() -> SessionSelectorOptions {
    SessionSelectorOptions {
        keybindings: Some(keybindings()),
        ..Default::default()
    }
}

fn render(s: &mut SessionSelectorComponent, width: u16) -> String {
    strip_vt_control_characters(&s.render(width).join("\n"))
}

/// Key input for an `alt+<char>` binding, read from the keybinding.
fn alt_key(id: &str) -> (String, String) {
    let key = keybindings().get_keys(id)[0].clone();
    let ch = key
        .strip_prefix("alt+")
        .unwrap_or_else(|| panic!("expected an alt+<char> binding for {id}, got {key}"));
    assert_eq!(ch.chars().count(), 1);
    (key.clone(), format!("\x1b{ch}"))
}

const CTRL_BACKSPACE: &str = "\x1b[127;5u";

/// Press a key on the list and let the selector act on it, as the pin's
/// list callbacks do.
fn press_list(s: &mut SessionSelectorComponent, data: &str) -> Vec<ListEvent> {
    let events = s.session_list().borrow_mut().handle_key(data);
    s.apply_list_events(events.clone());
    events
}

fn confirmation_changes(events: &[ListEvent]) -> Vec<Option<PathBuf>> {
    events
        .iter()
        .filter_map(|e| match e {
            ListEvent::DeleteConfirmationChange(p) => Some(p.clone()),
            _ => None,
        })
        .collect()
}

// ---- rename ---------------------------------------------------------------

#[test]
fn rename_key_opens_rename_mode_when_a_handler_is_set() {
    let _g = lock();
    let (_, rename_input) = alt_key("app.session.rename");
    let mut s = selector(
        vec![make("a", Make::default())],
        SessionSelectorOptions {
            rename_session: Some(Box::new(|_: &Path, _: &str| Ok(()))),
            show_rename_hint: Some(true),
            ..default_options()
        },
        None,
    );
    press_list(&mut s, &rename_input);
    let out = render(&mut s, 120);
    assert!(out.contains("Rename Session"), "{out}");
}

#[test]
fn hides_rename_hint_in_cli_resume_picker_configuration() {
    let _g = lock();
    let (rename_key, _) = alt_key("app.session.rename");
    let mut s = selector(
        vec![make("a", Make::default())],
        SessionSelectorOptions {
            show_rename_hint: Some(false),
            ..default_options()
        },
        None,
    );
    let out = render(&mut s, 120);
    assert!(!out.contains(&rename_key), "{out}");
    assert!(!out.contains("rename"), "{out}");
}

#[test]
fn enters_rename_mode_and_submits_with_enter() {
    let _g = lock();
    let (_, rename_input) = alt_key("app.session.rename");
    let session = make(
        "a",
        Make {
            name: Some("Old"),
            ..Default::default()
        },
    );
    let calls: Rc<RefCell<Vec<(PathBuf, String)>>> = Rc::default();
    let sink = calls.clone();
    let mut s = selector(
        vec![session.clone()],
        SessionSelectorOptions {
            rename_session: Some(Box::new(move |path: &Path, name: &str| {
                sink.borrow_mut()
                    .push((path.to_path_buf(), name.to_string()));
                Ok(())
            })),
            show_rename_hint: Some(true),
            ..default_options()
        },
        None,
    );

    press_list(&mut s, &rename_input);
    s.poll();

    let out = render(&mut s, 120);
    assert!(out.contains("Rename Session"), "{out}");
    assert!(!out.contains("Resume Session"), "{out}");

    s.handle_input("X");
    s.handle_input("\r");
    s.poll();

    assert_eq!(
        *calls.borrow(),
        vec![(session.path.clone(), "XOld".to_string())]
    );
}

// ---- path / delete ------------------------------------------------------------

fn two_sessions() -> Vec<SessionInfo> {
    vec![make("a", Make::default()), make("b", Make::default())]
}

#[test]
fn ctrl_backspace_is_not_delete_with_a_non_empty_query() {
    let _g = lock();
    let mut s = selector(two_sessions(), default_options(), None);
    let mut events = press_list(&mut s, "a");
    events.extend(press_list(&mut s, CTRL_BACKSPACE));
    assert!(confirmation_changes(&events).is_empty());
}

#[test]
fn delete_key_confirms_even_with_a_non_empty_query() {
    let _g = lock();
    let (_, delete) = alt_key("app.session.delete");
    let sessions = two_sessions();
    let mut s = selector(sessions.clone(), default_options(), None);
    let mut events = press_list(&mut s, "a");
    events.extend(press_list(&mut s, &delete));
    assert_eq!(
        confirmation_changes(&events),
        vec![Some(sessions[0].path.clone())]
    );
}

#[test]
fn ctrl_backspace_confirms_with_an_empty_query() {
    let _g = lock();
    let sessions = two_sessions();
    let s = selector(sessions.clone(), default_options(), None);
    let list = s.session_list();

    let events = list.borrow_mut().handle_key(CTRL_BACKSPACE);
    assert_eq!(
        confirmation_changes(&events),
        vec![Some(sessions[0].path.clone())]
    );

    // The pin replaces onDeleteSession here; the event is what it would receive.
    let events = list.borrow_mut().handle_key("\r");
    assert_eq!(confirmation_changes(&events), vec![None]);
    assert!(events.contains(&ListEvent::DeleteSession(sessions[0].path.clone())));
}

/// A loader for "all" that counts calls and finishes when the test says so.
fn deferred_all() -> (SessionsLoader, Rc<RefCell<Vec<LoadSink>>>) {
    let pending: Rc<RefCell<Vec<LoadSink>>> = Rc::default();
    let sink = pending.clone();
    (
        Box::new(move |s: LoadSink| sink.borrow_mut().push(s)),
        pending,
    )
}

fn scope_selector() -> (SessionSelectorComponent, Rc<RefCell<Vec<LoadSink>>>) {
    let (all_loader, pending) = deferred_all();
    let mut s = SessionSelectorComponent::new(
        fixed(vec![make("current", Make::default())]),
        all_loader,
        Box::new(|_| {}),
        Box::new(|| {}),
        default_options(),
        None,
    );
    s.poll();
    (s, pending)
}

#[test]
fn late_all_load_does_not_switch_scope_back_to_all() {
    let _g = lock();
    let (mut s, pending) = scope_selector();
    press_list(&mut s, "\t"); // current -> all (starts the load)
    press_list(&mut s, "\t"); // all -> current

    assert_eq!(pending.borrow().len(), 1);
    pending.borrow()[0].finish(Ok(vec![make("all", Make::default())]));
    s.poll();

    let out = render(&mut s, 120);
    assert!(out.contains("◉ Current Folder"), "{out}");
    assert!(!out.contains("◉ All"), "{out}");
}

#[test]
fn toggling_while_all_loads_starts_no_second_load() {
    let _g = lock();
    let (mut s, pending) = scope_selector();
    press_list(&mut s, "\t");
    press_list(&mut s, "\t");
    press_list(&mut s, "\t");
    assert_eq!(pending.borrow().len(), 1);
    pending.borrow()[0].finish(Ok(vec![make("all", Make::default())]));
    s.poll();
}

struct Aliases {
    _dir: tempfile::TempDir,
    parent_a: PathBuf,
    parent_b: PathBuf,
    child_b: PathBuf,
}

#[cfg(unix)]
fn symlinked_session_paths() -> Aliases {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();
    let shared = base.join("real").join("sessions");
    std::fs::create_dir_all(&shared).unwrap();
    for alias in ["alias-a", "alias-b"] {
        std::fs::create_dir_all(base.join(alias)).unwrap();
        std::os::unix::fs::symlink(&shared, base.join(alias).join("sessions")).unwrap();
    }
    std::fs::write(shared.join("parent.jsonl"), "parent\n").unwrap();
    std::fs::write(shared.join("child.jsonl"), "child\n").unwrap();
    Aliases {
        parent_a: base.join("alias-a/sessions/parent.jsonl"),
        parent_b: base.join("alias-b/sessions/parent.jsonl"),
        child_b: base.join("alias-b/sessions/child.jsonl"),
        _dir: dir,
    }
}

#[cfg(unix)]
#[test]
fn threads_sessions_across_symlink_aliases() {
    let _g = lock();
    let p = symlinked_session_paths();
    let sessions = vec![
        make(
            "parent",
            Make {
                path: Some(p.parent_b.clone()),
                name: Some("Parent"),
                modified: Some("2026-01-01T00:00:00Z"),
                ..Default::default()
            },
        ),
        make(
            "child",
            Make {
                path: Some(p.child_b.clone()),
                parent: Some(p.parent_a.clone()),
                name: Some("Child"),
                modified: Some("2025-12-31T00:00:00Z"),
                ..Default::default()
            },
        ),
    ];
    let mut s = selector(sessions, default_options(), None);
    let out = render(&mut s, 120);
    assert!(out.contains("Parent"), "{out}");
    assert!(out.contains("└─ Child"), "{out}");
}

#[cfg(unix)]
#[test]
fn current_session_is_active_across_symlink_aliases() {
    let _g = lock();
    let (_, delete) = alt_key("app.session.delete");
    let p = symlinked_session_paths();
    let sessions = vec![make(
        "parent",
        Make {
            path: Some(p.parent_b.clone()),
            name: Some("Parent"),
            ..Default::default()
        },
    )];
    let s = selector(sessions, default_options(), Some(&p.parent_a));
    let events = s.session_list().borrow_mut().handle_key(&delete);
    assert!(confirmation_changes(&events).is_empty());
    assert_eq!(
        events,
        vec![ListEvent::Error(
            "Cannot delete the currently active session".to_string()
        )]
    );
}

// ---- branch column -----------------------------------------------------------

fn render_sessions(sessions: Vec<SessionInfo>, width: u16) -> String {
    let mut s = selector(sessions, default_options(), None);
    render(&mut s, width)
}

fn vague(branch: Option<&'static str>, name: Option<&'static str>) -> SessionInfo {
    make(
        "a",
        Make {
            branch,
            name,
            first_message: Some("look at this"),
            ..Default::default()
        },
    )
}

#[test]
fn shows_the_branch_for_an_unnamed_vague_session() {
    let _g = lock();
    let out = render_sessions(vec![vague(Some("refactor-auth"), None)], 120);
    assert!(out.contains("refactor-auth"), "{out}");
    assert!(out.contains("look at this"), "{out}");
}

#[test]
fn hides_the_branch_for_a_named_session() {
    let _g = lock();
    let out = render_sessions(
        vec![vague(Some("refactor-auth"), Some("parser spike"))],
        120,
    );
    assert!(out.contains("parser spike"), "{out}");
    assert!(!out.contains("refactor-auth"), "{out}");
}

#[test]
fn hides_default_branches() {
    let _g = lock();
    for branch in ["main", "master", "trunk", "develop"] {
        let out = render_sessions(vec![vague(Some(branch), None)], 120);
        assert!(!out.contains(branch), "{branch}: {out}");
    }
}

#[test]
fn says_nothing_without_a_branch() {
    let _g = lock();
    let out = render_sessions(vec![vague(None, None)], 120);
    assert!(out.contains("look at this"), "{out}");
    assert!(out.contains(&session_slug_for("a")), "{out}");
}

#[test]
fn keeps_rows_inside_the_width() {
    let _g = lock();
    for width in [40u16, 60, 80, 120] {
        let out = render_sessions(
            vec![vague(Some("a-very-long-feature-branch-name-indeed"), None)],
            width,
        );
        for line in out.split('\n') {
            assert!(
                line.chars().count() <= width as usize,
                "width {width}: {line}"
            );
        }
    }
}

// ---- subagent runs ----------------------------------------------------------

/// A parent session and one subagent run of it, as the picker's loader lists them.
fn parent_and_run(state: SubagentState) -> Vec<SessionInfo> {
    let parent_path = PathBuf::from("/tmp/parent-run.jsonl");
    let parent = make(
        "parent",
        Make {
            path: Some(parent_path.clone()),
            name: Some("Parent"),
            modified: Some("2026-01-01T00:00:00Z"),
            ..Default::default()
        },
    );
    let mut run = make(
        "run-1",
        Make {
            path: Some(PathBuf::from("/tmp/parent-run/subagents/run-1.jsonl")),
            parent: Some(parent_path),
            first_message: Some("scan the repo"),
            modified: Some("2026-01-01T00:00:00Z"),
            ..Default::default()
        },
    );
    run.subagent = Some(SubagentRun {
        task_id: "run-1".into(),
        agent_type: "explore".into(),
        state,
    });
    vec![parent, run]
}

#[test]
fn a_subagent_run_nests_under_its_parent_in_the_threaded_view() {
    let _g = lock();
    let mut s = selector(
        parent_and_run(SubagentState::Finished("complete".into())),
        default_options(),
        None,
    );
    let out = render(&mut s, 120);
    assert!(out.contains("└─ scan the repo"), "{out}");
    assert!(out.contains("explore · complete"), "{out}");
}

#[test]
fn an_interrupted_or_running_run_says_so_in_its_row() {
    let _g = lock();
    let mut s = selector(
        parent_and_run(SubagentState::Interrupted),
        default_options(),
        None,
    );
    assert!(render(&mut s, 120).contains("explore · interrupted"));
    let mut s = selector(
        parent_and_run(SubagentState::Running),
        default_options(),
        None,
    );
    assert!(render(&mut s, 120).contains("explore · running"));
}

/// A search matches the parent rows only: a run whose own text matches is not
/// listed by itself, and a search does not draw the thread.
#[test]
fn a_search_matches_the_parent_rows_only() {
    let _g = lock();
    let sessions = parent_and_run(SubagentState::Finished("complete".into()));
    let mut s = selector(sessions, default_options(), None);
    // "scan" is only in the run's own text.
    for ch in "scan".chars() {
        press_list(&mut s, &ch.to_string());
    }
    let out = render(&mut s, 120);
    assert!(!out.contains("scan the repo"), "{out}");
    assert!(!out.contains("└─ "), "{out}");
}

#[test]
fn a_search_still_finds_the_parent() {
    let _g = lock();
    let sessions = parent_and_run(SubagentState::Finished("complete".into()));
    let mut s = selector(sessions, default_options(), None);
    for ch in "parent".chars() {
        press_list(&mut s, &ch.to_string());
    }
    let out = render(&mut s, 120);
    assert!(out.contains("Parent"), "{out}");
    assert!(!out.contains("└─ "), "{out}");
}

// ---- interrupted runs: view or resume ---------------------------------------

const DOWN_KEY: &str = "\x1b[B";

/// A parent with one run that stopped before it settled.
fn interrupted_run_rows() -> Vec<SessionInfo> {
    let parent_path = PathBuf::from("/tmp/parent-int.jsonl");
    let parent = make(
        "parent",
        Make {
            path: Some(parent_path.clone()),
            name: Some("Parent"),
            modified: Some("2026-01-01T00:00:00Z"),
            ..Default::default()
        },
    );
    let mut run = make(
        "run-2",
        Make {
            path: Some(PathBuf::from("/tmp/parent-int/subagents/run-2.jsonl")),
            parent: Some(parent_path),
            first_message: Some("audit the module"),
            modified: Some("2026-01-01T00:00:00Z"),
            ..Default::default()
        },
    );
    run.subagent = Some(SubagentRun {
        task_id: "run-2".into(),
        agent_type: "general-purpose".into(),
        state: SubagentState::Interrupted,
    });
    vec![parent, run]
}

fn options_with_resume(
    sink: Rc<RefCell<Vec<hoocode_code_tui_selectors::session_selector::SubagentRunRef>>>,
) -> SessionSelectorOptions {
    SessionSelectorOptions {
        keybindings: Some(keybindings()),
        resume_subagent: Some(Box::new(move |run| sink.borrow_mut().push(run))),
        ..Default::default()
    }
}

/// Enter on an interrupted run opens its choice, and the header says what the
/// keys do.
#[test]
fn enter_on_an_interrupted_run_opens_its_choice() {
    let _g = lock();
    let resumed = Rc::new(RefCell::new(Vec::new()));
    let mut s = selector(interrupted_run_rows(), options_with_resume(resumed), None);
    press_list(&mut s, DOWN_KEY);
    let events = press_list(&mut s, "\r");
    assert_eq!(events, vec![ListEvent::SubagentChoiceChange(true)]);
    let out = render(&mut s, 120);
    assert!(out.contains("v view · r resume · esc back"), "{out}");
}

/// `r` resumes the run the choice was opened on, with its parent and transcript.
#[test]
fn r_resumes_the_interrupted_run() {
    let _g = lock();
    let resumed = Rc::new(RefCell::new(Vec::new()));
    let mut s = selector(
        interrupted_run_rows(),
        options_with_resume(resumed.clone()),
        None,
    );
    press_list(&mut s, DOWN_KEY);
    press_list(&mut s, "\r");
    let events = press_list(&mut s, "r");
    assert_eq!(
        events,
        vec![
            ListEvent::SubagentChoiceChange(false),
            ListEvent::ResumeSubagent(
                hoocode_code_tui_selectors::session_selector::SubagentRunRef {
                    child: PathBuf::from("/tmp/parent-int/subagents/run-2.jsonl"),
                    parent: PathBuf::from("/tmp/parent-int.jsonl"),
                    task_id: "run-2".into(),
                    agent_type: "general-purpose".into(),
                }
            ),
        ]
    );
    assert_eq!(resumed.borrow().len(), 1);
}

/// `v` views the transcript: the run's file is opened like any session.
#[test]
fn v_views_the_interrupted_run() {
    let _g = lock();
    let mut s = selector(interrupted_run_rows(), default_options(), None);
    press_list(&mut s, DOWN_KEY);
    press_list(&mut s, "\r");
    let events = press_list(&mut s, "v");
    assert_eq!(
        events,
        vec![
            ListEvent::SubagentChoiceChange(false),
            ListEvent::Select(PathBuf::from("/tmp/parent-int/subagents/run-2.jsonl")),
        ]
    );
}

/// Escape closes the choice and leaves the picker open.
#[test]
fn esc_backs_out_of_the_choice_without_cancelling_the_picker() {
    let _g = lock();
    let mut s = selector(interrupted_run_rows(), default_options(), None);
    press_list(&mut s, DOWN_KEY);
    press_list(&mut s, "\r");
    let events = press_list(&mut s, "\x1b");
    assert_eq!(events, vec![ListEvent::SubagentChoiceChange(false)]);
}

/// Without a resume callback (the `--resume` picker) the choice offers view
/// only, and `r` does nothing.
#[test]
fn without_a_resume_callback_r_does_nothing() {
    let _g = lock();
    let mut s = selector(interrupted_run_rows(), default_options(), None);
    press_list(&mut s, DOWN_KEY);
    press_list(&mut s, "\r");
    let events = press_list(&mut s, "r");
    assert!(events.is_empty(), "{events:?}");
    assert!(render(&mut s, 120).contains("v view · esc back"));
}

/// Enter on a run that settled opens its transcript directly, no choice.
#[test]
fn enter_on_a_finished_run_opens_its_transcript() {
    let _g = lock();
    let mut rows = interrupted_run_rows();
    if let Some(run) = rows[1].subagent.as_mut() {
        run.state = SubagentState::Finished("complete".into());
    }
    let mut s = selector(rows, default_options(), None);
    press_list(&mut s, DOWN_KEY);
    let events = press_list(&mut s, "\r");
    assert_eq!(
        events,
        vec![ListEvent::Select(PathBuf::from(
            "/tmp/parent-int/subagents/run-2.jsonl"
        ))]
    );
}

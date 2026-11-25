//! Detached sessions in the task panel (SA-12): a session the open session spawned shows a
//! row whose state follows its file, and a change to parked or failed raises one notice.

use hoocode_agent_types::AgentMessage;
use hoocode_ai_types::{Content, TextContent, ToolResultMessage};
use hoocode_code_session::detached::{DETACHED_EXIT_ENTRY, PARKED_CALL_ENTRY};
use hoocode_code_session::{
    append_custom_entry_to_file, DetachedStatus, SessionManager, SESSION_FLUSH_DEADLINE,
};
use hoocode_code_task_store::{task_store, TaskKind, TaskSource, TaskStatus};
use serde_json::json;

use hoocode_code_subagents::detached_child::record_spawn;

use super::detached::{row_key, transition_notice};
use super::test_support::test_mode;
use super::*;

/// The texts of the notices on screen: the showing one, then the queued ones.
fn notice_texts(mode: &Mode) -> Vec<String> {
    let notifications = mode.notifications.borrow();
    notifications
        .showing()
        .into_iter()
        .chain(notifications.pending())
        .map(|n| n.title.clone())
        .collect()
}

/// A child session file in `dir` with one entry of `custom_type`, written and closed.
fn child_session(dir: &std::path::Path, custom_type: &str, data: serde_json::Value) -> PathBuf {
    let mut child = SessionManager::create("/work", Some(dir.to_path_buf()));
    let path = child.session_file().unwrap().to_path_buf();
    child.set_flush_each_entry(true);
    child.append_custom_entry(custom_type, Some(data));
    child.flush(SESSION_FLUSH_DEADLINE).unwrap();
    drop(child);
    path
}

/// A `Mode` whose session has a successful `SessionSpawn` result naming `child`.
fn parent_that_spawned(dir: &std::path::Path, child: &std::path::Path, goal: &str) -> Mode {
    let mode = test_mode(Some(dir));
    let text = format!(
        "Started a detached session: {goal}\n\nOpen it with:\n\n```\nhoocode --session {}\n```\n\nStatus: running.",
        child.display()
    );
    mode.session
        .session_manager()
        .append_message(AgentMessage::ToolResult(ToolResultMessage {
            tool_call_id: "spawn-1".into(),
            tool_name: "SessionSpawn".into(),
            content: vec![Content::Text(TextContent {
                text,
                text_signature: None,
            })],
            is_error: false,
            ..Default::default()
        }));
    mode
}

#[test]
fn a_parked_session_this_process_spawned_shows_a_parked_row_and_one_notice_then_done_settles_it() {
    let dir = tempfile::tempdir().unwrap();
    let child = child_session(
        dir.path(),
        PARKED_CALL_ENTRY,
        json!({"toolCallId": "c1", "toolName": "Write", "reason": "why", "target": null}),
    );
    let mut mode = test_mode(Some(dir.path()));
    mode.tick_detached();
    // The spawn the tool records while this session is open.
    record_spawn(child.clone(), "Fix the parser".into());
    mode.tick_detached();
    let row = task_store()
        .find_keyed(&row_key(&child))
        .expect("a row for the spawned session");
    assert_eq!(row.kind(), TaskKind::Detached);
    assert_eq!(row.source, Some(TaskSource::Detached));
    assert_eq!(row.title, "Fix the parser");
    assert_eq!(row.status, TaskStatus::Pending, "parked");
    assert_eq!(row.note.as_deref(), Some("parked"));
    assert_eq!(row.log_path.as_deref(), Some(child.as_path()));
    let notes = notice_texts(&mode);
    let parked: Vec<&String> = notes.iter().filter(|t| t.contains("is parked")).collect();
    assert_eq!(parked.len(), 1, "one notice: {notes:?}");
    assert_eq!(
        parked[0],
        &format!(
            "Detached session \"Fix the parser\" is parked: open it with hoocode --session {}",
            child.display()
        )
    );

    // A parked session is polled again, but a status that did not change raises nothing.
    mode.tick_detached();
    assert_eq!(
        notice_texts(&mode)
            .iter()
            .filter(|t| t.contains("is parked"))
            .count(),
        1
    );

    // The parked call is answered and the run ends: the row is done, with no notice.
    {
        let mut manager = SessionManager::open(&child, None, None);
        manager.set_flush_each_entry(true);
        manager.append_message(AgentMessage::ToolResult(ToolResultMessage {
            tool_call_id: "c1".into(),
            tool_name: "Write".into(),
            content: vec![Content::Text(TextContent {
                text: "allowed".into(),
                text_signature: None,
            })],
            ..Default::default()
        }));
    }
    append_custom_entry_to_file(&child, DETACHED_EXIT_ENTRY, json!({"status": "done"})).unwrap();
    mode.tick_detached();
    let row = task_store().find_keyed(&row_key(&child)).unwrap();
    assert_eq!(row.status, TaskStatus::Done);
    assert_eq!(row.note.as_deref(), Some("done"));
    assert_eq!(notice_texts(&mode).len(), 1, "no notice for done");
}

#[test]
fn a_run_that_ends_is_done_and_settled() {
    let dir = tempfile::tempdir().unwrap();
    let child = child_session(dir.path(), DETACHED_EXIT_ENTRY, json!({"status": "done"}));
    let mut mode = test_mode(Some(dir.path()));
    mode.tick_detached();
    record_spawn(child.clone(), "Write the docs".into());
    mode.tick_detached();
    let row = task_store().find_keyed(&row_key(&child)).unwrap();
    assert_eq!(row.status, TaskStatus::Done);
    assert_eq!(row.title, "Write the docs");
    assert!(notice_texts(&mode)
        .iter()
        .all(|t| !t.contains("Write the docs")));
}

#[test]
fn a_session_restored_on_resume_shows_its_state_and_no_notice() {
    let dir = tempfile::tempdir().unwrap();
    let child = child_session(
        dir.path(),
        PARKED_CALL_ENTRY,
        json!({"toolCallId": "c1", "toolName": "Write", "reason": "why", "target": null}),
    );
    // The open session's own result names the child: the rows come back from it.
    let mut mode = parent_that_spawned(dir.path(), &child, "Fix the parser");
    mode.tick_detached();
    let row = task_store().find_keyed(&row_key(&child)).unwrap();
    assert_eq!(row.status, TaskStatus::Pending, "parked");
    assert!(notice_texts(&mode).iter().all(|t| !t.contains("parked")));
}

#[test]
fn a_failed_session_names_its_reason_in_the_row_and_the_notice() {
    let dir = tempfile::tempdir().unwrap();
    let child = child_session(
        dir.path(),
        DETACHED_EXIT_ENTRY,
        json!({"status": "failed", "reason": "turn cap of 200 reached"}),
    );
    let mut mode = test_mode(Some(dir.path()));
    mode.tick_detached();
    record_spawn(child.clone(), "Port the lexer".into());
    mode.tick_detached();
    let row = task_store().find_keyed(&row_key(&child)).unwrap();
    assert_eq!(row.status, TaskStatus::Failed);
    assert_eq!(row.note.as_deref(), Some("failed: turn cap of 200 reached"));
    let notes = notice_texts(&mode);
    assert_eq!(
        notes
            .iter()
            .filter(|t| t.contains("failed: turn cap of 200 reached"))
            .count(),
        1,
        "{notes:?}"
    );
}

#[test]
fn a_failed_call_names_no_session_and_adds_no_row() {
    let dir = tempfile::tempdir().unwrap();
    let mode = test_mode(Some(dir.path()));
    mode.session
        .session_manager()
        .append_message(AgentMessage::ToolResult(ToolResultMessage {
            tool_call_id: "spawn-2".into(),
            tool_name: "SessionSpawn".into(),
            content: vec![Content::Text(TextContent {
                text: "SessionSpawn: the brief is missing: Goal".into(),
                text_signature: None,
            })],
            is_error: true,
            ..Default::default()
        }));
    let mut mode = mode;
    mode.tick_detached();
    assert!(mode.detached.rows.is_empty());
}

#[test]
fn only_parked_and_failed_changes_raise_a_notice() {
    let path = std::path::Path::new("/work/child.jsonl");
    assert!(transition_notice("g", path, &DetachedStatus::Done).is_none());
    assert!(transition_notice("g", path, &DetachedStatus::Running { pid: 1 }).is_none());
    let (kind, text) = transition_notice("g", path, &DetachedStatus::Parked).unwrap();
    assert_eq!(kind, NotificationKind::Info);
    assert_eq!(
        text,
        "Detached session \"g\" is parked: open it with hoocode --session /work/child.jsonl"
    );
    let (kind, text) = transition_notice(
        "g",
        path,
        &DetachedStatus::Failed {
            reason: "boom".into(),
        },
    )
    .unwrap();
    assert_eq!(kind, NotificationKind::Warning);
    assert_eq!(text, "Detached session \"g\" failed: boom");
}

//! Subagent transcripts: where they live, how a child names its parent, and
//! what a parent records about its runs.

use std::path::{Path, PathBuf};

use hoocode_agent_types::AgentMessage;
use hoocode_ai_types::{AssistantMessage, Content, Message, StopReason, UserMessage};
use hoocode_code_session::{
    append_custom_entry_to_file, load_entries_from_file, session_id_of_file, subagents_dir,
    FileEntry, SessionManager, SESSION_FLUSH_DEADLINE,
};

fn user(text: &str) -> AgentMessage {
    AgentMessage::from_message(Message::User(UserMessage {
        content: vec![Content::text(text)].into(),
        timestamp: 0,
    }))
}

fn assistant(text: &str) -> AgentMessage {
    AgentMessage::Assistant(AssistantMessage {
        content: vec![Content::text(text)],
        api: "openai-completions".into(),
        provider: "openai".into(),
        model: "test".into(),
        stop_reason: StopReason::Stop,
        timestamp: 0,
        ..Default::default()
    })
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "subagent-transcripts-{tag}-{}-{}",
        std::process::id(),
        hoocode_code_session::create_session_id()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// A session file on disk, with a user and an assistant message.
fn parent_with_history(dir: &Path) -> PathBuf {
    let mut manager = SessionManager::create("/work", Some(dir.to_path_buf()));
    let path = manager.session_file().expect("persisted").to_path_buf();
    manager.append_message(user("question"));
    manager.append_message(assistant("answer"));
    manager
        .flush(SESSION_FLUSH_DEADLINE)
        .expect("writes reached the OS");
    path
}

#[test]
fn the_transcript_dir_is_a_sibling_named_after_the_session() {
    let session = Path::new("/home/u/.hoocode/sessions/-w/2026-10-14T00-00-00-000Z_abc.jsonl");
    assert_eq!(
        subagents_dir(session),
        Path::new("/home/u/.hoocode/sessions/-w/2026-10-14T00-00-00-000Z_abc/subagents")
    );
}

#[test]
fn a_child_transcript_names_its_parent_once_it_is_written() {
    let dir = temp_dir("header");
    let parent = parent_with_history(&dir);
    let child = subagents_dir(&parent).join("child-1.jsonl");
    let mut manager = SessionManager::open(child.clone(), None, None);
    manager.set_parent_session(parent.to_string_lossy().into_owned());
    manager.append_message(user("scan the repo"));
    manager.append_message(assistant("done"));
    manager
        .flush(SESSION_FLUSH_DEADLINE)
        .expect("writes reached the OS");
    let reloaded = SessionManager::open(child.clone(), None, None);
    assert_eq!(
        reloaded.header().parent_session.as_deref(),
        Some(parent.to_string_lossy().as_ref())
    );
    assert_eq!(reloaded.session_file(), Some(child.as_path()));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_session_with_history_keeps_its_header_parent() {
    let dir = temp_dir("keep");
    let parent = parent_with_history(&dir);
    let mut manager = SessionManager::open(parent.clone(), None, None);
    manager.set_parent_session("/elsewhere/other.jsonl");
    assert_eq!(manager.header().parent_session, None);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_custom_entry_appended_to_a_file_chains_onto_its_last_entry() {
    let dir = temp_dir("append");
    let parent = parent_with_history(&dir);
    let before = load_entries_from_file(&parent);
    let last = before.last().and_then(|e| e.id().map(String::from));
    let id = append_custom_entry_to_file(
        &parent,
        "subagent_run",
        serde_json::json!({"taskId": "t1", "outcome": null}),
    )
    .expect("appended");
    session_flush();
    let after = load_entries_from_file(&parent);
    let Some(FileEntry::Custom {
        id: got,
        parent_id,
        custom_type,
        data,
        ..
    }) = after.last()
    else {
        panic!("the last entry is the custom one, got {after:?}");
    };
    assert_eq!(got, &id);
    assert_eq!(parent_id, &last);
    assert_eq!(custom_type, "subagent_run");
    assert_eq!(data.as_ref().unwrap()["taskId"], "t1");
    assert_eq!(session_id_of_file(&parent), Some(header_id(&after)));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn appending_to_a_missing_file_is_an_error() {
    let dir = temp_dir("missing");
    let err = append_custom_entry_to_file(
        &dir.join("nope.jsonl"),
        "subagent_run",
        serde_json::json!({}),
    );
    assert!(err.is_err());
    let _ = std::fs::remove_dir_all(dir);
}

fn session_flush() {
    let _ = hoocode_code_session::flush_session_writes(SESSION_FLUSH_DEADLINE);
}

fn header_id(entries: &[FileEntry]) -> String {
    match entries.first() {
        Some(FileEntry::Session(header)) => header.id.clone(),
        other => panic!("no header first: {other:?}"),
    }
}

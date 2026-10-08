//! Session entries go through the hoocode-session-io writer: they reach the file
//! in order, a flush waits for them, and a torn last line is skipped on read
//! and does not swallow the next entry.

use std::io::Write;
use std::path::PathBuf;

use hoocode_agent_types::AgentMessage;
use hoocode_ai_types::{AssistantMessage, Content, Message, StopReason, UserMessage};
use hoocode_code_session::{load_session_file, FileEntry, SessionManager, SESSION_FLUSH_DEADLINE};

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
        "session-writes-{tag}-{}-{}",
        std::process::id(),
        hoocode_code_session::create_session_id()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn message_count(entries: &[FileEntry]) -> usize {
    entries
        .iter()
        .filter(|e| matches!(e, FileEntry::Message { .. }))
        .count()
}

#[test]
fn entries_reach_the_file_in_order_and_the_file_reopens_intact() {
    let dir = temp_dir("order");
    let mut manager = SessionManager::create("/work", Some(dir.clone()));
    let path = manager.session_file().expect("persisted").to_path_buf();
    manager.append_message(user("q0"));
    manager.append_message(assistant("a0"));
    manager.append_message(user("q1"));
    manager.append_message(assistant("a1"));
    manager
        .flush(SESSION_FLUSH_DEADLINE)
        .expect("queued writes reached the OS");

    let raw = std::fs::read_to_string(&path).expect("session file");
    assert!(raw.ends_with('\n'), "every line is terminated");
    assert_eq!(raw.lines().count(), 5, "header plus four entries");

    let reopened = SessionManager::open(&path, None, None);
    assert_eq!(reopened.entries().len(), 4);
    let loaded = load_session_file(&path);
    assert_eq!(loaded.entries.len(), 5);
    assert_eq!(loaded.unrecognized, 0);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_torn_last_line_is_skipped_and_the_next_entry_survives() {
    let dir = temp_dir("torn");
    let mut manager = SessionManager::create("/work", Some(dir.clone()));
    let path = manager.session_file().expect("persisted").to_path_buf();
    manager.append_message(user("q0"));
    manager.append_message(assistant("a0"));
    manager.flush(SESSION_FLUSH_DEADLINE).expect("flushed");
    drop(manager);

    // A crash mid-write: a partial line with no newline.
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("open");
    file.write_all(br#"{"type":"message","id":"torn"#)
        .expect("tear");
    drop(file);

    let before = load_session_file(&path).entries.len();
    assert_eq!(before, 3, "the torn line is not an entry");

    let mut reopened = SessionManager::open(&path, None, None);
    reopened.append_message(user("after the crash"));
    reopened
        .flush(SESSION_FLUSH_DEADLINE)
        .expect("flushed after the crash");
    let after = load_session_file(&path);
    assert_eq!(after.entries.len(), 4, "the new entry is readable");
    assert_eq!(message_count(&after.entries), 3);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn in_memory_sessions_never_touch_the_writer() {
    let mut manager = SessionManager::in_memory("/work");
    manager.append_message(user("q"));
    manager.append_message(assistant("a"));
    assert!(manager.session_file().is_none());
    manager
        .flush(SESSION_FLUSH_DEADLINE)
        .expect("nothing queued");
}

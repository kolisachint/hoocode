//! Subagent transcripts: where they live, how a child names its parent, and
//! what a parent records about its runs.

use std::path::{Path, PathBuf};

use hoocode_agent_types::AgentMessage;
use hoocode_ai_types::{AssistantMessage, Content, Message, StopReason, UserMessage};
use hoocode_code_session::{
    append_custom_entry_to_file, build_session_info, list_subagent_sessions,
    load_entries_from_file, session_id_of_file, subagents_dir, FileEntry, SessionManager,
    SubagentState, SESSION_FLUSH_DEADLINE, SUBAGENT_RUN_ENTRY,
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

/// The parent's listing of its runs: each transcript is a run, with the state
/// its newest `subagent_run` record and the liveness check give.
#[test]
fn a_parents_runs_are_listed_with_their_state() {
    let dir = temp_dir("runs");
    let parent = parent_with_history(&dir);
    let transcripts = subagents_dir(&parent);
    std::fs::create_dir_all(&transcripts).unwrap();
    for name in ["run-a.jsonl", "run-b.jsonl", "run-c.jsonl"] {
        let child = transcripts.join(name);
        let mut manager = SessionManager::open(child.clone(), None, None);
        manager.set_parent_session(parent.to_string_lossy().into_owned());
        manager.append_message(user("scan the repo"));
        manager.append_message(assistant("done"));
        manager
            .flush(SESSION_FLUSH_DEADLINE)
            .expect("writes reached the OS");
    }
    let record = |task: &str, child: &str, outcome: serde_json::Value| {
        append_custom_entry_to_file(
            &parent,
            SUBAGENT_RUN_ENTRY,
            serde_json::json!({
                "taskId": task,
                "agentType": "explore",
                "description": "scan the repo",
                "childFile": child,
                "outcome": outcome,
            }),
        )
        .expect("record appended");
    };
    // run-a: dispatched, then settled.
    record("run-a", "run-a.jsonl", serde_json::Value::Null);
    record("run-a", "run-a.jsonl", serde_json::json!("complete"));
    // run-b: dispatched, never settled, its process gone.
    record("run-b", "run-b.jsonl", serde_json::Value::Null);
    // run-c: dispatched, never settled, its process still alive.
    record("run-c", "run-c.jsonl", serde_json::Value::Null);
    session_flush();

    let parent_info = build_session_info(&parent).expect("the parent lists");
    let runs = list_subagent_sessions(&[parent_info], &|task, _cwd| task == "run-c");
    assert_eq!(runs.len(), 3, "{runs:?}");
    let by_task = |task: &str| {
        runs.iter()
            .find(|r| r.subagent.as_ref().is_some_and(|s| s.task_id == task))
            .unwrap_or_else(|| panic!("no row for {task}"))
    };
    for task in ["run-a", "run-b", "run-c"] {
        let row = by_task(task);
        assert_eq!(
            row.parent_session_path.as_deref(),
            Some(parent.to_string_lossy().as_ref())
        );
        assert_eq!(row.subagent.as_ref().unwrap().agent_type, "explore");
    }
    assert_eq!(
        by_task("run-a").subagent.as_ref().unwrap().state,
        SubagentState::Finished("complete".into())
    );
    assert_eq!(
        by_task("run-b").subagent.as_ref().unwrap().state,
        SubagentState::Interrupted
    );
    assert_eq!(
        by_task("run-c").subagent.as_ref().unwrap().state,
        SubagentState::Running
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A session without a subagents dir has no runs, and the parent's own
/// listing is not changed by its runs.
#[test]
fn a_session_with_no_runs_lists_none() {
    let dir = temp_dir("no-runs");
    let parent = parent_with_history(&dir);
    let parent_info = build_session_info(&parent).expect("the parent lists");
    assert!(list_subagent_sessions(&[parent_info], &|_, _| false).is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

/// A run nests under the session whose directory holds it, even when its header
/// names a path that is gone (the session dir was moved or copied).
#[test]
fn a_run_nests_under_the_parent_it_is_found_in_not_its_header() {
    let dir = temp_dir("moved");
    let parent = parent_with_history(&dir);
    let transcripts = subagents_dir(&parent);
    std::fs::create_dir_all(&transcripts).unwrap();
    let child = transcripts.join("run-m.jsonl");
    let mut manager = SessionManager::open(child.clone(), None, None);
    manager.set_parent_session("/old-home/sessions/gone.jsonl");
    manager.append_message(user("scan the repo"));
    manager.append_message(assistant("done"));
    manager
        .flush(SESSION_FLUSH_DEADLINE)
        .expect("writes reached the OS");
    let parent_info = build_session_info(&parent).expect("the parent lists");
    let runs = list_subagent_sessions(&[parent_info], &|_, _| false);
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_eq!(
        runs[0].parent_session_path.as_deref(),
        Some(parent.to_string_lossy().as_ref())
    );
    let _ = std::fs::remove_dir_all(dir);
}

fn run_record(parent: &Path, task: &str) -> String {
    append_custom_entry_to_file(
        parent,
        SUBAGENT_RUN_ENTRY,
        serde_json::json!({"taskId": task, "childFile": format!("{task}.jsonl"), "outcome": null}),
    )
    .expect("record appended")
}

/// Whether any node in the tree is a subagent run record.
fn tree_has_run_record(nodes: &[hoocode_code_session::SessionTreeNode]) -> bool {
    nodes.iter().any(|n| {
        matches!(&n.entry, FileEntry::Custom { custom_type, .. } if custom_type == SUBAGENT_RUN_ENTRY)
            || tree_has_run_record(&n.children)
    })
}

/// A run's record is written while the parent is open in the app, and the app
/// then writes its next message. The record is not a branch of the conversation.
#[test]
fn a_run_record_written_mid_turn_is_not_a_fork_in_the_tree() {
    let dir = temp_dir("mid-turn");
    let parent = parent_with_history(&dir);
    let mut open = SessionManager::open(parent.clone(), None, None);
    run_record(&parent, "run-1");
    session_flush();
    open.append_message(user("next"));
    open.flush(SESSION_FLUSH_DEADLINE).expect("flushed");

    let reloaded = SessionManager::open(parent.clone(), None, None);
    let roots = reloaded.tree();
    assert!(
        !tree_has_run_record(&roots),
        "a run record is not a node of the session tree"
    );
    assert_eq!(roots.len(), 1, "one root");
    let mut node = &roots[0];
    let mut depth = 1;
    while let Some(child) = node.children.first() {
        assert_eq!(node.children.len(), 1, "no fork in a linear conversation");
        node = child;
        depth += 1;
    }
    assert_eq!(depth, 3, "user, assistant, next");
    let _ = std::fs::remove_dir_all(dir);
}

/// A record written after the last message does not move the leaf: the next
/// message continues the conversation, not the record.
#[test]
fn a_run_record_after_the_last_message_is_not_the_leaf() {
    let dir = temp_dir("leaf");
    let parent = parent_with_history(&dir);
    let last_message = load_entries_from_file(&parent)
        .last()
        .and_then(|e| e.id().map(String::from));
    run_record(&parent, "run-1");
    session_flush();

    let mut reopened = SessionManager::open(parent.clone(), None, None);
    assert_eq!(reopened.leaf_id().map(String::from), last_message);
    reopened.append_message(user("next"));
    reopened.flush(SESSION_FLUSH_DEADLINE).expect("flushed");
    let entries = load_entries_from_file(&parent);
    let next = entries.last().expect("a last entry");
    assert_eq!(
        next.parent_id().map(String::from),
        last_message,
        "the next message continues the conversation"
    );
    let _ = std::fs::remove_dir_all(dir);
}

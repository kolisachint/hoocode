//! `restore_plan`: a loaded session's plan is its last successful TodoWrite.

use hoocode_agent_types::AgentMessage;
use hoocode_ai_types::{
    AssistantMessage, Content, StopReason, ToolCallContent, ToolResultMessage, UserMessage,
};
use hoocode_code_task_store::{CreateTaskOptions, TaskStatus, TaskStore};
use hoocode_code_tools_optin::restore_plan;
use serde_json::{json, Value};

fn user() -> AgentMessage {
    AgentMessage::User(UserMessage {
        content: vec![Content::text("go")].into(),
        timestamp: 1,
    })
}

fn todo_call(id: &str, todos: Value, stop: StopReason) -> AgentMessage {
    AgentMessage::Assistant(AssistantMessage {
        content: vec![Content::ToolCall(ToolCallContent {
            id: id.into(),
            name: "TodoWrite".into(),
            arguments: json!({"todos": todos}),
            thought_signature: None,
        })],
        api: "anthropic-messages".into(),
        provider: "anthropic".into(),
        model: "claude-sonnet-4-5".into(),
        stop_reason: stop,
        timestamp: 1,
        ..Default::default()
    })
}

fn result(id: &str, is_error: bool) -> AgentMessage {
    AgentMessage::ToolResult(ToolResultMessage {
        duration_ms: None,
        tool_call_id: id.into(),
        tool_name: "TodoWrite".into(),
        content: vec![Content::text("done")],
        details: None,
        is_error,
        timestamp: 1,
    })
}

/// (title, status) of each plan item, in order.
fn plan(store: &TaskStore) -> Vec<(String, TaskStatus)> {
    store
        .list()
        .into_iter()
        .filter(|t| t.is_plan_item())
        .map(|t| (t.title, t.status))
        .collect()
}

#[test]
fn the_plan_is_the_last_todo_write_that_succeeded() {
    let store = TaskStore::new();
    let messages = vec![
        user(),
        todo_call(
            "t1",
            json!([
                {"content": "Read", "status": "completed"},
                {"content": "Ship", "status": "pending"},
            ]),
            StopReason::Stop,
        ),
        result("t1", false),
        todo_call(
            "t2",
            json!([{"content": "Other", "status": "pending"}]),
            StopReason::Stop,
        ),
        result("t2", true),
    ];
    restore_plan(&store, &messages);
    assert_eq!(
        plan(&store),
        vec![
            ("Read".to_string(), TaskStatus::Done),
            ("Ship".to_string(), TaskStatus::Pending),
        ]
    );
}

#[test]
fn a_session_without_a_plan_clears_the_plan_it_replaces() {
    let store = TaskStore::new();
    store.create("stale", CreateTaskOptions::default());
    restore_plan(&store, &[user()]);
    assert!(plan(&store).is_empty());
}

#[test]
fn an_unfinished_plan_settles_as_the_turn_did() {
    let store = TaskStore::new();
    let calls = |stop| {
        vec![
            user(),
            todo_call(
                "t1",
                json!([{"content": "Add tests", "status": "in_progress"}]),
                stop,
            ),
            result("t1", false),
        ]
    };
    restore_plan(&store, &calls(StopReason::Stop));
    assert_eq!(
        plan(&store),
        vec![("Add tests".to_string(), TaskStatus::Done)]
    );
    restore_plan(&store, &calls(StopReason::ToolUse));
    assert_eq!(
        plan(&store),
        vec![("Add tests".to_string(), TaskStatus::Cancelled)]
    );
}

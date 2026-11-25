#![allow(clippy::disallowed_methods)] // test code: #[tokio::test] expands to a runtime builder
//! A parked call of a detached run: open it, answer it, continue the run.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::common::{text_result, tool, Harness, HarnessOptions};
use hoocode_agent_types::AgentMessage;
use hoocode_ai_provider_faux::faux_assistant_message;
use hoocode_ai_types::{AssistantMessage, Content, StopReason, ToolCallContent, ToolResultMessage};
use hoocode_code_agent_session::{file_sha256, ParkedDecision, ParkedOutcome};
use hoocode_code_session::detached::PARKED_CALL_ENTRY;
use hoocode_code_session::FileEntry;
use serde_json::json;

const ECHO: &str = "Echo";

/// An assistant message with two `Echo` calls, `a` (on `x.txt`) and `b`.
fn two_calls() -> AssistantMessage {
    AssistantMessage {
        content: vec![
            Content::ToolCall(ToolCallContent {
                id: "a".into(),
                name: ECHO.into(),
                arguments: json!({"path": "x.txt"}),
                thought_signature: None,
            }),
            Content::ToolCall(ToolCallContent {
                id: "b".into(),
                name: ECHO.into(),
                arguments: json!({}),
                thought_signature: None,
            }),
        ],
        stop_reason: StopReason::ToolUse,
        ..Default::default()
    }
}

/// A harness whose `Echo` tool sets `ran` when it runs, with a branch that ends in the
/// two calls and a `parked_call` record for `a`. `x.txt` holds `contents` when parked.
fn parked_harness(contents: Option<&str>) -> (Harness, Arc<AtomicBool>, std::path::PathBuf) {
    let ran = Arc::new(AtomicBool::new(false));
    let flag = ran.clone();
    let echo = tool(ECHO, move |_| {
        flag.store(true, Ordering::SeqCst);
        text_result("ran")
    });
    let h = Harness::new(HarnessOptions {
        tools: vec![echo],
        ..Default::default()
    });
    let path = h.temp_dir.path().join("x.txt");
    let sha = match contents {
        Some(text) => {
            std::fs::write(&path, text).unwrap();
            file_sha256(&path)
        }
        None => None,
    };
    let message = two_calls();
    h.session
        .agent()
        .append_message(AgentMessage::Assistant(message.clone()));
    let mut manager = h.session.session_manager();
    manager.append_message(AgentMessage::Assistant(message));
    manager.append_custom_entry(
        PARKED_CALL_ENTRY,
        Some(json!({
            "toolCallId": "a",
            "toolName": ECHO,
            "reason": "writes a file",
            "target": {"path": path.display().to_string(), "sha256": sha},
        })),
    );
    drop(manager);
    (h, ran, path)
}

/// The persisted tool results on the session's entries, as (call id, text, is_error).
fn persisted_results(h: &Harness) -> Vec<(String, String, bool)> {
    h.session
        .session_manager()
        .entries()
        .iter()
        .filter_map(|entry| match entry {
            FileEntry::Message {
                message:
                    AgentMessage::ToolResult(ToolResultMessage {
                        tool_call_id,
                        content,
                        is_error,
                        ..
                    }),
                ..
            } => {
                let text = match content.first() {
                    Some(Content::Text(t)) => t.text.clone(),
                    other => format!("{other:?}"),
                };
                Some((tool_call_id.clone(), text, *is_error))
            }
            _ => None,
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn open_parked_call_names_the_call_its_input_and_its_target() {
    let (h, ran, path) = parked_harness(Some("one"));
    let call = h.session.open_parked_call().expect("a parked call");
    assert_eq!(call.tool_call_id, "a");
    assert_eq!(call.tool_name, ECHO);
    assert_eq!(call.input, json!({"path": "x.txt"}));
    assert_eq!(call.reason, "writes a file");
    assert_eq!(
        call.target.as_ref().map(|t| t.0.clone()),
        Some(path.clone())
    );
    assert!(!h.session.parked_call_target_changed(&call));
    assert!(!ran.load(Ordering::SeqCst), "opening runs nothing");
}

#[tokio::test(flavor = "multi_thread")]
async fn approve_runs_the_tool_once_and_answers_the_siblings_not_run() {
    let (h, ran, _) = parked_harness(Some("one"));
    let call = h.session.open_parked_call().unwrap();
    let outcome = h
        .session
        .resolve_parked_call(&call, ParkedDecision::Approve)
        .await;

    assert_eq!(outcome, ParkedOutcome::Ran);
    assert!(ran.load(Ordering::SeqCst));
    let results = persisted_results(&h);
    assert_eq!(
        results,
        vec![
            ("a".into(), "ran".into(), false),
            (
                "b".into(),
                "Not run: the session parked before this call ran.".into(),
                true
            ),
        ]
    );
    assert!(h.session.open_parked_call().is_none(), "answered now");
}

#[tokio::test(flavor = "multi_thread")]
async fn deny_appends_the_denial_and_the_tool_does_not_run() {
    let (h, ran, _) = parked_harness(Some("one"));
    let call = h.session.open_parked_call().unwrap();
    let outcome = h
        .session
        .resolve_parked_call(&call, ParkedDecision::Deny)
        .await;

    assert_eq!(outcome, ParkedOutcome::Denied);
    assert!(!ran.load(Ordering::SeqCst));
    let results = persisted_results(&h);
    assert_eq!(results[0], ("a".into(), "Denied by the user.".into(), true));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_changed_target_is_not_run_and_the_model_is_told() {
    let (h, ran, path) = parked_harness(Some("one"));
    let call = h.session.open_parked_call().unwrap();
    std::fs::write(&path, "two").unwrap();
    assert!(h.session.parked_call_target_changed(&call));

    let outcome = h
        .session
        .resolve_parked_call(&call, ParkedDecision::Approve)
        .await;

    assert_eq!(outcome, ParkedOutcome::TargetChanged);
    assert!(!ran.load(Ordering::SeqCst));
    let results = persisted_results(&h);
    assert_eq!(results[0].0, "a");
    assert!(results[0].1.starts_with("Not run: "), "{results:?}");
    assert!(results[0].1.ends_with(
        "changed after this call was parked. Read it again before deciding what to do."
    ));
    assert!(results[0].2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_that_appeared_after_parking_counts_as_changed() {
    let (h, _, path) = parked_harness(None);
    let call = h.session.open_parked_call().unwrap();
    assert!(!h.session.parked_call_target_changed(&call));
    std::fs::write(&path, "new").unwrap();
    assert!(h.session.parked_call_target_changed(&call));
}

#[tokio::test(flavor = "multi_thread")]
async fn continue_run_gives_the_model_the_results_and_takes_its_answer() {
    let (h, _, _) = parked_harness(Some("one"));
    let call = h.session.open_parked_call().unwrap();
    h.session
        .resolve_parked_call(&call, ParkedDecision::Approve)
        .await;
    h.set_responses(vec![
        faux_assistant_message("done", Default::default()).into()
    ]);

    h.session.continue_run().await.unwrap();

    assert_eq!(h.assistant_texts().last().map(String::as_str), Some("done"));
    assert_eq!(h.pending_response_count(), 0);
}

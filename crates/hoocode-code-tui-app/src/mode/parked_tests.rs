//! A parked call of a detached run, when its session opens (SA-12 row 4b): the card
//! shows, and the answer is written to the file.
//!
//! The test session has no model, so the run that continues after the answer fails at
//! preflight: the tests check the file and the card, not the continued run.

use std::time::Duration;

use hoocode_agent_types::AgentMessage;
use hoocode_ai_types::{AssistantMessage, Content, StopReason, ToolCallContent, ToolResultMessage};
use hoocode_code_agent_session::file_sha256;
use hoocode_code_session::detached::PARKED_CALL_ENTRY;
use hoocode_code_session::FileEntry;
use serde_json::{json, Value};

use super::parked::ParkedChoice;
use super::test_support::test_mode;
use super::*;

const CALL: &str = "call-1";

/// A mode whose session ends in an assistant message with one `Echo` call, parked on it.
/// `target` is the record's target (`null` for none).
fn parked_mode(target: Value) -> Mode {
    let mode = test_mode(None);
    let message = AssistantMessage {
        content: vec![Content::ToolCall(ToolCallContent {
            id: CALL.into(),
            name: "Echo".into(),
            arguments: json!({}),
            thought_signature: None,
        })],
        stop_reason: StopReason::ToolUse,
        ..Default::default()
    };
    mode.session
        .agent()
        .append_message(AgentMessage::Assistant(message.clone()));
    {
        let mut manager = mode.session.session_manager();
        manager.append_message(AgentMessage::Assistant(message));
        manager.append_custom_entry(
            PARKED_CALL_ENTRY,
            Some(json!({
                "toolCallId": CALL,
                "toolName": "Echo",
                "reason": "needs approval",
                "target": target,
            })),
        );
    }
    mode
}

/// The persisted tool results of the mode's session.
fn tool_results(mode: &Mode) -> Vec<ToolResultMessage> {
    mode.session
        .session_manager()
        .entries()
        .iter()
        .filter_map(|entry| match entry {
            FileEntry::Message {
                message: AgentMessage::ToolResult(result),
                ..
            } => Some(result.clone()),
            _ => None,
        })
        .collect()
}

fn text_of(result: &ToolResultMessage) -> String {
    match result.content.first() {
        Some(Content::Text(t)) => t.text.clone(),
        other => format!("{other:?}"),
    }
}

/// Answers the card with `choice` and handles the answer's event, as the UI loop does.
fn answer(mode: &mut Mode, choice: ParkedChoice) {
    mode.answer_parked(choice);
    let event = mode
        .rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the answer is reported");
    let AppEvent::Parked(result) = event else {
        panic!("expected Parked");
    };
    mode.finish_parked(result);
}

#[test]
fn opening_a_parked_session_shows_its_card() {
    let mut mode = parked_mode(Value::Null);
    mode.check_parked_call();
    assert!(mode.parked_card.is_some());
    assert_eq!(
        mode.parked_call.as_ref().map(|c| c.tool_call_id.as_str()),
        Some(CALL)
    );
}

#[test]
fn approve_answers_the_call_and_the_run_goes_on() {
    let mut mode = parked_mode(Value::Null);
    mode.check_parked_call();
    answer(&mut mode, ParkedChoice::Approve);

    assert!(mode.parked_card.is_none(), "the card is gone");
    assert!(mode.session.open_parked_call().is_none(), "answered");
    let results = tool_results(&mode);
    assert_eq!(results[0].tool_call_id, CALL);
    assert!(results[0].is_error, "the test session has no Echo tool");
    assert!(mode.prompt_pending, "the run continues in the background");
}

#[test]
fn deny_appends_the_denial_and_the_run_goes_on() {
    let mut mode = parked_mode(Value::Null);
    mode.check_parked_call();
    answer(&mut mode, ParkedChoice::Deny);

    let results = tool_results(&mode);
    assert_eq!(results[0].tool_call_id, CALL);
    assert_eq!(text_of(&results[0]), "Denied by the user.");
    assert!(results[0].is_error);
    assert!(mode.prompt_pending);
}

#[test]
fn a_changed_target_is_recorded_as_not_run_without_a_card() {
    let file = std::env::temp_dir().join(format!(
        "parked-target-{}-{}",
        std::process::id(),
        hoocode_code_session::create_session_id()
    ));
    std::fs::write(&file, "one").expect("write");
    let sha = file_sha256(&file);
    std::fs::write(&file, "two").expect("rewrite");
    let target = json!({"path": file.display().to_string(), "sha256": sha});

    let mut mode = parked_mode(target);
    mode.check_parked_call();
    assert!(mode.parked_card.is_none(), "no card for a changed target");
    let event = mode
        .rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the answer is reported");
    let AppEvent::Parked(result) = event else {
        panic!("expected Parked");
    };
    mode.finish_parked(result);

    let results = tool_results(&mode);
    assert!(text_of(&results[0]).starts_with("Not run: "), "{results:?}");
    assert!(results[0].is_error);
    let _ = std::fs::remove_file(&file);
}

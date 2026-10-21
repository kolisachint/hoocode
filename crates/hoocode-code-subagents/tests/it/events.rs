//! subagent-events.test.ts and subagent-line-classify.test.ts.

use hoocode_code_subagents::events::*;
use serde_json::{json, Value};

/// T3.1: the parent forwards every event the child writes to the UI, so the
/// shared set is the forwarded set (it used to be the progress set alone).
#[test]
fn the_parent_forwards_every_event_the_child_writes() {
    assert_eq!(FORWARDED_SUBAGENT_EVENTS, SUBAGENT_STDOUT_EVENT_TYPES);
    for t in SUBAGENT_PROGRESS_EVENTS {
        assert!(FORWARDED_SUBAGENT_EVENTS.contains(t));
    }
    assert!(SUBAGENT_STDOUT_EVENT_TYPES.contains(&"message_end"));
}

/// T3.1: the child keeps assistant text as coalesced `message_update` deltas.
/// Every other per-delta event is still dropped at the source.
#[test]
fn the_child_drops_the_per_delta_firehose_but_keeps_coalesced_text() {
    assert!(SUBAGENT_STDOUT_EVENT_TYPES.contains(&"message_update"));
    for t in ["message_start", "tool_execution_update", "turn_start"] {
        assert!(!SUBAGENT_STDOUT_EVENT_TYPES.contains(&t));
    }
}

fn progress(event: Value) -> SubagentStdoutLine {
    SubagentStdoutLine::Progress(event.as_object().unwrap().clone())
}

#[test]
fn treats_a_ping_line_as_a_heartbeat() {
    assert_eq!(
        classify_subagent_line(r#"{"ping":true}"#),
        SubagentStdoutLine::Heartbeat
    );
}

#[test]
fn forwards_coarse_lifecycle_events_as_progress() {
    for t in ["turn_end", "tool_execution_start", "tool_execution_end"] {
        let event = json!({"type": t, "foo": 1});
        assert_eq!(classify_subagent_line(&event.to_string()), progress(event));
    }
}

#[test]
fn forwards_message_update_and_message_end_and_drops_other_per_delta_events() {
    for t in ["message_update", "message_end"] {
        let event = json!({"type": t, "foo": 1});
        assert_eq!(classify_subagent_line(&event.to_string()), progress(event));
    }
    for t in ["message_start", "tool_execution_update", "turn_start"] {
        assert_eq!(
            classify_subagent_line(&json!({"type": t}).to_string()),
            SubagentStdoutLine::Ignore
        );
    }
}

#[test]
fn ignores_non_json_non_object_and_empty_lines() {
    for line in ["", "   ", "not json", "[1,2,3]", r#"{"type": "turn_end""#] {
        assert_eq!(classify_subagent_line(line), SubagentStdoutLine::Ignore);
    }
}

#[test]
fn ignores_an_event_whose_type_is_not_a_string() {
    assert_eq!(
        classify_subagent_line(r#"{"type":42}"#),
        SubagentStdoutLine::Ignore
    );
}

#[test]
fn tolerates_surrounding_whitespace() {
    assert_eq!(
        classify_subagent_line(r#"  {"type":"turn_end"}  "#),
        progress(json!({"type": "turn_end"}))
    );
}

#[test]
fn a_clearance_request_is_its_own_kind_of_line() {
    let line = r#"{"type":"clearance_request","id":3,"tool":"Shell","input":{"command":"ls"}}"#;
    match classify_subagent_line(line) {
        SubagentStdoutLine::Clearance(request) => {
            assert_eq!(request["id"], 3);
            assert_eq!(request["tool"], "Shell");
        }
        other => panic!("expected a clearance request, got {other:?}"),
    }
    // Not a progress event the UI would show as the run's activity.
    assert!(!FORWARDED_SUBAGENT_EVENTS.contains(&"clearance_request"));
}

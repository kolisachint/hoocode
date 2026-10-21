//! `core/subagent-events.ts` plus `classifySubagentLine` from
//! `core/subagent-pool.ts`: which child stdout events cross the pipe and what
//! the parent does with each line.
//!
//! A subagent child writes its session events (forwarded to the UI as
//! `task_progress`) and `message_end` (token usage for the parent's
//! [`TokenBudget`](crate::token_budget::TokenBudget)). Assistant text arrives
//! as `message_update` text deltas, coalesced at the source to
//! [`DELTA_THROTTLE`](crate::live::DELTA_THROTTLE); every other per-delta event
//! is dropped there. The result travels via `result.json` and liveness via
//! periodic `{"ping":true}` lines.

use serde_json::{Map, Value};

/// The coarse progress events (the task panel's activity is built from these).
pub const SUBAGENT_PROGRESS_EVENTS: &[&str] =
    &["turn_end", "tool_execution_start", "tool_execution_end"];

/// `SUBAGENT_STDOUT_EVENT_TYPES`: what the child writes. `message_update` is
/// the coalesced text delta (see the module doc), not the per-delta event.
pub const SUBAGENT_STDOUT_EVENT_TYPES: &[&str] = &[
    "turn_end",
    "tool_execution_start",
    "tool_execution_end",
    "message_update",
    "message_end",
];

/// `FORWARDED_SUBAGENT_EVENTS`: the pool forwards every event the child writes,
/// so the UI can show a run's live activity (T3.1).
pub const FORWARDED_SUBAGENT_EVENTS: &[&str] = SUBAGENT_STDOUT_EVENT_TYPES;

/// `SubagentStdoutLine`: the action for one child stdout line.
#[derive(Debug, Clone, PartialEq)]
pub enum SubagentStdoutLine {
    Heartbeat,
    Progress(Map<String, Value>),
    /// The child asks its parent to decide a gated call (see [`crate::clearance`]).
    Clearance(Map<String, Value>),
    Ignore,
}

/// Classify one complete JSONL line from a subagent's stdout.
pub fn classify_subagent_line(line: &str) -> SubagentStdoutLine {
    let trimmed = line.trim();
    if !trimmed.starts_with('{') {
        return SubagentStdoutLine::Ignore;
    }
    let Ok(Value::Object(parsed)) = serde_json::from_str::<Value>(trimmed) else {
        return SubagentStdoutLine::Ignore;
    };
    if parsed.get("ping") == Some(&Value::Bool(true)) {
        return SubagentStdoutLine::Heartbeat;
    }
    match parsed.get("type").and_then(Value::as_str) {
        Some(crate::clearance::REQUEST_TYPE) => SubagentStdoutLine::Clearance(parsed),
        Some(t) if FORWARDED_SUBAGENT_EVENTS.contains(&t) => SubagentStdoutLine::Progress(parsed),
        _ => SubagentStdoutLine::Ignore,
    }
}

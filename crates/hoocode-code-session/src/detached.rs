//! The state of a detached session (SA-12), read from its session file alone.
//!
//! A detached session is a child hoocode run that the parent hands work to
//! and does not wait for. Its file says how it ended: the `detached_exit`
//! record the run writes last, or, while a process holds the file lock, that
//! it is still running. An open `parked_call` record means it waits for an
//! answer.

use std::path::Path;

use hoocode_agent_types::AgentMessage;

use crate::entry::FileEntry;
use crate::lock::lock_holder;
use crate::manager::load_entries_from_file;

/// The `customType` of the record a detached run writes when it ends.
pub const DETACHED_EXIT_ENTRY: &str = "detached_exit";

/// The `customType` of the record a run writes when it parks on a tool call.
pub const PARKED_CALL_ENTRY: &str = "parked_call";

/// Reason shown when a file has no `detached_exit` record and no process holds it.
/// The reason of a file that has no `detached_exit` record and no process holds it.
pub const CRASHED_REASON: &str = "exited without a result (crashed or killed)";

/// Where a detached session stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetachedStatus {
    /// A process holds the session's lock. `pid` is 0 when the lock file does not say.
    Running { pid: u32 },
    /// Waiting on a tool call that no later tool result answers.
    Parked,
    /// Finished without error.
    Done,
    /// Finished with an error, or ended without writing a result.
    Failed { reason: String },
}

/// The status of the detached session in `path`.
pub fn detached_status(path: &Path) -> DetachedStatus {
    if let Some(pid) = lock_holder(path) {
        return DetachedStatus::Running { pid };
    }
    let entries = load_entries_from_file(path);
    if has_open_parked_call(&entries) {
        return DetachedStatus::Parked;
    }
    let exit = entries.iter().rev().find_map(|entry| match entry {
        FileEntry::Custom {
            custom_type, data, ..
        } if custom_type == DETACHED_EXIT_ENTRY => Some(data.clone().unwrap_or_default()),
        _ => None,
    });
    match exit {
        None => DetachedStatus::Failed {
            reason: CRASHED_REASON.to_string(),
        },
        Some(data) => match data.get("status").and_then(|s| s.as_str()) {
            Some("failed") => DetachedStatus::Failed {
                reason: data
                    .get("reason")
                    .and_then(|r| r.as_str())
                    .filter(|r| !r.is_empty())
                    .unwrap_or("failed")
                    .to_string(),
            },
            _ => DetachedStatus::Done,
        },
    }
}

/// Whether some `parked_call` record has no tool result after it.
fn has_open_parked_call(entries: &[FileEntry]) -> bool {
    entries.iter().enumerate().any(|(index, entry)| {
        let FileEntry::Custom {
            custom_type, data, ..
        } = entry
        else {
            return false;
        };
        if custom_type != PARKED_CALL_ENTRY {
            return false;
        }
        let Some(call_id) = data
            .as_ref()
            .and_then(|d| d.get("toolCallId"))
            .and_then(|id| id.as_str())
        else {
            return false;
        };
        !entries[index + 1..].iter().any(|later| {
            matches!(
                later,
                FileEntry::Message {
                    message: AgentMessage::ToolResult(result),
                    ..
                } if result.tool_call_id == call_id
            )
        })
    })
}

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

/// The reason of a file that has no `detached_exit` record after its last
/// answered park, and no process holds it.
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
    // An exit record written before the last answered park is stale: the run
    // went on after it, so only a later record says how the run ended.
    let cutoff = last_answer_index(&entries);
    let exit = entries
        .iter()
        .enumerate()
        .rev()
        .filter(|(index, _)| cutoff.is_none_or(|cutoff| *index > cutoff))
        .find_map(|(_, entry)| match entry {
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

/// The call id a `parked_call` record names.
fn parked_call_id(entry: &FileEntry) -> Option<&str> {
    let FileEntry::Custom {
        custom_type, data, ..
    } = entry
    else {
        return None;
    };
    if custom_type != PARKED_CALL_ENTRY {
        return None;
    }
    data.as_ref()?.get("toolCallId")?.as_str()
}

/// Index of the first tool result after the park at `park` that answers `call_id`.
fn answer_index(entries: &[FileEntry], park: usize, call_id: &str) -> Option<usize> {
    entries
        .iter()
        .enumerate()
        .skip(park + 1)
        .find_map(|(index, later)| {
            matches!(
                later,
                FileEntry::Message {
                    message: AgentMessage::ToolResult(result),
                    ..
                } if result.tool_call_id == call_id
            )
            .then_some(index)
        })
}

/// Whether some `parked_call` record has no tool result after it.
fn has_open_parked_call(entries: &[FileEntry]) -> bool {
    entries.iter().enumerate().any(|(index, entry)| {
        parked_call_id(entry).is_some_and(|call_id| answer_index(entries, index, call_id).is_none())
    })
}

/// Index of the latest answer to any answered `parked_call`.
fn last_answer_index(entries: &[FileEntry]) -> Option<usize> {
    entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            let call_id = parked_call_id(entry)?;
            answer_index(entries, index, call_id)
        })
        .max()
}

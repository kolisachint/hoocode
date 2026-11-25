//! Detached sessions in the task panel (SA-12): one row per session this session spawned
//! with `SessionSpawn`, its state read from its file on the one-second tick.
//!
//! The registry (`hoocode_code_subagents::detached_child`) holds the spawned sessions. It is
//! rebuilt from the open session's own `SessionSpawn` results whenever the open session
//! changes (start, resume, new, fork, switch), so the rows come back after a restart.
//! A row leaves when the session changes. A row that is parked or failed raises one notice
//! when it gets there.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hoocode_agent_types::AgentMessage;
use hoocode_ai_types::Content;
use hoocode_code_session::detached::{detached_status, DetachedStatus, CRASHED_REASON};
use hoocode_code_session::FileEntry;
use hoocode_code_subagents::detached_child::{
    parse_spawn_result, replace_spawned_sessions, spawned_sessions, SpawnedSession,
};
use hoocode_code_task_store::{task_store, CreateTaskOptions, TaskPatch, TaskSource, TaskStatus};

use super::*;
use crate::notification_panel::NotificationKind;

/// The tool that starts a detached session (its name in the transcript).
const SESSION_SPAWN_TOOL: &str = "SessionSpawn";

/// The key prefix of detached rows: `detached:<session file>`.
const KEY_PREFIX: &str = "detached:";

/// A failure reason is cut to this many characters in the row.
const REASON_MAX_CHARS: usize = 60;

/// A session started this recently may not hold its lock yet: a crash reading (no lock, no
/// exit record) is not a failure in this window. An explicit failure is shown at once.
const START_GRACE: Duration = Duration::from_secs(5);

/// The row key of a detached session.
pub(super) fn row_key(session: &Path) -> String {
    format!("{KEY_PREFIX}{}", session.display())
}

/// The sessions one open session watches, with the status each last showed.
#[derive(Debug, Default)]
pub(super) struct DetachedWatch {
    /// The open session file the watch was built for.
    session: Option<PathBuf>,
    pub(super) rows: Vec<Watched>,
}

#[derive(Debug)]
pub(super) struct Watched {
    spawn: SpawnedSession,
    status: DetachedStatus,
    /// The file length when `status` was read. A parked session's file is read again only
    /// once it grows or a process holds it, not every second.
    seen_len: u64,
}

fn file_len(path: &std::path::Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

/// Whether a poll can skip `watched`: a parked session nobody has opened since.
fn parked_and_untouched(watched: &Watched) -> bool {
    watched.status == DetachedStatus::Parked
        && file_len(&watched.spawn.session) == watched.seen_len
        && hoocode_code_session::lock_holder(&watched.spawn.session).is_none()
}

/// A status a row is in once it cannot change again (no poll needed).
fn is_settled(status: &DetachedStatus) -> bool {
    matches!(status, DetachedStatus::Done | DetachedStatus::Failed { .. })
}

/// The status word a row shows, and the task state that goes with it. A parked session is
/// `Pending`: the one state the panel draws in yellow.
fn row_state(status: &DetachedStatus) -> (TaskStatus, String) {
    match status {
        DetachedStatus::Running { .. } => (TaskStatus::InProgress, "running".into()),
        DetachedStatus::Parked => (TaskStatus::Pending, "parked".into()),
        DetachedStatus::Done => (TaskStatus::Done, "done".into()),
        DetachedStatus::Failed { reason } => (
            TaskStatus::Failed,
            format!("failed: {}", cut_chars(reason, REASON_MAX_CHARS)),
        ),
    }
}

fn cut_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let kept: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}…")
}

/// The notice a status change raises, if any: parked asks for the user, failed says why.
/// Running and done raise none.
pub(super) fn transition_notice(
    goal: &str,
    session: &Path,
    status: &DetachedStatus,
) -> Option<(NotificationKind, String)> {
    match status {
        DetachedStatus::Parked => Some((
            NotificationKind::Info,
            format!(
                "Detached session \"{goal}\" is parked: open it with hoocode --session {}",
                session.display()
            ),
        )),
        DetachedStatus::Failed { reason } => Some((
            NotificationKind::Warning,
            format!("Detached session \"{goal}\" failed: {reason}"),
        )),
        DetachedStatus::Running { .. } | DetachedStatus::Done => None,
    }
}

/// The spawned sessions named by the `SessionSpawn` results of a branch, in order. A failed
/// call names no session.
pub(super) fn spawns_in_branch(entries: &[&FileEntry]) -> Vec<SpawnedSession> {
    entries
        .iter()
        .filter_map(|entry| match entry {
            FileEntry::Message {
                message: AgentMessage::ToolResult(result),
                ..
            } if result.tool_name == SESSION_SPAWN_TOOL && !result.is_error => {
                let text = result
                    .content
                    .iter()
                    .filter_map(|content| match content {
                        Content::Text(text) => Some(text.text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                parse_spawn_result(&text).map(|(session, goal)| SpawnedSession {
                    session,
                    goal,
                    started_at: 0,
                })
            }
            _ => None,
        })
        .collect()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// Whether `spawn` was started within [`START_GRACE`].
fn is_young(spawn: &SpawnedSession, now: u64) -> bool {
    spawn.started_at > 0 && now.saturating_sub(spawn.started_at) < START_GRACE.as_millis() as u64
}

/// The status of `spawn` now. Within the start grace a crash reading keeps the earlier
/// status (`Running` for a session not yet read).
fn read_status(spawn: &SpawnedSession, prior: Option<&DetachedStatus>) -> DetachedStatus {
    let status = detached_status(&spawn.session);
    let crash_reading =
        matches!(&status, DetachedStatus::Failed { reason } if reason == CRASHED_REASON);
    match (crash_reading, is_young(spawn, now_ms())) {
        (true, true) => prior.cloned().unwrap_or(DetachedStatus::Running { pid: 0 }),
        _ => status,
    }
}

/// Makes the task-panel row of `spawn` show `status`.
fn publish_row(spawn: &SpawnedSession, status: &DetachedStatus) {
    let (state, note) = row_state(status);
    let key = row_key(&spawn.session);
    let title = if spawn.goal.is_empty() {
        "detached session".to_string()
    } else {
        spawn.goal.clone()
    };
    task_store().upsert_keyed(
        &key,
        &title,
        CreateTaskOptions {
            source: Some(TaskSource::Detached),
            key: Some(key.clone()),
            log_path: Some(spawn.session.clone()),
            ..Default::default()
        },
        TaskPatch {
            status: Some(state),
            note: Some(Some(note)),
            ..Default::default()
        },
    );
}

impl Mode {
    /// The detached rows, on the one-second tick: rebuild when the open session changed,
    /// pick up sessions spawned since, then poll the ones not yet settled.
    pub(super) fn tick_detached(&mut self) {
        let session = self
            .session
            .session_manager()
            .session_file()
            .map(Path::to_path_buf);
        if session != self.detached.session {
            self.detached_rebuild(session);
        }
        self.detached_pick_up_spawns();
        self.detached_poll();
    }

    /// Replaces the registry and the rows with the sessions the open session spawned.
    fn detached_rebuild(&mut self, session: Option<PathBuf>) {
        let found = {
            let manager = self.session.session_manager();
            spawns_in_branch(&manager.branch(None))
        };
        replace_spawned_sessions(found);
        task_store().remove_keys_where(|key| key.starts_with(KEY_PREFIX));
        self.detached = DetachedWatch {
            session,
            rows: Vec::new(),
        };
    }

    /// Adds a row for each registered session not yet watched, with its status read now.
    /// A session this process started that is already parked or failed raises its notice
    /// now. A session restored on resume (`started_at` 0) raises none: its state is old news.
    fn detached_pick_up_spawns(&mut self) {
        let mut notices = Vec::new();
        for spawn in spawned_sessions() {
            if self
                .detached
                .rows
                .iter()
                .any(|watched| watched.spawn.session == spawn.session)
            {
                continue;
            }
            let status = read_status(&spawn, None);
            publish_row(&spawn, &status);
            if spawn.started_at > 0 {
                notices.extend(transition_notice(&spawn.goal, &spawn.session, &status));
            }
            let seen_len = file_len(&spawn.session);
            self.detached.rows.push(Watched {
                spawn,
                status,
                seen_len,
            });
            self.dirty.set(true);
        }
        for (kind, text) in notices {
            self.notify(kind, &text);
        }
    }

    /// Reads the status of each session not yet settled. A change updates its row, and a
    /// change to parked or failed raises its notice.
    fn detached_poll(&mut self) {
        let changes: Vec<(usize, DetachedStatus)> = self
            .detached
            .rows
            .iter()
            .enumerate()
            .filter(|(_, watched)| !is_settled(&watched.status))
            .filter(|(_, watched)| !parked_and_untouched(watched))
            .filter_map(|(index, watched)| {
                let status = read_status(&watched.spawn, Some(&watched.status));
                (status != watched.status).then_some((index, status))
            })
            .collect();
        if changes.is_empty() {
            return;
        }
        let mut notices = Vec::new();
        for (index, status) in changes {
            let watched = &mut self.detached.rows[index];
            publish_row(&watched.spawn, &status);
            watched.seen_len = file_len(&watched.spawn.session);
            if let Some(notice) =
                transition_notice(&watched.spawn.goal, &watched.spawn.session, &status)
            {
                notices.push(notice);
            }
            watched.status = status;
        }
        for (kind, text) in notices {
            self.notify(kind, &text);
        }
        self.dirty.set(true);
    }
}

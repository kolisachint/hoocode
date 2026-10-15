//! MCP rows in the task store (`hoocode-code-task-store`), for the task panel's MCP tab.
//!
//! Two kinds of row, both `TaskSource::Mcp`:
//! - one row per server, `mcp-server:<name>`, with its state: connecting (`InProgress`), ready
//!   (`Done`), needs a login (`Pending`, action `Login`), not trusted (`Pending`) or failed
//!   (`Failed`, the reason as the note). A disabled server has no row.
//! - one row per in-flight tool call, `mcp-call:<id>`, whose tag is the server. It appears once the
//!   call has run for [`REVEAL_AFTER`] (one second), offers `Abort`, and leaves when the call ends,
//!   however it ends.
//!
//! The rows are published from memory only: no I/O, so this is cheap on the UI thread.
//! [`CallTracker`] takes the instants it is given, so the one-second rule can be tested with an
//! explicit clock; [`CallTracker::start`] adds the real timer.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use hoocode_code_task_store::{
    CreateTaskOptions, RowAction, TaskPatch, TaskSource, TaskStatus, TaskStore,
};
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;

use crate::mcp::{McpServerInfo, McpStatus};

/// The prefix of every MCP row's key (the task store routes row actions by it).
pub const ROW_PREFIX: &str = "mcp-";
/// The key prefix of server rows.
pub const SERVER_KEY_PREFIX: &str = "mcp-server:";
/// The key prefix of in-flight call rows.
pub const CALL_KEY_PREFIX: &str = "mcp-call:";
/// A call shows as a row once it has run this long.
pub const REVEAL_AFTER: Duration = Duration::from_secs(1);

/// The store's key for a server row.
pub fn server_key(name: &str) -> String {
    format!("{SERVER_KEY_PREFIX}{name}")
}

/// The store's key for an in-flight call row.
pub fn call_key(id: u64) -> String {
    format!("{CALL_KEY_PREFIX}{id}")
}

/// One in-flight call the tracker knows about.
struct Call {
    id: u64,
    server: String,
    tool: String,
    started: Instant,
    cancel: CancellationToken,
    /// Its row is in the store.
    shown: bool,
}

/// Publishes the MCP rows of one hub, and tracks its in-flight calls. Shared by the hub's clones.
#[derive(Default)]
pub struct CallTracker {
    /// Where rows go; `None` until the session attaches a store (rows are then not published).
    store: Mutex<Option<&'static TaskStore>>,
    calls: Mutex<Vec<Call>>,
    next_id: AtomicU64,
}

/// Ends its call when dropped, so a call leaves the store however it ends.
pub struct CallGuard {
    tracker: Arc<CallTracker>,
    id: u64,
}

impl Drop for CallGuard {
    fn drop(&mut self) {
        self.tracker.finish(self.id);
    }
}

impl CallTracker {
    /// Publishes into `store` from now on (`None` stops publishing).
    pub fn set_store(&self, store: Option<&'static TaskStore>) {
        *lock(&self.store) = store;
    }

    fn store(&self) -> Option<&'static TaskStore> {
        *lock(&self.store)
    }

    /// Makes the server rows match `servers`: each one's state, and no row for a server that is no
    /// longer listed or is disabled. Call it under the hub's lock: the store's listeners run in this
    /// call, and must not call the hub.
    pub fn publish_servers(&self, servers: &[McpServerInfo]) {
        let Some(store) = self.store() else {
            return;
        };
        store.batch(|s| {
            let mut live: Vec<String> = Vec::new();
            for info in servers {
                let Some((status, note, action)) = server_patch(info) else {
                    continue;
                };
                let key = server_key(&info.name);
                s.upsert_keyed(
                    &key,
                    &info.name,
                    server_options(&key, None),
                    TaskPatch {
                        status: Some(status),
                        note: Some(note),
                        action: Some(action),
                        ..Default::default()
                    },
                );
                live.push(key);
            }
            s.remove_keys_where(|key| {
                key.starts_with(SERVER_KEY_PREFIX) && !live.iter().any(|k| k == key)
            });
        });
    }

    /// Records a call that started at `started` and returns the guard that ends it. The row appears
    /// when [`reveal_due`](Self::reveal_due) finds the call has run for [`REVEAL_AFTER`].
    pub fn start_at(
        tracker: &Arc<Self>,
        server: &str,
        tool: &str,
        cancel: CancellationToken,
        started: Instant,
    ) -> CallGuard {
        let id = tracker.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        lock(&tracker.calls).push(Call {
            id,
            server: server.to_owned(),
            tool: tool.to_owned(),
            started,
            cancel,
            shown: false,
        });
        CallGuard {
            tracker: tracker.clone(),
            id,
        }
    }

    /// [`start_at`](Self::start_at) now, plus a timer on `handle` that reveals the row once the call
    /// has run for [`REVEAL_AFTER`]. The timer does nothing when the call has already ended.
    pub fn start(
        tracker: &Arc<Self>,
        server: &str,
        tool: &str,
        cancel: CancellationToken,
        handle: &Handle,
    ) -> CallGuard {
        let guard = Self::start_at(tracker, server, tool, cancel, Instant::now());
        let timer = tracker.clone();
        handle.spawn(async move {
            tokio::time::sleep(REVEAL_AFTER).await;
            timer.reveal_due(Instant::now());
        });
        guard
    }

    /// Publishes the row of every call that has run for [`REVEAL_AFTER`] at `now`, and has none yet.
    pub fn reveal_due(&self, now: Instant) {
        let Some(store) = self.store() else {
            return;
        };
        let mut calls = lock(&self.calls);
        for call in calls.iter_mut().filter(|c| !c.shown) {
            let ran = now.checked_duration_since(call.started).unwrap_or_default();
            if ran < REVEAL_AFTER {
                continue;
            }
            call.shown = true;
            let key = call_key(call.id);
            store.upsert_keyed(
                &key,
                &call.tool,
                server_options(&key, Some(call.server.clone())),
                TaskPatch {
                    status: Some(TaskStatus::InProgress),
                    note: Some(None),
                    action: Some(Some(RowAction::Abort)),
                    ..Default::default()
                },
            );
        }
    }

    /// The call ended: forget it and remove its row.
    fn finish(&self, id: u64) {
        let shown = {
            let mut calls = lock(&self.calls);
            let Some(index) = calls.iter().position(|c| c.id == id) else {
                return;
            };
            calls.remove(index).shown
        };
        if shown {
            if let Some(store) = self.store() {
                store.remove_keyed(&call_key(id));
            }
        }
    }

    /// Cancels the call `id`. Errors when it has already ended.
    pub fn abort(&self, id: u64) -> Result<(), String> {
        let cancel = {
            let calls = lock(&self.calls);
            calls
                .iter()
                .find(|c| c.id == id)
                .map(|c| c.cancel.clone())
                .ok_or_else(|| "That call already finished.".to_owned())?
        };
        cancel.cancel();
        Ok(())
    }

    /// The session ended: remove every MCP row.
    pub fn clear(&self) {
        if let Some(store) = self.store() {
            store.remove_keys_where(|key| {
                key.starts_with(SERVER_KEY_PREFIX) || key.starts_with(CALL_KEY_PREFIX)
            });
        }
    }
}

/// Creation options of an MCP row: source [`TaskSource::Mcp`], keyed, tagged with the server for a call.
fn server_options(key: &str, server: Option<String>) -> CreateTaskOptions {
    CreateTaskOptions {
        source: Some(TaskSource::Mcp),
        subagent_mode: server,
        key: Some(key.to_owned()),
        ..Default::default()
    }
}

/// A server's row state: status, note and the action it offers. `None` when it has no row.
fn server_patch(info: &McpServerInfo) -> Option<(TaskStatus, Option<String>, Option<RowAction>)> {
    Some(match &info.status {
        McpStatus::Connecting => (TaskStatus::InProgress, Some("connecting".to_owned()), None),
        McpStatus::Connected => (
            TaskStatus::Done,
            Some(format!("ready, {} tools", info.tool_count)),
            None,
        ),
        McpStatus::AuthNeeded => (
            TaskStatus::Pending,
            Some("needs a login".to_owned()),
            Some(RowAction::Login),
        ),
        McpStatus::NotTrusted => (TaskStatus::Pending, Some("not trusted".to_owned()), None),
        McpStatus::Failed(reason) => (TaskStatus::Failed, Some(reason.clone()), None),
        McpStatus::Disabled => return None,
    })
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(name: &str, status: McpStatus, tools: usize) -> McpServerInfo {
        McpServerInfo {
            name: name.to_owned(),
            source: "user".to_owned(),
            status,
            tool_count: tools,
        }
    }

    /// A tracker publishing into a store that lives for the test.
    fn tracker_with_store() -> (Arc<CallTracker>, &'static TaskStore) {
        let store: &'static TaskStore = Box::leak(Box::new(TaskStore::new()));
        let tracker = Arc::new(CallTracker::default());
        tracker.set_store(Some(store));
        (tracker, store)
    }

    #[test]
    fn server_rows_show_each_state_and_the_login_action_only_when_it_needs_one() {
        let (tracker, store) = tracker_with_store();
        tracker.publish_servers(&[
            info("a", McpStatus::Connecting, 0),
            info("b", McpStatus::Connected, 3),
            info("c", McpStatus::AuthNeeded, 0),
            info("d", McpStatus::Failed("boom".into()), 0),
            info("e", McpStatus::NotTrusted, 0),
            info("f", McpStatus::Disabled, 0),
        ]);
        let row = |name: &str| store.find_keyed(&server_key(name)).unwrap();
        assert_eq!(row("a").status, TaskStatus::InProgress);
        assert_eq!(row("b").status, TaskStatus::Done);
        assert_eq!(row("b").note.as_deref(), Some("ready, 3 tools"));
        assert_eq!(row("c").status, TaskStatus::Pending);
        assert_eq!(row("c").action, Some(RowAction::Login));
        assert_eq!(row("d").status, TaskStatus::Failed);
        assert_eq!(row("d").note.as_deref(), Some("boom"));
        assert_eq!(row("e").note.as_deref(), Some("not trusted"));
        assert_eq!(row("b").action, None);
        assert!(
            store.find_keyed(&server_key("f")).is_none(),
            "disabled has no row"
        );
        assert_eq!(row("c").kind(), hoocode_code_task_store::TaskKind::Mcp);

        // A state change updates the same row; a server that leaves loses its row.
        let id = row("c").id;
        tracker.publish_servers(&[info("c", McpStatus::Connected, 1)]);
        let now = row("c");
        assert_eq!(now.id, id);
        assert_eq!(now.status, TaskStatus::Done);
        assert_eq!(now.action, None, "the login offer goes with the state");
        assert!(store.find_keyed(&server_key("a")).is_none());
    }

    #[test]
    fn a_call_shows_only_after_one_second_and_leaves_when_it_ends() {
        let (tracker, store) = tracker_with_store();
        let t0 = Instant::now();
        let guard = CallTracker::start_at(&tracker, "docs", "search", CancellationToken::new(), t0);
        let key = call_key(1);

        tracker.reveal_due(t0 + Duration::from_millis(999));
        assert!(store.find_keyed(&key).is_none(), "not before one second");

        tracker.reveal_due(t0 + REVEAL_AFTER);
        let row = store.find_keyed(&key).unwrap();
        assert_eq!(row.title, "search");
        assert_eq!(
            row.subagent_mode.as_deref(),
            Some("docs"),
            "the tag is the server"
        );
        assert_eq!(row.status, TaskStatus::InProgress);
        assert_eq!(row.action, Some(RowAction::Abort));

        // Revealing again does not add a second row.
        tracker.reveal_due(t0 + Duration::from_secs(5));
        assert_eq!(store.list().len(), 1);

        drop(guard);
        assert!(
            store.find_keyed(&key).is_none(),
            "the row leaves when the call ends"
        );
    }

    #[test]
    fn a_call_that_ends_before_one_second_never_shows() {
        let (tracker, store) = tracker_with_store();
        let t0 = Instant::now();
        let guard = CallTracker::start_at(&tracker, "docs", "quick", CancellationToken::new(), t0);
        tracker.reveal_due(t0 + Duration::from_millis(400));
        drop(guard);
        tracker.reveal_due(t0 + Duration::from_secs(3));
        assert!(store.list().is_empty());
    }

    #[test]
    fn abort_cancels_the_call_and_errors_once_it_has_ended() {
        let (tracker, store) = tracker_with_store();
        let t0 = Instant::now();
        let token = CancellationToken::new();
        let guard = CallTracker::start_at(&tracker, "docs", "slow", token.clone(), t0);
        tracker.reveal_due(t0 + REVEAL_AFTER);
        assert!(!token.is_cancelled());
        assert!(store.find_keyed(&call_key(1)).is_some());

        tracker.abort(1).unwrap();
        assert!(token.is_cancelled());

        drop(guard);
        let gone = tracker.abort(1).unwrap_err();
        assert!(gone.contains("already finished"), "{gone}");
        assert!(store.find_keyed(&call_key(1)).is_none());
    }

    #[test]
    fn clear_removes_every_mcp_row_and_nothing_else() {
        let (tracker, store) = tracker_with_store();
        tracker.publish_servers(&[info("a", McpStatus::Connected, 1)]);
        store.create("a plan item", CreateTaskOptions::default());
        let t0 = Instant::now();
        let _guard = CallTracker::start_at(&tracker, "a", "x", CancellationToken::new(), t0);
        tracker.reveal_due(t0 + REVEAL_AFTER);
        assert_eq!(store.list().len(), 3);
        tracker.clear();
        let left: Vec<String> = store.list().into_iter().map(|t| t.title).collect();
        assert_eq!(left, vec!["a plan item".to_owned()]);
    }

    #[test]
    fn nothing_is_published_without_a_store() {
        let tracker = Arc::new(CallTracker::default());
        tracker.publish_servers(&[info("a", McpStatus::Connected, 1)]);
        let t0 = Instant::now();
        let _guard = CallTracker::start_at(&tracker, "a", "x", CancellationToken::new(), t0);
        tracker.reveal_due(t0 + REVEAL_AFTER);
        tracker.clear();
    }

    #[test]
    fn a_row_action_on_a_row_that_does_not_offer_it_is_refused() {
        let (tracker, store) = tracker_with_store();
        tracker.publish_servers(&[info("a", McpStatus::Connected, 1)]);
        let id = store.find_keyed(&server_key("a")).unwrap().id;
        let refused = store
            .perform(id, RowAction::Abort, Arc::new(|_| {}))
            .unwrap_err();
        assert!(refused.contains("no Abort action"), "{refused}");
    }
}

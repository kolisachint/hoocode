//! `core/subagent-inbox.ts`: the notify-and-pull bookkeeping behind background
//! `Task` dispatch and the `AgentOutput` tool.
//!
//! A background dispatch gives the parent a compact notification
//! ("explore#1 finished"); the body waits here, keyed by task id, until the
//! model pulls it with AgentOutput. Lifecycle per task:
//!
//! ```text
//! (starting) ──▶ running ──▶ done (body kept) ──collect──▶ collected (body dropped)
//!                        ├─▶ failed / stalled / timeout / cancelled
//! ```
//!
//! A background call's record is made at its placeholder (`starting`, keyed by
//! its label until the execute binds a task id), so the run is listed, can be
//! asked about and can be cancelled from the moment the model is told its
//! handle. Records, labels and label bindings are per session (the
//! dispatching session's file): `/new` and app-server sessions do not see each
//! other's runs, and a resumed session continues its label numbers from the
//! handles already in its file, so a handle never names two runs.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::Duration;

use regex::bytes::Regex;
use serde_json::Value;
use tokio::sync::Notify;

use crate::pool::{ResultStatus, SubagentPool, TaskResult};

/// `TaskLifecycle`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskLifecycle {
    Running,
    Done,
    Failed,
    Stalled,
    Timeout,
    Cancelled,
    Collected,
}

impl TaskLifecycle {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Stalled => "stalled",
            Self::Timeout => "timeout",
            Self::Cancelled => "cancelled",
            Self::Collected => "collected",
        }
    }

    /// Still doing work.
    fn is_outstanding(self) -> bool {
        self == Self::Running
    }
}

/// `InboxRecord`.
#[derive(Debug, Clone, PartialEq)]
pub struct InboxRecord {
    pub task_id: String,
    /// Friendly handle shown to the model, e.g. `explore#1`.
    pub label: String,
    pub agent_type: String,
    /// The call's `description` (what this run is for), so two runs of one
    /// agent type can be told apart wherever the model sees the label.
    pub description: Option<String>,
    /// The session the run belongs to (the dispatching session's file), `None`
    /// outside a session. Listings and labels are per session.
    pub session: Option<String>,
    /// Reserved at the call's placeholder, before its execute bound a task id:
    /// `task_id` is a placeholder key until [`SubagentInbox::start_in`].
    pub starting: bool,
    /// A cancel was asked for. The execute stops the run as soon as the pool
    /// holds it (see [`SubagentInbox::request_cancel`]).
    pub cancel_requested: bool,
    pub lifecycle: TaskLifecycle,
    /// Epoch ms.
    pub started_at: u64,
    pub ended_at: Option<u64>,
    /// The tool the subagent is running, from the pool's progress stream.
    pub last_activity: Option<String>,
    /// First line of the result, kept after the body is collected.
    pub summary_line: Option<String>,
    /// The full summary, only while `done` and uncollected.
    pub body: Option<String>,
    pub error: Option<String>,
}

impl InboxRecord {
    /// The label with what the run is for: `explore#2 ("audit the db layer")`,
    /// or the bare label when the call had no description.
    pub fn named(&self) -> String {
        named(&self.label, self.description.as_deref())
    }
}

/// `explore#2 ("audit the db layer")`: how a run is named to the model.
pub fn named(label: &str, description: Option<&str>) -> String {
    match description.map(str::trim).filter(|d| !d.is_empty()) {
        Some(description) => format!("{label} (\"{description}\")"),
        None => label.to_string(),
    }
}

/// Settled records kept for late pulls (running records are never pruned).
const MAX_SETTLED: usize = 50;

fn now_ms() -> u64 {
    hoocode_code_task_store::now_ms()
}

/// First non-empty line, capped at `max` characters (UTF-16 units in JS).
fn first_line(text: &str, max: usize) -> String {
    let line = text
        .trim()
        .split('\n')
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    let units: Vec<u16> = line.encode_utf16().collect();
    if units.len() > max {
        format!("{}\u{2026}", String::from_utf16_lossy(&units[..max - 1]))
    } else {
        line.to_string()
    }
}

fn fail_lifecycle(status: Option<ResultStatus>) -> TaskLifecycle {
    match status {
        Some(ResultStatus::Stalled) => TaskLifecycle::Stalled,
        Some(ResultStatus::Timeout) => TaskLifecycle::Timeout,
        Some(ResultStatus::Cancelled) => TaskLifecycle::Cancelled,
        _ => TaskLifecycle::Failed,
    }
}

/// "Currently running X" from a forwarded progress event.
fn activity_from_event(event: &Value) -> Option<String> {
    match event.get("type").and_then(Value::as_str) {
        Some("tool_execution_start") => event
            .get("toolName")
            .and_then(Value::as_str)
            .map(String::from),
        Some("turn_end") => Some("thinking".into()),
        _ => None,
    }
}

/// A handle in a session file: `explore#3`, `general-purpose#12`.
static HANDLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([A-Za-z0-9_.:-]+)#([0-9]+)").expect("handle regex"));

/// The highest `type#N` per agent type in a session file, read line by line
/// as bytes (a session file can be large, and is not guaranteed UTF-8 at
/// every byte). Only lines with a `#` are searched. A file that does not
/// exist yet is a new session: no handles. `None` when the file exists but
/// cannot be read (permissions, an I/O error): nothing is learned, and the
/// caller tries again later.
fn handles_in(path: &str) -> Option<HashMap<String, u64>> {
    use std::io::BufRead;
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Some(HashMap::new()),
        Err(_) => return None,
    };
    let mut reader = std::io::BufReader::new(file);
    let mut seen: HashMap<String, u64> = HashMap::new();
    let mut line = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => return None,
        }
        if !line.contains(&b'#') {
            continue;
        }
        for caps in HANDLE.captures_iter(&line) {
            let (Ok(name), Ok(n)) = (
                std::str::from_utf8(&caps[1]),
                std::str::from_utf8(&caps[2]).map(str::parse::<u64>),
            ) else {
                continue;
            };
            let Ok(n) = n else { continue };
            let max = seen.entry(name.to_string()).or_insert(0);
            *max = (*max).max(n);
        }
    }
    Some(seen)
}

type Scope = Option<String>;

/// The record key of a run reserved at its placeholder, before its task id.
fn reserved_key(session: Option<&str>, label: &str) -> String {
    format!("starting:{}:{label}", session.unwrap_or(""))
}

/// `explore#3` as (`explore`, 3).
fn parse_handle(handle: &str) -> Option<(&str, u64)> {
    let (agent_type, n) = handle.rsplit_once('#')?;
    Some((agent_type, n.parse().ok()?))
}

/// What [`SubagentInbox::request_cancel`] found.
#[derive(Debug, Clone, PartialEq)]
pub enum CancelAsk {
    /// Reserved, not started: settled `cancelled` here; it will not start.
    BeforeStart(InboxRecord),
    /// Running: flagged; the caller kills it through the pool.
    Running(InboxRecord),
    /// Already settled: nothing to cancel.
    Settled(InboxRecord),
}

/// A label given out at a placeholder, until the call's execute takes it.
struct Reservation {
    description: String,
    session: Scope,
    label: String,
}

#[derive(Default)]
struct InboxState {
    records: HashMap<String, InboxRecord>,
    /// Insertion order, for listing and pruning.
    order: Vec<String>,
    /// (session, agent type) -> the last label number given out.
    label_counters: HashMap<(Scope, String), u64>,
    /// Sessions whose file was read for handles, and the highest number seen
    /// there per agent type: those handles belong to an earlier process.
    seeded: HashMap<String, HashMap<String, u64>>,
    /// Labels given out at a background call's placeholder, by (session, tool
    /// call id): two sessions whose providers both number calls `call_0` must
    /// not take each other's (see [`SubagentInbox::reserve_label`]).
    reserved_labels: HashMap<(Scope, String), Reservation>,
    observed_pools: HashSet<u64>,
    /// (session, model-facing label `explore#4`) -> the task id whose dispatch
    /// dir holds its `session.jsonl`. A resumed run is bound to the original session.
    session_ids: HashMap<(Scope, String), String>,
    /// Bindings in order, so `session_ids` is capped like `records`: the
    /// oldest binding goes once more than `MAX_SETTLED` are held.
    session_order: VecDeque<(Scope, String)>,
}

impl InboxState {
    /// Any session: a task id, or the first record with that label.
    fn get(&self, handle: &str) -> Option<&InboxRecord> {
        self.records
            .get(handle)
            .or_else(|| self.records.values().find(|r| r.label == handle))
    }

    /// A task id (unique everywhere), or a label within `session`.
    fn key_in(&self, session: Option<&str>, handle: &str) -> Option<String> {
        if self.records.contains_key(handle) {
            return Some(handle.to_string());
        }
        self.order
            .iter()
            .find(|id| {
                self.records
                    .get(*id)
                    .is_some_and(|r| r.label == handle && r.session.as_deref() == session)
            })
            .cloned()
    }

    fn key_any(&self, handle: &str) -> Option<String> {
        if self.records.contains_key(handle) {
            return Some(handle.to_string());
        }
        self.order
            .iter()
            .find(|id| self.records.get(*id).is_some_and(|r| r.label == handle))
            .cloned()
    }

    fn list(&self, session: Option<Option<&str>>) -> Vec<InboxRecord> {
        self.order
            .iter()
            .filter_map(|id| self.records.get(id))
            .filter(|r| session.is_none_or(|s| r.session.as_deref() == s))
            .cloned()
            .collect()
    }

    fn outstanding_count(&self, session: Option<Option<&str>>) -> usize {
        self.records
            .values()
            .filter(|r| r.lifecycle.is_outstanding())
            .filter(|r| session.is_none_or(|s| r.session.as_deref() == s))
            .count()
    }

    fn next_label(&mut self, session: Option<&str>, agent_type: &str) -> String {
        let n = self
            .label_counters
            .entry((session.map(String::from), agent_type.to_string()))
            .or_insert(0);
        *n += 1;
        format!("{agent_type}#{n}")
    }

    /// Drop the oldest settled records past the cap.
    fn prune(&mut self) {
        let mut settled = self
            .order
            .iter()
            .filter(|id| {
                self.records
                    .get(*id)
                    .is_some_and(|r| !r.lifecycle.is_outstanding())
            })
            .count();
        if settled <= MAX_SETTLED {
            return;
        }
        let mut kept = Vec::new();
        for id in std::mem::take(&mut self.order) {
            let is_settled = self
                .records
                .get(&id)
                .is_some_and(|r| !r.lifecycle.is_outstanding());
            if is_settled && settled > MAX_SETTLED {
                self.records.remove(&id);
                settled -= 1;
                continue;
            }
            kept.push(id);
        }
        self.order = kept;
    }

    fn settle(rec: &mut InboxRecord, lifecycle: TaskLifecycle, reason: &str) {
        rec.ended_at = Some(now_ms());
        rec.last_activity = None;
        rec.lifecycle = lifecycle;
        rec.error = Some(reason.to_string());
        rec.summary_line = Some(reason.to_string());
    }
}

/// `SubagentInbox`.
#[derive(Default)]
pub struct SubagentInbox {
    state: Mutex<InboxState>,
    /// Woken whenever a record settles (the wait helpers).
    settled: Notify,
    /// Woken whenever a cancel is asked for (the executes watching their run).
    cancel_asked: Notify,
}

impl SubagentInbox {
    fn state(&self) -> MutexGuard<'_, InboxState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn notify_settle(&self) {
        self.settled.notify_waiters();
    }

    /// Resolve once the task settles (at once if settled or unknown), bounded
    /// by `timeout`. Any session.
    pub async fn wait_for(&self, handle: &str, timeout: Duration) -> Option<InboxRecord> {
        self.wait_for_in(None, handle, timeout).await
    }

    /// [`Self::wait_for`] for a handle of `session` (`None`: any session).
    pub async fn wait_for_in(
        &self,
        session: Option<Option<&str>>,
        handle: &str,
        timeout: Duration,
    ) -> Option<InboxRecord> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let notified = self.settled.notified();
            let current = match session {
                Some(session) => self.get_in(session, handle),
                None => self.get(handle),
            };
            match &current {
                Some(rec) if rec.lifecycle.is_outstanding() => {}
                _ => return current,
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                return match session {
                    Some(session) => self.get_in(session, handle),
                    None => self.get(handle),
                };
            }
        }
    }

    /// Resolve once nothing is outstanding (the swarm barrier), bounded by
    /// `timeout`. Every session.
    pub async fn wait_for_all(&self, timeout: Duration) {
        self.wait_for_all_in(None, timeout).await
    }

    /// The swarm barrier over one session's runs (`None`: every session).
    pub async fn wait_for_all_in(&self, session: Option<Option<&str>>, timeout: Duration) {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let notified = self.settled.notified();
            if self.state().outstanding_count(session) == 0 {
                return;
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                return;
            }
        }
    }

    /// Read `session`'s file once for the handles it already holds, so a
    /// resumed session's labels continue after them instead of starting over
    /// at `#1` (a new `explore#1` would then answer for the old one).
    fn seed(&self, session: Option<&str>) {
        let Some(session) = session else { return };
        if self.state().seeded.contains_key(session) {
            return;
        }
        // Unreadable: learn nothing and do not remember the session as read,
        // so the next call tries again.
        let Some(seen) = handles_in(session) else {
            return;
        };
        let mut state = self.state();
        if state.seeded.contains_key(session) {
            return;
        }
        for (agent_type, n) in &seen {
            let counter = state
                .label_counters
                .entry((Some(session.to_string()), agent_type.clone()))
                .or_insert(0);
            *counter = (*counter).max(*n);
        }
        state.seeded.insert(session.to_string(), seen);
    }

    /// The next label for an agent type (`explore#1`, `explore#2`, ...),
    /// outside any session.
    pub fn next_label(&self, agent_type: &str) -> String {
        self.next_label_in(None, agent_type)
    }

    /// The next label for an agent type in `session`.
    pub fn next_label_in(&self, session: Option<&str>, agent_type: &str) -> String {
        self.seed(session);
        self.state().next_label(session, agent_type)
    }

    /// Give a background Agent call its label when the call is detached, and
    /// list its run at once as `starting`, so its placeholder can name the run
    /// and the run can be asked about or cancelled before its execute starts.
    /// The loop makes placeholders one at a time in tool-call order, so labels
    /// follow the order the model wrote the calls; the executes then start
    /// concurrently, in any order, and each takes its own label back with
    /// [`Self::take_label`].
    ///
    /// `None` when `call_id` is already reserved (an id repeated within one
    /// message): that call gets no handle of its own in its placeholder.
    pub fn reserve_label(
        &self,
        call_id: &str,
        agent_type: &str,
        description: &str,
        session: Option<&str>,
    ) -> Option<String> {
        self.seed(session);
        let mut state = self.state();
        let call_key = (session.map(String::from), call_id.to_string());
        if state.reserved_labels.contains_key(&call_key) {
            return None;
        }
        let label = state.next_label(session, agent_type);
        let key = reserved_key(session, &label);
        state.records.insert(
            key.clone(),
            InboxRecord {
                task_id: key.clone(),
                label: label.clone(),
                agent_type: agent_type.to_string(),
                description: Some(description.trim().to_string()).filter(|d| !d.is_empty()),
                session: session.map(String::from),
                starting: true,
                cancel_requested: false,
                lifecycle: TaskLifecycle::Running,
                started_at: now_ms(),
                ended_at: None,
                last_activity: None,
                summary_line: None,
                body: None,
                error: None,
            },
        );
        state.order.push(key);
        state.reserved_labels.insert(
            call_key,
            Reservation {
                description: description.to_string(),
                session: session.map(String::from),
                label: label.clone(),
            },
        );
        Some(label)
    }

    /// The label reserved for `call_id` (and removed), or the next one when
    /// the call had none (a foreground call). A reservation is always taken by
    /// its call, even when the execute sees other arguments than the
    /// placeholder did (validated or rewritten ones): the model was told that
    /// handle, and a second label would leave the reserved run listed as
    /// running forever. A changed description replaces the reserved one.
    pub fn take_label(
        &self,
        call_id: &str,
        agent_type: &str,
        description: &str,
        session: Option<&str>,
    ) -> String {
        let mut state = self.state();
        let call_key = (session.map(String::from), call_id.to_string());
        if let Some(reservation) = state.reserved_labels.remove(&call_key) {
            let description = description.trim();
            if reservation.description.trim() != description && !description.is_empty() {
                let key = reserved_key(reservation.session.as_deref(), &reservation.label);
                if let Some(rec) = state.records.get_mut(&key) {
                    rec.description = Some(description.to_string());
                }
            }
            return reservation.label;
        }
        drop(state);
        self.next_label_in(session, agent_type)
    }

    /// Settle `label`'s run in `session` as failed if it is still running
    /// (reserved or bound): its Agent call ended, or panicked, without
    /// starting or settling it. `None` when there is nothing to settle.
    ///
    /// The reason says which: "not started" for a run still reserved,
    /// "stopped unexpectedly" for one its execute had started.
    pub fn settle_abandoned(&self, session: Option<&str>, label: &str) -> Option<InboxRecord> {
        let rec = {
            let mut state = self.state();
            let key = state.key_in(session, label)?;
            let rec = state.records.get_mut(&key)?;
            if rec.lifecycle != TaskLifecycle::Running {
                return None;
            }
            let reason = if rec.starting {
                "not started: the Agent call ended before starting this run"
            } else {
                "stopped unexpectedly: the Agent call ended without a result for this run"
            };
            InboxState::settle(rec, TaskLifecycle::Failed, reason);
            rec.clone()
        };
        self.notify_settle();
        Some(rec)
    }

    /// Whether `handle` names a run held for another session of this process
    /// (e.g. one `/fork`ed or switched away from) and not for `session`.
    pub fn is_other_sessions_run(&self, session: Option<&str>, handle: &str) -> bool {
        let state = self.state();
        state.key_in(session, handle).is_none()
            && state
                .records
                .values()
                .any(|r| r.label == handle && r.session.as_deref() != session)
    }

    /// Whether `handle` (`explore#3`) names a run of an earlier process: the
    /// session's file held it when this process first read it, and no record
    /// of it is held now.
    pub fn is_earlier_run(&self, session: Option<&str>, handle: &str) -> bool {
        let Some(session) = session else { return false };
        let Some((agent_type, n)) = parse_handle(handle) else {
            return false;
        };
        self.seed(Some(session));
        let state = state_seeded_max(&self.state(), session, agent_type);
        n <= state && self.get_in(Some(session), handle).is_none()
    }

    /// Record that `label` (the handle the model sees) resumes the session
    /// stored under `session_id`. Called at every dispatch, foreground and
    /// background. Outside any session.
    pub fn bind_label(&self, label: &str, session_id: &str) {
        self.bind_label_in(None, label, session_id)
    }

    /// [`Self::bind_label`] within `session`.
    pub fn bind_label_in(&self, session: Option<&str>, label: &str, session_id: &str) {
        let mut state = self.state();
        let key = (session.map(String::from), label.to_string());
        if state
            .session_ids
            .insert(key.clone(), session_id.to_string())
            .is_none()
        {
            state.session_order.push_back(key);
        }
        while state.session_order.len() > MAX_SETTLED {
            if let Some(oldest) = state.session_order.pop_front() {
                state.session_ids.remove(&oldest);
            }
        }
    }

    /// The task id whose session a `resume_task_id` handle names. Accepts a
    /// label (`explore#4`); `None` for anything else, which the caller takes
    /// as a raw task id. Outside any session.
    pub fn session_id_for(&self, handle: &str) -> Option<String> {
        self.session_id_for_in(None, handle)
    }

    /// [`Self::session_id_for`] within `session`.
    pub fn session_id_for_in(&self, session: Option<&str>, handle: &str) -> Option<String> {
        self.state()
            .session_ids
            .get(&(session.map(String::from), handle.to_string()))
            .cloned()
    }

    /// Track a pool's `task_progress` so running records carry a live
    /// activity. Idempotent per pool.
    pub fn observe(self: &Arc<Self>, pool: &SubagentPool) {
        if !self.state().observed_pools.insert(pool.id()) {
            return;
        }
        let inbox = Arc::downgrade(self);
        pool.on(move |event| {
            if event.name != "task_progress" {
                return;
            }
            let Some(inbox) = inbox.upgrade() else { return };
            let Some(task_id) = event.data.get("task_id").and_then(Value::as_str) else {
                return;
            };
            let mut state = inbox.state();
            let Some(rec) = state.records.get_mut(task_id) else {
                return;
            };
            if rec.lifecycle != TaskLifecycle::Running {
                return;
            }
            if let Some(activity) = activity_from_event(&event.data["event"]) {
                rec.last_activity = Some(activity);
            }
        });
    }

    /// Register a freshly dispatched background task as running, outside any
    /// session.
    pub fn start(
        &self,
        task_id: &str,
        label: &str,
        agent_type: &str,
        description: &str,
    ) -> InboxRecord {
        self.start_in(None, task_id, label, agent_type, description)
    }

    /// Register a freshly dispatched background task as running. A run
    /// reserved at its placeholder is bound to `task_id` here, in its place in
    /// the listing; if it was cancelled while starting, the record comes back
    /// `cancelled` and the caller must not dispatch it.
    pub fn start_in(
        &self,
        session: Option<&str>,
        task_id: &str,
        label: &str,
        agent_type: &str,
        description: &str,
    ) -> InboxRecord {
        let mut state = self.state();
        let key = reserved_key(session, label);
        let description = Some(description.trim().to_string()).filter(|d| !d.is_empty());
        if let Some(mut rec) = state.records.remove(&key) {
            rec.task_id = task_id.to_string();
            rec.starting = false;
            if rec.description.is_none() {
                rec.description = description;
            }
            if let Some(slot) = state.order.iter_mut().find(|id| **id == key) {
                *slot = task_id.to_string();
            }
            state.records.insert(task_id.to_string(), rec.clone());
            return rec;
        }
        let rec = InboxRecord {
            task_id: task_id.to_string(),
            label: label.to_string(),
            agent_type: agent_type.to_string(),
            description,
            session: session.map(String::from),
            starting: false,
            cancel_requested: false,
            lifecycle: TaskLifecycle::Running,
            started_at: now_ms(),
            ended_at: None,
            last_activity: None,
            summary_line: None,
            body: None,
            error: None,
        };
        state.records.insert(task_id.to_string(), rec.clone());
        state.order.push(task_id.to_string());
        state.prune();
        rec
    }

    /// Settle a run reserved at its placeholder that never started (its
    /// execute refused the call). `None` when there is no such run.
    pub fn fail_reserved(
        &self,
        session: Option<&str>,
        label: &str,
        reason: &str,
    ) -> Option<InboxRecord> {
        let rec = {
            let mut state = self.state();
            let rec = state.records.get_mut(&reserved_key(session, label))?;
            if rec.lifecycle != TaskLifecycle::Running {
                return None;
            }
            InboxState::settle(rec, TaskLifecycle::Failed, reason);
            let rec = rec.clone();
            state.prune();
            rec
        };
        self.notify_settle();
        Some(rec)
    }

    /// A reserved run of `session` that its execute has not started yet.
    pub fn reserved(&self, session: Option<&str>, label: &str) -> Option<InboxRecord> {
        self.state()
            .records
            .get(&reserved_key(session, label))
            .cloned()
    }

    /// Ask for a run to stop. A run still starting is settled `cancelled`
    /// here and never starts; a running one is flagged, and the execute that
    /// awaits it kills it through the pool as soon as the pool holds it (see
    /// [`Self::cancel_requested`]). `None` for an unknown handle.
    pub fn request_cancel(&self, session: Option<&str>, handle: &str) -> Option<CancelAsk> {
        let ask = {
            let mut state = self.state();
            let key = state.key_in(session, handle)?;
            let rec = state.records.get_mut(&key)?;
            if rec.lifecycle != TaskLifecycle::Running {
                return Some(CancelAsk::Settled(rec.clone()));
            }
            rec.cancel_requested = true;
            if rec.starting {
                InboxState::settle(rec, TaskLifecycle::Cancelled, "cancelled before it started");
                CancelAsk::BeforeStart(rec.clone())
            } else {
                CancelAsk::Running(rec.clone())
            }
        };
        self.cancel_asked.notify_waiters();
        if matches!(ask, CancelAsk::BeforeStart(_)) {
            self.notify_settle();
        }
        Some(ask)
    }

    /// Whether a cancel was asked for this task id.
    pub fn cancel_requested(&self, task_id: &str) -> bool {
        self.state()
            .records
            .get(task_id)
            .is_some_and(|r| r.cancel_requested)
    }

    /// Resolves at the next [`Self::request_cancel`]. Create it (and enable
    /// it) before checking [`Self::cancel_requested`], so no ask is missed.
    pub fn cancel_notified(&self) -> tokio::sync::futures::Notified<'_> {
        self.cancel_asked.notified()
    }

    /// Settle a task from its dispatch result: the body on success, the
    /// reason on failure.
    pub fn finish(&self, task_id: &str, result: &TaskResult) -> Option<InboxRecord> {
        let rec = {
            let mut state = self.state();
            let rec = state.records.get_mut(task_id)?;
            rec.ended_at = Some(now_ms());
            rec.last_activity = None;
            let r = result.result.as_ref();
            if r.is_some_and(|r| r.ok) {
                let summary = r
                    .and_then(|r| r.result_data.as_ref())
                    .and_then(|d| d.get("summary"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("(subagent returned no output)")
                    .to_string();
                rec.lifecycle = TaskLifecycle::Done;
                rec.summary_line = Some(first_line(&summary, 120));
                rec.body = Some(summary);
            } else {
                let status = r.and_then(|r| r.status);
                rec.lifecycle = fail_lifecycle(status);
                let error = r
                    .and_then(|r| r.error.clone())
                    .unwrap_or_else(|| match status {
                        Some(status) => format!("subagent {}", status.as_str()),
                        None => "unknown error".into(),
                    });
                rec.summary_line = Some(error.clone());
                rec.error = Some(error);
            }
            rec.clone()
        };
        self.notify_settle();
        Some(rec)
    }

    /// Settle a task that never produced a result (a failed dispatch).
    pub fn fail(
        &self,
        task_id: &str,
        reason: &str,
        lifecycle: TaskLifecycle,
    ) -> Option<InboxRecord> {
        let rec = {
            let mut state = self.state();
            let rec = state.records.get_mut(task_id)?;
            InboxState::settle(rec, lifecycle, reason);
            rec.clone()
        };
        self.notify_settle();
        Some(rec)
    }

    /// A record by task id or label, in any session.
    pub fn get(&self, handle: &str) -> Option<InboxRecord> {
        self.state().get(handle).cloned()
    }

    /// A record by task id, or by label within `session`.
    pub fn get_in(&self, session: Option<&str>, handle: &str) -> Option<InboxRecord> {
        let state = self.state();
        let key = state.key_in(session, handle)?;
        state.records.get(&key).cloned()
    }

    /// Read a done task's body once and mark it collected (the body is
    /// dropped so it is not re-fed to the model). Any session.
    pub fn collect(&self, handle: &str) -> Option<(InboxRecord, String)> {
        let key = self.state().key_any(handle)?;
        self.collect_key(&key)
    }

    /// [`Self::collect`] for a handle of `session`.
    pub fn collect_in(&self, session: Option<&str>, handle: &str) -> Option<(InboxRecord, String)> {
        let key = self.state().key_in(session, handle)?;
        self.collect_key(&key)
    }

    fn collect_key(&self, key: &str) -> Option<(InboxRecord, String)> {
        let mut state = self.state();
        let rec = state.records.get_mut(key)?;
        if rec.lifecycle != TaskLifecycle::Done {
            return None;
        }
        let body = rec.body.take()?;
        rec.lifecycle = TaskLifecycle::Collected;
        let mut record = rec.clone();
        record.body = Some(body.clone());
        Some((record, body))
    }

    /// All records, oldest first, in every session.
    pub fn list(&self) -> Vec<InboxRecord> {
        self.state().list(None)
    }

    /// One session's records, oldest first.
    pub fn list_in(&self, session: Option<&str>) -> Vec<InboxRecord> {
        self.state().list(Some(session))
    }

    /// Records still doing work, in every session.
    pub fn outstanding(&self) -> Vec<InboxRecord> {
        self.list()
            .into_iter()
            .filter(|r| r.lifecycle.is_outstanding())
            .collect()
    }

    /// One session's records still doing work.
    pub fn outstanding_in(&self, session: Option<&str>) -> Vec<InboxRecord> {
        self.list_in(session)
            .into_iter()
            .filter(|r| r.lifecycle.is_outstanding())
            .collect()
    }

    /// Settle records the pool has forgotten about.
    ///
    /// `prune` deliberately never touches a running record, which is right
    /// while the run really is running — and wrong when the settle event was
    /// lost (a dropped notification, a crash between the two). Such a handle
    /// then reports `running` forever, in the roster the model reads and in the
    /// task panel a human watches. A record is reconciled when the pool says it
    /// is neither running nor queued *and* it is older than `max_age_ms`, so a
    /// dispatch that has not reached `pull` yet is never mistaken for a lost
    /// one.
    ///
    /// `is_live` is the pool's answer for that task id.
    pub fn reconcile(&self, is_live: impl Fn(&str) -> bool, max_age_ms: u64) -> Vec<InboxRecord> {
        let mut state = self.state();
        let now = now_ms();
        let mut reconciled = Vec::new();
        for id in state.order.clone() {
            let Some(record) = state.records.get_mut(&id) else {
                continue;
            };
            if record.lifecycle != TaskLifecycle::Running || is_live(&id) {
                continue;
            }
            if now.saturating_sub(record.started_at) < max_age_ms {
                continue;
            }
            record.lifecycle = TaskLifecycle::Failed;
            record.ended_at = Some(now);
            record.error = Some("the dispatch is no longer running and never reported back".into());
            record.summary_line = Some("reconciled: no result".into());
            reconciled.push(record.clone());
        }
        reconciled
    }

    /// Test/teardown helper.
    pub fn clear(&self) {
        let mut state = self.state();
        state.records.clear();
        state.order.clear();
        state.label_counters.clear();
        state.seeded.clear();
        state.reserved_labels.clear();
        state.session_ids.clear();
        state.session_order.clear();
    }
}

fn state_seeded_max(state: &InboxState, session: &str, agent_type: &str) -> u64 {
    state
        .seeded
        .get(session)
        .and_then(|seen| seen.get(agent_type))
        .copied()
        .unwrap_or(0)
}

static INBOX: LazyLock<Arc<SubagentInbox>> = LazyLock::new(Arc::default);

/// The process-wide inbox shared by the Task and AgentOutput tools.
pub fn subagent_inbox() -> &'static Arc<SubagentInbox> {
    &INBOX
}

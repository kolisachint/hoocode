//! `core/task-store.ts`: a minimal in-process task store.
//!
//! Tasks (TodoWrite plan items, subagent delegations, MCP calls) and the
//! agents that own them, with change listeners for the task panel. Mutations
//! inside [`TaskStore::batch`] notify once, when the outermost batch ends.
//! Listeners run outside the store's lock, so they may read the store.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

/// `TaskStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Pending,
    InProgress,
    Done,
    Failed,
    /// A user-initiated stop, distinct from `Failed`.
    Cancelled,
}

impl TaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn is_active(self) -> bool {
        matches!(self, Self::Pending | Self::InProgress)
    }
}

/// `TaskSource`: what kind of background work owns a task (unset: the main
/// agent).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskSource {
    Subagent,
    Mcp,
    /// A scheduled job (`/loop`, the Cron tools).
    Schedule,
}

/// An action a row offers the panel (Enter or `x` on the row). The producer
/// performs it through [`RowActions`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowAction {
    /// Log in to an MCP server that needs a login (`/mcp login <server>`).
    Login,
    /// Cancel an in-flight MCP call.
    Abort,
}

/// Receives a line of text for the user from an action (a login link, an outcome).
pub type ActionNotifier = Arc<dyn Fn(String) + Send + Sync>;

/// Performs the [`RowAction`]s of the rows a producer owns. `key` is the row's
/// stable key ([`Task::key`]). Called off the store's lock.
pub trait RowActions: Send + Sync {
    fn perform(&self, key: &str, action: RowAction, notify: ActionNotifier) -> Result<(), String>;
}

/// `TaskAgentKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskAgentKind {
    Main,
    Subagent,
}

/// `TaskAgentState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskAgentState {
    Active,
    Running,
    Done,
    Queued,
    Idle,
    Waiting,
    Failed,
    Cancelled,
}

/// Token and cost totals for an agent.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AgentStats {
    pub input: f64,
    pub output: f64,
    pub cost: f64,
}

/// `TaskAgent`: a group header in the panel's grouped views.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskAgent {
    pub id: String,
    pub name: String,
    pub role: Option<String>,
    pub kind: TaskAgentKind,
    pub state: Option<TaskAgentState>,
    /// Live activity of a running subagent; `""` clears it.
    pub activity: Option<String>,
    pub stats: Option<AgentStats>,
    /// Which attempt this is (1 for the first try, 2 for a fallback retry).
    pub attempt: Option<u32>,
    /// The model the child actually resolved to, not the one that was asked for.
    pub model: Option<String>,
    /// When the run's own deadline is (epoch ms). The panel renders the
    /// countdown from this rather than from a string that would go stale.
    pub deadline_at: Option<u64>,
    /// How the run ended, when it did: `complete`, `partial`, `timeout`, ...
    pub outcome: Option<String>,
    /// The child's self-reported confidence, when it wrote one.
    pub confidence: Option<f64>,
    /// Why it ended like that, in one line.
    pub cause: Option<String>,
}

/// Token and cost usage attributed to a task.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TaskUsage {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
    pub cost: f64,
}

/// `Task`.
#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    pub id: u64,
    pub title: String,
    pub status: TaskStatus,
    pub source: Option<TaskSource>,
    /// The row's `[tag]`: subagent type or MCP server name.
    pub subagent_mode: Option<String>,
    /// Id of the owning [`TaskAgent`].
    pub agent: Option<String>,
    /// The task that spawned this one (subagent trees).
    pub parent_task_id: Option<u64>,
    /// The TodoWrite plan item this run works on (display link only).
    pub linked_task_id: Option<u64>,
    /// A short warning cue.
    pub note: Option<String>,
    /// Canonical TodoWrite item content (the title flips with status).
    pub todo_content: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    pub usage: Option<TaskUsage>,
    /// Stable identity of a row its producer keeps up to date (a scheduled job,
    /// an MCP server, an MCP call). A keyed row keeps its `id` until it is
    /// removed, and survives [`TaskStore::reset`].
    pub key: Option<String>,
    /// Epoch milliseconds of the next run, for a schedule row.
    pub next_at: Option<u64>,
    /// The action the panel can offer on this row, if any.
    pub action: Option<RowAction>,
}

/// `CreateTaskOptions`.
#[derive(Debug, Clone, Default)]
pub struct CreateTaskOptions {
    pub source: Option<TaskSource>,
    pub subagent_mode: Option<String>,
    pub agent: Option<String>,
    pub parent_task_id: Option<u64>,
    pub linked_task_id: Option<u64>,
    /// The row's stable key (see [`Task::key`]).
    pub key: Option<String>,
}

/// `TaskPatch`: `None` leaves a field alone, except `note`, where
/// `Some(None)` clears it.
#[derive(Debug, Clone, Default)]
pub struct TaskPatch {
    pub title: Option<String>,
    pub status: Option<TaskStatus>,
    pub source: Option<TaskSource>,
    pub subagent_mode: Option<String>,
    pub agent: Option<String>,
    pub usage: Option<TaskUsage>,
    pub note: Option<Option<String>>,
    pub parent_task_id: Option<u64>,
    pub linked_task_id: Option<u64>,
    pub todo_content: Option<String>,
    /// `Some(None)` clears it.
    pub next_at: Option<Option<u64>>,
    /// `Some(None)` clears it.
    pub action: Option<Option<RowAction>>,
}

/// `TaskAgentPatch`.
#[derive(Debug, Clone, Default)]
pub struct TaskAgentPatch {
    pub name: Option<String>,
    pub role: Option<String>,
    pub kind: Option<TaskAgentKind>,
    pub state: Option<TaskAgentState>,
    pub activity: Option<String>,
    pub stats: Option<AgentStats>,
    pub attempt: Option<u32>,
    pub model: Option<String>,
    pub deadline_at: Option<u64>,
    pub outcome: Option<String>,
    pub confidence: Option<f64>,
    pub cause: Option<String>,
}

/// `taskOwnerId`: the explicit agent, else `subagent` for subagent-sourced
/// work, else `main`.
pub fn task_owner_id(agent: Option<&str>, source: Option<TaskSource>) -> String {
    match (agent, source) {
        (Some(agent), _) => agent.to_owned(),
        (None, Some(TaskSource::Subagent)) => "subagent".into(),
        _ => "main".into(),
    }
}

/// What a task row stands for in the task panel. Shell rows come with T5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskKind {
    /// The main agent's own TodoWrite plan item.
    Plan,
    /// A dispatched subagent run.
    Subagent,
    /// An MCP server's state, or an MCP call.
    Mcp,
    /// A scheduled job: its fire, run and finish, and its next run.
    Schedule,
}

impl Task {
    pub fn owner_id(&self) -> String {
        task_owner_id(self.agent.as_deref(), self.source)
    }

    /// The main agent's own TodoWrite plan item: no source, no owning agent,
    /// no parent. `todo.rs` settles these at the end of a request.
    pub fn is_plan_item(&self) -> bool {
        self.source.is_none() && self.agent.is_none() && self.parent_task_id.is_none()
    }

    /// The row kind, from the source. Every task without a source is a plan
    /// item for this purpose.
    pub fn kind(&self) -> TaskKind {
        match self.source {
            Some(TaskSource::Subagent) => TaskKind::Subagent,
            Some(TaskSource::Mcp) => TaskKind::Mcp,
            Some(TaskSource::Schedule) => TaskKind::Schedule,
            None => TaskKind::Plan,
        }
    }
}

type Listener = Arc<dyn Fn() + Send + Sync>;

struct State {
    tasks: Vec<Task>,
    agents: Vec<TaskAgent>,
    next_id: u64,
    batch_depth: usize,
    mutation_count: u64,
    listeners: Vec<(u64, Listener)>,
    next_listener: u64,
    row_actions: Option<Arc<dyn RowActions>>,
}

/// `TaskStore`.
pub struct TaskStore {
    state: Mutex<State>,
}

impl Default for TaskStore {
    fn default() -> Self {
        Self::new()
    }
}

static CLOCK_OFFSET_MS: AtomicU64 = AtomicU64::new(0);

/// Wall-clock milliseconds for task timing: the store's timestamps, the task
/// panel's run clocks and the subagent inbox's elapsed times all read this,
/// so tests can move them together with [`advance_clock_for_tests`].
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
        + CLOCK_OFFSET_MS.load(Ordering::SeqCst)
}

/// Move [`now_ms`] forward (`vi.advanceTimersByTime` in the TS tests).
pub fn advance_clock_for_tests(ms: u64) {
    CLOCK_OFFSET_MS.fetch_add(ms, Ordering::SeqCst);
}

fn apply_agent_patch(agent: &mut TaskAgent, patch: TaskAgentPatch) {
    if let Some(name) = patch.name {
        agent.name = name;
    }
    if patch.role.is_some() {
        agent.role = patch.role;
    }
    if let Some(kind) = patch.kind {
        agent.kind = kind;
    }
    if patch.attempt.is_some() {
        agent.attempt = patch.attempt;
    }
    if patch.model.is_some() {
        agent.model = patch.model;
    }
    if patch.deadline_at.is_some() {
        agent.deadline_at = patch.deadline_at;
    }
    if patch.outcome.is_some() {
        agent.outcome = patch.outcome;
    }
    if patch.confidence.is_some() {
        agent.confidence = patch.confidence;
    }
    if patch.cause.is_some() {
        agent.cause = patch.cause;
    }
    if patch.state.is_some() {
        agent.state = patch.state;
    }
    if patch.activity.is_some() {
        agent.activity = patch.activity;
    }
    if patch.stats.is_some() {
        agent.stats = patch.stats;
    }
}

/// Unsubscribes its listener when called (`subscribe`'s return value).
pub struct Subscription<'a> {
    store: &'a TaskStore,
    id: u64,
}

impl Subscription<'_> {
    pub fn unsubscribe(self) {
        self.store.lock().listeners.retain(|(id, _)| *id != self.id);
    }
}

impl TaskStore {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State {
                tasks: Vec::new(),
                agents: Vec::new(),
                next_id: 1,
                batch_depth: 0,
                mutation_count: 0,
                listeners: Vec::new(),
                next_listener: 0,
                row_actions: None,
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Bump the version and, outside a batch, notify (with the lock released).
    fn emit(&self, mut state: MutexGuard<'_, State>) {
        state.mutation_count += 1;
        if state.batch_depth > 0 {
            return;
        }
        let listeners: Vec<Listener> = state.listeners.iter().map(|(_, l)| l.clone()).collect();
        drop(state);
        for listener in listeners {
            listener();
        }
    }

    /// `version`: bumped on every mutation, batched or not.
    pub fn version(&self) -> u64 {
        self.lock().mutation_count
    }

    /// `batch`: listeners are notified once, when the outermost batch ends.
    pub fn batch<T>(&self, f: impl FnOnce(&Self) -> T) -> T {
        self.lock().batch_depth += 1;
        struct Guard<'a>(&'a TaskStore);
        impl Drop for Guard<'_> {
            fn drop(&mut self) {
                let mut state = self.0.lock();
                state.batch_depth -= 1;
                if state.batch_depth == 0 {
                    // As in hoocode, the flush itself counts as a mutation.
                    self.0.emit(state);
                }
            }
        }
        let _guard = Guard(self);
        f(self)
    }

    /// `create`: a pending task (a blank title becomes `(untitled task)`).
    pub fn create(&self, title: &str, options: CreateTaskOptions) -> Task {
        let mut state = self.lock();
        let now = now_ms();
        let title = title.trim();
        let task = Task {
            id: state.next_id,
            title: if title.is_empty() {
                "(untitled task)".into()
            } else {
                title.to_owned()
            },
            status: TaskStatus::Pending,
            source: options.source,
            subagent_mode: options.subagent_mode,
            agent: options.agent,
            parent_task_id: options.parent_task_id,
            linked_task_id: options.linked_task_id,
            note: None,
            todo_content: None,
            created_at: now,
            updated_at: now,
            usage: None,
            key: options.key,
            next_at: None,
            action: None,
        };
        state.next_id += 1;
        state.tasks.push(task.clone());
        self.emit(state);
        task
    }

    /// `update`; unknown ids are ignored.
    pub fn update(&self, id: u64, patch: TaskPatch) {
        let mut state = self.lock();
        let Some(task) = state.tasks.iter_mut().find(|t| t.id == id) else {
            return;
        };
        if let Some(title) = patch.title {
            task.title = title;
        }
        if let Some(status) = patch.status {
            task.status = status;
        }
        if patch.source.is_some() {
            task.source = patch.source;
        }
        if patch.subagent_mode.is_some() {
            task.subagent_mode = patch.subagent_mode;
        }
        if patch.agent.is_some() {
            task.agent = patch.agent;
        }
        if patch.parent_task_id.is_some() {
            task.parent_task_id = patch.parent_task_id;
        }
        if patch.linked_task_id.is_some() {
            task.linked_task_id = patch.linked_task_id;
        }
        if patch.todo_content.is_some() {
            task.todo_content = patch.todo_content;
        }
        if patch.usage.is_some() {
            task.usage = patch.usage;
        }
        if let Some(note) = patch.note {
            task.note = note;
        }
        if let Some(next_at) = patch.next_at {
            task.next_at = next_at;
        }
        if let Some(action) = patch.action {
            task.action = action;
        }
        task.updated_at = now_ms();
        self.emit(state);
    }

    /// Creates the row with `key` when there is none, then applies `title` (when
    /// not empty) and `patch` to it, in one step: a keyed row is never seen
    /// twice. Returns the row.
    pub fn upsert_keyed(
        &self,
        key: &str,
        title: &str,
        options: CreateTaskOptions,
        patch: TaskPatch,
    ) -> Task {
        let mut state = self.lock();
        let now = now_ms();
        let index = match state
            .tasks
            .iter()
            .position(|t| t.key.as_deref() == Some(key))
        {
            Some(index) => index,
            None => {
                let id = state.next_id;
                state.next_id += 1;
                state.tasks.push(Task {
                    id,
                    title: String::new(),
                    status: TaskStatus::Pending,
                    source: options.source,
                    subagent_mode: options.subagent_mode,
                    agent: options.agent,
                    parent_task_id: options.parent_task_id,
                    linked_task_id: options.linked_task_id,
                    note: None,
                    todo_content: None,
                    created_at: now,
                    updated_at: now,
                    usage: None,
                    key: Some(key.to_owned()),
                    next_at: None,
                    action: None,
                });
                state.tasks.len() - 1
            }
        };
        let task = &mut state.tasks[index];
        if !title.is_empty() {
            task.title = title.to_owned();
        }
        if let Some(status) = patch.status {
            task.status = status;
        }
        if let Some(note) = patch.note {
            task.note = note;
        }
        if let Some(next_at) = patch.next_at {
            task.next_at = next_at;
        }
        if let Some(action) = patch.action {
            task.action = action;
        }
        if patch.subagent_mode.is_some() {
            task.subagent_mode = patch.subagent_mode;
        }
        task.updated_at = now;
        let row = task.clone();
        self.emit(state);
        row
    }

    /// Removes the row with `key`. True when there was one.
    pub fn remove_keyed(&self, key: &str) -> bool {
        self.remove_keys_where(|k| k == key) > 0
    }

    /// Removes every keyed row whose key `pred` accepts; returns how many went.
    pub fn remove_keys_where(&self, pred: impl Fn(&str) -> bool) -> usize {
        let mut state = self.lock();
        let before = state.tasks.len();
        state.tasks.retain(|t| match t.key.as_deref() {
            Some(key) => !pred(key),
            None => true,
        });
        let removed = before - state.tasks.len();
        if removed > 0 {
            self.emit(state);
        }
        removed
    }

    /// The row with `key`, if there is one.
    pub fn find_keyed(&self, key: &str) -> Option<Task> {
        self.lock()
            .tasks
            .iter()
            .find(|t| t.key.as_deref() == Some(key))
            .cloned()
    }

    /// Registers who performs the rows' [`RowAction`]s, replacing any earlier
    /// one; `None` removes it.
    pub fn set_row_actions(&self, handler: Option<Arc<dyn RowActions>>) {
        self.lock().row_actions = handler;
    }

    /// Performs `action` on row `id`. Errors when the row is gone, does not offer
    /// `action`, or no handler is registered. The handler runs off the store's lock.
    pub fn perform(
        &self,
        id: u64,
        action: RowAction,
        notify: ActionNotifier,
    ) -> Result<(), String> {
        let (key, handler) = {
            let state = self.lock();
            let task = state
                .tasks
                .iter()
                .find(|t| t.id == id)
                .ok_or_else(|| "That row is gone.".to_owned())?;
            if task.action != Some(action) {
                return Err(format!("That row has no {action:?} action."));
            }
            let key = task
                .key
                .clone()
                .ok_or_else(|| "That row has no key.".to_owned())?;
            let handler = state
                .row_actions
                .clone()
                .ok_or_else(|| "Nothing performs that action.".to_owned())?;
            (key, handler)
        };
        handler.perform(&key, action, notify)
    }

    /// `upsertAgent`: merge into an agent with the same id, or create it.
    pub fn upsert_agent(
        &self,
        id: &str,
        name: &str,
        kind: TaskAgentKind,
        patch: TaskAgentPatch,
    ) -> TaskAgent {
        let mut state = self.lock();
        let agent = match state.agents.iter_mut().find(|a| a.id == id) {
            Some(existing) => {
                apply_agent_patch(
                    existing,
                    TaskAgentPatch {
                        name: Some(name.to_owned()),
                        kind: Some(kind),
                        ..patch
                    },
                );
                existing.clone()
            }
            None => {
                let created = TaskAgent {
                    id: id.to_owned(),
                    name: name.to_owned(),
                    role: patch.role,
                    kind,
                    state: patch.state,
                    activity: patch.activity,
                    stats: patch.stats,
                    attempt: patch.attempt,
                    model: patch.model,
                    deadline_at: patch.deadline_at,
                    outcome: patch.outcome,
                    confidence: patch.confidence,
                    cause: patch.cause,
                };
                state.agents.push(created.clone());
                created
            }
        };
        self.emit(state);
        agent
    }

    /// `patchAgent`; unknown ids are ignored.
    pub fn patch_agent(&self, id: &str, patch: TaskAgentPatch) {
        let mut state = self.lock();
        let Some(agent) = state.agents.iter_mut().find(|a| a.id == id) else {
            return;
        };
        apply_agent_patch(agent, patch);
        self.emit(state);
    }

    /// `addAgentStats`: add a usage delta (totals start at zero).
    pub fn add_agent_stats(&self, id: &str, delta: AgentStats) {
        let mut state = self.lock();
        let Some(agent) = state.agents.iter_mut().find(|a| a.id == id) else {
            return;
        };
        let stats = agent.stats.get_or_insert_with(AgentStats::default);
        stats.input += delta.input;
        stats.output += delta.output;
        stats.cost += delta.cost;
        self.emit(state);
    }

    pub fn agents(&self) -> Vec<TaskAgent> {
        self.lock().agents.clone()
    }

    /// `remove`; unknown ids are ignored.
    pub fn remove(&self, id: u64) {
        let mut state = self.lock();
        let Some(index) = state.tasks.iter().position(|t| t.id == id) else {
            return;
        };
        state.tasks.remove(index);
        self.emit(state);
    }

    /// `arrange`: permute the given tasks into this relative order within
    /// their existing slots; ignored unless every id resolves.
    pub fn arrange(&self, ids: &[u64]) {
        let mut state = self.lock();
        let ordered: Vec<Task> = ids
            .iter()
            .filter_map(|id| state.tasks.iter().find(|t| t.id == *id).cloned())
            .collect();
        if ordered.len() != ids.len() {
            return;
        }
        let slots: Vec<usize> = state
            .tasks
            .iter()
            .enumerate()
            .filter(|(_, t)| ids.contains(&t.id))
            .map(|(i, _)| i)
            .collect();
        if slots.len() != ordered.len() {
            return;
        }
        for (slot, task) in slots.into_iter().zip(ordered) {
            state.tasks[slot] = task;
        }
        self.emit(state);
    }

    /// `reset`: drop finished tasks (a new user message arrived). Keyed rows are
    /// kept whatever their status, since their producer owns them. Numbering
    /// restarts once nothing is left. Agents with stats survive.
    pub fn reset(&self) {
        let mut state = self.lock();
        let active: Vec<Task> = state
            .tasks
            .iter()
            .filter(|t| t.status.is_active() || t.key.is_some())
            .cloned()
            .collect();
        if active.len() == state.tasks.len() && state.next_id == 1 {
            return;
        }
        let has_stats = |a: &TaskAgent| {
            a.stats
                .is_some_and(|s| s.input > 0.0 || s.output > 0.0 || s.cost > 0.0)
        };
        if active.is_empty() {
            state.next_id = 1;
            state
                .agents
                .retain(|a| a.kind != TaskAgentKind::Subagent && has_stats(a));
        } else {
            let live: Vec<String> = active.iter().map(Task::owner_id).collect();
            state.agents.retain(|a| {
                live.contains(&a.id) || (a.kind != TaskAgentKind::Subagent && has_stats(a))
            });
        }
        state.tasks = active;
        self.emit(state);
    }

    /// `list`: every task, in panel order.
    pub fn list(&self) -> Vec<Task> {
        self.lock().tasks.clone()
    }

    /// `clear`: wipe everything and restart numbering (tests).
    pub fn clear(&self) {
        let mut state = self.lock();
        state.tasks.clear();
        state.agents.clear();
        state.next_id = 1;
        self.emit(state);
    }

    /// `subscribe`.
    pub fn subscribe(&self, listener: impl Fn() + Send + Sync + 'static) -> Subscription<'_> {
        let mut state = self.lock();
        let id = state.next_listener;
        state.next_listener += 1;
        state.listeners.push((id, Arc::new(listener)));
        Subscription { store: self, id }
    }
}

static GLOBAL: LazyLock<TaskStore> = LazyLock::new(TaskStore::new);

/// The shared, process-wide store (`taskStore`).
pub fn task_store() -> &'static TaskStore {
    &GLOBAL
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(options: CreateTaskOptions) -> Task {
        TaskStore::new().create("t", options)
    }

    #[test]
    fn only_a_sourceless_ownerless_parentless_task_is_a_plan_item() {
        assert!(task(CreateTaskOptions::default()).is_plan_item());
        assert!(!task(CreateTaskOptions {
            source: Some(TaskSource::Subagent),
            ..Default::default()
        })
        .is_plan_item());
        assert!(!task(CreateTaskOptions {
            source: Some(TaskSource::Mcp),
            ..Default::default()
        })
        .is_plan_item());
        assert!(!task(CreateTaskOptions {
            agent: Some("worker".into()),
            ..Default::default()
        })
        .is_plan_item());
        assert!(!task(CreateTaskOptions {
            parent_task_id: Some(1),
            ..Default::default()
        })
        .is_plan_item());
    }

    #[test]
    fn kind_follows_the_source_and_defaults_to_plan() {
        assert_eq!(task(CreateTaskOptions::default()).kind(), TaskKind::Plan);
        let run = task(CreateTaskOptions {
            source: Some(TaskSource::Subagent),
            ..Default::default()
        });
        assert_eq!(run.kind(), TaskKind::Subagent);
        let call = task(CreateTaskOptions {
            source: Some(TaskSource::Mcp),
            ..Default::default()
        });
        assert_eq!(call.kind(), TaskKind::Mcp);
        let job = task(CreateTaskOptions {
            source: Some(TaskSource::Schedule),
            ..Default::default()
        });
        assert_eq!(job.kind(), TaskKind::Schedule);
        assert!(!job.is_plan_item());
    }

    fn schedule_options(key: &str) -> CreateTaskOptions {
        CreateTaskOptions {
            source: Some(TaskSource::Schedule),
            key: Some(key.to_owned()),
            ..Default::default()
        }
    }

    fn status_patch(status: TaskStatus) -> TaskPatch {
        TaskPatch {
            status: Some(status),
            ..Default::default()
        }
    }

    #[test]
    fn a_keyed_row_keeps_its_id_across_updates_and_survives_reset() {
        let store = TaskStore::new();
        let first = store.upsert_keyed(
            "schedule:a",
            "check the build",
            schedule_options("schedule:a"),
            status_patch(TaskStatus::Pending),
        );
        let again = store.upsert_keyed(
            "schedule:a",
            "check the build",
            schedule_options("schedule:a"),
            TaskPatch {
                next_at: Some(Some(42)),
                ..status_patch(TaskStatus::InProgress)
            },
        );
        assert_eq!(first.id, again.id, "one row per key");
        assert_eq!(store.list().len(), 1);
        let row = store.find_keyed("schedule:a").unwrap();
        assert_eq!(row.status, TaskStatus::InProgress);
        assert_eq!(row.next_at, Some(42));
        assert_eq!(row.kind(), TaskKind::Schedule);

        // A finished plan item goes; the keyed row stays, with its id.
        let plan = store.create("plan", CreateTaskOptions::default());
        store.update(plan.id, status_patch(TaskStatus::Done));
        store.upsert_keyed(
            "schedule:a",
            "",
            CreateTaskOptions::default(),
            status_patch(TaskStatus::Done),
        );
        store.reset();
        let kept = store.list();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].key.as_deref(), Some("schedule:a"));
        assert_eq!(kept[0].id, first.id);
        assert_eq!(
            kept[0].title, "check the build",
            "an empty title keeps the old one"
        );
    }

    #[test]
    fn a_removed_keyed_row_leaves_the_store_and_others_stay() {
        let store = TaskStore::new();
        for key in ["schedule:a", "schedule:b", "mcp-server:docs"] {
            store.upsert_keyed(key, key, CreateTaskOptions::default(), TaskPatch::default());
        }
        assert!(store.remove_keyed("schedule:a"));
        assert!(!store.remove_keyed("schedule:a"), "already gone");
        let removed = store.remove_keys_where(|k| k.starts_with("schedule:"));
        assert_eq!(removed, 1);
        let keys: Vec<Option<String>> = store.list().into_iter().map(|t| t.key).collect();
        assert_eq!(keys, vec![Some("mcp-server:docs".to_owned())]);
    }

    #[test]
    fn a_keyed_row_is_notified_once_per_mutation_and_batches_collapse() {
        let store = TaskStore::new();
        let hits = std::sync::Arc::new(AtomicU64::new(0));
        let counter = hits.clone();
        let _sub = store.subscribe(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        store.batch(|s| {
            s.upsert_keyed(
                "schedule:a",
                "a",
                CreateTaskOptions::default(),
                TaskPatch::default(),
            );
            s.upsert_keyed(
                "schedule:b",
                "b",
                CreateTaskOptions::default(),
                TaskPatch::default(),
            );
        });
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        store.remove_keys_where(|_| true);
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        // Nothing removed: no notification.
        store.remove_keys_where(|_| true);
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    /// Records what it was asked to do, and answers with `result`.
    struct Recorder {
        calls: Mutex<Vec<(String, RowAction)>>,
        result: Result<(), String>,
    }

    impl RowActions for Recorder {
        fn perform(
            &self,
            key: &str,
            action: RowAction,
            notify: ActionNotifier,
        ) -> Result<(), String> {
            self.calls.lock().unwrap().push((key.to_owned(), action));
            notify("from the handler".to_owned());
            self.result.clone()
        }
    }

    #[test]
    fn a_row_action_goes_to_the_handler_with_the_row_key_only_when_offered() {
        let store = TaskStore::new();
        let recorder = Arc::new(Recorder {
            calls: Mutex::new(Vec::new()),
            result: Ok(()),
        });
        store.set_row_actions(Some(recorder.clone()));
        store.upsert_keyed(
            "mcp-call:7",
            "echo",
            CreateTaskOptions::default(),
            TaskPatch {
                action: Some(Some(RowAction::Abort)),
                ..Default::default()
            },
        );
        let id = store.find_keyed("mcp-call:7").unwrap().id;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let notify: ActionNotifier = Arc::new(move |m| sink.lock().unwrap().push(m));

        // A row does not offer Login, so the handler is not asked.
        let refused = store.perform(id, RowAction::Login, notify.clone());
        assert!(refused.unwrap_err().contains("no Login action"));
        assert!(recorder.calls.lock().unwrap().is_empty());

        store.perform(id, RowAction::Abort, notify.clone()).unwrap();
        assert_eq!(
            *recorder.calls.lock().unwrap(),
            vec![("mcp-call:7".to_owned(), RowAction::Abort)]
        );
        assert_eq!(*seen.lock().unwrap(), vec!["from the handler".to_owned()]);

        // A removed row is gone; with no handler, nothing performs the action.
        store.remove_keyed("mcp-call:7");
        assert!(store.perform(id, RowAction::Abort, notify.clone()).is_err());
        store.set_row_actions(None);
        store.upsert_keyed(
            "mcp-call:8",
            "echo",
            CreateTaskOptions::default(),
            TaskPatch {
                action: Some(Some(RowAction::Abort)),
                ..Default::default()
            },
        );
        let id = store.find_keyed("mcp-call:8").unwrap().id;
        let none = store.perform(id, RowAction::Abort, notify).unwrap_err();
        assert!(none.contains("Nothing performs"), "{none}");
    }
}

//! `core/subagent-pool.ts`: runs subagents as child processes with bounded
//! concurrency, a priority FIFO queue and automatic slot refill.
//!
//! Each child is `<executable> [prefix args] --mode json --session <file>
//! --task-id <id> ... <prompt>`. Its stdout carries progress events,
//! `message_end` (token usage) and `{"ping":true}` heartbeats; its success is a
//! verified `result.json` in `.hoocode/dispatch/<task_id>/`, not its exit code.
//!
//! Events (`PoolEvent::name`): `task_done`, `task_failed`, `task_stalled`,
//! `task_timeout`, `task_cancelled`, `budget_warning`, `budget_exceeded`
//! (advisory, never kills) and `task_progress` (coarse lifecycle events for
//! the UI).

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hoocode_ai_types::Model;
use hoocode_code_models::ResolvedScoped;
use hoocode_code_resources::{
    load_agent_registry, AgentDefinition, AgentRegistry, AgentSource, LoadAgentRegistryOptions,
    MODEL_INHERIT,
};
use hoocode_code_rpc::JsonlLineReader;
use hoocode_code_session::{append_custom_entry_to_file, subagents_dir, SUBAGENT_RUN_ENTRY};
use regex::Regex;
use serde_json::{json, Map, Value};
use tokio::sync::oneshot;

use crate::agent_log::agent_log;
use crate::depth::{
    current_subagent_depth, resolve_max_subagent_depth, tool_allowlist_needs_mcp,
    DEFER_MCP_SCHEMAS_ENV, SUBAGENT_DEPTH_ENV, SUBAGENT_SKIP_MCP_ENV,
};
use crate::dispatch::DispatchEvaluator;
use crate::events::{classify_subagent_line, SubagentStdoutLine};
use crate::ledger;
use crate::lifeguard::{
    kill_process_tree, progress_stall_ms_from_env, tool_limit_from_start, LifeguardEvent,
    SubagentLifeguard,
};
use crate::model_categories::{select_model, CategorySettings, ModelRequest, ModelSelection};
use crate::output_verifier::OutputVerifier;
use crate::result::write_file_atomic;
use crate::runner::{ProcessRunner, RunSpec, Runner};
use crate::token_budget::{TokenBudget, TokenBudgetOptions};

/// Provider/model failures where retrying with the parent's model can recover.
///
/// Deviation: hoocode's pattern plus two groups it is missing, both of which
/// were observed killing real subagents in `hoobot/.hoocode/dispatch`:
///
/// * **region** — `400 Upstream request failed: This Go model requires Global
///   regions.` A gateway account in the wrong region rejects a model the
///   catalog lists and the parent model does not use.
/// * **provider failure** — `Provider finish_reason: error`, which killed the
///   one recorded run that reached completion.
///
/// Kept as one pattern (hoocode's shape) rather than a classifier enum; the
/// alternatives are grouped so they can be lifted out later.
static INHERITED_MODEL_FALLBACK_ERROR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r#"(?i)"#,
        // hoocode's groups, verbatim.
        r#"usage[_\s-]?limit|subscription|quota|rate.?limit|too many requests|429|insufficient|"#,
        r#"out of credit|credit balance|billing|payment required|402|"#,
        r#"model[^\n]*(not found|unavailable|not available|not supported|does not exist|invalid|unsupported)|"#,
        r#"no api key|no auth configured|authentication|unauthorized|forbidden|permission"#,
        // region / geo-fencing: the catalog lists models the account cannot call.
        r#"|requires?[^\n]{0,24}regions?|region[^\n]{0,16}(not available|unsupported|restricted)|"#,
        r#"not available in (your |this )?region|unsupported_regions?|geo[_\s-]?restricted"#,
        // a provider that failed the turn outright, without saying why
        r#"|finish[_\s-]?reason["\s:=]*error|upstream request failed"#,
    ))
    .expect("valid regex")
});

/// Hard cap on assistant turns when an agent does not set `maxTurns` (the
/// token budget only warns, so this is the guaranteed stop).
pub const DEFAULT_SUBAGENT_MAX_TURNS: u64 = 50;

/// The plugin tools (`hoocode-code-tools-optin`). A subagent is never given them: every child is
/// started with them in `--disallowed-tools`, and a child does not register them at all.
pub const PLUGIN_TOOL_NAMES: [&str; 5] = [
    "SearchPlugins",
    "InstallPlugin",
    "UninstallPlugin",
    "ListPlugins",
    "ProposePlugin",
];

/// The `--disallowed-tools` list of a child: the agent definition's own list, then the plugin
/// tools (every child, whatever the definition says).
pub fn subagent_disallowed_tools(own: Option<&[String]>) -> Vec<String> {
    let mut out: Vec<String> = own.unwrap_or_default().to_vec();
    for name in PLUGIN_TOOL_NAMES {
        if !out.iter().any(|t| t == name) {
            out.push(name.to_owned());
        }
    }
    out
}

/// Tail cap on the captured stdout/stderr per task.
const MAX_CAPTURED_STREAM_CHARS: usize = 256 * 1024;
/// How much of a failed run's stderr goes into `output.json`. A provider error
/// is a line or two at the end; the rest is noise.
const OUTPUT_JSON_STDERR_TAIL_CHARS: usize = 8 * 1024;
/// Cap on one un-terminated stdout line (a runaway writer is dropped).
const MAX_SUBAGENT_EVENT_LINE_CHARS: usize = 8 * 1024 * 1024;
/// How long to wait for stdio to close after the child exits (grandchildren
/// may hold the pipes).
const EXIT_STDIO_GRACE: Duration = Duration::from_millis(100);

/// The env prefix hoocode stamps on children (read back via any prefix).
const CHILD_ENV_PREFIX: &str = "HOOCODE_";

/// What [`SubagentPool::statuses`] reports per task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunningSince {
    Running(u64),
    Queued,
}

impl RunningSince {
    /// Epoch ms, or 0 for a queued task (it has no start yet).
    pub fn since(self) -> u64 {
        match self {
            Self::Running(at) => at,
            Self::Queued => 0,
        }
    }

    pub fn is_running(self) -> bool {
        matches!(self, Self::Running(_))
    }
}

/// `SubagentPoolTask`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SubagentPoolTask {
    pub task_id: String,
    pub agent_type: String,
    pub task: String,
    pub token_budget: Option<u64>,
    pub cwd: Option<PathBuf>,
    pub model: Option<String>,
    pub provider: Option<String>,
    /// The dispatching session's own concrete model, and the one the inherited-
    /// model fallback runs on. `model` alone is not enough: when the caller
    /// asked for a model tier it holds a *category*, which resolves to
    /// the model that just failed, so a fallback built from it retries the same
    /// model and changes nothing.
    pub inherited_model: Option<String>,
    /// The explicit `effort` (a thinking level name), if the caller set one.
    pub effort: Option<String>,
    /// Session file to persist/continue. Default: `<parent>/subagents/<task>.jsonl`
    /// when there is a parent session, else the task's dispatch dir.
    pub session_file: Option<PathBuf>,
    /// The session file of the session that dispatched this task. Its transcript
    /// goes under `<parent>/subagents/`.
    pub parent_session_file: Option<PathBuf>,
    /// Internal: retry with the caller's model after the preferred one failed.
    pub use_inherited_model_fallback: bool,
    /// The caller asked for a notification instead of an inline answer. Only
    /// telemetry reads this: `mode` in the ledger.
    pub background: Option<bool>,
    /// When this task entered the queue (epoch ms). Ageing uses it so a long
    /// wait eventually outranks the priority tiers above it.
    pub queued_at: Option<u64>,
}

/// Terminal status of a result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultStatus {
    Complete,
    Partial,
    Failed,
    Stalled,
    Timeout,
    Cancelled,
}

impl ResultStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Partial => "partial",
            Self::Failed => "failed",
            Self::Stalled => "stalled",
            Self::Timeout => "timeout",
            Self::Cancelled => "cancelled",
        }
    }
}

/// `SubagentResult`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SubagentResult {
    pub task_id: String,
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
    /// Advisory: the task went over its token budget.
    pub budget_exceeded: Option<bool>,
    pub status: Option<ResultStatus>,
    /// The parsed `result.json`, when there is one.
    pub result_data: Option<Map<String, Value>>,
    /// This run used the inherited-model fallback.
    pub used_inherited_model_fallback: Option<bool>,
}

/// `TaskResult` of [`SubagentPool::dispatch`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TaskResult {
    /// The evaluator kept the task inline (depth guard).
    pub handled_inline: bool,
    pub task_id: Option<String>,
    pub agent_type: Option<String>,
    pub reason: Option<String>,
    pub result: Option<SubagentResult>,
    /// Milliseconds, when delegated.
    pub duration: Option<u64>,
}

/// `DispatchOptions`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DispatchOptions {
    /// Skip evaluation and use this agent type.
    pub force_agent: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    /// The dispatching session's own model: the default, and the model the
    /// inherited-model fallback runs on.
    pub inherited_model: Option<String>,
    /// The explicit `effort` (a thinking level name).
    pub effort: Option<String>,
    /// Session file to persist/continue (resume).
    pub session_file: Option<PathBuf>,
    /// The session file of the dispatching session. Its subagent transcripts
    /// are under `<parent>/subagents/`, and a resume looks there first.
    pub parent_session_file: Option<PathBuf>,
    /// Caller-supplied task id (default: a generated `dispatch-…` id).
    pub task_id: Option<String>,
    /// Whether the caller wanted a background run (telemetry only).
    pub background: Option<bool>,
}

/// `SubagentPoolOptions`.
#[derive(Clone, Default)]
pub struct SubagentPoolOptions {
    /// The hoocode executable (or a runtime when `prefix_args` names a script).
    pub executable: PathBuf,
    pub prefix_args: Vec<String>,
    /// Default 5.
    pub max_concurrency: Option<usize>,
    /// Default: the process cwd.
    pub cwd: Option<PathBuf>,
    /// Default: the process environment.
    pub env: Option<HashMap<String, String>>,
    /// Replace how dispatches run. Default: a child process per dispatch.
    pub runner: Option<Arc<dyn Runner>>,
    /// Default: four times `max_concurrency`. A dispatch beyond it is refused
    /// with a message the caller can act on, rather than queued into a session
    /// that will never finish them.
    pub max_queued: Option<usize>,
    /// Finished results kept in memory for late pulls. Default [`MAX_COMPLETED`].
    pub max_completed: Option<usize>,
    pub default_token_budget: Option<u64>,
    /// Non-default skill paths forwarded to every child via `--skill`.
    pub skill_paths: Vec<String>,
    pub settings: Option<CategorySettings>,
    /// Available models for deriving model-category defaults (snapshot).
    pub available_models: Vec<Model>,
    /// The scoped models subagents may use (`scopedModels`, resolved). Empty:
    /// no scope, and every available model is allowed.
    pub scope: Vec<ResolvedScoped>,
}

/// `get_status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Running,
    Queued,
    Done,
    Failed,
    Stalled,
    Timeout,
    Cancelled,
    Unknown,
}

impl TaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Queued => "queued",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Stalled => "stalled",
            Self::Timeout => "timeout",
            Self::Cancelled => "cancelled",
            Self::Unknown => "unknown",
        }
    }
}

/// A pool event: hoocode's event name and payload.
#[derive(Debug, Clone, PartialEq)]
pub struct PoolEvent {
    pub name: &'static str,
    pub data: Value,
}

/// Pool errors (hoocode's thrown/rejected messages).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolError(pub String);

impl std::fmt::Display for PoolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PoolError {}

type Listener = Arc<dyn Fn(&PoolEvent) + Send + Sync>;
type Waiter = oneshot::Sender<Result<SubagentResult, PoolError>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KillReason {
    Stalled,
    Timeout,
    Cancelled,
}

impl KillReason {
    fn status(self) -> ResultStatus {
        match self {
            Self::Stalled => ResultStatus::Stalled,
            Self::Timeout => ResultStatus::Timeout,
            Self::Cancelled => ResultStatus::Cancelled,
        }
    }

    /// A kill reason that another model could plausibly fix.
    ///
    /// A `Stalled` or `Timeout` run is retried only when the classifier finds a
    /// provider error in what the child captured — reaching a deadline is on
    /// its own not evidence that the model was at fault. A `Cancelled` is never
    /// retried: the user asked for it to stop, and quietly restarting the work
    /// they just cancelled is worse than losing it.
    fn worth_retrying_on_another_model(self) -> bool {
        matches!(self, Self::Stalled | Self::Timeout)
    }
}

/// `SubagentSlot`: a running child.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentSlot {
    pub pid: u32,
    pub agent_type: String,
    pub task_id: String,
    /// Epoch ms.
    pub spawned_at: u64,
    pub token_budget: u64,
}

#[derive(Default)]
struct PoolState {
    slots: HashMap<String, SubagentSlot>,
    queue: VecDeque<SubagentPoolTask>,
    completed: HashMap<String, SubagentResult>,
    /// Insertion order of `completed`, so the oldest can be dropped: a finished
    /// result carries its captured stdout and stderr, and nothing used to
    /// release them, so a long session grew without bound.
    completed_order: VecDeque<String>,
    waiters: HashMap<String, Waiter>,
    budgets: HashMap<String, Arc<Mutex<TokenBudget>>>,
    kill_reasons: HashMap<String, KillReason>,
    /// Terminal status, kept after `wait_for` consumes the result.
    task_status: HashMap<String, TaskStatus>,
    skill_paths: Vec<String>,
    registry: Option<Arc<AgentRegistry>>,
    scope: Vec<ResolvedScoped>,
    disposed: bool,
}

struct PoolInner {
    id: u64,
    skill_paths: Vec<String>,
    /// How a dispatch runs. The product ships [`crate::runner::ProcessRunner`];
    /// the seam is what makes an in-process runner (and a process-free test
    /// suite) possible at all.
    runner: Arc<dyn Runner>,
    max_concurrency: usize,
    max_queued: usize,
    max_completed: usize,
    cwd: PathBuf,
    env: HashMap<String, String>,
    default_token_budget: u64,
    settings: Option<CategorySettings>,
    available_models: Vec<Model>,
    verifier: OutputVerifier,
    lifeguard: Arc<SubagentLifeguard>,
    state: Mutex<PoolState>,
    listeners: Mutex<Vec<Listener>>,
}

/// `SubagentPool`. Cheap to clone; must be created inside a tokio runtime.
#[derive(Clone)]
pub struct SubagentPool {
    inner: Arc<PoolInner>,
}

/// The cold pool's default concurrency. The warm pool shares it: one ceiling
/// for the process, not one per execution path.
pub const DEFAULT_MAX_CONCURRENCY: usize = 5;

/// Finished results held in memory. Beyond this the oldest is dropped; its
/// dispatch dir and ledger line are the durable record.
const MAX_COMPLETED: usize = 64;

/// Queue slots per running slot before a dispatch is refused outright.
pub const QUEUE_PER_SLOT: usize = 4;

/// A queued task's effective priority improves by one step per this long spent
/// waiting, so the two-tier priority cannot starve `code-review` behind a
/// stream of `explore` runs (the finding from Q9 in the design review).
const AGEING_STEP_MS: u64 = 60_000;

/// The prompt every child gets, on every dispatch path (cold command line,
/// in-process runner, warm worker): one wrapper, so the three cannot drift
/// (card 9, 4i). The brief (goal, context, workspace, done criteria) is in the
/// task text the parent wrote, so the wrapper adds nothing but the marker.
pub fn child_prompt(task: &str) -> String {
    format!("Task: {}", task.trim())
}

/// `"Read,Shell"` → `["Read", "Shell"]`.
fn split_csv(value: String) -> Vec<String> {
    value
        .split(',')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Append to a capped capture buffer, keeping the most recent tail.
fn append_tail(current: &mut String, chunk: &str) {
    current.push_str(chunk);
    if current.len() > MAX_CAPTURED_STREAM_CHARS {
        let mut cut = current.len() - MAX_CAPTURED_STREAM_CHARS;
        while !current.is_char_boundary(cut) {
            cut += 1;
        }
        current.drain(..cut);
    }
}

/// The last `cap` bytes of `text`, on a char boundary.
fn tail(text: &str, cap: usize) -> String {
    if text.len() <= cap {
        return text.to_string();
    }
    let mut cut = text.len() - cap;
    while !text.is_char_boundary(cut) {
        cut += 1;
    }
    text[cut..].to_string()
}

/// Higher runs first: read-only investigation often unblocks other work.
fn priority_of(agent_type: &str) -> u8 {
    if agent_type == "explore" || agent_type == "plan" {
        2
    } else {
        1
    }
}

/// `Math.random().toString(36).slice(2, 8)`: six base-36 characters.
/// Priority with ageing: higher wins, and a task gains one step per
/// [`AGEING_STEP_MS`] spent waiting, so nothing in the queue can be starved
/// indefinitely by a stream of higher-priority work.
fn effective_priority(agent_type: &str, queued_at: u64, now: u64) -> i64 {
    let base = priority_of(agent_type) as i64;
    let waited = now.saturating_sub(queued_at) as i64;
    base.saturating_add(waited / AGEING_STEP_MS as i64)
}

fn random_suffix() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut seed = nanos
        ^ (std::process::id() as u64).rotate_left(32)
        ^ COUNTER
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15);
    (0..6)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            DIGITS[(seed % 36) as usize] as char
        })
        .collect()
}

impl SubagentPool {
    pub fn new(options: SubagentPoolOptions) -> Self {
        let cwd = options
            .cwd
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| ".".into()));
        let lifeguard = SubagentLifeguard::new(&cwd);
        // The lifeguard runs in this process, so the stall override is read
        // from the parent's environment (the child env is cleared and rebuilt).
        let env = options.env.unwrap_or_else(|| std::env::vars().collect());
        lifeguard.set_progress_stall_ms(progress_stall_ms_from_env(&env));
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let skill_paths = options.skill_paths.clone();
        let inner = Arc::new(PoolInner {
            id: NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            runner: options.runner.clone().unwrap_or_else(|| {
                Arc::new(ProcessRunner::new(
                    options.executable.clone(),
                    options.prefix_args.clone(),
                ))
            }),
            max_concurrency: options.max_concurrency.unwrap_or(DEFAULT_MAX_CONCURRENCY),
            max_queued: options.max_queued.unwrap_or_else(|| {
                options.max_concurrency.unwrap_or(DEFAULT_MAX_CONCURRENCY) * QUEUE_PER_SLOT
            }),
            max_completed: options.max_completed.unwrap_or(MAX_COMPLETED),
            env,
            default_token_budget: options.default_token_budget.unwrap_or(0),
            settings: options.settings,
            available_models: options.available_models,
            verifier: OutputVerifier::new(cwd.clone()),
            lifeguard: lifeguard.clone(),
            cwd,
            skill_paths,
            state: Mutex::new(PoolState {
                scope: options.scope,
                ..Default::default()
            }),
            listeners: Mutex::new(Vec::new()),
        });
        let weak: Weak<PoolInner> = Arc::downgrade(&inner);
        lifeguard.on_event(move |event| {
            let Some(inner) = weak.upgrade() else { return };
            let (task_id, pid, reason, name) = match event {
                LifeguardEvent::Stalled { task_id, pid } => {
                    (task_id, pid, KillReason::Stalled, "task_stalled")
                }
                LifeguardEvent::Timeout { task_id, pid } => {
                    (task_id, pid, KillReason::Timeout, "task_timeout")
                }
            };
            inner.state().kill_reasons.insert(task_id.clone(), reason);
            inner.emit(name, json!({"task_id": task_id, "pid": pid}));
        });
        Self { inner }
    }

    /// A process-unique id for this pool.
    pub fn id(&self) -> u64 {
        self.inner.id
    }

    /// Listen for pool events.
    pub fn on(&self, listener: impl Fn(&PoolEvent) + Send + Sync + 'static) {
        self.inner
            .listeners
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(Arc::new(listener));
    }

    /// Test hook: deliver a synthetic event to the listeners.
    #[doc(hidden)]
    pub fn emit_for_testing(&self, name: &'static str, data: Value) {
        self.inner.emit(name, data);
    }

    /// The lifeguard (tests inject stall verdicts through it).
    pub fn lifeguard(&self) -> &Arc<SubagentLifeguard> {
        &self.inner.lifeguard
    }

    /// Update the skill paths forwarded to new subagents.
    pub fn update_skill_paths(&self, paths: Vec<String>) {
        self.inner.state().skill_paths = paths;
    }

    /// External in-process load (background MCP tools) for the lifeguard.
    pub fn set_external_load(&self, count: i64) {
        self.inner.lifeguard.set_external_load(count);
    }

    /// Use this registry instead of loading one for the pool's cwd.
    pub fn set_registry(&self, registry: AgentRegistry) {
        self.inner.state().registry = Some(Arc::new(registry));
    }

    /// Replace the scoped models subagents may use. Applies to dispatches that
    /// start after this call.
    pub fn set_scope(&self, scope: Vec<ResolvedScoped>) {
        self.inner.state().scope = scope;
    }

    /// The model and effort a dispatch of `agent_type` runs on for `request`.
    /// The agent definition's `model:` is the pin; `request.pin` is ignored.
    pub fn select_model(&self, agent_type: &str, request: &ModelRequest) -> ModelSelection {
        self.inner.select(agent_type, request.clone(), true)
    }

    /// Queue a task; it runs when a slot is free.
    pub fn spawn(&self, task: SubagentPoolTask) -> Result<(), PoolError> {
        {
            let mut state = self.inner.state();
            if state.disposed {
                return Err(PoolError("SubagentPool has been disposed".into()));
            }
            if state.slots.contains_key(&task.task_id)
                || state.queue.iter().any(|t| t.task_id == task.task_id)
                || state.completed.contains_key(&task.task_id)
            {
                return Err(PoolError(format!("Duplicate task_id: {}", task.task_id)));
            }
            // Admission control: an unbounded queue is unbounded memory and a
            // parent that will never see those results. Refusing loudly is
            // something the caller can act on; a queue that grows is not.
            if state.queue.len() >= self.inner.max_queued {
                return Err(PoolError(format!(
                    "Dispatch queue is full ({} waiting, {} running). Try again once one finishes.",
                    state.queue.len(),
                    state.slots.len()
                )));
            }
            let mut task = task;
            // Written under the lock and before the task is queued, so no pull
            // can start the run (and write its settle record) ahead of this one.
            if let Some(parent) = &task.parent_session_file {
                let child = self.inner.child_session_file(&task);
                write_run_record(parent, &task, &child, None);
            }
            let now = now_ms();
            task.queued_at.get_or_insert(now);
            let p = effective_priority(&task.agent_type, task.queued_at.unwrap_or(now), now);
            match state.queue.iter().position(|t| {
                effective_priority(&t.agent_type, t.queued_at.unwrap_or(now), now) < p
            }) {
                Some(idx) => state.queue.insert(idx, task),
                None => state.queue.push_back(task),
            }
        }
        self.inner.pull();
        Ok(())
    }

    /// Every tracked task with its status and when it started: what a panel
    /// needs to offer "cancel the newest one" without a focus ring.
    pub fn statuses(&self) -> Vec<(String, RunningSince)> {
        let state = self.inner.state();
        let mut out: Vec<(String, RunningSince)> = state
            .slots
            .values()
            .map(|slot| (slot.task_id.clone(), RunningSince::Running(slot.spawned_at)))
            .collect();
        out.extend(
            state
                .queue
                .iter()
                .map(|task| (task.task_id.clone(), RunningSince::Queued)),
        );
        out
    }

    /// Current status of a task.
    pub fn get_status(&self, task_id: &str) -> TaskStatus {
        let state = self.inner.state();
        if state.slots.contains_key(task_id) {
            return TaskStatus::Running;
        }
        if state.queue.iter().any(|t| t.task_id == task_id) {
            return TaskStatus::Queued;
        }
        if let Some(status) = state.task_status.get(task_id) {
            return *status;
        }
        match state.completed.get(task_id) {
            Some(result) => match result.status {
                Some(ResultStatus::Stalled) => TaskStatus::Stalled,
                Some(ResultStatus::Timeout) => TaskStatus::Timeout,
                Some(ResultStatus::Cancelled) => TaskStatus::Cancelled,
                _ if result.ok => TaskStatus::Done,
                _ => TaskStatus::Failed,
            },
            None => TaskStatus::Unknown,
        }
    }

    /// Cancel on the user's behalf. A queued task settles at once; a running
    /// task's process tree is killed and it settles `cancelled` on exit
    /// (unless it already wrote a valid result.json). False for unknown ids.
    pub fn cancel(&self, task_id: &str) -> bool {
        let mut state = self.inner.state();
        if let Some(idx) = state.queue.iter().position(|t| t.task_id == task_id) {
            let Some(task) = state.queue.remove(idx) else {
                return false;
            };
            drop(state);
            let result = SubagentResult {
                task_id: task_id.to_string(),
                error: Some("cancelled before start".into()),
                status: Some(ResultStatus::Cancelled),
                ..Default::default()
            };
            self.inner
                .emit("task_cancelled", json!({"task_id": task_id}));
            self.inner
                .record_attempt(&task, &result, 0, 0, 0, None, false);
            self.inner.resolve_waiter(task_id, result);
            return true;
        }
        let Some(pid) = state.slots.get(task_id).map(|s| s.pid) else {
            return false;
        };
        state
            .kill_reasons
            .insert(task_id.to_string(), KillReason::Cancelled);
        drop(state);
        if pid > 0 {
            kill_process_tree(pid);
        }
        true
    }

    /// Wait for a task and take its result.
    pub async fn wait_for(&self, task_id: &str) -> Result<SubagentResult, PoolError> {
        let rx = {
            let mut state = self.inner.state();
            if state.disposed {
                return Err(PoolError("SubagentPool has been disposed".into()));
            }
            if let Some(result) = state.completed.remove(task_id) {
                return Ok(result);
            }
            let (tx, rx) = oneshot::channel();
            state.waiters.insert(task_id.to_string(), tx);
            rx
        };
        rx.await
            .unwrap_or_else(|_| Err(PoolError("SubagentPool disposed".into())))
    }

    /// The running child for a task.
    pub fn slot(&self, task_id: &str) -> Option<SubagentSlot> {
        self.inner.state().slots.get(task_id).cloned()
    }

    pub fn running_count(&self) -> usize {
        self.inner.state().slots.len()
    }

    /// Whether this pool is running or has queued the task. A task it holds is
    /// live before its child has written a pid (a queued one has none yet).
    pub fn is_live(&self, task_id: &str) -> bool {
        self.inner.is_live_task(task_id)
    }

    pub fn queued_count(&self) -> usize {
        self.inner.state().queue.len()
    }

    /// Dispatch through the evaluator and wait: inline when the depth guard
    /// keeps it (and no agent is forced), else spawn and wait.
    pub async fn dispatch(
        &self,
        task: &str,
        options: DispatchOptions,
    ) -> Result<TaskResult, PoolError> {
        if self.inner.state().disposed {
            return Err(PoolError("SubagentPool has been disposed".into()));
        }
        let begin = self.begin_dispatch(task, options)?;
        let Some((task_id, agent_type, start)) = begin.spawned else {
            return Ok(TaskResult {
                handled_inline: true,
                reason: begin.reason,
                ..Default::default()
            });
        };
        let result = self.wait_for(&task_id).await?;
        Ok(TaskResult {
            handled_inline: false,
            task_id: Some(task_id),
            agent_type: Some(agent_type),
            reason: begin.reason,
            result: Some(result),
            duration: Some(now_ms().saturating_sub(start)),
        })
    }

    /// Fire-and-forget dispatch for background agents; poll with
    /// [`get_status`](Self::get_status) / [`collect`](Self::collect).
    pub fn dispatch_detached(
        &self,
        task: &str,
        options: DispatchOptions,
    ) -> Result<TaskResult, PoolError> {
        if self.inner.state().disposed {
            return Err(PoolError("SubagentPool has been disposed".into()));
        }
        let begin = self.begin_dispatch(task, options)?;
        Ok(match begin.spawned {
            None => TaskResult {
                handled_inline: true,
                reason: begin.reason,
                ..Default::default()
            },
            Some((task_id, agent_type, _)) => TaskResult {
                handled_inline: false,
                task_id: Some(task_id),
                agent_type: Some(agent_type),
                reason: begin.reason,
                ..Default::default()
            },
        })
    }

    fn begin_dispatch(&self, task: &str, options: DispatchOptions) -> Result<Begin, PoolError> {
        // Shedding (process memory above its soft limit): no new children until it lifts.
        if hoocode_runtime::shedding() {
            return Err(PoolError(
                "Subagents are paused: memory is above the soft limit. Try again once it recovers."
                    .into(),
            ));
        }
        let analysis = DispatchEvaluator.evaluate_with_env(task, &self.inner.env);
        if options.force_agent.is_none() && !analysis.should_delegate {
            return Ok(Begin {
                reason: Some(analysis.reason),
                spawned: None,
            });
        }
        let agent_type = options
            .force_agent
            .clone()
            .unwrap_or_else(|| "general-purpose".into());
        let task_id = options
            .task_id
            .clone()
            .unwrap_or_else(|| format!("dispatch-{}-{}", now_ms(), random_suffix()));
        let reason = if options.force_agent.is_some() {
            "user_override".to_string()
        } else {
            analysis.reason
        };
        let complexity = analysis.estimated_complexity.as_str();
        let child_depth = current_subagent_depth(&self.inner.env) + 1;
        agent_log(&format!(
            "[DISPATCH] agent={agent_type} depth={child_depth} reason={reason} complexity={complexity} task_id={task_id}"
        ));
        // Housekeeping: old dispatch dirs go on the Low lane, off the dispatch path.
        // Dirs of tasks this pool is running or has queued are never removed.
        // Liveness is queried per dir at removal time, not snapshotted here.
        let cwd = self.inner.cwd.clone();
        let inner = Arc::clone(&self.inner);
        hoocode_runtime::spawn_bg(async move {
            prune_dispatch_dirs_checked(&cwd, DISPATCH_KEEP, DISPATCH_MAX_AGE, &|name| {
                inner.is_live_task(name)
            });
        });
        let parent_session = options
            .parent_session_file
            .as_deref()
            .and_then(hoocode_code_session::session_id_of_file);
        self.inner.write_dispatch_log(
            &task_id,
            &agent_type,
            &reason,
            complexity,
            task,
            child_depth,
            parent_session.as_deref(),
        );
        let start = now_ms();
        self.spawn(SubagentPoolTask {
            task_id: task_id.clone(),
            agent_type: agent_type.clone(),
            task: task.to_string(),
            model: options.model,
            provider: options.provider,
            inherited_model: options.inherited_model,
            effort: options.effort,
            session_file: options.session_file,
            parent_session_file: options.parent_session_file,
            cwd: Some(self.inner.cwd.clone()),
            background: options.background,
            ..Default::default()
        })?;
        Ok(Begin {
            reason: Some(reason),
            spawned: Some((task_id, agent_type, start)),
        })
    }

    /// Read a completed task's result without consuming it.
    pub fn collect(&self, task_id: &str) -> Option<SubagentResult> {
        self.inner.state().completed.get(task_id).cloned()
    }

    /// The persisted session file for a task.
    pub fn get_session_file(&self, task_id: &str, cwd: Option<&Path>) -> PathBuf {
        self.inner.session_file(task_id, cwd)
    }

    /// Continue a previously dispatched subagent's persisted session with a
    /// follow-up prompt, as the agent type it was dispatched with.
    pub async fn resume(
        &self,
        task_id: &str,
        prompt: &str,
        options: DispatchOptions,
    ) -> Result<TaskResult, PoolError> {
        if self.inner.state().disposed {
            return Err(PoolError("SubagentPool has been disposed".into()));
        }
        let parent = options.parent_session_file.as_deref();
        let named = options.session_file.as_deref();
        let Some(session_file) = resume_transcript(&self.inner.cwd, task_id, parent, named) else {
            let expected = match parent {
                Some(parent) => subagents_dir(parent).join(format!("{task_id}.jsonl")),
                None => self.get_session_file(task_id, None),
            };
            return Err(PoolError(format!(
                "No resumable session for task \"{task_id}\" (expected {}).",
                expected.display()
            )));
        };
        // Resuming makes the session the newest thing on disk, so the prune
        // (which ages a dir by its newest activity) cannot delete it between
        // now and the child's first write.
        touch_dispatch_dir(&hoocode_code_paths::dispatch_task_dir(
            &self.inner.cwd,
            task_id,
        ));
        let agent_type = self
            .inner
            .read_dispatch_agent_type(task_id)
            .unwrap_or_else(|| "general-purpose".into());
        self.dispatch(
            prompt,
            DispatchOptions {
                force_agent: Some(agent_type),
                session_file: Some(session_file),
                ..options
            },
        )
        .await
    }

    /// Kill all running processes, clear the queue, fail pending waiters.
    pub fn dispose(&self) {
        let (pids, waiters) = {
            let mut state = self.inner.state();
            if state.disposed {
                return;
            }
            state.disposed = true;
            let pids: Vec<u32> = state.slots.values().map(|s| s.pid).collect();
            state.slots.clear();
            state.queue.clear();
            let waiters: Vec<Waiter> = state.waiters.drain().map(|(_, w)| w).collect();
            state.completed.clear();
            state.completed_order.clear();
            state.budgets.clear();
            state.kill_reasons.clear();
            state.task_status.clear();
            (pids, waiters)
        };
        // The whole process group: a subagent's own children must not outlive
        // the pool.
        for pid in pids.into_iter().filter(|p| *p > 0) {
            kill_process_tree(pid);
        }
        for waiter in waiters {
            let _ = waiter.send(Err(PoolError("SubagentPool disposed".into())));
        }
        self.inner.lifeguard.dispose();
        self.inner
            .listeners
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    /// `shouldRetryWithInheritedModel`: a failed built-in or project agent
    /// that pins its own model retries once with the caller's model.
    pub fn should_retry_with_inherited_model(
        &self,
        task: &SubagentPoolTask,
        result: &SubagentResult,
    ) -> bool {
        self.inner.should_retry_with_inherited_model(task, result)
    }

    /// `isInheritedModelFallbackError`.
    pub fn is_inherited_model_fallback_error(result: &SubagentResult) -> bool {
        is_inherited_model_fallback_error(result)
    }
}

struct Begin {
    reason: Option<String>,
    spawned: Option<(String, String, u64)>,
}

fn is_inherited_model_fallback_error(result: &SubagentResult) -> bool {
    let data = Value::Object(result.result_data.clone().unwrap_or_default()).to_string();
    let text = [result.error.as_deref().unwrap_or(""), &result.stderr, &data]
        .into_iter()
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    INHERITED_MODEL_FALLBACK_ERROR.is_match(&text)
}

impl PoolInner {
    fn state(&self) -> MutexGuard<'_, PoolState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn emit(&self, name: &'static str, data: Value) {
        let listeners = self
            .listeners
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let event = PoolEvent { name, data };
        for listener in listeners {
            listener(&event);
        }
    }

    fn registry(&self) -> Arc<AgentRegistry> {
        if let Some(registry) = self.state().registry.clone() {
            return registry;
        }
        let registry = Arc::new(load_agent_registry(&LoadAgentRegistryOptions::new(
            self.cwd.to_string_lossy(),
        )));
        self.state().registry = Some(registry.clone());
        registry
    }

    /// The selection for `request`. `use_pin` adds the agent definition's
    /// `model:` as the default.
    fn select(&self, agent_type: &str, request: ModelRequest, use_pin: bool) -> ModelSelection {
        let pin = use_pin
            .then(|| self.definition(agent_type).and_then(|d| d.model))
            .flatten()
            .filter(|m| !m.is_empty() && m != MODEL_INHERIT);
        let scope = self.state().scope.clone();
        select_model(
            &ModelRequest { pin, ..request },
            self.settings.as_ref(),
            &self.available_models,
            &scope,
        )
    }

    fn definition(&self, agent_type: &str) -> Option<AgentDefinition> {
        if agent_type.is_empty() {
            return None;
        }
        self.registry().get(agent_type).cloned()
    }

    fn session_file(&self, task_id: &str, cwd: Option<&Path>) -> PathBuf {
        hoocode_code_paths::dispatch_task_dir(cwd.unwrap_or(&self.cwd), task_id)
            .join("session.jsonl")
    }

    /// The transcript a task writes. An explicit session file (a resume or a
    /// fork) wins. Otherwise a run with a parent session writes
    /// `<parent>/subagents/<task>.jsonl`, and a run without one keeps its
    /// scratch session in its dispatch dir.
    fn child_session_file(&self, task: &SubagentPoolTask) -> PathBuf {
        if let Some(file) = &task.session_file {
            return file.clone();
        }
        match &task.parent_session_file {
            Some(parent) => subagents_dir(parent).join(format!("{}.jsonl", task.task_id)),
            None => self.session_file(&task.task_id, task.cwd.as_deref()),
        }
    }

    /// Task ids this pool is running or has queued: their dispatch dirs are
    /// never pruned.
    /// Whether the pool is running or has queued this task.
    fn is_live_task(&self, task_id: &str) -> bool {
        let state = self.state();
        state.slots.contains_key(task_id) || state.queue.iter().any(|t| t.task_id == task_id)
    }

    fn read_dispatch_agent_type(&self, task_id: &str) -> Option<String> {
        let path =
            hoocode_code_paths::dispatch_task_dir(&self.cwd, task_id).join("dispatch-log.json");
        let parsed: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
        parsed
            .get("agent_type")
            .and_then(Value::as_str)
            .map(String::from)
    }

    #[allow(clippy::too_many_arguments)]
    fn write_dispatch_log(
        &self,
        task_id: &str,
        agent_type: &str,
        reason: &str,
        complexity: &str,
        task: &str,
        depth: u32,
        parent_session: Option<&str>,
    ) {
        let log = json!({
            "timestamp": chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
            "task_id": task_id,
            "agent_type": agent_type,
            "depth": depth,
            "reason": reason,
            "complexity": complexity,
            "task": task,
            "parent_session": parent_session,
        });
        let path =
            hoocode_code_paths::dispatch_task_dir(&self.cwd, task_id).join("dispatch-log.json");
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, serde_json::to_string_pretty(&log).unwrap_or_default());
    }

    /// `output.json`: the post-mortem for a run that did not succeed.
    ///
    /// Deviation: hoocode embeds the whole captured `stdout`, which is a 256KB
    /// tail of the child's JSONL event stream — the recorded failures wrote
    /// 267-275KB files, most of it the task prompt echoed back. Nothing reads
    /// this file in code; it exists for whoever is debugging a failed dispatch,
    /// and the transcript is already on disk in `session.jsonl`. So this carries
    /// the outcome, the cause, and the tail of stderr (where a provider error
    /// lives), and leaves the transcript where it belongs.
    fn write_output_json(&self, task_id: &str, result: &SubagentResult) {
        // JSON.stringify drops undefined fields.
        let mut output = Map::new();
        output.insert("task_id".into(), Value::from(result.task_id.clone()));
        output.insert("ok".into(), Value::from(result.ok));
        output.insert("exit_code".into(), json!(result.exit_code));
        if let Some(status) = result.status {
            output.insert("status".into(), Value::from(status.as_str()));
        }
        output.insert(
            "stderr_tail".into(),
            Value::from(tail(&result.stderr, OUTPUT_JSON_STDERR_TAIL_CHARS)),
        );
        if let Some(error) = &result.error {
            output.insert("error".into(), Value::from(error.clone()));
        }
        if let Some(exceeded) = result.budget_exceeded {
            output.insert("budget_exceeded".into(), Value::from(exceeded));
        }
        if let Some(data) = &result.result_data {
            output.insert("result_data".into(), Value::Object(data.clone()));
        }
        let path = hoocode_code_paths::dispatch_task_dir(&self.cwd, task_id).join("output.json");
        let text = serde_json::to_string_pretty(&Value::Object(output)).unwrap_or_default();
        let _ = write_file_atomic(&path, &text);
    }

    fn read_result_json(&self, task_id: &str, cwd: &Path) -> Option<Map<String, Value>> {
        let path = hoocode_code_paths::dispatch_task_dir(cwd, task_id).join("result.json");
        match serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()? {
            Value::Object(map) => Some(map),
            _ => None,
        }
    }

    /// Start queued tasks while slots are free.
    fn pull(self: &Arc<Self>) {
        loop {
            let task = {
                let mut state = self.state();
                if state.disposed || state.slots.len() >= self.max_concurrency {
                    return;
                }
                match state.queue.pop_front() {
                    Some(task) => task,
                    None => return,
                }
            };
            self.start_task(task, false);
        }
    }

    /// Everything one dispatch needs, resolved: the command line for the
    /// process runner, and the same facts as fields so a runner that builds a
    /// session instead of a command line does not have to parse argv back out.
    fn run_spec(&self, task: &SubagentPoolTask) -> RunSpec {
        let argv = self.build_args(task);
        let mut system_prompt = None;
        let mut tools = None;
        let mut disallowed_tools = None;
        let mut model = None;
        let mut provider = task.provider.clone();
        let mut max_turns = DEFAULT_SUBAGENT_MAX_TURNS;
        let mut deadline_ms = None;
        let mut rest = argv.iter();
        while let Some(arg) = rest.next() {
            let flag = arg.as_str();
            let mut value = || rest.next().cloned();
            match flag {
                "--system-prompt" => system_prompt = value(),
                "--model" => model = value(),
                "--provider" => provider = value(),
                "--max-turns" => {
                    if let Some(raw) = value() {
                        max_turns = raw.parse().unwrap_or(max_turns);
                    }
                }
                "--deadline-ms" => deadline_ms = value().and_then(|raw| raw.parse().ok()),
                "--tools" => tools = value().map(split_csv),
                "--disallowed-tools" => disallowed_tools = value().map(split_csv),
                _ => {}
            }
        }
        // The prompt is the trailing positional argument the pool appends; a
        // runner that builds a session gets the same text the child would.
        let prompt = child_prompt(&task.task);
        RunSpec {
            task_id: task.task_id.clone(),
            agent_type: task.agent_type.clone(),
            cwd: task.cwd.clone().unwrap_or_else(|| self.cwd.clone()),
            argv,
            env: self.child_env(task),
            prompt,
            max_turns,
            deadline_ms,
            system_prompt,
            tools,
            disallowed_tools,
            model,
            provider,
            skill_paths: self.skill_paths.clone(),
        }
    }

    /// The child's command line.
    fn build_args(&self, task: &SubagentPoolTask) -> Vec<String> {
        let session_file = self.child_session_file(task);
        // `prefix_args` belongs to the runner now: it prepends them, so a
        // runner that never builds a command line does not carry them.
        let mut args: Vec<String> = Vec::new();
        args.extend([
            "--mode".into(),
            "json".into(),
            "--session".into(),
            session_file.to_string_lossy().into_owned(),
            "--task-id".into(),
            task.task_id.clone(),
        ]);
        if let Some(parent) = &task.parent_session_file {
            args.extend([
                "--parent-session".into(),
                parent.to_string_lossy().into_owned(),
            ]);
        }

        let def = self.definition(&task.agent_type);
        if !task.agent_type.is_empty() {
            if let Some(prompt) = def.as_ref().map(|d| &d.prompt).filter(|p| !p.is_empty()) {
                args.extend(["--system-prompt".into(), prompt.clone()]);
            }
            // A `delegate: true` agent may dispatch itself, but only while the
            // child it becomes can still nest.
            let child_depth = current_subagent_depth(&self.env) + 1;
            let can_child_delegate = def.as_ref().and_then(|d| d.delegate) == Some(true)
                && child_depth < resolve_max_subagent_depth(None, &self.env);
            let mut tools = def.as_ref().and_then(|d| d.tools.clone());
            if can_child_delegate {
                if let Some(tools) = &mut tools {
                    for t in ["Agent", "AgentOutput"] {
                        if !tools.iter().any(|x| x == t) {
                            tools.push(t.into());
                        }
                    }
                }
            }
            if let Some(tools) = tools.filter(|t| !t.is_empty()) {
                args.extend(["--tools".into(), tools.join(",")]);
            }
            let disallowed =
                subagent_disallowed_tools(def.as_ref().and_then(|d| d.disallowed_tools.as_deref()));
            args.extend(["--disallowed-tools".into(), disallowed.join(",")]);
            if can_child_delegate {
                args.push("--enable-subagents".into());
                if let Some(to) = def
                    .as_ref()
                    .and_then(|d| d.delegate_to.as_ref())
                    .filter(|t| !t.is_empty())
                {
                    args.extend(["--delegate-allow".into(), to.join(",")]);
                }
            }
        }

        // A definition's explicit model wins (unless `inherit`), else the
        // caller's; a category resolves to a concrete model or to nothing.
        let selection = self.select_for_task(task);
        if let Some(model) = &selection.model {
            args.extend(["--model".into(), model.clone()]);
        }
        if let Some(level) = &selection.effort {
            args.extend(["--thinking".into(), level.as_str().into()]);
        }
        // A run-only scope (`--models`) reaches nested children, so their
        // subagents stay within it. An explicit `--model` still wins in the child.
        if let Some(models) = crate::instance::scoped_models_override_flag() {
            args.extend(["--models".into(), models]);
        }
        let model = selection.model.clone();
        // --provider would filter out a model id carrying another provider's
        // prefix, so it only goes along with a bare model id.
        if let Some(provider) = task.provider.as_ref().filter(|p| !p.is_empty()) {
            if model.as_ref().is_none_or(|m| !m.contains('/')) {
                args.extend(["--provider".into(), provider.clone()]);
            }
        }

        let max_turns = def
            .as_ref()
            .and_then(|d| d.max_turns)
            .filter(|n| *n > 0)
            .unwrap_or(DEFAULT_SUBAGENT_MAX_TURNS);
        args.extend(["--max-turns".into(), max_turns.to_string()]);
        // The same deadline `SubagentLifeguard` enforces (2 hours for every
        // agent type, 2026-10-10), so the child can ask to wrap up and write a
        // result before we kill it. The base is sent, not the load-scaled
        // budget: under load we widen the kill, so the wrap-up still lands first.
        args.extend([
            "--deadline-ms".into(),
            crate::lifeguard::base_timeout_ms(&task.agent_type).to_string(),
        ]);

        for path in self.state().skill_paths.clone() {
            args.extend(["--skill".into(), path]);
        }

        args.push(child_prompt(&task.task));
        args
    }

    /// The child's environment: depth + 1, MCP skipped for an MCP-free
    /// allowlist, deferred MCP schemas off.
    fn child_env(&self, task: &SubagentPoolTask) -> HashMap<String, String> {
        let mut env = self.env.clone();
        let depth = current_subagent_depth(&self.env) + 1;
        for prefix in hoocode_code_paths::ENV_PREFIXES {
            env.remove(&format!("{prefix}{SUBAGENT_DEPTH_ENV}"));
        }
        env.insert(
            format!("{CHILD_ENV_PREFIX}{SUBAGENT_DEPTH_ENV}"),
            depth.to_string(),
        );
        let def = self.definition(&task.agent_type);
        if !tool_allowlist_needs_mcp(def.as_ref().and_then(|d| d.tools.as_deref())) {
            env.insert(
                format!("{CHILD_ENV_PREFIX}{SUBAGENT_SKIP_MCP_ENV}"),
                "1".into(),
            );
        }
        for prefix in hoocode_code_paths::ENV_PREFIXES {
            env.remove(&format!("{prefix}{DEFER_MCP_SCHEMAS_ENV}"));
        }
        env
    }

    fn budget_for(self: &Arc<Self>, task: &SubagentPoolTask) -> Arc<Mutex<TokenBudget>> {
        if let Some(budget) = self.state().budgets.get(&task.task_id) {
            return budget.clone();
        }
        let mut budget = TokenBudget::new(
            &task.task_id,
            &task.agent_type,
            TokenBudgetOptions {
                limit: task.token_budget,
                cwd: Some(task.cwd.clone().unwrap_or_else(|| self.cwd.clone())),
            },
        );
        let weak = Arc::downgrade(self);
        budget.on_warning(Box::new(move |data| {
            if let Some(inner) = weak.upgrade() {
                inner.emit("budget_warning", data.clone());
            }
        }));
        // Advisory: telemetry only; --max-turns is the hard stop.
        let weak = Arc::downgrade(self);
        budget.on_exceeded(Box::new(move |data| {
            if let Some(inner) = weak.upgrade() {
                inner.emit("budget_exceeded", data.clone());
            }
        }));
        let budget = Arc::new(Mutex::new(budget));
        self.state()
            .budgets
            .insert(task.task_id.clone(), budget.clone());
        budget
    }

    /// Start a task in a child process (one retry on a spawn failure).
    fn start_task(self: &Arc<Self>, task: SubagentPoolTask, is_retry: bool) {
        let budget = self.budget_for(&task);
        let spec = self.run_spec(&task);
        let mut handle = match self.runner.spawn(spec) {
            Ok(handle) => handle,
            Err(_) if !is_retry => return self.start_task(task, true),
            Err(_) => {
                let result = SubagentResult {
                    task_id: task.task_id.clone(),
                    error: Some("Spawn failed synchronously".into()),
                    status: Some(ResultStatus::Failed),
                    ..Default::default()
                };
                self.record_attempt(&task, &result, 0, 0, 0, None, false);
                self.emit(
                    "task_failed",
                    json!({"task_id": task.task_id, "error": "Spawn failed synchronously"}),
                );
                self.resolve_waiter(&task.task_id, result);
                self.state().budgets.remove(&task.task_id);
                self.pull();
                return;
            }
        };
        let pid = handle.pid();
        let spawned_at = now_ms();
        // The sweep decides a dispatch dir is dead by reading this file. It has
        // always read it; nothing ever wrote it, so reaping was age-only and a
        // genuinely orphaned child was never detected by liveness.
        let task_cwd = task.cwd.clone().unwrap_or_else(|| self.cwd.clone());
        write_pid_file(&task_cwd, &task.task_id, pid);
        self.state().slots.insert(
            task.task_id.clone(),
            SubagentSlot {
                pid,
                agent_type: task.agent_type.clone(),
                task_id: task.task_id.clone(),
                spawned_at,
                token_budget: task.token_budget.unwrap_or(self.default_token_budget),
            },
        );
        self.lifeguard.monitor(&task.task_id, &task.agent_type, pid);
        // What a live row needs and nothing else could tell it: which attempt
        // this is, the model the child *actually* resolved to (the caller asked
        // for a tier, not a model), and when the run's own deadline is.
        self.emit(
            "task_started",
            json!({
                "task_id": task.task_id,
                "agent_type": task.agent_type,
                "model": self.resolve_task_model(&task),
                "provider": task.provider,
                "attempt": if task.use_inherited_model_fallback { 2 } else { 1 },
                "deadline_at": now_ms() + crate::lifeguard::base_timeout_ms(&task.agent_type),
                "spawned_at": spawned_at,
            }),
        );

        let (mut stdout_rx, mut stderr_rx) = handle.take_streams();
        let captured = Arc::new(Mutex::new((String::new(), String::new())));

        let out_task = {
            let inner = self.clone();
            let captured = captured.clone();
            let budget = budget.clone();
            let task_id = task.task_id.clone();
            let agent_type = task.agent_type.clone();
            tokio::spawn(async move {
                let stdout = &mut stdout_rx;
                let mut reader = JsonlLineReader::with_max_buffer(MAX_SUBAGENT_EVENT_LINE_CHARS);
                while let Some(chunk) = stdout.recv().await {
                    let n = chunk.len();
                    // Any output is a heartbeat: a busy child is alive even
                    // before its ping line is parsed.
                    append_tail(
                        &mut captured.lock().unwrap_or_else(|e| e.into_inner()).0,
                        &String::from_utf8_lossy(&chunk[..n]),
                    );
                    inner.lifeguard.record_heartbeat(&task_id);
                    for line in reader.push(&chunk[..n]) {
                        inner.handle_stdout_line(&task_id, &agent_type, &budget, &line);
                    }
                }
                if let Some(line) = reader.finish() {
                    inner.handle_stdout_line(&task_id, &agent_type, &budget, &line);
                }
            })
        };
        let err_task = {
            let captured = captured.clone();
            tokio::spawn(async move {
                let stderr = &mut stderr_rx;
                while let Some(chunk) = stderr.recv().await {
                    let n = chunk.len();
                    append_tail(
                        &mut captured.lock().unwrap_or_else(|e| e.into_inner()).1,
                        &String::from_utf8_lossy(&chunk[..n]),
                    );
                }
            })
        };

        let inner = self.clone();
        tokio::spawn(async move {
            let status = handle.wait().await;
            // Stdio may stay open in grandchildren: wait briefly for it.
            let _ = tokio::time::timeout(EXIT_STDIO_GRACE, async {
                let _ = out_task.await;
                let _ = err_task.await;
            })
            .await;
            inner.lifeguard.untrack(&task.task_id);
            let (stdout, stderr) = captured.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let retry_scheduled = match status {
                Ok(status) => inner.settle(&task, &budget, spawned_at, status, stdout, stderr),
                Err(err) => {
                    inner.settle_error(&task, &budget, spawned_at, is_retry, err, stdout, stderr)
                }
            };
            if !retry_scheduled {
                inner.state().budgets.remove(&task.task_id);
            }
            inner.pull();
        });
    }

    fn handle_stdout_line(
        &self,
        task_id: &str,
        agent_type: &str,
        budget: &Arc<Mutex<TokenBudget>>,
        line: &str,
    ) {
        budget
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .process_line(line);
        match classify_subagent_line(line) {
            SubagentStdoutLine::Heartbeat => self.lifeguard.record_heartbeat(task_id),
            SubagentStdoutLine::Progress(event) => {
                // Forward progress, not liveness: a child whose heartbeats come
                // from a timer can be parked inside a provider call forever, and
                // this is the signal that tells the difference. A tool in flight
                // is tracked separately so a long build is not reaped as idle.
                let call_id = event
                    .get("toolCallId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                match event.get("type").and_then(Value::as_str) {
                    Some("tool_execution_start") => {
                        let tool = event
                            .get("toolName")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        let limit = tool_limit_from_start(tool, event.get("args"), &self.env);
                        self.lifeguard.record_tool_start(task_id, call_id, limit);
                    }
                    Some("tool_execution_end") => self.lifeguard.record_tool_end(task_id, call_id),
                    _ => self.lifeguard.record_progress(task_id),
                }
                self.emit(
                    "task_progress",
                    json!({"task_id": task_id, "agent_type": agent_type, "event": event}),
                );
            }
            SubagentStdoutLine::Ignore => {
                // Not forwarded to the UI, but a model turn ending (message_end)
                // or streaming is progress: a long turn must not be reaped as stalled.
                if ["message_start", "message_update", "message_end"]
                    .iter()
                    .any(|t| line.contains(&format!("\"{t}\"")))
                {
                    self.lifeguard.record_progress(task_id);
                }
            }
        }
    }

    /// The child exited: settle the task. Returns whether a retry was queued.
    fn settle(
        self: &Arc<Self>,
        task: &SubagentPoolTask,
        budget: &Arc<Mutex<TokenBudget>>,
        spawned_at: u64,
        code: Option<i32>,
        stdout: String,
        stderr: String,
    ) -> bool {
        let task_id = task.task_id.as_str();
        let cwd = task.cwd.clone().unwrap_or_else(|| self.cwd.clone());
        let (tokens_generated, budget_exceeded, peak_context) = {
            let mut budget = budget.lock().unwrap_or_else(|e| e.into_inner());
            budget.flush();
            (budget.used(), budget.is_exceeded(), budget.peak_context())
        };
        let kill_reason = {
            let mut state = self.state();
            state.slots.remove(task_id);
            state.kill_reasons.remove(task_id)
        };
        let duration = now_ms().saturating_sub(spawned_at);

        // Success is a verified result.json, not the exit code: a child that
        // finished its work can still be reaped before it exits on its own.
        let verification = self.verifier.verify(task_id, Some(&cwd));
        let mut cleanly_completed = code == Some(0) || verification.valid;
        if cleanly_completed && verification.valid {
            let failed = self
                .read_result_json(task_id, &cwd)
                .is_some_and(|rd| rd.get("status").and_then(Value::as_str) == Some("failed"));
            if failed {
                cleanly_completed = false;
            }
        }

        // Killed (reaped or cancelled) before producing a valid result.
        if let Some(reason) = kill_reason.filter(|_| !verification.valid) {
            let result = SubagentResult {
                task_id: task_id.to_string(),
                stdout,
                stderr,
                exit_code: code,
                status: Some(reason.status()),
                used_inherited_model_fallback: Some(task.use_inherited_model_fallback),
                ..Default::default()
            };
            // A killed task used to return here, before the ladder was
            // consulted at all, so a run that died because its model was
            // unreachable never retried on a reachable one. The classifier
            // reads the child's captured stderr, where a provider error lives;
            // `error` stays unset so the parent's message is still hoocode's
            // "subagent <status>".
            if reason.worth_retrying_on_another_model()
                && self.should_retry_with_inherited_model(task, &result)
            {
                agent_log(&format!(
                    "[DISPATCH] agent={} task_id={task_id} {:?} after the preferred model failed; retrying with inherited model",
                    task.agent_type, reason
                ));
                self.record_attempt(
                    task,
                    &result,
                    duration,
                    tokens_generated,
                    peak_context,
                    code,
                    false,
                );
                self.cleanup_retry_artifacts(task);
                self.state().queue.push_front(SubagentPoolTask {
                    use_inherited_model_fallback: true,
                    ..task.clone()
                });
                return true;
            }

            self.record_attempt(
                task,
                &result,
                duration,
                tokens_generated,
                peak_context,
                code,
                false,
            );
            self.write_output_json(task_id, &result);
            let name = match reason {
                KillReason::Stalled => "task_stalled",
                KillReason::Timeout => "task_timeout",
                KillReason::Cancelled => "task_cancelled",
            };
            self.emit(
                name,
                json!({
                    "task_id": task_id,
                    "agent_type": task.agent_type,
                    "duration": duration,
                    "tokens_generated": tokens_generated,
                    "status": reason.status().as_str(),
                    "cause": kill_reason_cause(result.error.as_deref()),
                }),
            );
            self.resolve_waiter(task_id, result);
            return false;
        }

        let mut result = SubagentResult {
            task_id: task_id.to_string(),
            ok: cleanly_completed,
            stdout,
            stderr,
            exit_code: code,
            budget_exceeded: Some(budget_exceeded),
            status: Some(if cleanly_completed {
                ResultStatus::Complete
            } else {
                ResultStatus::Failed
            }),
            used_inherited_model_fallback: Some(task.use_inherited_model_fallback),
            ..Default::default()
        };

        if result.ok {
            if !verification.valid {
                result.ok = false;
                result.error = verification.reason.clone();
                result.status = Some(ResultStatus::Failed);
                self.record_attempt(
                    task,
                    &result,
                    duration,
                    tokens_generated,
                    peak_context,
                    code,
                    false,
                );
                self.write_output_json(task_id, &result);
                self.emit(
                    "task_failed",
                    json!({"task_id": task_id, "agent_type": task.agent_type, "duration": duration, "tokens_generated": tokens_generated, "error": verification.reason}),
                );
                self.resolve_waiter(task_id, result);
                return false;
            }
            result.result_data = self.read_result_json(task_id, &cwd);
            // A partial result is still a success, but the model may resume it:
            // the transcript and the dispatch dir are kept. Dirs are pruned
            // at dispatch time, see `prune_dispatch_dirs`.
            if result
                .result_data
                .as_ref()
                .and_then(|d| d.get("status"))
                .and_then(Value::as_str)
                == Some("partial")
            {
                result.status = Some(ResultStatus::Partial);
            }
            let status = result.status.unwrap_or(ResultStatus::Complete);
            self.record_attempt(
                task,
                &result,
                duration,
                tokens_generated,
                peak_context,
                code,
                verification.valid,
            );
            self.emit(
                "task_done",
                json!({
                    "task_id": task_id,
                    "agent_type": task.agent_type,
                    "duration": duration,
                    "tokens_generated": tokens_generated,
                    "status": status.as_str(),
                    "confidence": result.result_data.as_ref().and_then(|d| d.get("confidence")).cloned().unwrap_or(Value::Null),
                    "files_changed": result.result_data.as_ref().and_then(|d| d.get("files_changed")).cloned().unwrap_or(Value::Null),
                }),
            );
            self.resolve_waiter(task_id, result);
            return false;
        }

        // Failure: keep the dispatch dir; surface the concrete cause.
        result.result_data = self.read_result_json(task_id, &cwd);
        if result.error.is_none() {
            result.error = Some(derive_failure_reason(&result));
        }
        if self.should_retry_with_inherited_model(task, &result) {
            agent_log(&format!(
                "[DISPATCH] agent={} task_id={task_id} preferred model failed; retrying with inherited model",
                task.agent_type
            ));
            self.record_attempt(
                task,
                &result,
                duration,
                tokens_generated,
                peak_context,
                code,
                false,
            );
            self.cleanup_retry_artifacts(task);
            self.state().queue.push_front(SubagentPoolTask {
                use_inherited_model_fallback: true,
                ..task.clone()
            });
            return true;
        }
        self.record_attempt(
            task,
            &result,
            duration,
            tokens_generated,
            peak_context,
            code,
            false,
        );
        self.write_output_json(task_id, &result);
        let error = result
            .error
            .clone()
            .unwrap_or_else(|| format!("Exited with code {}", js_code(code)));
        self.emit(
            "task_failed",
            json!({"task_id": task_id, "agent_type": task.agent_type, "duration": duration, "tokens_generated": tokens_generated, "error": error}),
        );
        self.resolve_waiter(task_id, result);
        false
    }

    /// Waiting on the child failed: retry once, else fail.
    #[allow(clippy::too_many_arguments)]
    fn settle_error(
        self: &Arc<Self>,
        task: &SubagentPoolTask,
        budget: &Arc<Mutex<TokenBudget>>,
        spawned_at: u64,
        is_retry: bool,
        err: std::io::Error,
        stdout: String,
        stderr: String,
    ) -> bool {
        self.state().slots.remove(&task.task_id);
        let (tokens_generated, peak_context) = {
            let mut budget = budget.lock().unwrap_or_else(|e| e.into_inner());
            budget.flush();
            (budget.used(), budget.peak_context())
        };
        if !is_retry {
            self.start_task(task.clone(), true);
            return true;
        }
        let error = err.to_string();
        let result = SubagentResult {
            task_id: task.task_id.clone(),
            stdout,
            stderr,
            error: Some(error.clone()),
            status: Some(ResultStatus::Failed),
            used_inherited_model_fallback: Some(task.use_inherited_model_fallback),
            ..Default::default()
        };
        self.record_attempt(
            task,
            &result,
            now_ms().saturating_sub(spawned_at),
            tokens_generated,
            peak_context,
            None,
            false,
        );
        self.write_output_json(&task.task_id, &result);
        self.emit(
            "task_failed",
            json!({"task_id": task.task_id, "agent_type": task.agent_type, "duration": now_ms().saturating_sub(spawned_at), "tokens_generated": tokens_generated, "error": error}),
        );
        self.resolve_waiter(&task.task_id, result);
        false
    }

    fn should_retry_with_inherited_model(
        &self,
        task: &SubagentPoolTask,
        result: &SubagentResult,
    ) -> bool {
        if task.use_inherited_model_fallback || task.session_file.is_some() {
            return false;
        }
        // Falling back needs a concrete model to fall back *to*. `task.model`
        // may be a model category, which would resolve to the model that
        // just failed.
        if task.inherited_model.as_deref().is_none_or(str::is_empty) {
            return false;
        }
        let Some(def) = self.definition(&task.agent_type) else {
            return false;
        };
        if !matches!(def.source, AgentSource::Builtin | AgentSource::Project) {
            return false;
        }
        if def
            .model
            .as_deref()
            .is_none_or(|m| m.is_empty() || m == MODEL_INHERIT)
        {
            return false;
        }
        is_inherited_model_fallback_error(result)
    }

    fn cleanup_retry_artifacts(&self, task: &SubagentPoolTask) {
        let cwd = task.cwd.clone().unwrap_or_else(|| self.cwd.clone());
        let dir = hoocode_code_paths::dispatch_task_dir(&cwd, &task.task_id);
        let _ = std::fs::remove_file(self.child_session_file(task));
        let _ = std::fs::remove_file(dir.join("result.json"));
        let _ = std::fs::remove_file(dir.join("output.json"));
    }

    /// The model and effort a task runs on. One implementation, shared by
    /// `build_args` and the ledger, so a recorded model can never disagree with
    /// the model the child actually ran on. An inherited-model retry asks for
    /// nothing and does not use the agent's pin.
    fn select_for_task(&self, task: &SubagentPoolTask) -> ModelSelection {
        let fallback = task.use_inherited_model_fallback;
        self.select(
            &task.agent_type,
            ModelRequest {
                ask: if fallback { None } else { task.model.clone() },
                effort: task.effort.clone(),
                pin: None,
                inherited: task.inherited_model.clone().filter(|m| !m.is_empty()),
            },
            !fallback,
        )
    }

    /// The `--model` a task runs on, or `None` when the child resolves its own.
    fn resolve_task_model(&self, task: &SubagentPoolTask) -> Option<String> {
        self.select_for_task(task).model
    }

    /// One [`ledger`] line per attempt, from every terminal path, so the ledger
    /// cannot miss a settle or count one twice. `verified` is the output
    /// verifier's verdict, which is not the same as `result.ok`: a run cut
    /// short by its deadline settles `partial` and is both ok and verified.
    #[allow(clippy::too_many_arguments)]
    fn record_attempt(
        &self,
        task: &SubagentPoolTask,
        result: &SubagentResult,
        duration_ms: u64,
        tokens_generated: u64,
        peak_context: u64,
        exit_code: Option<i32>,
        verified: bool,
    ) {
        let cwd = task.cwd.clone().unwrap_or_else(|| self.cwd.clone());
        let resolved = self.resolve_task_model(task);
        // The child's own verdict wins over the pool's: a run that was SIGTERMed
        // and wrapped up on the way out settles `complete` here (the file is
        // valid) while `result.json` says `partial`. Reporting `partial` keeps
        // "cut short" visible in the stats instead of quietly counting as a
        // finished run.
        let reported = result
            .result_data
            .as_ref()
            .and_then(|data| data.get("status"))
            .and_then(Value::as_str)
            .filter(|status| !status.is_empty());
        let status =
            reported.unwrap_or_else(|| result.status.map(|s| s.as_str()).unwrap_or("failed"));
        let attempt = ledger::attempt_from_result(
            &task.task_id,
            &task.agent_type,
            task.background.unwrap_or(false),
            // The pool's own env, the same one `child_env` stamps children from.
            current_subagent_depth(&self.env) as u8 + 1,
            task.model.as_deref(),
            resolved.as_deref(),
            task.provider.as_deref(),
            task.use_inherited_model_fallback,
            status,
            result.ok,
            verified,
            duration_ms,
            tokens_generated,
            peak_context,
            exit_code,
            result.budget_exceeded,
            result.error.as_deref(),
        );
        let confidence_source = result
            .result_data
            .as_ref()
            .map(|m| Value::Object(m.clone()));
        let mut attempt = ledger::with_confidence(attempt, confidence_source.as_ref());
        attempt.parent_session = task
            .parent_session_file
            .as_deref()
            .and_then(hoocode_code_session::session_id_of_file);
        ledger::append(&cwd, &attempt);
        if let Some(parent) = &task.parent_session_file {
            let child = self.child_session_file(task);
            write_run_record(parent, task, &child, Some(status));
        }
    }

    fn resolve_waiter(&self, task_id: &str, result: SubagentResult) {
        let mut state = self.state();
        let status = match result.status {
            Some(ResultStatus::Stalled) => TaskStatus::Stalled,
            Some(ResultStatus::Timeout) => TaskStatus::Timeout,
            Some(ResultStatus::Cancelled) => TaskStatus::Cancelled,
            _ if result.ok => TaskStatus::Done,
            _ => TaskStatus::Failed,
        };
        state.task_status.insert(task_id.to_string(), status);
        if let Some(waiter) = state.waiters.remove(task_id) {
            drop(state);
            let _ = waiter.send(Ok(result));
            return;
        }
        state.completed.insert(task_id.to_string(), result);
        // Bounded: a finished result carries its captured streams, and nothing
        // used to release them, so a long session grew until it hurt.
        state.completed_order.push_back(task_id.to_string());
        while state.completed_order.len() > self.max_completed {
            let Some(oldest) = state.completed_order.pop_front() else {
                break;
            };
            state.completed.remove(&oldest);
            state.task_status.remove(&oldest);
        }
    }
}

/// One line, for the panel: the cause if there is one, else the status.
fn kill_reason_cause(error: Option<&str>) -> String {
    match error {
        Some(text) if !text.is_empty() => text.split_whitespace().collect::<Vec<_>>().join(" "),
        _ => "killed before it reported".into(),
    }
}

/// `String(code)` for an exit code (`null` when killed by a signal).
fn js_code(code: Option<i32>) -> String {
    code.map_or_else(|| "null".into(), |c| c.to_string())
}

/// The child's result.json summary, else the stderr tail, else the exit code.
fn write_pid_file(cwd: &Path, task_id: &str, pid: u32) {
    let dir = hoocode_code_paths::dispatch_task_dir(cwd, task_id);
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    if let Err(error) = std::fs::write(dir.join("pid"), pid.to_string()) {
        agent_log(&format!(
            "[DISPATCH] cannot write pid for {task_id}: {error}"
        ));
    }
}

fn derive_failure_reason(result: &SubagentResult) -> String {
    let summary = result
        .result_data
        .as_ref()
        .and_then(|d| d.get("summary"))
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    if !summary.is_empty() {
        return summary.to_string();
    }
    // `Note:` lines are benign chatter (e.g. the json-mode approval notice),
    // never the cause of a failure.
    let lines: Vec<&str> = result
        .stderr
        .split('\n')
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("Note:"))
        .collect();
    // Prefer lines that look like the real error over whatever came last.
    let errors: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|l| looks_like_error(l))
        .collect();
    let picked = if errors.is_empty() {
        &lines[..]
    } else {
        &errors[..]
    };
    let tail = picked[picked.len().saturating_sub(5)..].join("\n");
    if !tail.is_empty() {
        return tail;
    }
    format!("Exited with code {}", js_code(result.exit_code))
}

/// A stderr line that names a failure (error, ENOSPC, missing API key, ...).
fn looks_like_error(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    [
        "error",
        "failed",
        "failure",
        "enospc",
        "no api key",
        "panic",
        "unauthorized",
        "forbidden",
        "quota",
        "rate limit",
        "denied",
        "exception",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

/// Dispatch dirs are kept so a model can resume them. Keep the newest `keep`
/// `dispatch-*` dirs; remove the rest once they are older than `max_age`.
///
/// A dir's age is its most recent activity: its own mtime or any direct
/// entry's (a resumed run appends to the old `session.jsonl` without touching
/// the dir's mtime), so a session that is still running is never removed.
pub fn prune_dispatch_dirs(cwd: &Path, keep: usize, max_age: Duration) {
    prune_dispatch_dirs_keeping(cwd, keep, max_age, &HashSet::new());
}

/// [`prune_dispatch_dirs`], never removing the dirs named in `live` (tasks
/// the pool is running or has queued).
pub fn prune_dispatch_dirs_keeping(
    cwd: &Path,
    keep: usize,
    max_age: Duration,
    live: &HashSet<String>,
) {
    prune_dispatch_dirs_checked(cwd, keep, max_age, &|name| live.contains(name));
}

/// [`prune_dispatch_dirs_keeping`] with liveness asked per dir. The listing is
/// a snapshot, so a dir can be resumed or a task queued after it was read; the
/// age and liveness are checked again right before its removal.
fn prune_dispatch_dirs_checked(
    cwd: &Path,
    keep: usize,
    max_age: Duration,
    is_live: &dyn Fn(&str) -> bool,
) {
    let Ok(entries) = std::fs::read_dir(hoocode_code_paths::dispatch_root(cwd)) else {
        return;
    };
    let mut dirs: Vec<(SystemTime, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with("dispatch-"))
        .filter(|e| e.path().is_dir())
        .filter_map(|e| Some((last_activity(&e.path())?, e.path())))
        .collect();
    dirs.sort_by(|a, b| b.0.cmp(&a.0));
    let is_old = |modified: SystemTime| {
        SystemTime::now()
            .duration_since(modified)
            .is_ok_and(|age| age > max_age)
    };
    for (index, (modified, path)) in dirs.into_iter().enumerate() {
        if index < keep || !is_old(modified) {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if is_live(name) {
            continue;
        }
        // Re-check just before removal: the dir may have been resumed (its
        // activity is newer now) or its task started since the listing.
        let fresh = last_activity(&path).is_some_and(is_old);
        if fresh && !is_live(name) {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}

/// Whether a run is still going: this process's pool holds it (running or
/// queued), or its child process is alive, by the pid its dispatch dir records.
/// A run with no settle record and neither is was cut short: the app quit, or
/// the process died before it settled.
pub fn run_is_live(cwd: &Path, task_id: &str) -> bool {
    crate::instance::pool_has_live_task(task_id)
        || crate::lifeguard::has_running_pid(&hoocode_code_paths::dispatch_task_dir(cwd, task_id))
}

/// One line of a run's description: its first line, cut short.
fn describe_task(task: &str) -> String {
    task.lines()
        .next()
        .unwrap_or_default()
        .trim()
        .chars()
        .take(120)
        .collect()
}

/// Append a `subagent_run` record for a run to its parent session. Written
/// twice: at dispatch with no `outcome`, and at settle with the status it
/// settled with. The newest record for a transcript is the run's state. A
/// failure to write is logged and does not fail the run: the record is the
/// parent's view of it, not the run's result.
fn write_run_record(parent: &Path, task: &SubagentPoolTask, child: &Path, outcome: Option<&str>) {
    let data = json!({
        "taskId": task.task_id,
        "agentType": task.agent_type,
        "description": describe_task(&task.task),
        "childFile": child.file_name().map(|n| n.to_string_lossy().into_owned()),
        "outcome": outcome,
    });
    if let Err(err) = append_custom_entry_to_file(parent, SUBAGENT_RUN_ENTRY, data) {
        agent_log(&format!(
            "[SUBAGENT] could not record run {} in its parent session: {err}",
            task.task_id
        ));
    }
}

/// Where a run's transcript is, if it still exists: its copy under the parent
/// session's `subagents/`, else the dispatch dir's (runs from before transcripts
/// moved there). `None` when neither exists.
pub fn run_transcript(cwd: &Path, task_id: &str, parent_session: Option<&Path>) -> Option<PathBuf> {
    // A task id names a file under a directory. One read from a session file
    // must not reach outside it, so only a plain name is a task id.
    let plain = !task_id.is_empty()
        && task_id != "."
        && task_id != ".."
        && !task_id.contains(['/', '\\', '\0']);
    if !plain {
        return None;
    }
    if let Some(parent) = parent_session {
        let file = subagents_dir(parent).join(format!("{task_id}.jsonl"));
        if file.exists() {
            return Some(file);
        }
    }
    let file = hoocode_code_paths::dispatch_task_dir(cwd, task_id).join("session.jsonl");
    file.exists().then_some(file)
}

/// The transcript a resume continues. A transcript the caller named (the
/// picker's row) wins: a resumed run has a new task id, and its record keeps
/// the old transcript's file name, so the id no longer names the file. Else
/// the run's own transcript, by task id.
pub fn resume_transcript(
    cwd: &Path,
    task_id: &str,
    parent_session: Option<&Path>,
    named: Option<&Path>,
) -> Option<PathBuf> {
    match named {
        Some(file) if file.is_file() => Some(file.to_path_buf()),
        _ => run_transcript(cwd, task_id, parent_session),
    }
}

/// Mark a dispatch dir (and its `session.jsonl`, if any) as just used, so the
/// prune treats a session being resumed as recent.
fn touch_dispatch_dir(dir: &Path) {
    let now = SystemTime::now();
    for path in [dir.to_path_buf(), dir.join("session.jsonl")] {
        if let Ok(file) = std::fs::File::open(&path) {
            let _ = file.set_modified(now);
        }
    }
}

/// The newest mtime among a dispatch dir and its direct entries.
fn last_activity(dir: &Path) -> Option<SystemTime> {
    let own = std::fs::metadata(dir).ok()?.modified().ok()?;
    let newest_entry = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|e| e.metadata().ok()?.modified().ok())
        .max();
    Some(newest_entry.map_or(own, |entry| entry.max(own)))
}

/// Dispatch dirs kept at most this many (newest first).
const DISPATCH_KEEP: usize = 50;
/// ...and removed only once older than this.
const DISPATCH_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

#[cfg(test)]
mod prune_recheck_tests {
    use std::cell::Cell;

    use super::*;

    fn old_dir(cwd: &Path, name: &str) -> PathBuf {
        let path = hoocode_code_paths::dispatch_task_dir(cwd, name);
        std::fs::create_dir_all(&path).unwrap();
        let when = SystemTime::now() - Duration::from_secs(3 * 24 * 60 * 60);
        std::fs::File::open(&path)
            .unwrap()
            .set_modified(when)
            .unwrap();
        path
    }

    /// A task that starts after the listing (so the first liveness check says
    /// "not live") must still be kept: the re-check right before removal sees it.
    #[test]
    fn a_task_started_after_the_listing_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = old_dir(dir.path(), "dispatch-race");
        let calls = Cell::new(0);
        prune_dispatch_dirs_checked(dir.path(), 0, DISPATCH_MAX_AGE, &|_| {
            calls.set(calls.get() + 1);
            // First query (the listing pass) says idle; the re-check says live.
            calls.get() > 1
        });
        assert!(
            calls.get() >= 2,
            "liveness must be re-checked before removal"
        );
        assert!(path.exists());
    }

    /// Control: an old idle dir is removed.
    #[test]
    fn an_old_idle_dir_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let path = old_dir(dir.path(), "dispatch-idle");
        prune_dispatch_dirs_checked(dir.path(), 0, DISPATCH_MAX_AGE, &|_| false);
        assert!(!path.exists());
    }
}

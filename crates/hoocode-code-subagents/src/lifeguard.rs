//! `core/lifeguard.ts`: watches running subagent processes for heartbeats and
//! hard timeouts, and sweeps stale dispatch directories.
//!
//! A child must print `{"ping":true}` periodically (any stdout counts, see the
//! pool). A child silent past the load-scaled threshold is reaped and reported
//! `stalled`; one past its hard timeout is reported `timeout`. Under load
//! (several subagents, background MCP tools) both budgets widen, up to a
//! ceiling, so a starved parent does not reap healthy children.
//!
//! **Liveness is two-tier, because a ping is not evidence of progress.** A
//! child blocked inside a provider call keeps its heartbeat timer running, so
//! it looks alive forever: measured on 2026-10-05, a child waiting on a
//! provider that never answered was still running at 95s with an empty ledger,
//! and the only thing that would have ended it was the ten-minute hard
//! deadline. Silence past [`HEARTBEAT_MISS_THRESHOLD_MS`] is therefore one
//! signal, and *no forward progress* past [`PROGRESS_STALL_THRESHOLD_MS`] is
//! the other; both reap the same way. The progress bar is generous because a
//! recorded subagent turn took up to 65s. A child with a tool in flight counts
//! as making progress (a long build is busy work), so only a child parked with
//! no tool running can trip the progress check.
//!
//! **Reaping is SIGTERM, grace, then SIGKILL.** A child that gets SIGTERM
//! writes its partial `result.json` and exits (see `runtime.rs`), and the
//! pool already accepts a valid result from a killed task, so a reap can now
//! return real work. Before this, a SIGKILL on the spot discarded everything
//! the child had done — the October incident, in a different disguise.
//!
//! Deviation: hoocode also hooks the parent's SIGINT/SIGTERM to shut children
//! down gracefully; here the host calls [`SubagentLifeguard::graceful_shutdown`]
//! from its own signal handling, since a library must not take over signals.
//!
//! **Hard deadline: one value for every agent type (2026-10-10, user decision).**
//! [`base_timeout_ms`] returns [`SUBAGENT_DEADLINE_MS`] (2 hours) for every
//! type. This replaces the per-type 5-20 minute table. That table was itself
//! the fix for the recorded runs (`hoobot/.hoocode/dispatch`): `TIMEOUTS_MS`
//! was keyed on names we never shipped, so every agent got the 5-minute
//! default, and four of ten runs died `timeout` at exactly 300s, three of them
//! `code-review` holding 220-297s of finished work. Long runs are now the
//! parent's to manage: it polls with `AgentOutput` and can stop one with
//! `AgentOutput(cancel: true)`. Hung children are still reaped early by the
//! stall checks above, so the long deadline costs only real work.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use tokio::task::JoinHandle;

use crate::agent_log::agent_log;

/// Hard deadline for every subagent, whatever its type.
///
/// Decision of 2026-10-10 (user): every type may run for up to 2 hours. The
/// parent polls with `AgentOutput` and stops a run it no longer needs with
/// `AgentOutput(cancel: true)`.
pub const SUBAGENT_DEADLINE_MS: u64 = 2 * 60 * 60 * 1000;

/// Most the load multiplier may add to a deadline: 30 minutes.
const LOAD_HEADROOM_MS: u64 = 30 * 60 * 1000;

/// The longest any run's budget can get under load: the base plus
/// [`LOAD_HEADROOM_MS`], so 2h30m. `AgentOutput` derives its reconcile age
/// from this.
pub const MAX_SCALED_DEADLINE_MS: u64 = SUBAGENT_DEADLINE_MS + LOAD_HEADROOM_MS;

/// Base hard deadline for an agent type: [`SUBAGENT_DEADLINE_MS`] for all of
/// them (2026-10-10). The argument is kept so callers and plugin agents need no
/// change.
///
/// Wall-clock, before the load multiplier ([`SubagentLifeguard::set_external_load`]
/// and the pool's own concurrency both widen it). Under load the budget never
/// passes [`MAX_SCALED_DEADLINE_MS`]; see [`load_ceiling_ms`].
pub fn base_timeout_ms(_agent_type: &str) -> u64 {
    SUBAGENT_DEADLINE_MS
}

const HEARTBEAT_MISS_THRESHOLD_MS: u64 = 60_000;
/// No `turn_end` / tool event for this long is a stall even if the child is
/// still writing heartbeats. Ten minutes: a long thinking turn or a slow
/// provider can legitimately take minutes, and at 150s (the old value) those
/// runs were reaped mid-turn. It still catches a child parked inside a
/// provider call, which no heartbeat ever will.
pub const PROGRESS_STALL_THRESHOLD_MS: u64 = 600_000;
/// Environment override for [`PROGRESS_STALL_THRESHOLD_MS`], read by the
/// parent from its pool environment (the lifeguard runs in the parent). The
/// evals use it to make a stall reap take seconds instead of ten minutes.
pub const STALL_THRESHOLD_ENV: &str = "HOOCODE_SUBAGENT_STALL_MS";

/// The progress stall threshold for a pool: [`STALL_THRESHOLD_ENV`] when it is
/// a positive integer, else [`PROGRESS_STALL_THRESHOLD_MS`].
pub fn progress_stall_ms_from_env(env: &HashMap<String, String>) -> u64 {
    env.get(STALL_THRESHOLD_ENV)
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|ms| *ms > 0)
        .unwrap_or(PROGRESS_STALL_THRESHOLD_MS)
}
/// A tool in flight counts as busy work only this long without any progress
/// event (before the load multiplier). A tool that declares its own timeout
/// (the Shell tool) gets `max(this, declared + TOOL_TIMEOUT_MARGIN_MS)`
/// instead; see [`ToolLimit`]. This is the bound for a tool that declares none.
pub const TOOL_IN_FLIGHT_STALL_MS: u64 = 15 * 60 * 1000;
/// Slack added to a tool's declared timeout before it counts as wedged: the
/// tool kills itself at the declared time, and its end event follows.
pub const TOOL_TIMEOUT_MARGIN_MS: u64 = 60_000;

/// What a running tool declares about its own run time, from its start event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolLimit {
    /// The tool declares no limit: [`TOOL_IN_FLIGHT_STALL_MS`] (load-scaled) applies.
    Undeclared,
    /// The tool's own timeout, in milliseconds.
    Declared(u64),
    /// The tool runs without a limit (a declared timeout of 0). It is never
    /// reaped for lack of progress while it runs; the heartbeat check still applies.
    Unlimited,
}

/// The Shell tool's default timeout in a subagent. Mirrors
/// `DEFAULT_BASH_TIMEOUT_SECS` in `hoocode-code-tool-bash`, which this crate
/// does not depend on.
pub const SUBAGENT_SHELL_DEFAULT_TIMEOUT_SECS: f64 = 600.0;
/// The env override for that default. Mirrors `DEFAULT_BASH_TIMEOUT_ENV` in
/// `hoocode-code-tool-bash`. Children inherit the parent's environment.
pub const SHELL_DEFAULT_TIMEOUT_ENV: &str = "HOOCODE_BASH_DEFAULT_TIMEOUT_SECS";

/// What a tool declares about its run time, from its `tool_execution_start`
/// event (`toolName`, `args`). Only the Shell tool declares one; its `timeout`
/// follows the tool's own rules, as a subagent: `0` is unlimited, a positive
/// value is the limit, and an omitted, negative or non-finite value is the
/// subagent default (the env override when positive, else 600s).
pub fn tool_limit_from_start(
    tool_name: &str,
    args: Option<&Value>,
    env: &HashMap<String, String>,
) -> ToolLimit {
    if !matches!(tool_name, "Shell" | "Bash") {
        return ToolLimit::Undeclared;
    }
    let requested = args.and_then(|a| a.get("timeout")).and_then(Value::as_f64);
    match requested {
        Some(0.0) => ToolLimit::Unlimited,
        Some(t) if t.is_finite() && t > 0.0 => ToolLimit::Declared(secs_to_ms(t)),
        _ => ToolLimit::Declared(secs_to_ms(shell_default_timeout_secs(env))),
    }
}

fn shell_default_timeout_secs(env: &HashMap<String, String>) -> f64 {
    env.get(SHELL_DEFAULT_TIMEOUT_ENV)
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|t| t.is_finite() && *t > 0.0)
        .unwrap_or(SUBAGENT_SHELL_DEFAULT_TIMEOUT_SECS)
}

/// Seconds to whole milliseconds; the float-to-int cast saturates.
fn secs_to_ms(secs: f64) -> u64 {
    (secs * 1000.0) as u64
}
/// How long a SIGTERM'd child may take to write its result and exit before the
/// group is killed outright.
pub const STALL_TERM_GRACE_MS: u64 = 30_000;
const HEARTBEAT_CHECK_INTERVAL_MS: u64 = 5_000;
const PARENT_SHUTDOWN_GRACE_MS: u64 = 5_000;
/// Each additional concurrent task adds this fraction to both budgets.
const LOAD_TOLERANCE_PER_PROCESS: f64 = 0.5;
/// Ceiling on the load multiplier: a stuck child is still reaped eventually.
const MAX_LOAD_MULTIPLIER: f64 = 4.0;

/// The most a run's hard deadline may reach under load: 4x the base, capped at
/// the base plus [`LOAD_HEADROOM_MS`]. Uncapped, 4x a 2-hour base is 8 hours,
/// which is why the multiplier is capped here and not on the stall budgets.
fn load_ceiling_ms(base: u64) -> u64 {
    ((base as f64 * MAX_LOAD_MULTIPLIER) as u64).min(base + LOAD_HEADROOM_MS)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Kill a process group (the pool spawns children as group leaders), falling
/// back to the single process (`killProcessTree`).
pub fn kill_process_tree(pid: u32) {
    #[cfg(unix)]
    {
        let pid = pid as libc::pid_t;
        // SAFETY: plain kill(2) calls.
        unsafe {
            if libc::kill(-pid, libc::SIGKILL) != 0 {
                libc::kill(pid, libc::SIGKILL);
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .spawn();
    }
}

/// Why the lifeguard is stopping a child. The child reads the signal to tell
/// the two apart, so a stall is never reported as a run that ran out of time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReapReason {
    /// No heartbeat, no progress, or over the memory limit: SIGUSR1.
    Stalled,
    /// The hard deadline passed: SIGTERM.
    Deadline,
}

/// Ask a reaped child to stop so it writes its partial result.
///
/// - [`ReapReason::Stalled`]: SIGUSR1 to the child's pid **only**. The child
///   wraps up and exits; its grandchildren are not touched here (see
///   [`sweep_group`], which runs once the child has exited).
/// - [`ReapReason::Deadline`]: SIGTERM to the whole group.
///
/// A pid of 0 means there is no process, so nothing is sent.
#[cfg(unix)]
fn reap_group(pid: u32, reason: ReapReason) {
    // A pid of 0 means there is no process — an in-process runner, or a spawn
    // that never produced one. `kill(-0, …)` would signal *every* process in
    // the caller's own group, which is the parent agent in the worst case, so
    // the guard `kill_tree` has belongs here too.
    if pid == 0 {
        return;
    }
    // SAFETY: plain kill(2) calls.
    unsafe {
        match reason {
            ReapReason::Stalled => {
                libc::kill(pid as libc::pid_t, libc::SIGUSR1);
            }
            ReapReason::Deadline => {
                libc::kill(-(pid as libc::pid_t), libc::SIGTERM);
            }
        }
    }
}

#[cfg(not(unix))]
fn reap_group(pid: u32, _reason: ReapReason) {
    if pid > 0 {
        kill_process_tree(pid);
    }
}

/// After a reaped child has exited: SIGTERM whatever is left in its process
/// group now, then SIGKILL the group after `grace_ms` if anything survives.
/// The pgid is the child's pid (the pool spawns children as group leaders).
///
/// The leader is already reaped, so an empty group makes both `kill` calls
/// fail with ESRCH, which is the normal case and harmless.
#[cfg(unix)]
pub(crate) fn sweep_group(pgid: u32, grace_ms: u64) {
    if pgid == 0 {
        return;
    }
    // The pid is reused only after it is reaped and the kernel wraps around,
    // and the grace timer below fires later still. A pgid whose leader is gone
    // is still a group while any member lives, so the timer's `kill(group, 0)`
    // and SIGKILL target the original group unless its last member exited and
    // the id was reassigned within `grace_ms`. That window is accepted: the
    // pool only sweeps children it reaped itself, and SIGKILL is the last resort.
    let group = -(pgid as libc::pid_t);
    // SAFETY: plain kill(2) calls on a process group.
    unsafe {
        libc::kill(group, libc::SIGTERM);
    }
    hoocode_runtime::spawn_bg(async move {
        tokio::time::sleep(Duration::from_millis(grace_ms)).await;
        // SAFETY: signal 0 only checks that the group still has members.
        if unsafe { libc::kill(group, 0) } == 0 {
            agent_log(&format!(
                "[LIFEGUARD] process group {pgid} still alive {grace_ms}ms after its child exited; killing"
            ));
            // SAFETY: plain kill(2) call on the same group.
            unsafe {
                libc::kill(group, libc::SIGKILL);
            }
        }
    });
}

/// No-op off unix. The reap has already killed the tree, and the pid may have
/// been reused after `wait`, so signalling it now could hit an unrelated process.
#[cfg(not(unix))]
pub(crate) fn sweep_group(_pid: u32, _grace_ms: u64) {}

#[cfg(unix)]
fn terminate_group(pid: u32) {
    // Graceful shutdown: the parent is exiting, so every child gets SIGTERM.
    reap_group(pid, ReapReason::Deadline);
}

#[cfg(not(unix))]
fn terminate_group(pid: u32) {
    if pid > 0 {
        kill_process_tree(pid);
    }
}

/// What the lifeguard reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifeguardEvent {
    Stalled { task_id: String, pid: u32 },
    Timeout { task_id: String, pid: u32 },
}

type Listener = Arc<dyn Fn(&LifeguardEvent) + Send + Sync>;

#[derive(Debug, Clone)]
struct Monitored {
    pid: u32,
    agent_type: String,
}

#[derive(Default)]
struct State {
    processes: HashMap<String, Monitored>,
    last_heartbeat: HashMap<String, u64>,
    timeouts: HashMap<String, JoinHandle<()>>,
    started_at: HashMap<String, u64>,
    base_timeout_ms: HashMap<String, u64>,
    last_check_at: u64,
    external_load: u64,
    /// Last forward progress (a turn or tool event), per task. Unlike
    /// `last_heartbeat` this does not move on a ping.
    last_progress: HashMap<String, u64>,
    /// Tools currently running per task, keyed by tool call id, with what each
    /// declared. A task with any entry is busy, so the progress check uses the
    /// tool bound instead of the idle threshold.
    tools_in_flight: HashMap<String, HashMap<String, ToolLimit>>,
    /// Reaped (stop signal sent) but not yet exited: not re-reported each tick,
    /// and the grace timer that escalates to SIGKILL is held here.
    reaping: HashSet<String>,
    grace_timers: HashMap<String, JoinHandle<()>>,
    /// The pid of each reaped child. On exit its process group is swept, so
    /// grandchildren that ignored the stop signal are not orphaned (a stall
    /// signal goes to the child alone).
    reaped: HashMap<String, u32>,
    /// The no-progress threshold (before the load multiplier).
    progress_stall_ms: u64,
    disposed: bool,
}

impl State {
    fn load_multiplier(&self) -> f64 {
        let concurrent = self.processes.len() as u64 + self.external_load;
        let mult = 1.0 + (concurrent.saturating_sub(1)) as f64 * LOAD_TOLERANCE_PER_PROCESS;
        mult.min(MAX_LOAD_MULTIPLIER)
    }

    /// The no-progress bound while a task's tools run, before the loop-lag
    /// allowance: the larger of the load-scaled [`TOOL_IN_FLIGHT_STALL_MS`] and
    /// each declared timeout plus [`TOOL_TIMEOUT_MARGIN_MS`]. `None` when any
    /// running tool is unlimited, so the progress check cannot fire.
    fn tool_in_flight_bound_ms(&self, task_id: &str) -> Option<f64> {
        let tools = self.tools_in_flight.get(task_id)?;
        let mut bound = TOOL_IN_FLIGHT_STALL_MS as f64 * self.load_multiplier();
        for limit in tools.values() {
            match limit {
                ToolLimit::Unlimited => return None,
                ToolLimit::Declared(ms) => {
                    bound = bound.max(ms.saturating_add(TOOL_TIMEOUT_MARGIN_MS) as f64);
                }
                ToolLimit::Undeclared => {}
            }
        }
        Some(bound)
    }
}

/// `SubagentLifeguard`.
pub struct SubagentLifeguard {
    state: Mutex<State>,
    /// SIGTERM-to-SIGKILL grace, overridable so tests need not wait 30s.
    term_grace_ms: std::sync::atomic::AtomicU64,
    listeners: Mutex<Vec<Listener>>,
    check_task: Mutex<Option<JoinHandle<()>>>,
}

impl SubagentLifeguard {
    /// Sweep stale dispatch dirs under `cwd` and start the heartbeat check.
    /// Must run inside a tokio runtime.
    pub fn new(cwd: impl AsRef<Path>) -> Arc<Self> {
        let swept = cwd.as_ref().to_path_buf();
        hoocode_runtime::spawn_bg(async move { sweep_old_agents(&swept) });
        let guard = Arc::new(Self {
            state: Mutex::new(State {
                last_check_at: now_ms(),
                progress_stall_ms: PROGRESS_STALL_THRESHOLD_MS,
                ..Default::default()
            }),
            term_grace_ms: std::sync::atomic::AtomicU64::new(STALL_TERM_GRACE_MS),
            listeners: Mutex::new(Vec::new()),
            check_task: Mutex::new(None),
        });
        let weak = Arc::downgrade(&guard);
        let task = tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(Duration::from_millis(HEARTBEAT_CHECK_INTERVAL_MS));
            interval.tick().await;
            loop {
                interval.tick().await;
                let Some(guard) = weak.upgrade() else { break };
                guard.check_heartbeats();
            }
        });
        *guard.check_task.lock().unwrap_or_else(|e| e.into_inner()) = Some(task);
        guard
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Listen for `stalled` / `timeout`.
    pub fn on_event(&self, listener: impl Fn(&LifeguardEvent) + Send + Sync + 'static) {
        self.listeners
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(Arc::new(listener));
    }

    fn emit(&self, event: LifeguardEvent) {
        let listeners = self
            .listeners
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        for listener in listeners {
            listener(&event);
        }
    }

    /// Set the no-progress stall threshold (before the load multiplier). The
    /// pool sets it from [`STALL_THRESHOLD_ENV`].
    pub fn set_progress_stall_ms(&self, ms: u64) {
        self.state().progress_stall_ms = ms.max(1);
    }

    /// Count of external in-process tasks (background MCP tools) sharing the
    /// parent; widens the budgets like extra subagents. Negative is 0.
    pub fn set_external_load(&self, count: i64) {
        self.state().external_load = count.max(0) as u64;
    }

    /// Start monitoring a child. The caller reports its exit with
    /// [`untrack`](Self::untrack).
    pub fn monitor(self: &Arc<Self>, task_id: &str, agent_type: &str, pid: u32) {
        let mut state = self.state();
        if state.disposed {
            return;
        }
        let now = now_ms();
        state.processes.insert(
            task_id.to_string(),
            Monitored {
                pid,
                agent_type: agent_type.to_string(),
            },
        );
        state.last_heartbeat.insert(task_id.to_string(), now);
        state.last_progress.insert(task_id.to_string(), now);
        let base = base_timeout_ms(agent_type);
        state.started_at.insert(task_id.to_string(), now);
        state.base_timeout_ms.insert(task_id.to_string(), base);
        let delay =
            ((base as f64 * state.load_multiplier()).round() as u64).min(load_ceiling_ms(base));
        let handle = self.arm_timeout(task_id, delay);
        if let Some(old) = state.timeouts.insert(task_id.to_string(), handle) {
            old.abort();
        }
    }

    fn arm_timeout(self: &Arc<Self>, task_id: &str, delay_ms: u64) -> JoinHandle<()> {
        let weak: Weak<Self> = Arc::downgrade(self);
        let task_id = task_id.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            if let Some(guard) = weak.upgrade() {
                guard.handle_timeout(&task_id);
            }
        })
    }

    /// Record a heartbeat for a monitored task.
    pub fn record_heartbeat(&self, task_id: &str) {
        let mut state = self.state();
        if state.processes.contains_key(task_id) {
            state.last_heartbeat.insert(task_id.to_string(), now_ms());
        }
    }

    /// Record forward progress: a turn ended or a tool started or finished.
    /// Unlike a heartbeat this cannot be produced by a timer alone, so it is
    /// what the progress stall threshold watches.
    pub fn record_progress(&self, task_id: &str) {
        let mut state = self.state();
        if state.processes.contains_key(task_id) {
            state.last_progress.insert(task_id.to_string(), now_ms());
        }
    }

    /// The last heartbeat (epoch ms), if monitored.
    pub fn last_heartbeat_at(&self, task_id: &str) -> Option<u64> {
        self.state().last_heartbeat.get(task_id).copied()
    }

    pub fn is_monitoring(&self, task_id: &str) -> bool {
        self.state().processes.contains_key(task_id)
    }

    /// A tool started in a monitored child. Counts as progress, and keeps the
    /// child from being reaped as idle until the matching [`record_tool_end`].
    /// `call_id` is the tool call id; `limit` is what the tool declared.
    pub fn record_tool_start(&self, task_id: &str, call_id: &str, limit: ToolLimit) {
        let mut state = self.state();
        if state.processes.contains_key(task_id) {
            state
                .tools_in_flight
                .entry(task_id.to_string())
                .or_default()
                .insert(call_id.to_string(), limit);
            state.last_progress.insert(task_id.to_string(), now_ms());
        }
    }

    /// A tool finished in a monitored child. Counts as progress. An end with no
    /// matching start is harmless.
    pub fn record_tool_end(&self, task_id: &str, call_id: &str) {
        let mut state = self.state();
        if state.processes.contains_key(task_id) {
            if let Some(tools) = state.tools_in_flight.get_mut(task_id) {
                tools.remove(call_id);
                if tools.is_empty() {
                    state.tools_in_flight.remove(task_id);
                }
            }
            state.last_progress.insert(task_id.to_string(), now_ms());
        }
    }

    /// Test hook: backdate a task's last heartbeat.
    #[doc(hidden)]
    pub fn set_last_heartbeat_for_testing(&self, task_id: &str, at_ms: u64) {
        self.state()
            .last_heartbeat
            .insert(task_id.to_string(), at_ms);
    }

    /// Test hook: backdate a task's last forward progress.
    #[doc(hidden)]
    pub fn set_last_progress_for_testing(&self, task_id: &str, at_ms: u64) {
        self.state()
            .last_progress
            .insert(task_id.to_string(), at_ms);
    }

    /// Test hook: shorten the SIGTERM grace so the escalation path is testable.
    #[doc(hidden)]
    pub fn set_term_grace_for_testing(&self, ms: u64) {
        self.term_grace_ms
            .store(ms, std::sync::atomic::Ordering::Relaxed);
    }

    /// Test hook: backdate the last heartbeat check (event-loop lag).
    #[doc(hidden)]
    pub fn set_last_check_for_testing(&self, at_ms: u64) {
        self.state().last_check_at = at_ms;
    }

    /// Test hook: replace a task's hard timeout with a short one.
    #[doc(hidden)]
    pub fn set_timeout_for_testing(self: &Arc<Self>, task_id: &str, delay: Duration) {
        let handle = self.arm_timeout(task_id, delay.as_millis() as u64);
        if let Some(old) = self.state().timeouts.insert(task_id.to_string(), handle) {
            old.abort();
        }
    }

    /// Test hook: deliver an event as if the lifeguard had decided it (no kill).
    #[doc(hidden)]
    pub fn inject_event_for_testing(&self, event: LifeguardEvent) {
        self.emit(event);
    }

    /// Reap every task that is silent past the load-scaled threshold, or that
    /// has made no forward progress past its own (run every 5s).
    pub fn check_heartbeats(self: &Arc<Self>) {
        let now = now_ms();
        let stalled: Vec<String> = {
            let mut state = self.state();
            // Forgive the parent's own starvation (the check ran late).
            let loop_lag = now
                .saturating_sub(state.last_check_at)
                .saturating_sub(HEARTBEAT_CHECK_INTERVAL_MS);
            state.last_check_at = now;
            let threshold =
                HEARTBEAT_MISS_THRESHOLD_MS as f64 * state.load_multiplier() + loop_lag as f64;
            let progress_threshold =
                state.progress_stall_ms as f64 * state.load_multiplier() + loop_lag as f64;
            state
                .processes
                .keys()
                .filter(|id| !state.reaping.contains(*id))
                .filter(|id| {
                    let silent = state
                        .last_heartbeat
                        .get(*id)
                        .is_some_and(|last| now.saturating_sub(*last) as f64 > threshold);
                    // A pinging child that has not finished a turn in minutes
                    // and has no tool running is parked, not busy. A tool in
                    // flight is busy work up to its bound (see
                    // `tool_in_flight_bound_ms`); an unlimited tool never trips
                    // this check. The heartbeat check above still applies.
                    let limit = match state.tool_in_flight_bound_ms(id) {
                        Some(bound) => bound + loop_lag as f64,
                        None if state.tools_in_flight.contains_key(*id) => f64::INFINITY,
                        None => progress_threshold,
                    };
                    let idle = state
                        .last_progress
                        .get(*id)
                        .is_some_and(|last| now.saturating_sub(*last) as f64 > limit);
                    silent || idle
                })
                .cloned()
                .collect()
        };
        // A child over its memory budget is reaped through the stall path:
        // SIGTERM, grace, then SIGKILL, and its partial result is kept.
        for task_id in stalled.into_iter().chain(self.children_over_memory_limit()) {
            self.handle_stalled(&task_id);
        }
    }

    /// Children whose resident memory is above
    /// [`hoocode_runtime::CHILD_RSS_LIMIT_BYTES`] (2 GiB each).
    fn children_over_memory_limit(&self) -> Vec<String> {
        let candidates: Vec<(String, u32)> = {
            let state = self.state();
            state
                .processes
                .iter()
                .filter(|(id, _)| !state.reaping.contains(*id))
                .map(|(id, monitored)| (id.clone(), monitored.pid))
                .collect()
        };
        candidates
            .into_iter()
            .filter(|(task_id, pid)| {
                let Some(rss) = hoocode_runtime::child_rss_bytes(*pid) else {
                    return false;
                };
                if rss <= hoocode_runtime::CHILD_RSS_LIMIT_BYTES {
                    return false;
                }
                agent_log(&format!(
                    "[LIFEGUARD] task_id={task_id} rss_mb={} above the {} MiB child limit; reaping",
                    rss / hoocode_runtime::MIB,
                    hoocode_runtime::CHILD_RSS_LIMIT_BYTES / hoocode_runtime::MIB,
                ));
                true
            })
            .map(|(task_id, _)| task_id)
            .collect()
    }

    fn handle_stalled(self: &Arc<Self>, task_id: &str) {
        let (monitored, line) = {
            let mut state = self.state();
            let Some(monitored) = state.processes.get(task_id).cloned() else {
                return;
            };
            if !state.reaping.insert(task_id.to_string()) {
                return;
            }
            let silent = state
                .last_heartbeat
                .get(task_id)
                .map_or(-1, |last| now_ms().saturating_sub(*last) as i64);
            let idle = state
                .last_progress
                .get(task_id)
                .map_or(-1, |last| now_ms().saturating_sub(*last) as i64);
            let line = format!(
                "[LIFEGUARD] stalled task_id={task_id} agent={} silent_ms={silent} no_progress_ms={idle} concurrent={} load_mult={:.2} base_threshold_ms={HEARTBEAT_MISS_THRESHOLD_MS} progress_threshold_ms={}",
                monitored.agent_type,
                state.processes.len(),
                state.load_multiplier(),
                state.progress_stall_ms,
            );
            (monitored, line)
        };
        agent_log(&line);
        // Stop signal first: the child writes its partial result and exits, and
        // the pool accepts a valid result from a reaped task, so the work
        // survives. The grace timer is the backstop for a child too wedged to
        // catch a signal at all.
        self.terminate_then_kill(task_id, monitored.pid, ReapReason::Stalled);
        self.emit(LifeguardEvent::Stalled {
            task_id: task_id.to_string(),
            pid: monitored.pid,
        });
    }

    /// Send the stop signal (see [`ReapReason`]), then SIGKILL the child's
    /// process group if the child has not left within the grace. The child's
    /// exit cancels this timer and sweeps the group via
    /// [`untrack`](Self::untrack).
    fn terminate_then_kill(self: &Arc<Self>, task_id: &str, pid: u32, reason: ReapReason) {
        // Record the reap before signalling: a child that exits as soon as it
        // is signalled must still find its entry, or its group is never swept.
        self.state().reaped.insert(task_id.to_string(), pid);
        reap_group(pid, reason);
        let weak = Arc::downgrade(self);
        let owned = task_id.to_string();
        let grace_ms = self
            .term_grace_ms
            .load(std::sync::atomic::Ordering::Relaxed);
        let handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(grace_ms)).await;
            let Some(guard) = weak.upgrade() else { return };
            let still_running = guard
                .state()
                .processes
                .get(&owned)
                .is_some_and(|monitored| monitored.pid == pid);
            if !still_running {
                return;
            }
            agent_log(&format!(
                "[LIFEGUARD] task_id={owned} did not exit within {grace_ms}ms of SIGTERM; killing"
            ));
            kill_tree(pid);
        });
        if let Some(old) = self
            .state()
            .grace_timers
            .insert(task_id.to_string(), handle)
        {
            old.abort();
        }
    }

    fn handle_timeout(self: &Arc<Self>, task_id: &str) {
        let monitored = {
            let mut state = self.state();
            let Some(monitored) = state.processes.get(task_id).cloned() else {
                return;
            };
            if state.reaping.contains(task_id) {
                return;
            }
            // Under load, re-arm rather than kill, up to load_ceiling_ms(base).
            let now = now_ms();
            let started = state.started_at.get(task_id).copied().unwrap_or(now);
            let base = state
                .base_timeout_ms
                .get(task_id)
                .copied()
                .unwrap_or_else(|| base_timeout_ms(&monitored.agent_type));
            let elapsed = now.saturating_sub(started);
            let ceiling = load_ceiling_ms(base);
            let mult = state.load_multiplier();
            if mult > 1.0 && elapsed < ceiling {
                let remaining = ceiling - elapsed;
                let next = ((base as f64 * mult).round() as u64)
                    .min(HEARTBEAT_CHECK_INTERVAL_MS.max(remaining));
                drop(state);
                let handle = self.arm_timeout(task_id, next);
                self.state().timeouts.insert(task_id.to_string(), handle);
                return;
            }
            state.reaping.insert(task_id.to_string());
            state.timeouts.remove(task_id);
            monitored
        };
        // SIGTERM with grace, not an instant SIGKILL: the child writes the
        // partial result it already has, as it does for a stall.
        self.terminate_then_kill(task_id, monitored.pid, ReapReason::Deadline);
        self.emit(LifeguardEvent::Timeout {
            task_id: task_id.to_string(),
            pid: monitored.pid,
        });
    }

    /// The child exited: stop monitoring it.
    pub fn untrack(&self, task_id: &str) {
        let mut state = self.state();
        if let Some(timeout) = state.timeouts.remove(task_id) {
            timeout.abort();
        }
        // The child left: nothing left to escalate against for the child
        // itself. A reaped child's group may still hold grandchildren, so it is
        // swept (SIGTERM now, SIGKILL after the grace) rather than dropped.
        if let Some(grace) = state.grace_timers.remove(task_id) {
            grace.abort();
        }
        if let Some(pid) = state.reaped.remove(task_id) {
            let grace = self
                .term_grace_ms
                .load(std::sync::atomic::Ordering::Relaxed);
            sweep_group(pid, grace);
        }
        state.processes.remove(task_id);
        state.last_heartbeat.remove(task_id);
        state.last_progress.remove(task_id);
        state.tools_in_flight.remove(task_id);
        state.started_at.remove(task_id);
        state.base_timeout_ms.remove(task_id);
        state.reaping.remove(task_id);
    }

    /// The parent is shutting down: SIGTERM every child's process group, then
    /// kill the trees after a grace period.
    pub fn graceful_shutdown(self: &Arc<Self>) {
        let pids: Vec<u32> = self.state().processes.values().map(|m| m.pid).collect();
        for pid in pids.iter().filter(|p| **p > 0) {
            terminate_group(*pid);
        }
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(PARENT_SHUTDOWN_GRACE_MS)).await;
            if let Some(guard) = weak.upgrade() {
                let pids: Vec<u32> = guard.state().processes.values().map(|m| m.pid).collect();
                for pid in pids {
                    kill_tree(pid);
                }
            }
        });
    }

    /// Kill all monitored processes and stop monitoring.
    pub fn dispose(&self) {
        let pids: Vec<u32> = {
            let mut state = self.state();
            if state.disposed {
                return;
            }
            state.disposed = true;
            for (_, timeout) in state.timeouts.drain() {
                timeout.abort();
            }
            for (_, grace) in state.grace_timers.drain() {
                grace.abort();
            }
            let pids = state.processes.values().map(|m| m.pid).collect();
            state.processes.clear();
            state.last_heartbeat.clear();
            state.last_progress.clear();
            state.tools_in_flight.clear();
            state.started_at.clear();
            state.base_timeout_ms.clear();
            state.reaping.clear();
            state.reaped.clear();
            pids
        };
        if let Some(task) = self
            .check_task
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            task.abort();
        }
        for pid in pids {
            kill_tree(pid);
        }
        self.listeners
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }
}

impl Drop for SubagentLifeguard {
    fn drop(&mut self) {
        if let Some(task) = self
            .check_task
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            task.abort();
        }
    }
}

/// A pid of 0 means the spawn never produced a process: nothing to kill.
fn kill_tree(pid: u32) {
    if pid > 0 {
        kill_process_tree(pid);
    }
}

/// Remove dispatch dirs older than 24 hours whose `pid` file names no live
/// process.
fn sweep_old_agents(cwd: &Path) {
    let dispatch_dir = hoocode_code_paths::dispatch_root(cwd);
    let Ok(entries) = std::fs::read_dir(&dispatch_dir) else {
        return;
    };
    let cutoff = Duration::from_secs(24 * 60 * 60);
    for entry in entries.flatten() {
        let path: PathBuf = entry.path();
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if !meta.is_dir() {
            continue;
        }
        let old = meta
            .modified()
            .ok()
            .and_then(|m| m.elapsed().ok())
            .is_some_and(|age| age > cutoff);
        if old && !has_running_pid(&path) {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

fn has_running_pid(dir: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(dir.join("pid")) else {
        return false;
    };
    let digits: String = text
        .trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    let Ok(pid) = digits.parse::<i64>() else {
        return false;
    };
    #[cfg(unix)]
    {
        // SAFETY: signal 0 only checks that the process exists.
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

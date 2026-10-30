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
//! The base is [`SUBAGENT_DEADLINE_MS`] (4 hours) unless the pool sets another
//! through [`SubagentLimits`](crate::limits::SubagentLimits) (the
//! `subagentDeadlineMinutes` setting or `HOOCODE_SUBAGENT_DEADLINE_MS`), and it
//! is the same for every type. This replaces the per-type 5-20 minute table. That table was itself
//! the fix for the recorded runs (`hoobot/.hoocode/dispatch`): `TIMEOUTS_MS`
//! was keyed on names we never shipped, so every agent got the 5-minute
//! default, and four of ten runs died `timeout` at exactly 300s, three of them
//! `code-review` holding 220-297s of finished work. Long runs are now the
//! parent's to manage: it polls with `AgentOutput` and can stop one with
//! `AgentOutput(cancel: true)`. Hung children are still reaped early by the
//! stall checks above, so the long deadline costs only real work.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use tokio::task::JoinHandle;

use crate::agent_log::agent_log;

/// Default hard deadline for every subagent, whatever its type.
///
/// Decision of 2026-10-10 (user): every type may run for up to 2 hours. 2026-10-16:
/// 4 hours. The parent polls with `AgentOutput` and stops a run it no longer
/// needs with `AgentOutput(cancel: true)`. The pool uses the configured value,
/// which defaults to this one.
pub const SUBAGENT_DEADLINE_MS: u64 = 4 * 60 * 60 * 1000;

/// Most the load multiplier may add to a deadline: 30 minutes.
const LOAD_HEADROOM_MS: u64 = 30 * 60 * 1000;

/// The longest the default run budget can get under load: the default base
/// plus [`LOAD_HEADROOM_MS`], so 4h30m. A pool with a different base uses
/// [`max_run_budget_ms`].
pub const MAX_SCALED_DEADLINE_MS: u64 = SUBAGENT_DEADLINE_MS + LOAD_HEADROOM_MS;

/// The longest a run's budget can get under load for a base deadline: the
/// base widened by the load multiplier, capped at the base plus
/// [`LOAD_HEADROOM_MS`]. `AgentOutput` derives its reconcile age from this.
pub fn max_run_budget_ms(base: u64) -> u64 {
    load_ceiling_ms(base)
}

const HEARTBEAT_MISS_THRESHOLD_MS: u64 = 60_000;
/// No `turn_end` / tool event for this long is a stall even if the child is
/// still writing heartbeats. Ten minutes: a long thinking turn or a slow
/// provider can legitimately take minutes, and at 150s (the old value) those
/// runs were reaped mid-turn. It still catches a child parked inside a
/// provider call, which no heartbeat ever will.
///
/// 2026-10-16: 20 minutes (was 10). The `subagentStallSeconds` setting and
/// `HOOCODE_SUBAGENT_STALL_MS` override it; see [`crate::limits`].
pub const PROGRESS_STALL_THRESHOLD_MS: u64 = 1_200_000;
/// Environment override for the progress stall threshold, in milliseconds.
/// The evals use it to make a stall reap take seconds instead of minutes.
/// Read by [`crate::limits::SubagentLimits::resolve`].
pub const STALL_THRESHOLD_ENV: &str = "HOOCODE_SUBAGENT_STALL_MS";
/// A tool in flight counts as busy work only this long without any progress
/// event (before the load multiplier). A tool that declares its own timeout
/// (the Shell tool) gets `max(this, declared + TOOL_TIMEOUT_MARGIN_MS)`
/// instead; see [`ToolLimit`]. This is the bound for a tool that declares none.
/// 30 minutes, so it sits above the 20 minute progress stall. The
/// `subagentToolStallSeconds` setting and `HOOCODE_SUBAGENT_TOOL_STALL_MS`
/// override it; see [`crate::limits`].
pub const TOOL_IN_FLIGHT_STALL_MS: u64 = 30 * 60 * 1000;
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
/// does not depend on. The resolved value (env, then setting, then this) is
/// passed to children in [`SHELL_DEFAULT_TIMEOUT_ENV`]; see [`crate::limits`].
pub const SUBAGENT_SHELL_DEFAULT_TIMEOUT_SECS: f64 = 1200.0;
/// The env name the Shell tool reads for that default. Mirrors
/// `DEFAULT_BASH_TIMEOUT_ENV` in `hoocode-code-tool-bash`.
pub const SHELL_DEFAULT_TIMEOUT_ENV: &str = "HOOCODE_BASH_DEFAULT_TIMEOUT_SECS";

/// What a tool declares about its run time, from its `tool_execution_start`
/// event (`toolName`, `args`). Only the Shell tool declares one; its `timeout`
/// follows the tool's own rules, as a subagent: `0` is unlimited, a positive
/// value is the limit, and an omitted, negative or non-finite value is the
/// subagent default `default_shell_secs` (the resolved shell timeout).
pub fn tool_limit_from_start(
    tool_name: &str,
    args: Option<&Value>,
    default_shell_secs: f64,
) -> ToolLimit {
    if !matches!(tool_name, "Shell" | "Bash") {
        return ToolLimit::Undeclared;
    }
    let requested = args.and_then(|a| a.get("timeout")).and_then(Value::as_f64);
    match requested {
        Some(0.0) => ToolLimit::Unlimited,
        Some(t) if t.is_finite() && t > 0.0 => ToolLimit::Declared(secs_to_ms(t)),
        _ => ToolLimit::Declared(secs_to_ms(default_shell_secs)),
    }
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
    ((base as f64 * MAX_LOAD_MULTIPLIER) as u64).min(base.saturating_add(LOAD_HEADROOM_MS))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Process ids of our spawned children, by whether they are still unreaped.
///
/// A pid stays valid for signals until the child is reaped (a zombie keeps
/// its pid), and may be reused by any process after that. The cold pool's
/// `ProcessHandle` registers its child at spawn and, once the child has exited
/// but before it is reaped, marks it exited under this lock
/// ([`child_exited`]). Every kill by pid goes through [`with_live_child`],
/// which holds the lock while it signals, so a pid is never signalled (or its
/// tree walked) after the reap could have freed it for reuse.
/// A child process as a kill names it: its pid and the spawn generation the
/// registry gave it. A pid alone is not enough: the pool and the lifeguard
/// keep a child's pid for a moment after it is reaped, and the kernel may give
/// that pid to the pool's *next* child; a kill by the stale (pid, generation)
/// is then refused instead of landing on the other subagent's tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ChildPid {
    pub pid: u32,
    /// 0: not spawned through the registry (another runner, a test).
    pub generation: u64,
}

#[derive(Default)]
struct Children {
    /// pid -> generation, for children not yet reaped. Bounded by the number
    /// of live children: an entry goes when its child is marked exited.
    live: HashMap<u32, u64>,
    next_generation: u64,
}

static CHILDREN: LazyLock<Mutex<Children>> = LazyLock::new(Mutex::default);

fn children() -> std::sync::MutexGuard<'static, Children> {
    CHILDREN.lock().unwrap_or_else(|e| e.into_inner())
}

/// What a kill may do with a [`ChildPid`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PidState {
    /// Our child, this generation, not reaped: safe to signal and to walk.
    Live,
    /// Ours but not this generation (reaped, or its pid reused by a newer
    /// child of ours), or a pid-only kill of a pid we hold: never signal it.
    Stale,
    /// Not spawned through the registry and not a pid we hold.
    Unknown,
}

impl Monitored {
    fn child(&self) -> ChildPid {
        ChildPid {
            pid: self.pid,
            generation: self.generation,
        }
    }
}

impl Children {
    fn state_of(&self, child: ChildPid) -> PidState {
        match (self.live.get(&child.pid), child.generation) {
            (Some(&live), generation) if live == generation => PidState::Live,
            (Some(_), _) => PidState::Stale,
            (None, 0) => PidState::Unknown,
            (None, _) => PidState::Stale,
        }
    }
}

/// Register a child spawned by the pool, right after the spawn. Returns its
/// generation, which every later kill of it must name.
pub fn child_spawned(pid: u32) -> u64 {
    if pid == 0 {
        return 0;
    }
    let mut children = children();
    children.next_generation += 1;
    let generation = children.next_generation;
    children.live.insert(pid, generation);
    generation
}

/// The child has exited and is about to be reaped. Waits for a kill that is
/// signalling it to finish. A generation that is not the live one is ignored.
pub fn child_exited(pid: u32, generation: u64) {
    let mut children = children();
    if children.live.get(&pid) == Some(&generation) {
        children.live.remove(&pid);
    }
}

/// The live generation of `pid`, or 0 when no child of ours holds it.
pub fn live_generation(pid: u32) -> u64 {
    children().live.get(&pid).copied().unwrap_or(0)
}

/// Whether `child` is a live child of ours, of that generation.
pub fn is_live_child(child: ChildPid) -> bool {
    children().state_of(child) == PidState::Live
}

/// Run `kill` with `child`'s state, holding the registry lock, so the child
/// cannot be reaped while it runs.
fn with_live_child<T>(child: ChildPid, kill: impl FnOnce(PidState) -> T) -> T {
    let children = children();
    let state = children.state_of(child);
    let out = kill(state);
    drop(children);
    out
}

/// Run blocking process-table work off the async scheduler: inline when not
/// on a runtime worker, else in `block_in_place` (it stays synchronous, so a
/// dispose still finishes its kills before it returns).
fn off_scheduler<T>(work: impl FnOnce() -> T) -> T {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(work)
        }
        _ => work(),
    }
}

/// SIGKILLs what it holds when dropped while armed: processes a kill froze
/// (SIGSTOP) must not be left stopped forever if the kill unwinds midway.
#[cfg(unix)]
pub(crate) struct StoppedGuard {
    pids: Vec<u32>,
    groups: Vec<u32>,
    armed: bool,
}

#[cfg(unix)]
impl StoppedGuard {
    pub(crate) fn new() -> Self {
        Self {
            pids: Vec::new(),
            groups: Vec::new(),
            armed: true,
        }
    }

    pub(crate) fn stop_pid(&mut self, pid: u32) {
        // SAFETY: kill(2) on a process the caller has checked is ours to stop.
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGSTOP);
        }
        self.pids.push(pid);
    }

    pub(crate) fn stop_group(&mut self, pgid: u32) {
        // SAFETY: as above, on a group our unreaped child leads.
        unsafe {
            libc::kill(-(pgid as libc::pid_t), libc::SIGSTOP);
        }
        self.groups.push(pgid);
    }

    /// The kill went through: nothing to clean up.
    pub(crate) fn disarm(mut self) {
        self.armed = false;
    }
}

#[cfg(unix)]
impl Drop for StoppedGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        // SAFETY: the same processes and groups this guard stopped.
        unsafe {
            for &group in &self.groups {
                libc::kill(-(group as libc::pid_t), libc::SIGKILL);
            }
            for &pid in &self.pids {
                libc::kill(pid as libc::pid_t, libc::SIGKILL);
            }
        }
    }
}

/// Kill a process group by pid alone: for processes not spawned through the
/// registry. A pid a child of ours holds is refused (use
/// [`kill_child_trees`] with its generation), so a stale copy can never kill
/// a newer child that reused it.
pub fn kill_process_tree(pid: u32) {
    kill_child_trees(&[ChildPid { pid, generation: 0 }]);
}

/// Kill each child's process group (the pool spawns children as group
/// leaders), falling back to the single process (`killProcessTree`).
///
/// For a live child of ours of that generation, its own commands are reached
/// too: the Shell tool starts each command in a process group of its own,
/// which the child's group kill does not touch. The tree is frozen and walked
/// (see [`kill_trees`]) before anything is killed, reading the process table
/// once. A child of ours that is not live in that generation (reaped, or its
/// pid now held by a newer child) is never signalled.
pub fn kill_child_trees(targets: &[ChildPid]) {
    #[cfg(unix)]
    {
        let targets: Vec<ChildPid> = targets.iter().copied().filter(|c| c.pid > 0).collect();
        if targets.is_empty() {
            return;
        }
        off_scheduler(|| {
            let children = children();
            let mut live = Vec::new();
            for &target in &targets {
                match children.state_of(target) {
                    PidState::Live => live.push(target.pid),
                    // Not ours to track (another runner): the group kill only.
                    PidState::Unknown => kill_group_or_pid(target.pid),
                    PidState::Stale => agent_log(&format!(
                        "[LIFEGUARD] refused to kill pid {} (generation {}): no longer that child",
                        target.pid, target.generation
                    )),
                }
            }
            if !live.is_empty() {
                // Freeze each child's group first, so nothing forks between
                // the walk and the kill.
                let mut stopped = StoppedGuard::new();
                for &pid in &live {
                    stopped.stop_group(pid);
                }
                kill_trees(&live, &ProcTable);
                for &pid in &live {
                    kill_group_or_pid(pid);
                }
                stopped.disarm();
            }
            drop(children);
        });
    }
    #[cfg(not(unix))]
    for target in targets {
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/T", "/PID", &target.pid.to_string()])
            .spawn();
    }
}

#[cfg(unix)]
fn kill_group_or_pid(pid: u32) {
    let pid = pid as libc::pid_t;
    // SAFETY: plain kill(2) calls.
    unsafe {
        if libc::kill(-pid, libc::SIGKILL) != 0 {
            libc::kill(pid, libc::SIGKILL);
        }
    }
}

/// One row of the process table: (pid, parent pid, process group).
pub type ProcRow = (u32, u32, u32);

/// Where the process table comes from (a stub in tests).
pub trait ProcessTable {
    fn rows(&self) -> Vec<ProcRow>;
}

/// The real process table.
pub struct ProcTable;

impl ProcessTable for ProcTable {
    fn rows(&self) -> Vec<ProcRow> {
        let rows = process_table();
        if rows.is_empty() {
            static WARNED: std::sync::Once = std::sync::Once::new();
            WARNED.call_once(|| {
                agent_log("[LIFEGUARD] the process table came back empty; a subagent's own commands cannot be found to stop");
            });
        }
        rows
    }
}

/// (pid, parent pid, process group) of every process, from /proc.
#[cfg(target_os = "linux")]
fn process_table() -> Vec<ProcRow> {
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    dir.filter_map(|entry| {
        let pid: u32 = entry.ok()?.file_name().to_str()?.parse().ok()?;
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        // `pid (comm) state ppid pgrp …`; comm may hold spaces and parens.
        let mut fields = stat.rsplit_once(')')?.1.split_whitespace();
        let _state = fields.next()?;
        let ppid = fields.next()?.parse().ok()?;
        let pgid = fields.next()?.parse().ok()?;
        Some((pid, ppid, pgid))
    })
    .collect()
}

/// (pid, parent pid, process group) of every process, from `/bin/ps`.
#[cfg(all(unix, not(target_os = "linux")))]
fn process_table() -> Vec<ProcRow> {
    let Ok(out) = std::process::Command::new("/bin/ps")
        .args(["-A", "-o", "pid=", "-o", "ppid=", "-o", "pgid="])
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let mut f = line.split_whitespace().map(|n| n.parse::<u32>().ok());
            Some((f.next()??, f.next()??, f.next()??))
        })
        .collect()
}

/// What to kill below `root`: its descendants, and the process groups that a
/// descendant leads. Empty unless `root` is a direct child of `self_pid` (the
/// cold child and the warm worker both are): a pid that is not ours (reused,
/// or a grandchild) is never walked. Never `self_pid`, an ancestor of it, a
/// group led by either, group 1, or group 0.
pub fn plan_tree_kill(rows: &[ProcRow], root: u32, self_pid: u32) -> (Vec<u32>, Vec<u32>) {
    if root == 0
        || !rows
            .iter()
            .any(|&(p, ppid, _)| p == root && ppid == self_pid)
    {
        return (Vec::new(), Vec::new());
    }
    // This process and its ancestors, and the groups they are in.
    let mut protected = vec![self_pid];
    let mut cursor = self_pid;
    while let Some(&(_, ppid, _)) = rows.iter().find(|&&(p, _, _)| p == cursor) {
        if ppid == 0 || protected.contains(&ppid) {
            break;
        }
        protected.push(ppid);
        cursor = ppid;
    }
    let protected_groups: Vec<u32> = rows
        .iter()
        .filter(|&&(p, _, _)| protected.contains(&p))
        .map(|&(_, _, g)| g)
        .collect();
    let mut pids = Vec::new();
    let mut groups = Vec::new();
    let mut frontier = vec![root];
    while let Some(parent) = frontier.pop() {
        for &(child, ppid, pgid) in rows {
            if ppid != parent || child == root || pids.contains(&child) {
                continue;
            }
            if protected.contains(&child) {
                continue;
            }
            pids.push(child);
            frontier.push(child);
            // Only a group the descendant leads, and never a protected one:
            // one that joined a foreign group does not take that group down.
            if pgid == child
                && pgid > 1
                && pgid != root
                && !protected_groups.contains(&pgid)
                && !groups.contains(&pgid)
            {
                groups.push(pgid);
            }
        }
    }
    (pids, groups)
}

/// Freeze, walk and SIGKILL the descendants of each `root` (our unreaped
/// children; the caller holds the registry lock or the child lock). The tree
/// is walked twice: after the first walk every descendant found is stopped,
/// so the second catches anything forked in between. Kills: every group a
/// descendant leads, then every descendant.
#[cfg(unix)]
pub fn kill_trees(roots: &[u32], table: &dyn ProcessTable) {
    let self_pid = std::process::id();
    let plan = |rows: &[ProcRow]| {
        let mut pids = Vec::new();
        let mut groups = Vec::new();
        for &root in roots {
            let (p, g) = plan_tree_kill(rows, root, self_pid);
            pids.extend(p);
            groups.extend(g);
        }
        (pids, groups)
    };
    let (first, _) = plan(&table.rows());
    if first.is_empty() {
        return;
    }
    // Stopped now; killed below, or by the guard if this unwinds first.
    let mut stopped = StoppedGuard::new();
    for &pid in &first {
        stopped.stop_pid(pid);
    }
    let (pids, groups) = plan(&table.rows());
    // SAFETY: kill(2) on descendants of our unreaped child and the groups
    // they lead (see `plan_tree_kill` for what is never included).
    unsafe {
        for group in groups {
            libc::kill(-(group as libc::pid_t), libc::SIGKILL);
        }
        for pid in pids.into_iter().chain(first) {
            libc::kill(pid as libc::pid_t, libc::SIGKILL);
        }
    }
    stopped.disarm();
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
fn reap_group(child: ChildPid, reason: ReapReason) {
    let pid = child.pid;
    // A pid of 0 means there is no process — an in-process runner, or a spawn
    // that never produced one. `kill(-0, …)` would signal *every* process in
    // the caller's own group, which is the parent agent in the worst case, so
    // the guard `kill_tree` has belongs here too.
    if pid == 0 {
        return;
    }
    // A child of ours already reaped may have its pid reused: leave it alone.
    with_live_child(child, |state| {
        if state == PidState::Stale {
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
    });
}

#[cfg(not(unix))]
fn reap_group(child: ChildPid, _reason: ReapReason) {
    kill_tree(child);
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
fn terminate_group(child: ChildPid) {
    // Graceful shutdown: the parent is exiting, so every child gets SIGTERM.
    reap_group(child, ReapReason::Deadline);
}

#[cfg(not(unix))]
fn terminate_group(child: ChildPid) {
    kill_tree(child);
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
    /// The registry generation at `monitor` time (see [`ChildPid`]).
    generation: u64,
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
    /// The no-progress bound while a tool is in flight and declares no limit
    /// (before the load multiplier). See [`TOOL_IN_FLIGHT_STALL_MS`].
    tool_stall_ms: u64,
    /// The base hard deadline for every run (before the load multiplier).
    base_deadline_ms: u64,
    /// Resident memory above which a child is reaped.
    max_rss_bytes: u64,
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
        let mut bound = self.tool_stall_ms as f64 * self.load_multiplier();
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
                tool_stall_ms: TOOL_IN_FLIGHT_STALL_MS,
                base_deadline_ms: SUBAGENT_DEADLINE_MS,
                max_rss_bytes: hoocode_runtime::CHILD_RSS_LIMIT_BYTES,
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
    /// pool sets it from its resolved limits.
    pub fn set_progress_stall_ms(&self, ms: u64) {
        self.state().progress_stall_ms = ms.max(1);
    }

    /// Set the no-progress bound for a tool in flight that declares no limit
    /// (before the load multiplier). The pool sets it from its resolved limits.
    pub fn set_tool_stall_ms(&self, ms: u64) {
        self.state().tool_stall_ms = ms.max(1);
    }

    /// Set the base hard deadline for every run (before the load multiplier).
    /// The pool sets it from its resolved limits.
    pub fn set_base_deadline_ms(&self, ms: u64) {
        self.state().base_deadline_ms = ms.max(1);
    }

    /// Set the child resident-memory cap in bytes. The pool sets it from its
    /// resolved limits.
    pub fn set_max_rss_bytes(&self, bytes: u64) {
        self.state().max_rss_bytes = bytes.max(1);
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
                generation: live_generation(pid),
                agent_type: agent_type.to_string(),
            },
        );
        state.last_heartbeat.insert(task_id.to_string(), now);
        state.last_progress.insert(task_id.to_string(), now);
        let base = state.base_deadline_ms;
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

    /// Children whose resident memory is above the child RSS cap (4 GiB each
    /// by default, see [`set_max_rss_bytes`](Self::set_max_rss_bytes)).
    fn children_over_memory_limit(&self) -> Vec<String> {
        let (candidates, limit): (Vec<(String, u32)>, u64) = {
            let state = self.state();
            let candidates = state
                .processes
                .iter()
                .filter(|(id, _)| !state.reaping.contains(*id))
                .map(|(id, monitored)| (id.clone(), monitored.pid))
                .collect();
            (candidates, state.max_rss_bytes)
        };
        candidates
            .into_iter()
            .filter(|(task_id, pid)| {
                let Some(rss) = hoocode_runtime::child_rss_bytes(*pid) else {
                    return false;
                };
                if rss <= limit {
                    return false;
                }
                agent_log(&format!(
                    "[LIFEGUARD] task_id={task_id} rss_mb={} above the {} MiB child limit; reaping",
                    rss / hoocode_runtime::MIB,
                    limit / hoocode_runtime::MIB,
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
        self.terminate_then_kill(task_id, monitored.child(), ReapReason::Stalled);
        self.emit(LifeguardEvent::Stalled {
            task_id: task_id.to_string(),
            pid: monitored.pid,
        });
    }

    /// Send the stop signal (see [`ReapReason`]), then SIGKILL the child's
    /// process group if the child has not left within the grace. The child's
    /// exit cancels this timer and sweeps the group via
    /// [`untrack`](Self::untrack).
    fn terminate_then_kill(self: &Arc<Self>, task_id: &str, child: ChildPid, reason: ReapReason) {
        let pid = child.pid;
        // Record the reap before signalling: a child that exits as soon as it
        // is signalled must still find its entry, or its group is never swept.
        self.state().reaped.insert(task_id.to_string(), pid);
        reap_group(child, reason);
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
                .is_some_and(|monitored| monitored.child() == child);
            if !still_running {
                return;
            }
            agent_log(&format!(
                "[LIFEGUARD] task_id={owned} did not exit within {grace_ms}ms of SIGTERM; killing"
            ));
            kill_tree(child);
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
                .unwrap_or(state.base_deadline_ms);
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
        self.terminate_then_kill(task_id, monitored.child(), ReapReason::Deadline);
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
        let pids: Vec<ChildPid> = self
            .state()
            .processes
            .values()
            .map(Monitored::child)
            .collect();
        for child in pids.iter().filter(|c| c.pid > 0) {
            terminate_group(*child);
        }
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(PARENT_SHUTDOWN_GRACE_MS)).await;
            if let Some(guard) = weak.upgrade() {
                let pids: Vec<ChildPid> = guard
                    .state()
                    .processes
                    .values()
                    .map(Monitored::child)
                    .collect();
                kill_child_trees(&pids);
            }
        });
    }

    /// Kill all monitored processes and stop monitoring.
    pub fn dispose(&self) {
        let pids: Vec<ChildPid> = {
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
            let pids: Vec<ChildPid> = state.processes.values().map(Monitored::child).collect();
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
        kill_child_trees(&pids);
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
fn kill_tree(child: ChildPid) {
    if child.pid > 0 {
        kill_child_trees(&[child]);
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

pub(crate) fn has_running_pid(dir: &Path) -> bool {
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

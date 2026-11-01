//! The scheduled-task store: `<cwd>/.agents/scheduled_tasks.json`, the same
//! file and shape hoocode-ts writes (`{"tasks": [...]}`), so both tools see
//! one set of schedules.
//!
//! Every operation re-reads the file, so a task created by another process is
//! seen. Mutations and fire claims run under a process-wide mutex and a lock
//! file (`<store>.lock`, created with `create_new`), so two processes that
//! share a store fire a due task once. Writes go to a temporary file that is
//! renamed over the store.
//!
//! Every task records its owner, the process that created it. Only the owner
//! fires a due task, so an idle hoocode that happens to share the folder does
//! not run another session's schedule. A task whose owner has exited, and a
//! legacy task with no owner, is adopted by the first process that finds it
//! due (see [`TaskStore::claim_due_tick_with`]).

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::cron;

/// The store's path under the working directory.
pub const STORE_RELATIVE_PATH: &str = ".agents/scheduled_tasks.json";
/// The store hoocode-ts wrote before `.agents/`; read when the new store is absent.
pub const LEGACY_STORE_RELATIVE_PATH: &str = ".hoocode/scheduled_tasks.json";

/// A task due while the agent was busy still fires on the next idle tick when
/// that tick comes within this many minutes of its due minute; later, it is
/// skipped and reported.
pub const LATE_FIRE_MINUTES: i64 = 10;
/// How far back a tick looks for a task's due minute. A day covers hourly and
/// daily schedules. A rarer schedule whose due minute is older than this is
/// not reported as skipped.
const LOOKBACK_MINUTES: i64 = 24 * 60;

/// A lock file older than this belongs to a process that died holding it.
const STALE_LOCK: Duration = Duration::from_secs(10);
/// How long to wait for the lock before going ahead without it (best effort, as TS).
const LOCK_WAIT: Duration = Duration::from_secs(3);

/// Serializes store access within this process; the lock file does the rest.
static PROCESS_LOCK: Mutex<()> = Mutex::new(());
static ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// `ScheduledTask`, field for field as hoocode-ts stores it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledTask {
    pub id: String,
    /// 5-field cron expression in local time.
    pub cron: String,
    /// Prompt re-submitted on each fire.
    pub prompt: String,
    /// Recurring (fire on every match) or one-shot (deleted after its fire).
    pub recurring: bool,
    /// Milliseconds since the Unix epoch.
    pub created_at: u64,
    /// Minute-bucket key (epoch minutes) of the last fire.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_minute: Option<i64>,
    /// Pid of the process that owns the task: only it fires the task, until it
    /// exits. Absent on tasks written before owners were recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_pid: Option<u32>,
    /// The owner's session id, for display. Absent when the creator did not
    /// know it (the Cron tools record the pid only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_session_id: Option<String>,
    /// How many times the task has fired.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub fire_count: u64,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

/// The process (and, when known, the session) a store acts for: who a new task
/// belongs to, and who a claim fires for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Owner {
    pub pid: u32,
    pub session_id: Option<String>,
}

impl Owner {
    /// This process, with the session id if the caller has one.
    pub fn this_process(session_id: Option<String>) -> Self {
        Self {
            pid: std::process::id(),
            session_id,
        }
    }
}

/// Whether a process with this pid is running. Used to tell a task whose owner
/// has exited (adopt it) from one whose owner is busy (leave it alone).
///
/// On unix this is `kill(pid, 0)`: alive when it succeeds, or fails with
/// `EPERM` (the process exists but belongs to someone else). Elsewhere there is
/// no cheap check, so the owner is assumed alive: the task then waits for its
/// owner (or `CronDelete`) rather than risk two sessions running it. A pid
/// that the system has reused for another process also reads as alive.
pub fn process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // 0 and negative values would signal a process group, not a process.
        let Ok(pid) = i32::try_from(pid) else {
            return false;
        };
        if pid == 0 {
            return false;
        }
        // SAFETY: signal 0 only checks that the process can be signalled.
        if unsafe { libc::kill(pid, 0) } == 0 {
            return true;
        }
        io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        true
    }
}

/// Whether a prompt is a slash command or prompt template. Such a prompt must
/// reach the session unchanged to expand, so it gets no scheduled-task line.
pub fn is_slash_prompt(prompt: &str) -> bool {
    prompt.trim_start().starts_with('/')
}

/// The text a fired task is sent as: a line naming the task, then its prompt,
/// so the user and the model can see the message was scheduled. A slash prompt
/// is sent as it is (see [`is_slash_prompt`]); the caller shows the fire in a
/// notice instead.
pub fn fire_message(id: &str, cron: &str, recurring: bool, prompt: &str) -> String {
    if is_slash_prompt(prompt) {
        return prompt.to_owned();
    }
    format!(
        "⏰ Scheduled task {id} ({cron}, {})\n{prompt}",
        if recurring { "recurring" } else { "once" }
    )
}

impl ScheduledTask {
    /// The text this task is sent as when it fires (see [`fire_message`]).
    pub fn fire_message(&self) -> String {
        fire_message(&self.id, &self.cron, self.recurring, &self.prompt)
    }
}

/// One task that fired in a tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fired {
    pub id: String,
    pub cron: String,
    pub recurring: bool,
    /// The stored prompt, without the scheduled-task line.
    pub prompt: String,
}

impl Fired {
    /// The text to submit: the prompt under its scheduled-task line, or the
    /// prompt alone when it is a slash command.
    pub fn message(&self) -> String {
        fire_message(&self.id, &self.cron, self.recurring, &self.prompt)
    }

    /// Whether the prompt is a slash command, sent without a task line.
    pub fn is_slash(&self) -> bool {
        is_slash_prompt(&self.prompt)
    }
}

/// A task this process took over in a tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adopted {
    pub task_id: String,
    /// The previous owner's pid; `None` for a legacy task that had no owner.
    pub previous_pid: Option<u32>,
    /// The previous owner's session id, when it was recorded.
    pub previous_session: Option<String>,
}

/// What one tick claimed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Claimed {
    /// The tasks to submit now, in stored order.
    pub fired: Vec<Fired>,
    /// The tasks behind `fired`, as they were when they fired (a one-shot is
    /// already gone from the store).
    pub fired_jobs: Vec<ScheduledTask>,
    /// Prompts whose due minute is more than [`LATE_FIRE_MINUTES`] old. They
    /// do not run; the caller says so.
    pub skipped: Vec<String>,
    /// Tasks taken over from an owner that has exited, or from no owner. They
    /// belong to this process now; the caller says so.
    pub adopted: Vec<Adopted>,
}

/// One tick's claim and, when the store could be read under the lock, the
/// store's tasks after it: what the task panel shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tick {
    pub claimed: Claimed,
    /// `None` when the lock was not taken (nothing was read).
    pub jobs: Option<Vec<ScheduledTask>>,
}

#[derive(Serialize, Deserialize, Default)]
struct StoreFile {
    #[serde(default)]
    tasks: Vec<ScheduledTask>,
}

/// The task store at one path, with the legacy path it falls back to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskStore {
    path: PathBuf,
    legacy_path: Option<PathBuf>,
    /// Who new tasks belong to, and who claims fire for.
    owner: Owner,
}

impl TaskStore {
    /// The store for a working directory.
    pub fn for_cwd(cwd: &Path) -> Self {
        Self {
            path: cwd.join(STORE_RELATIVE_PATH),
            legacy_path: Some(cwd.join(LEGACY_STORE_RELATIVE_PATH)),
            owner: Owner::this_process(None),
        }
    }

    /// A store at an explicit path, with no legacy fallback.
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            legacy_path: None,
            owner: Owner::this_process(None),
        }
    }

    /// The same store acting for `owner` instead of this process.
    pub fn with_owner(mut self, owner: Owner) -> Self {
        self.owner = owner;
        self
    }

    /// The same store acting for this process and the session `session_id`.
    pub fn with_session_id(self, session_id: Option<String>) -> Self {
        self.with_owner(Owner::this_process(session_id))
    }

    /// Who new tasks belong to, and who claims fire for.
    pub fn owner(&self) -> &Owner {
        &self.owner
    }

    /// The file this store writes.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `load()`: the primary store, or the legacy one until the primary exists.
    /// A missing or unreadable file is an empty store (best effort, as TS).
    fn read(&self) -> Vec<ScheduledTask> {
        let source = match &self.legacy_path {
            Some(legacy) if !self.path.exists() && legacy.exists() => legacy.as_path(),
            _ => self.path.as_path(),
        };
        fs::read_to_string(source)
            .ok()
            .and_then(|text| serde_json::from_str::<StoreFile>(&text).ok())
            .map(|file| file.tasks)
            .unwrap_or_default()
    }

    /// `persist()`: write the whole list through a temporary file and rename it.
    fn write(&self, tasks: &[ScheduledTask]) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut text = serde_json::to_string_pretty(&StoreFile {
            tasks: tasks.to_vec(),
        })
        .map_err(io::Error::other)?;
        text.push('\n');
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let tmp = self
            .path
            .with_extension(format!("json.{}.{nanos}.tmp", std::process::id()));
        {
            let mut file = fs::File::create(&tmp)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
        }
        if let Err(error) = fs::rename(&tmp, &self.path) {
            let _ = fs::remove_file(&tmp);
            return Err(error);
        }
        Ok(())
    }

    /// Run `f` holding the process lock and the lock file. `f` is told whether
    /// the lock file was taken: when it was not within [`LOCK_WAIT`], `f` still
    /// runs (a create or delete is best effort, as TS), but a fire claim must
    /// not, since another process may be claiming the same minute.
    fn with_lock<T>(&self, f: impl FnOnce(bool) -> T) -> T {
        let _process = PROCESS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let file = FileLock::acquire(self.lock_path());
        f(file.held())
    }

    fn lock_path(&self) -> PathBuf {
        let mut name = self.path.file_name().unwrap_or_default().to_os_string();
        name.push(".lock");
        self.path.with_file_name(name)
    }

    /// `list()`: every task, in stored order.
    pub fn list(&self) -> Vec<ScheduledTask> {
        self.read()
    }

    /// `create()`: store a new task. Fails when the store cannot be written.
    pub fn create(&self, cron: &str, prompt: &str, recurring: bool) -> io::Result<ScheduledTask> {
        let task = ScheduledTask {
            id: new_id(),
            cron: cron.to_owned(),
            prompt: prompt.to_owned(),
            recurring,
            created_at: now_millis(),
            last_run_minute: None,
            owner_pid: Some(self.owner.pid),
            owner_session_id: self.owner.session_id.clone(),
            fire_count: 0,
        };
        self.with_lock(|_| {
            let mut tasks = self.read();
            tasks.push(task.clone());
            self.write(&tasks)
        })?;
        Ok(task)
    }

    /// `delete(id)`: true when a task with that id was removed.
    pub fn delete(&self, id: &str) -> io::Result<bool> {
        self.with_lock(|_| {
            let mut tasks = self.read();
            let before = tasks.len();
            tasks.retain(|t| t.id != id);
            let removed = tasks.len() < before;
            if removed {
                self.write(&tasks)?;
            }
            Ok(removed)
        })
    }

    /// `tick(now)` without the idle check: claim the tasks due at `now`.
    ///
    /// A task is due at its latest matching minute after its last fire (or
    /// after it was created). A tick up to [`LATE_FIRE_MINUTES`] after that
    /// minute fires it; a later tick skips it and reports it. Either way the
    /// due minute is recorded, so each due minute is handled once. A one-shot
    /// task is removed. Callers check that the agent is idle first.
    ///
    /// Only the owner claims a task; see [`claim_due_tick_with`](Self::claim_due_tick_with).
    pub fn claim_due(&self, now: &DateTime<Local>) -> io::Result<Claimed> {
        self.claim_due_tick(now).map(|tick| tick.claimed)
    }

    /// [`claim_due`](Self::claim_due), and the store's tasks after the claim
    /// (see [`Tick`]). The tasks are read under the same lock, so the panel
    /// costs no extra read.
    pub fn claim_due_tick(&self, now: &DateTime<Local>) -> io::Result<Tick> {
        self.claim_due_tick_with(now, &process_alive)
    }

    /// [`claim_due`](Self::claim_due) with `alive` deciding whether an owner pid
    /// is running (tests pass their own).
    pub fn claim_due_with(
        &self,
        now: &DateTime<Local>,
        alive: &dyn Fn(u32) -> bool,
    ) -> io::Result<Claimed> {
        self.claim_due_tick_with(now, alive)
            .map(|tick| tick.claimed)
    }

    /// [`claim_due_tick`](Self::claim_due_tick) with `alive` deciding whether an
    /// owner pid is running.
    ///
    /// This store's owner claims a due task when it owns it. It also claims,
    /// and adopts (the task's owner becomes this one, and the claim reports it
    /// in [`Claimed::adopted`]), a due task that has no owner or whose owner
    /// pid is not alive. A due task of a live other owner is left untouched:
    /// its last-run minute does not move and nothing is reported, so its owner
    /// still fires it.
    pub fn claim_due_tick_with(
        &self,
        now: &DateTime<Local>,
        alive: &dyn Fn(u32) -> bool,
    ) -> io::Result<Tick> {
        self.with_lock(|held| {
            // Without the lock file another process could claim the same minute
            // too, so this tick fires nothing; the next tick claims it, still
            // within the late window for a task due a moment ago.
            if !held {
                return Ok(Tick::default());
            }
            let now_minute = now.timestamp().div_euclid(60);
            let mut tasks = self.read();
            let mut claimed = Claimed::default();
            let mut spent = Vec::new();
            let mut changed = false;
            for task in tasks.iter_mut() {
                let created_minute = i64::try_from(task.created_at / 60_000).unwrap_or(i64::MAX);
                let floor = task
                    .last_run_minute
                    .map_or(created_minute, |minute| (minute + 1).max(created_minute));
                let Some(due) = latest_due_minute(&task.cron, now, now_minute, floor) else {
                    continue;
                };
                match task.owner_pid {
                    Some(pid) if pid == self.owner.pid => {
                        // Ours; keep the session id current (it changes on /new).
                        if self.owner.session_id.is_some()
                            && task.owner_session_id != self.owner.session_id
                        {
                            task.owner_session_id = self.owner.session_id.clone();
                        }
                    }
                    Some(pid) if alive(pid) => continue,
                    previous => {
                        claimed.adopted.push(Adopted {
                            task_id: task.id.clone(),
                            previous_pid: previous,
                            previous_session: task.owner_session_id.take(),
                        });
                        task.owner_pid = Some(self.owner.pid);
                        task.owner_session_id = self.owner.session_id.clone();
                    }
                }
                changed = true;
                task.last_run_minute = Some(due);
                if now_minute - due <= LATE_FIRE_MINUTES {
                    task.fire_count += 1;
                    claimed.fired.push(Fired {
                        id: task.id.clone(),
                        cron: task.cron.clone(),
                        recurring: task.recurring,
                        prompt: task.prompt.clone(),
                    });
                    claimed.fired_jobs.push(task.clone());
                } else {
                    claimed.skipped.push(task.prompt.clone());
                }
                if !task.recurring {
                    spent.push(task.id.clone());
                }
            }
            if changed {
                tasks.retain(|t| !spent.contains(&t.id));
                self.write(&tasks)?;
            }
            Ok(Tick {
                claimed,
                jobs: Some(tasks),
            })
        })
    }
}

/// The latest minute in `floor..=now_minute`, and within [`LOOKBACK_MINUTES`]
/// of it, at which `cron` matches. `now` is a time in `now_minute`.
fn latest_due_minute(
    cron: &str,
    now: &DateTime<Local>,
    now_minute: i64,
    floor: i64,
) -> Option<i64> {
    let lowest = floor.max(now_minute - LOOKBACK_MINUTES);
    (lowest..=now_minute).rev().find(|minute| {
        let at = *now - chrono::Duration::minutes(now_minute - minute);
        cron::matches(cron, &at)
    })
}

/// `Date.now().toString(36) + counter.toString(36)`, prefixed `task_`.
fn new_id() -> String {
    let counter = ID_COUNTER.fetch_add(1, Ordering::SeqCst) + 1;
    format!(
        "task_{}{}",
        to_base36(u128::from(now_millis())),
        to_base36(u128::from(counter))
    )
}

fn to_base36(mut n: u128) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n == 0 {
        return "0".into();
    }
    let mut out = Vec::new();
    while n > 0 {
        out.push(DIGITS[(n % 36) as usize]);
        n /= 36;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// An acquired `<store>.lock`, removed on drop.
struct FileLock {
    path: Option<PathBuf>,
}

impl FileLock {
    /// Whether the lock file is ours.
    fn held(&self) -> bool {
        self.path.is_some()
    }

    fn acquire(path: PathBuf) -> Self {
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let started = std::time::Instant::now();
        loop {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(_) => return Self { path: Some(path) },
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    if is_stale(&path) {
                        let _ = fs::remove_file(&path);
                        continue;
                    }
                    if started.elapsed() >= LOCK_WAIT {
                        return Self { path: None };
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return Self { path: None },
            }
        }
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            let _ = fs::remove_file(path);
        }
    }
}

fn is_stale(path: &Path) -> bool {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age > STALE_LOCK)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn store_in(dir: &tempfile::TempDir) -> TaskStore {
        TaskStore::for_cwd(dir.path())
    }

    #[test]
    fn create_list_and_delete_round_trip_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(&dir);
        let task = store
            .create("*/5 * * * *", "check the build", true)
            .unwrap();
        assert!(task.id.starts_with("task_"));
        assert_eq!(store.list(), vec![task.clone()]);

        let text =
            std::fs::read_to_string(dir.path().join(".agents/scheduled_tasks.json")).unwrap();
        assert!(text.starts_with("{\n  \"tasks\": ["), "{text}");
        assert!(text.contains("\"createdAt\""), "{text}");
        assert!(
            !text.contains("lastRunMinute"),
            "unset minute is omitted: {text}"
        );

        assert!(store.delete(&task.id).unwrap());
        assert!(!store.delete(&task.id).unwrap());
        assert!(store.list().is_empty());
        assert!(!dir
            .path()
            .join(".agents/scheduled_tasks.json.lock")
            .exists());
    }

    #[test]
    fn legacy_store_is_read_until_the_primary_exists() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join(LEGACY_STORE_RELATIVE_PATH);
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(
            &legacy,
            r#"{"tasks":[{"id":"task_old","cron":"0 * * * *","prompt":"p","recurring":false,"createdAt":1}]}"#,
        )
        .unwrap();
        let store = store_in(&dir);
        assert_eq!(store.list()[0].id, "task_old");
        store.create("0 9 * * *", "new", true).unwrap();
        assert_eq!(
            store.list().len(),
            2,
            "the legacy tasks migrate into the primary store"
        );
    }

    /// Adds a task created at `created_at` (ms), without `create`'s clock.
    fn put(store: &TaskStore, cron: &str, prompt: &str, recurring: bool, created_at: u64) {
        let mut tasks = store.read();
        tasks.push(ScheduledTask {
            id: new_id(),
            cron: cron.into(),
            prompt: prompt.into(),
            recurring,
            created_at,
            last_run_minute: None,
            owner_pid: None,
            owner_session_id: None,
            fire_count: 0,
        });
        store.write(&tasks).unwrap();
    }

    /// Adds a task owned by process `pid`, created at the epoch.
    fn put_owned(store: &TaskStore, cron: &str, prompt: &str, pid: u32, session: Option<&str>) {
        put(store, cron, prompt, true, 0);
        let mut tasks = store.read();
        let last = tasks.last_mut().unwrap();
        last.owner_pid = Some(pid);
        last.owner_session_id = session.map(str::to_owned);
        store.write(&tasks).unwrap();
    }

    /// The prompts of the tasks a claim fired, in order.
    fn prompts(claimed: &Claimed) -> Vec<&str> {
        claimed.fired.iter().map(|f| f.prompt.as_str()).collect()
    }

    fn at(hour: u32, minute: u32, second: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(2026, 10, 8, hour, minute, second)
            .single()
            .unwrap()
    }

    #[test]
    fn claim_fires_each_matching_minute_once_and_drops_one_shots() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(&dir);
        put(&store, "30 9 * * *", "recurring", true, 0);
        put(&store, "30 9 * * *", "once", false, 0);
        let claimed = store.claim_due(&at(9, 30, 10)).unwrap();
        assert_eq!(prompts(&claimed), vec!["recurring", "once"]);
        assert!(claimed.skipped.is_empty());
        // A second tick in the same minute fires nothing.
        assert_eq!(store.claim_due(&at(9, 30, 40)).unwrap(), Claimed::default());
        let tasks = store.list();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].prompt, "recurring");
        assert!(tasks[0].last_run_minute.is_some());
    }

    #[test]
    fn a_task_due_while_busy_fires_on_the_next_idle_tick_within_ten_minutes() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(&dir);
        put(&store, "30 9 * * *", "recurring", true, 0);
        put(&store, "30 9 * * *", "once", false, 0);
        // Busy from 9:30; the first idle tick is at 9:39, nine minutes late.
        let claimed = store.claim_due(&at(9, 39, 5)).unwrap();
        assert_eq!(prompts(&claimed), vec!["recurring", "once"]);
        assert!(claimed.skipped.is_empty());
        // It fired once: the rest of that tick and the next ones claim nothing.
        assert_eq!(store.claim_due(&at(9, 39, 35)).unwrap(), Claimed::default());
        assert_eq!(store.claim_due(&at(9, 40, 5)).unwrap(), Claimed::default());
        assert_eq!(store.list().len(), 1);
    }

    #[test]
    fn a_task_due_more_than_ten_minutes_ago_is_skipped_with_a_notice_once() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(&dir);
        put(&store, "30 9 * * *", "recurring", true, 0);
        put(&store, "30 9 * * *", "once", false, 0);
        // Eleven minutes late: not run, reported.
        let claimed = store.claim_due(&at(9, 41, 5)).unwrap();
        assert!(claimed.fired.is_empty());
        assert_eq!(claimed.skipped, vec!["recurring", "once"]);
        // Reported once: a later idle tick does not repeat it, and the one-shot is gone.
        assert_eq!(store.claim_due(&at(9, 42, 5)).unwrap(), Claimed::default());
        let tasks = store.list();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].prompt, "recurring");
    }

    #[test]
    fn a_task_created_after_its_due_minute_does_not_fire_for_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(&dir);
        let created = Local
            .with_ymd_and_hms(2026, 10, 8, 9, 35, 0)
            .single()
            .unwrap();
        put(
            &store,
            "30 9 * * *",
            "late",
            true,
            u64::try_from(created.timestamp_millis()).unwrap(),
        );
        assert_eq!(store.claim_due(&at(9, 36, 0)).unwrap(), Claimed::default());
    }

    #[test]
    fn a_tick_reports_the_fired_jobs_and_the_store_after_the_claim() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(&dir);
        put(&store, "30 9 * * *", "recurring", true, 0);
        put(&store, "30 9 * * *", "once", false, 0);
        put(&store, "0 3 * * *", "other", true, 0);
        let tick = store.claim_due_tick(&at(9, 30, 10)).unwrap();
        assert_eq!(prompts(&tick.claimed), vec!["recurring", "once"]);
        // The fired jobs are as they were when they fired, the one-shot included.
        let fired: Vec<(&str, bool)> = tick
            .claimed
            .fired_jobs
            .iter()
            .map(|j| (j.prompt.as_str(), j.recurring))
            .collect();
        assert_eq!(fired, vec![("recurring", true), ("once", false)]);
        // The store after the claim: the one-shot is gone, the others remain.
        let live: Vec<String> = tick
            .jobs
            .expect("the lock was taken")
            .into_iter()
            .map(|j| j.prompt)
            .collect();
        assert_eq!(live, vec!["recurring", "other"]);
    }

    #[test]
    fn two_stores_on_one_file_fire_a_due_task_once() {
        // Two sessions on the same directory: the second claim sees the first's
        // due minute, and the task is not fired again.
        let dir = tempfile::tempdir().unwrap();
        put(&store_in(&dir), "30 9 * * *", "shared", true, 0);
        let first = store_in(&dir);
        let second = store_in(&dir);
        assert_eq!(
            prompts(&first.claim_due(&at(9, 30, 10)).unwrap()),
            vec!["shared"]
        );
        assert_eq!(
            second.claim_due(&at(9, 30, 12)).unwrap(),
            Claimed::default()
        );
    }

    #[test]
    fn a_claim_that_cannot_take_the_lock_fires_nothing_and_waits_for_the_next_tick() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(&dir);
        put(&store, "30 9 * * *", "recurring", true, 0);
        // Another process holds a fresh lock file.
        std::fs::create_dir_all(dir.path().join(".agents")).unwrap();
        let lock = dir.path().join(".agents/scheduled_tasks.json.lock");
        std::fs::write(&lock, "").unwrap();
        assert_eq!(store.claim_due(&at(9, 30, 10)).unwrap(), Claimed::default());
        assert!(store.list()[0].last_run_minute.is_none());
        std::fs::remove_file(&lock).unwrap();
        assert_eq!(
            prompts(&store.claim_due(&at(9, 30, 40)).unwrap()),
            vec!["recurring"]
        );
    }

    /// A store at `dir` acting for the process `pid` (a test stands in for several).
    fn store_for(dir: &tempfile::TempDir, pid: u32, session: Option<&str>) -> TaskStore {
        store_in(dir).with_owner(Owner {
            pid,
            session_id: session.map(str::to_owned),
        })
    }

    /// Everyone but `dead` is running.
    fn all_alive_but(dead: u32) -> impl Fn(u32) -> bool {
        move |pid| pid != dead
    }

    #[test]
    fn the_owner_fires_its_task_and_counts_the_fire() {
        let dir = tempfile::tempdir().unwrap();
        let owner = store_for(&dir, 100, Some("sess-a"));
        let task = owner.create("30 9 * * *", "mine", true).unwrap();
        assert_eq!(task.owner_pid, Some(100));
        assert_eq!(task.owner_session_id.as_deref(), Some("sess-a"));
        // Backdate it, so the 9:30 slot is after its creation.
        let mut tasks = owner.read();
        tasks[0].created_at = 0;
        owner.write(&tasks).unwrap();

        let claimed = owner.claim_due_with(&at(9, 30, 10), &|_| true).unwrap();
        assert_eq!(prompts(&claimed), vec!["mine"]);
        assert!(claimed.adopted.is_empty(), "its own task is not adopted");
        let after = owner.list();
        assert_eq!(after[0].fire_count, 1);
        assert_eq!(after[0].owner_pid, Some(100));
        // A later day fires it again and counts again.
        let next = Local
            .with_ymd_and_hms(2026, 10, 9, 9, 30, 5)
            .single()
            .unwrap();
        owner.claim_due_with(&next, &|_| true).unwrap();
        assert_eq!(owner.list()[0].fire_count, 2);
    }

    #[test]
    fn a_process_does_not_fire_or_touch_a_task_whose_owner_is_alive() {
        let dir = tempfile::tempdir().unwrap();
        put_owned(&store_in(&dir), "30 9 * * *", "theirs", 100, Some("sess-a"));
        let bystander = store_for(&dir, 200, Some("sess-b"));
        // Pid 100 is alive: the bystander claims nothing, at 9:30 or when late.
        let alive = |_: u32| true;
        for when in [at(9, 30, 10), at(9, 41, 0)] {
            let tick = bystander.claim_due_tick_with(&when, &alive).unwrap();
            assert_eq!(tick.claimed, Claimed::default());
        }
        let task = &bystander.list()[0];
        assert_eq!(task.last_run_minute, None, "the due minute is left alone");
        assert_eq!(task.owner_pid, Some(100));
        assert_eq!(task.fire_count, 0);
        // The owner still fires it afterwards.
        let owner = store_for(&dir, 100, Some("sess-a"));
        assert_eq!(
            prompts(&owner.claim_due_with(&at(9, 31, 0), &alive).unwrap()),
            vec!["theirs"]
        );
    }

    #[test]
    fn a_task_whose_owner_has_exited_is_adopted_and_fired() {
        let dir = tempfile::tempdir().unwrap();
        put_owned(&store_in(&dir), "30 9 * * *", "orphan", 100, Some("sess-a"));
        let adopter = store_for(&dir, 200, Some("sess-b"));
        let claimed = adopter
            .claim_due_with(&at(9, 30, 10), &all_alive_but(100))
            .unwrap();
        assert_eq!(prompts(&claimed), vec!["orphan"]);
        assert_eq!(
            claimed.adopted,
            vec![Adopted {
                task_id: claimed.fired[0].id.clone(),
                previous_pid: Some(100),
                previous_session: Some("sess-a".into()),
            }]
        );
        let task = &adopter.list()[0];
        assert_eq!(task.owner_pid, Some(200));
        assert_eq!(task.owner_session_id.as_deref(), Some("sess-b"));
        // It is theirs now: the old owner's pid, if it came back, would not fire it.
        let next = Local
            .with_ymd_and_hms(2026, 10, 9, 9, 30, 5)
            .single()
            .unwrap();
        let back = store_for(&dir, 100, Some("sess-a"));
        let alive = |_: u32| true;
        assert_eq!(
            back.claim_due_with(&next, &alive).unwrap(),
            Claimed::default()
        );
        let claimed = adopter.claim_due_with(&next, &alive).unwrap();
        assert_eq!(prompts(&claimed), vec!["orphan"]);
        assert!(claimed.adopted.is_empty(), "adopted once");
    }

    #[test]
    fn a_task_with_no_owner_is_adopted_by_the_first_claimer() {
        let dir = tempfile::tempdir().unwrap();
        put(&store_in(&dir), "30 9 * * *", "legacy", true, 0);
        let adopter = store_for(&dir, 200, None);
        let claimed = adopter.claim_due_with(&at(9, 30, 10), &|_| true).unwrap();
        assert_eq!(prompts(&claimed), vec!["legacy"]);
        assert_eq!(claimed.adopted.len(), 1);
        assert_eq!(claimed.adopted[0].previous_pid, None);
        assert_eq!(adopter.list()[0].owner_pid, Some(200));
    }

    #[test]
    fn a_task_nobody_is_due_to_fire_is_not_adopted() {
        let dir = tempfile::tempdir().unwrap();
        put_owned(&store_in(&dir), "30 9 * * *", "later", 100, None);
        let mut tasks = store_in(&dir).read();
        tasks[0].created_at = u64::try_from(at(7, 0, 0).timestamp_millis()).unwrap();
        store_in(&dir).write(&tasks).unwrap();
        let other = store_for(&dir, 200, None);
        let tick = other
            .claim_due_tick_with(&at(8, 0, 0), &all_alive_but(100))
            .unwrap();
        assert_eq!(tick.claimed, Claimed::default());
        assert_eq!(other.list()[0].owner_pid, Some(100));
    }

    #[test]
    fn a_store_written_before_owners_loads_and_stays_without_them() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(STORE_RELATIVE_PATH);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{"tasks":[{"id":"task_old","cron":"0 * * * *","prompt":"p","recurring":true,"createdAt":1}]}"#,
        )
        .unwrap();
        let store = store_in(&dir);
        let task = store.list().remove(0);
        assert_eq!(
            (task.owner_pid, task.owner_session_id, task.fire_count),
            (None, None, 0)
        );
        store.write(&store.list()).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            !text.contains("ownerPid") && !text.contains("fireCount"),
            "{text}"
        );
        let created = store.create("0 * * * *", "new", true).unwrap();
        assert_eq!(created.owner_pid, Some(std::process::id()));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"ownerPid\""), "{text}");
        assert!(!text.contains("ownerSessionId"), "{text}");
    }

    #[test]
    fn a_fire_is_sent_under_its_scheduled_task_line() {
        let dir = tempfile::tempdir().unwrap();
        put(&store_in(&dir), "30 9 * * *", "check the build", false, 0);
        let claimed = store_in(&dir).claim_due(&at(9, 30, 10)).unwrap();
        let fired = &claimed.fired[0];
        assert_eq!(
            fired.message(),
            format!(
                "⏰ Scheduled task {} (30 9 * * *, once)\ncheck the build",
                fired.id
            )
        );
        assert_eq!(claimed.fired_jobs[0].fire_message(), fired.message());
        assert_eq!(fired.prompt, "check the build");
    }

    #[test]
    fn a_slash_prompt_is_sent_unchanged_and_still_pairs_with_its_row() {
        let dir = tempfile::tempdir().unwrap();
        put(&store_in(&dir), "30 9 * * *", "/review the diff", true, 0);
        put(&store_in(&dir), "30 9 * * *", "  /loop 5m x", true, 0);
        let claimed = store_in(&dir).claim_due(&at(9, 30, 10)).unwrap();
        assert_eq!(claimed.fired[0].message(), "/review the diff");
        assert_eq!(claimed.fired[1].message(), "  /loop 5m x");
        assert!(claimed.fired.iter().all(Fired::is_slash));
        // The row matches on the same text the tick sends.
        assert_eq!(claimed.fired_jobs[0].fire_message(), "/review the diff");
    }

    #[test]
    fn process_alive_knows_this_process_and_rejects_impossible_pids() {
        assert!(process_alive(std::process::id()));
        #[cfg(unix)]
        {
            assert!(!process_alive(0));
            assert!(!process_alive(u32::MAX));
        }
    }

    #[test]
    fn ids_are_base36_of_time_and_counter() {
        assert_eq!(to_base36(0), "0");
        assert_eq!(to_base36(35), "z");
        assert_eq!(to_base36(36), "10");
        assert!(new_id().starts_with("task_"));
        assert_ne!(new_id(), new_id());
    }

    #[test]
    fn a_stale_lock_file_is_taken_over() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(&dir);
        std::fs::create_dir_all(dir.path().join(".agents")).unwrap();
        let lock = dir.path().join(".agents/scheduled_tasks.json.lock");
        std::fs::write(&lock, "").unwrap();
        let old = SystemTime::now() - Duration::from_secs(60);
        std::fs::File::options()
            .write(true)
            .open(&lock)
            .unwrap()
            .set_modified(old)
            .unwrap();
        store.create("* * * * *", "x", true).unwrap();
        assert!(!lock.exists());
    }
}

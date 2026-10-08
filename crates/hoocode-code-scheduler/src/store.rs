//! The scheduled-task store: `<cwd>/.agents/scheduled_tasks.json`, the same
//! file and shape hoocode-ts writes (`{"tasks": [...]}`), so both tools see
//! one set of schedules.
//!
//! Every operation re-reads the file, so a task created by another process is
//! seen. Mutations and fire claims run under a process-wide mutex and a lock
//! file (`<store>.lock`, created with `create_new`), so two processes that
//! share a store fire a due task once. Writes go to a temporary file that is
//! renamed over the store.

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
}

impl TaskStore {
    /// The store for a working directory.
    pub fn for_cwd(cwd: &Path) -> Self {
        Self {
            path: cwd.join(STORE_RELATIVE_PATH),
            legacy_path: Some(cwd.join(LEGACY_STORE_RELATIVE_PATH)),
        }
    }

    /// A store at an explicit path, with no legacy fallback.
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            legacy_path: None,
        }
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

    /// Run `f` holding the process lock and the lock file. If the lock file
    /// cannot be taken within [`LOCK_WAIT`], run `f` anyway, as TS does.
    fn with_lock<T>(&self, f: impl FnOnce() -> T) -> T {
        let _process = PROCESS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _file = FileLock::acquire(self.lock_path());
        f()
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
        };
        self.with_lock(|| {
            let mut tasks = self.read();
            tasks.push(task.clone());
            self.write(&tasks)
        })?;
        Ok(task)
    }

    /// `delete(id)`: true when a task with that id was removed.
    pub fn delete(&self, id: &str) -> io::Result<bool> {
        self.with_lock(|| {
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

    /// `tick(now)` without the idle check: claim every task due at `now` and
    /// return the prompts to submit, in stored order. A recurring task is
    /// marked with its minute so it fires once per minute; a one-shot task is
    /// removed. Callers check that the agent is idle first.
    pub fn claim_due(&self, now: &DateTime<Local>) -> io::Result<Vec<String>> {
        self.with_lock(|| {
            let minute_key = now.timestamp().div_euclid(60);
            let mut tasks = self.read();
            let mut fired = Vec::new();
            let mut spent = Vec::new();
            for task in tasks.iter_mut() {
                if task.last_run_minute == Some(minute_key) || !cron::matches(&task.cron, now) {
                    continue;
                }
                task.last_run_minute = Some(minute_key);
                fired.push(task.prompt.clone());
                if !task.recurring {
                    spent.push(task.id.clone());
                }
            }
            if !fired.is_empty() {
                tasks.retain(|t| !spent.contains(&t.id));
                self.write(&tasks)?;
            }
            Ok(fired)
        })
    }
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

    #[test]
    fn claim_fires_each_matching_minute_once_and_drops_one_shots() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(&dir);
        store.create("30 9 * * *", "recurring", true).unwrap();
        store.create("30 9 * * *", "once", false).unwrap();
        let due = Local
            .with_ymd_and_hms(2026, 10, 8, 9, 30, 10)
            .single()
            .unwrap();
        assert_eq!(store.claim_due(&due).unwrap(), vec!["recurring", "once"]);
        // A second tick in the same minute fires nothing.
        let later = Local
            .with_ymd_and_hms(2026, 10, 8, 9, 30, 40)
            .single()
            .unwrap();
        assert!(store.claim_due(&later).unwrap().is_empty());
        let tasks = store.list();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].prompt, "recurring");
        assert!(tasks[0].last_run_minute.is_some());
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

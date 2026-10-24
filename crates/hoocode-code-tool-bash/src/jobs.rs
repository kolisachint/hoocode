//! Background shell jobs: the registry behind `Shell` with `run_in_background`,
//! `ShellOutput` and `ShellKill` (docs/design/tui-activity.md, T5.1 and T5.3).
//!
//! A job is a shell command that runs detached from the turn. Its stdout and
//! stderr go to `<dir>/<id>.log`, where `dir` is the session's `jobs/` folder
//! (the caller picks it). The registry keeps every job's state, a kill request
//! per job, and the jobs that finished but were not yet announced to the model
//! ([`JobRegistry::take_completions`]). Jobs run on the `hoocode-io` runtime, so
//! a job outlives the tool call that started it.
//!
//! With [`JobRegistry::set_task_rows`], each job is a row of the task store
//! (`shell:<id>`, [`ROW_KEY_PREFIX`]): running while it runs, `Done` or `Failed`
//! when it exits, `Cancelled` when it is killed, and the row's action is `Abort`
//! while it runs. The panel's Abort stops the job without waiting. Job ids are
//! unique within the process, so rows of two sessions never share a key.

use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

use hoocode_code_task_store::{
    ActionNotifier, CreateTaskOptions, RowAction, RowActions, TaskPatch, TaskSource, TaskStatus,
    TaskStore,
};
use hoocode_code_tool_api::{node_fs_error, ToolError};
use hoocode_runtime::io_handle;
use process_wrap::tokio::ChildWrapper;

use crate::operations::{build_command_with_output, LocalBashOperations};
use crate::shell::{
    get_shell_config, track_detached_child_pid, untrack_detached_child_pid, ShellEnv,
};

/// Poll interval for exit and kill.
const POLL: Duration = Duration::from_millis(10);
/// How long [`JobRegistry::kill`] waits for the exit to be recorded.
const KILL_WAIT: Duration = Duration::from_secs(2);
/// How often a finished job's group is checked for processes that are still running.
const GROUP_POLL: Duration = Duration::from_millis(250);
/// The most bytes one [`JobRegistry::read`] returns.
pub const OUTPUT_READ_MAX: usize = 32 * 1024;
/// The prefix of a job's task-store row key: `shell:<job id>`.
pub const ROW_KEY_PREFIX: &str = "shell:";

/// The task-store key of a job's row.
pub fn row_key(job_id: &str) -> String {
    format!("{ROW_KEY_PREFIX}{job_id}")
}

/// Every registry of this process, for the calls that reach all jobs (the
/// quit prompt, the panel's Abort). Dead entries are dropped as they are seen.
static REGISTRIES: Mutex<Vec<Weak<Shared>>> = Mutex::new(Vec::new());
/// The last job number handed out in this process.
static LAST_JOB: AtomicU64 = AtomicU64::new(0);

/// Where a job stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    Running,
    /// The shell exited; `None` when it ended by a signal.
    Exited(Option<i32>),
    /// Stopped by [`JobRegistry::kill`] or [`JobRegistry::kill_all`].
    Killed,
}

impl JobState {
    pub fn is_running(self) -> bool {
        self == JobState::Running
    }

    /// A short phrase for tool output: `running`, `exited 0`, `killed`.
    pub fn describe(self) -> String {
        match self {
            JobState::Running => "running".into(),
            JobState::Exited(Some(code)) => format!("exited {code}"),
            JobState::Exited(None) => "exited by signal".into(),
            JobState::Killed => "killed".into(),
        }
    }
}

/// A job as the registry reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobInfo {
    /// `job-<n>`, unique within the process.
    pub id: String,
    pub command: String,
    pub log_path: PathBuf,
    pub state: JobState,
}

/// One read of a job's log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRead {
    pub text: String,
    /// The byte offset the text starts at.
    pub from: u64,
    /// The byte offset to pass as `since` next time.
    pub next: u64,
    /// The log's size when it was read.
    pub size: u64,
    pub state: JobState,
    /// Whether the job may still write to the log: it runs, or a process of its
    /// group is alive (see [`output_open`]). Only [`follow_log`] fills in the group
    /// part; [`read_log`] sets this from the job's state alone. The group is the
    /// only signal, so a descendant that leaves it (setsid, double fork) is not
    /// seen, and following can stop while it still writes. A follower stops only
    /// when this is false and the text is read to its end.
    pub open: bool,
}

struct Entry {
    info: JobInfo,
    kill: Arc<AtomicBool>,
    /// The job's process group (its shell's pid) while any process of the group
    /// may still run: a background descendant can outlive the shell.
    group: Option<u32>,
}

#[derive(Default)]
struct State {
    jobs: Vec<Entry>,
    /// Ids of jobs that finished and are not announced yet, in finish order.
    announce: Vec<String>,
}

struct Shared {
    dir: PathBuf,
    ops: LocalBashOperations,
    state: Mutex<State>,
    /// Where the jobs' rows go, once [`JobRegistry::set_task_rows`] has been called.
    rows: Mutex<Option<&'static TaskStore>>,
}

/// The background jobs of one session. Clones share the same jobs.
#[derive(Clone)]
pub struct JobRegistry {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for JobRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobRegistry")
            .field("dir", &self.shared.dir)
            .finish_non_exhaustive()
    }
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(|e| e.into_inner())
}

/// What the job task needs to start its shell.
struct Launch {
    shell: String,
    args: Vec<String>,
    command: String,
    cwd: PathBuf,
    env: ShellEnv,
    nice: u64,
}

impl JobRegistry {
    /// A registry whose logs go to `dir` (created on the first spawn). `ops`
    /// gives the shell and the niceness, as for the foreground Shell.
    pub fn new(dir: impl Into<PathBuf>, ops: LocalBashOperations) -> Self {
        let shared = Arc::new(Shared {
            dir: dir.into(),
            ops,
            state: Mutex::new(State::default()),
            rows: Mutex::new(None),
        });
        let mut registries = lock_list();
        registries.retain(|weak| weak.strong_count() > 0);
        registries.push(Arc::downgrade(&shared));
        drop(registries);
        Self { shared }
    }

    /// Publishes this registry's jobs as rows of `store` (see the module docs),
    /// and lets the store's Abort reach them. Call it once, when the session
    /// attaches to the store; jobs started before that have no rows.
    pub fn set_task_rows(&self, store: &'static TaskStore) {
        *lock_option(&self.shared.rows) = Some(store);
        store.set_row_actions(ROW_KEY_PREFIX, Some(Arc::new(ShellRows)));
    }

    /// The folder the logs are written to.
    pub fn dir(&self) -> &Path {
        &self.shared.dir
    }

    /// Starts `command` in `cwd` and returns at once. The job runs until it
    /// exits or is killed; its output goes to its log file. Fails when the
    /// shell cannot start, and then no job is recorded.
    pub fn spawn(&self, command: &str, cwd: &Path, env: ShellEnv) -> Result<JobInfo, ToolError> {
        let config = get_shell_config(self.shared.ops.shell_path.as_deref())?;
        if !cwd.exists() {
            return Err(format!(
                "Working directory does not exist: {}\nCannot execute bash commands.",
                cwd.display()
            )
            .into());
        }
        std::fs::create_dir_all(&self.shared.dir).map_err(|e| {
            format!(
                "Cannot create the jobs folder {}: {e}",
                self.shared.dir.display()
            )
        })?;

        let (id, log_path, stdout_log) = create_job_log(&self.shared.dir)?;
        // Both streams write to the one log file: the second is a duplicate of the
        // first, so they share an open file and O_APPEND keeps each write whole.
        let stderr_log = stdout_log
            .try_clone()
            .map_err(|e| format!("Cannot open {}: {e}", log_path.display()))?;

        let info = JobInfo {
            id: id.clone(),
            command: command.to_owned(),
            log_path,
            state: JobState::Running,
        };
        let kill = Arc::new(AtomicBool::new(false));
        lock(&self.shared.state).jobs.push(Entry {
            info: info.clone(),
            kill: kill.clone(),
            group: None,
        });
        // The row exists before the job can exit, so its exit is never published first.
        publish_start(&self.shared, &info);

        let launch = Launch {
            shell: config.shell,
            args: config.args,
            command: command.to_owned(),
            cwd: cwd.to_path_buf(),
            env,
            nice: self.shared.ops.nice,
        };
        let (started_tx, started_rx) = mpsc::sync_channel::<Result<(), ToolError>>(1);
        // Detached on purpose: the task reports through `started_tx`, then owns the job.
        drop(io_handle().spawn(run_job(
            self.shared.clone(),
            id.clone(),
            launch,
            kill,
            (stdout_log, stderr_log),
            started_tx,
        )));
        let failure = match started_rx.recv() {
            Ok(Ok(())) => return Ok(info),
            Ok(Err(error)) => error,
            Err(_) => "background job task ended before it started".into(),
        };
        lock(&self.shared.state).jobs.retain(|e| e.info.id != id);
        remove_row(&self.shared, &id);
        Err(failure)
    }

    /// The job with this id, if the registry started it.
    pub fn info(&self, id: &str) -> Option<JobInfo> {
        lock(&self.shared.state)
            .jobs
            .iter()
            .find(|e| e.info.id == id)
            .map(|e| e.info.clone())
    }

    /// Every job that is still running, oldest first.
    pub fn running_jobs(&self) -> Vec<JobInfo> {
        lock(&self.shared.state)
            .jobs
            .iter()
            .filter(|e| e.info.state.is_running())
            .map(|e| e.info.clone())
            .collect()
    }

    /// Asks every running job to stop and returns how many were asked. A
    /// finished job's leftover processes are signalled too. The exits are
    /// recorded shortly after; this does not wait for them.
    pub fn kill_all(&self) -> usize {
        let st = lock(&self.shared.state);
        let mut asked = 0;
        for entry in &st.jobs {
            if entry.info.state.is_running() {
                ask_stop(entry);
                asked += 1;
            } else if let Some(group) = entry.group {
                kill_group(group);
            }
        }
        asked
    }

    /// Stops one job. Returns its state after a short wait for the exit to be
    /// recorded. A job whose shell already exited is returned as it is, but
    /// the processes it left running in its group are signalled.
    pub fn kill(&self, id: &str) -> Result<JobInfo, ToolError> {
        {
            let st = lock(&self.shared.state);
            let entry = st
                .jobs
                .iter()
                .find(|e| e.info.id == id)
                .ok_or_else(|| format!("No background job with id {id}."))?;
            if entry.info.state.is_running() {
                ask_stop(entry);
            } else if let Some(group) = entry.group {
                kill_group(group);
            }
        }
        let deadline = Instant::now() + KILL_WAIT;
        loop {
            let info = self
                .info(id)
                .ok_or_else(|| format!("No background job with id {id}."))?;
            if !info.state.is_running() || Instant::now() >= deadline {
                return Ok(info);
            }
            std::thread::sleep(POLL);
        }
    }

    /// Reads a job's log. `since` is a byte offset (from an earlier read's
    /// `next`); without it, the last [`OUTPUT_READ_MAX`] bytes are returned.
    /// At most [`OUTPUT_READ_MAX`] bytes are read, and a read never ends inside
    /// a UTF-8 character, so `next` is always a safe place to resume.
    pub fn read(&self, id: &str, since: Option<u64>) -> Result<JobRead, ToolError> {
        let info = self
            .info(id)
            .ok_or_else(|| format!("No background job with id {id}."))?;
        read_log(&info.log_path, info.state, since)
    }

    /// The jobs that finished since the last call, in finish order. Each job
    /// is returned once; the caller tells the model about it.
    pub fn take_completions(&self) -> Vec<JobInfo> {
        let mut st = lock(&self.shared.state);
        let ids = std::mem::take(&mut st.announce);
        ids.iter()
            .filter_map(|id| st.jobs.iter().find(|e| &e.info.id == id))
            .map(|e| e.info.clone())
            .collect()
    }
}

/// Reads a job's log at `path`, which a job of state `state` writes (see
/// [`JobRegistry::read`] for `since`). It needs no registry, so a view can
/// follow a log off the UI thread.
pub fn read_log(path: &Path, state: JobState, since: Option<u64>) -> Result<JobRead, ToolError> {
    let unreadable = |e: std::io::Error| format!("Cannot read {}: {e}", path.display());
    let mut file = std::fs::File::open(path).map_err(unreadable)?;
    let size = file.metadata().map(|m| m.len()).unwrap_or(0);
    let mut from = match since {
        Some(offset) => offset.min(size),
        None => size.saturating_sub(OUTPUT_READ_MAX as u64),
    };
    file.seek(SeekFrom::Start(from)).map_err(unreadable)?;
    let mut buf = Vec::new();
    file.take(OUTPUT_READ_MAX as u64)
        .read_to_end(&mut buf)
        .map_err(unreadable)?;
    // A tail that starts inside a character starts after its continuation bytes.
    if since.is_none() && from > 0 {
        let skip = buf
            .iter()
            .take(3)
            .take_while(|b| **b & 0xC0 == 0x80)
            .count();
        buf.drain(..skip);
        from += skip as u64;
    }
    let whole = match std::str::from_utf8(&buf) {
        Ok(_) => buf.len(),
        // Cut inside a character at the end: leave it for the next read, unless
        // the job has finished (nothing more will complete it).
        Err(e) if e.error_len().is_none() && state.is_running() => e.valid_up_to(),
        Err(_) => buf.len(),
    };
    Ok(JobRead {
        text: String::from_utf8_lossy(&buf[..whole]).into_owned(),
        from,
        next: from + whole as u64,
        size,
        state,
        open: state.is_running(),
    })
}

/// Whether a job of this process may still write to `log_path`: the job runs,
/// or a process of its group is still alive. A background descendant can outlive
/// the shell, so the output stays open after the shell exits (T5.3). `None` when
/// no job of this process has that log.
pub fn output_open(log_path: &Path) -> Option<bool> {
    live_registries().iter().find_map(|shared| {
        lock(&shared.state)
            .jobs
            .iter()
            .find(|e| e.info.log_path == log_path)
            .map(|e| e.info.state.is_running() || e.group.is_some())
    })
}

/// How many jobs are running in this process, in every registry.
pub fn running_job_count() -> usize {
    running_jobs().len()
}

/// Every running job in this process, in every registry (the end-of-shift
/// card lists them as satellites).
pub fn running_jobs() -> Vec<JobInfo> {
    live_registries()
        .iter()
        .flat_map(|shared| {
            lock(&shared.state)
                .jobs
                .iter()
                .filter(|e| e.info.state.is_running())
                .map(|e| e.info.clone())
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Asks every running job in this process to stop now, and signals their
/// process groups, so the kills land even when the process exits next. Returns
/// how many were asked. The exits are not waited for.
pub fn kill_all_jobs() -> usize {
    live_registries()
        .iter()
        .map(|shared| {
            let st = lock(&shared.state);
            let mut asked = 0;
            for entry in &st.jobs {
                if entry.info.state.is_running() {
                    ask_stop(entry);
                    asked += 1;
                } else if let Some(group) = entry.group {
                    // A finished job's leftover processes are signalled too.
                    kill_group(group);
                }
            }
            asked
        })
        .sum()
}

/// The registries that are still alive, after dropping the dead ones.
fn live_registries() -> Vec<Arc<Shared>> {
    let mut registries = lock_list();
    registries.retain(|weak| weak.strong_count() > 0);
    registries.iter().filter_map(Weak::upgrade).collect()
}

fn lock_list() -> MutexGuard<'static, Vec<Weak<Shared>>> {
    REGISTRIES.lock().unwrap_or_else(|e| e.into_inner())
}

fn lock_option<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// Asks a running job to stop, without waiting: its shell is killed by its own
/// task, and its process group is signalled now.
fn ask_stop(entry: &Entry) {
    entry.kill.store(true, Ordering::SeqCst);
    if let Some(group) = entry.group {
        kill_group(group);
    }
}

/// The panel's Abort on a job row: stops the job, and fails when it has ended.
struct ShellRows;

impl RowActions for ShellRows {
    fn perform(&self, key: &str, action: RowAction, _notify: ActionNotifier) -> Result<(), String> {
        let id = key
            .strip_prefix(ROW_KEY_PREFIX)
            .filter(|_| action == RowAction::Abort)
            .ok_or_else(|| "Only a shell job row can be stopped.".to_owned())?;
        for shared in live_registries() {
            let st = lock(&shared.state);
            if let Some(entry) = st.jobs.iter().find(|e| e.info.id == id) {
                if !entry.info.state.is_running() {
                    return Err("That job already finished.".into());
                }
                ask_stop(entry);
                return Ok(());
            }
        }
        Err("That job is gone.".into())
    }
}

/// The task-store row of a job that has just started: running, with Abort.
fn publish_start(shared: &Shared, info: &JobInfo) {
    let Some(store) = *lock_option(&shared.rows) else {
        return;
    };
    let key = row_key(&info.id);
    store.upsert_keyed(
        &key,
        &info.command,
        CreateTaskOptions {
            source: Some(TaskSource::Shell),
            key: Some(key.clone()),
            log_path: Some(info.log_path.clone()),
            ..Default::default()
        },
        TaskPatch {
            status: Some(TaskStatus::InProgress),
            note: Some(None),
            action: Some(Some(RowAction::Abort)),
            ..Default::default()
        },
    );
}

/// The job's row once it has ended: `Done`, `Failed` (with the exit status as
/// the note), or `Cancelled` when it was killed. The row stays, so its log can
/// still be opened from the panel.
fn publish_end(shared: &Shared, id: &str, state: JobState) {
    let Some(store) = *lock_option(&shared.rows) else {
        return;
    };
    let (status, note) = match state {
        JobState::Running => return,
        JobState::Exited(Some(0)) => (TaskStatus::Done, None),
        JobState::Exited(Some(code)) => (TaskStatus::Failed, Some(format!("exit {code}"))),
        JobState::Exited(None) => (TaskStatus::Failed, Some("ended by a signal".to_owned())),
        JobState::Killed => (TaskStatus::Cancelled, None),
    };
    if let Some(task) = store.find_keyed(&row_key(id)) {
        store.update(
            task.id,
            TaskPatch {
                status: Some(status),
                note: Some(note),
                action: Some(None),
                ..Default::default()
            },
        );
    }
}

/// Removes a job's row (its job did not start).
fn remove_row(shared: &Shared, id: &str) {
    if let Some(store) = *lock_option(&shared.rows) {
        store.remove_keyed(&row_key(id));
    }
}

/// The most reads one [`follow_log`] call makes, so it returns at most
/// `FOLLOW_CHUNKS * OUTPUT_READ_MAX` bytes and the view keeps up with a fast job.
const FOLLOW_CHUNKS: usize = 8;
/// How far behind a follower may be before it skips to the tail of the log.
pub const FOLLOW_LAG_MAX: u64 = 2 * OUTPUT_READ_MAX as u64;

/// Reads a log for a view that follows it: from `since` (or the tail when `None`),
/// then on in bounded chunks until the end is reached. A follower more than
/// [`FOLLOW_LAG_MAX`] behind starts at the tail instead; the text it skipped is
/// not returned, and `from` says where the returned text starts.
pub fn follow_log(path: &Path, state: JobState, since: Option<u64>) -> Result<JobRead, ToolError> {
    // Asked before the log is read: output that comes after this point is left for
    // the next read. A job that reports closed has written everything already, as
    // far as its process group shows: a descendant that left the group is not seen,
    // so output can still follow after it reports closed.
    let open = output_open(path).unwrap_or(state.is_running());
    let state = if open { JobState::Running } else { state };
    let size = std::fs::metadata(path)
        .map(|m| m.len())
        .map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    let start = since.filter(|offset| size.saturating_sub(*offset) <= FOLLOW_LAG_MAX);
    let mut read = read_log(path, state, start)?;
    read.open = open;
    for _ in 1..FOLLOW_CHUNKS {
        if read.next >= read.size {
            break;
        }
        let more = read_log(path, state, Some(read.next))?;
        if more.next == read.next {
            break;
        }
        read.text.push_str(&more.text);
        read.next = more.next;
        read.size = more.size;
    }
    Ok(read)
}

/// Takes the next job id and creates its log file, which must be new. A log that
/// already exists (a job of the same number from an earlier process that wrote to
/// this folder) is skipped, so no job ever appends to another's log.
fn create_job_log(dir: &Path) -> Result<(String, PathBuf, std::fs::File), ToolError> {
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("Cannot create the jobs folder {}: {e}", dir.display()))?;
    loop {
        let id = format!("job-{}", LAST_JOB.fetch_add(1, Ordering::SeqCst) + 1);
        let path = dir.join(format!("{id}.log"));
        match OpenOptions::new().create_new(true).append(true).open(&path) {
            Ok(file) => return Ok((id, path, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("Cannot open {}: {e}", path.display()).into()),
        }
    }
}

/// The notice the model gets when a job finishes.
pub fn completion_notice(info: &JobInfo) -> String {
    let outcome = match info.state {
        JobState::Exited(Some(0)) => "finished (exit 0)".to_owned(),
        JobState::Exited(Some(code)) => format!("finished (exit {code})"),
        JobState::Exited(None) => "ended by a signal".to_owned(),
        JobState::Killed => "was killed".to_owned(),
        JobState::Running => "is still running".to_owned(),
    };
    format!(
        "Background shell job {} {outcome}: `{}`. Its output is in {}; read it with ShellOutput.",
        info.id,
        info.command,
        info.log_path.display()
    )
}

/// Starts the job's shell, with its output going straight to the log, and records its exit.
async fn run_job(
    shared: Arc<Shared>,
    id: String,
    launch: Launch,
    kill: Arc<AtomicBool>,
    logs: (std::fs::File, std::fs::File),
    started: mpsc::SyncSender<Result<(), ToolError>>,
) {
    let (stdout_log, stderr_log) = logs;
    let Launch {
        shell,
        args,
        command,
        cwd,
        env,
        nice,
    } = launch;
    let mut wrap = build_command_with_output(
        &shell,
        &args,
        &command,
        &cwd,
        &env,
        nice,
        (
            std::process::Stdio::from(stdout_log),
            std::process::Stdio::from(stderr_log),
        ),
    );
    let mut child = match wrap.spawn() {
        Ok(child) => child,
        Err(e) => {
            let code = node_fs_error(e, "spawn", None).code;
            let _ = started.send(Err(format!("spawn {shell} {code}").into()));
            return;
        }
    };
    // Without a process id there is no group to signal, and killpg(0) would signal
    // this process, so the job is not started.
    let Some(pid) = child.id() else {
        stop_and_reap(&mut *child).await;
        let _ = started.send(Err("spawn: the shell has no process id".into()));
        return;
    };
    set_group(&shared, &id, Some(pid));
    let _ = started.send(Ok(()));
    track_detached_child_pid(pid);

    let mut killed = false;
    let code = loop {
        if !killed && kill.load(Ordering::SeqCst) {
            let _ = child.start_kill();
            killed = true;
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                // The flag was read above, before this exit was seen. A kill that
                // landed in between set the flag before it signalled the shell, so
                // reading it again catches an exit that the kill caused.
                killed |= kill.load(Ordering::SeqCst);
                break status.code();
            }
            Ok(None) => {}
            Err(_) => break None,
        }
        tokio::time::sleep(POLL).await;
    };

    untrack_detached_child_pid(pid);

    let state = if killed {
        JobState::Killed
    } else {
        JobState::Exited(code)
    };
    {
        let mut st = lock(&shared.state);
        if let Some(entry) = st.jobs.iter_mut().find(|e| e.info.id == id) {
            entry.info.state = state;
        }
        st.announce.push(id.clone());
    }
    publish_end(&shared, &id, state);

    // The shell is gone, but a background descendant may still run in its group.
    // Keep the group id until the group is empty, so ShellKill can still reach it.
    // The check and the clearing happen under the state lock, where every kill
    // reads the group id, so no kill signals a group id that is already free.
    loop {
        {
            let mut st = lock(&shared.state);
            if !group_alive(pid) {
                if let Some(entry) = st.jobs.iter_mut().find(|e| e.info.id == id) {
                    entry.group = None;
                }
                break;
            }
        }
        tokio::time::sleep(GROUP_POLL).await;
    }
}

/// Kills a child that is not going to be used and waits for its exit, so the
/// process is reaped and no zombie is left behind.
async fn stop_and_reap(child: &mut dyn ChildWrapper) {
    let _ = child.start_kill();
    let _ = child.wait().await;
}

fn set_group(shared: &Shared, id: &str, group: Option<u32>) {
    if let Some(entry) = lock(&shared.state)
        .jobs
        .iter_mut()
        .find(|e| e.info.id == id)
    {
        entry.group = group;
    }
}

/// Sends SIGKILL to the process group `pgid` (a job's shell and its descendants).
#[cfg(unix)]
fn kill_group(pgid: u32) {
    if let Ok(pgid) = libc::pid_t::try_from(pgid) {
        // SAFETY: killpg(2) with a group id and a signal number has no memory effects.
        unsafe {
            libc::killpg(pgid, libc::SIGKILL);
        }
    }
}

/// Whether the group `pgid` has a process that is not a zombie. `killpg(pgid, 0)`
/// also succeeds while only zombies remain (a grandchild reparented to an init
/// that does not reap), so on Linux the members are read from /proc instead.
#[cfg(target_os = "linux")]
fn group_alive(pgid: u32) -> bool {
    let Ok(pgid) = libc::pid_t::try_from(pgid) else {
        return false;
    };
    proc_group_alive(pgid).unwrap_or_else(|| killpg_alive(pgid))
}

/// Whether any process is still in the group `pgid`.
#[cfg(all(unix, not(target_os = "linux")))]
fn group_alive(pgid: u32) -> bool {
    libc::pid_t::try_from(pgid).is_ok_and(killpg_alive)
}

/// `killpg(pgid, 0)`: whether the group exists, zombies included.
#[cfg(unix)]
fn killpg_alive(pgid: libc::pid_t) -> bool {
    // SAFETY: signal 0 only checks that the group exists.
    let rc = unsafe { libc::killpg(pgid, 0) };
    rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Whether a process of group `pgid` that is not a zombie is listed in /proc.
/// `None` when /proc cannot be read.
#[cfg(target_os = "linux")]
fn proc_group_alive(pgid: libc::pid_t) -> Option<bool> {
    let entries = std::fs::read_dir("/proc").ok()?;
    let alive = entries.flatten().any(|entry| {
        std::fs::read_to_string(entry.path().join("stat"))
            .is_ok_and(|stat| stat_in_live_group(&stat, pgid))
    });
    Some(alive)
}

/// Reads a /proc/<pid>/stat line: whether the process is in group `pgid` and is
/// not a zombie. The command name (field 2) sits in parentheses and may hold
/// spaces or `)`, so the fields are read after its last `)`: state is field 3,
/// pgrp is field 5.
#[cfg(target_os = "linux")]
fn stat_in_live_group(stat: &str, pgid: libc::pid_t) -> bool {
    let Some((_, rest)) = stat.rsplit_once(')') else {
        return false;
    };
    let mut fields = rest.split_whitespace();
    let state = fields.next();
    let pgrp = fields.nth(1).and_then(|f| f.parse::<libc::pid_t>().ok());
    state != Some("Z") && pgrp == Some(pgid)
}

// Job groups are a unix notion; a Windows job object ends with its shell.
#[cfg(not(unix))]
fn kill_group(_pgid: u32) {}

#[cfg(not(unix))]
fn group_alive(_pgid: u32) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::{get_shell_config, get_shell_env};

    /// Lives here, not in `tests/it`: [`kill_all_jobs`] reaches every registry of the
    /// process, and the integration tests run jobs of their own in parallel.
    #[test]
    fn kill_all_jobs_stops_every_running_job_of_the_process() {
        let dir = tempfile::tempdir().expect("temp dir");
        let jobs = JobRegistry::new(dir.path().join("jobs"), LocalBashOperations::new(None));
        let a = jobs
            .spawn("sleep 30", dir.path(), get_shell_env())
            .expect("spawn");
        let b = jobs
            .spawn("sleep 30", dir.path(), get_shell_env())
            .expect("spawn");
        assert!(running_job_count() >= 2);
        assert!(kill_all_jobs() >= 2);
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline
            && [&a.id, &b.id]
                .iter()
                .any(|id| jobs.info(id).is_some_and(|i| i.state.is_running()))
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(jobs.info(&a.id).map(|i| i.state), Some(JobState::Killed));
        assert_eq!(jobs.info(&b.id).map(|i| i.state), Some(JobState::Killed));
    }

    /// The no-pid branch of `run_job` cannot be reached with a real process, so
    /// this checks the step it runs: the child is killed and reaped. A zombie
    /// still answers `kill(pid, 0)`, so the pid being gone shows it was waited on.
    #[cfg(unix)]
    #[test]
    fn a_child_that_is_stopped_before_use_is_reaped() {
        let dir = tempfile::tempdir().expect("temp dir");
        let config = get_shell_config(None).expect("a shell");
        // Spawned and stopped on the runtime, as `run_job` does (tokio needs its context).
        let pid = io_handle().block_on(async {
            let mut wrap = build_command_with_output(
                &config.shell,
                &config.args,
                "sleep 30",
                dir.path(),
                &get_shell_env(),
                0,
                (std::process::Stdio::null(), std::process::Stdio::null()),
            );
            let mut child = wrap.spawn().expect("spawn");
            let pid = child.id().expect("a started child has a pid");
            assert_eq!(unsafe { libc::kill(pid as libc::pid_t, 0) }, 0, "alive");
            stop_and_reap(&mut *child).await;
            pid
        });

        let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
        let errno = std::io::Error::last_os_error().raw_os_error();
        assert_eq!(
            (rc, errno),
            (-1, Some(libc::ESRCH)),
            "the child is reaped, not a zombie"
        );
    }

    /// A group whose leader exited but is not waited on has only a zombie left.
    /// `killpg` still succeeds for it, so [`group_alive`] must read /proc instead.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_group_with_only_a_zombie_is_not_alive() {
        use std::os::unix::process::CommandExt;
        let mut child = std::process::Command::new("sh")
            .args(["-c", "exit 0"])
            .process_group(0)
            .spawn()
            .expect("spawn");
        let pgid = libc::pid_t::try_from(child.id()).expect("pid");
        let is_zombie = || {
            std::fs::read_to_string(format!("/proc/{pgid}/stat")).is_ok_and(|stat| {
                stat.rsplit_once(')')
                    .and_then(|(_, rest)| rest.split_whitespace().next())
                    == Some("Z")
            })
        };
        let deadline = Instant::now() + Duration::from_secs(15);
        while !is_zombie() {
            assert!(Instant::now() < deadline, "the child never became a zombie");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(killpg_alive(pgid), "killpg still sees the zombie");
        assert!(
            !group_alive(pgid as u32),
            "a zombie-only group is not alive"
        );
        child.wait().expect("reap");
    }

    /// A group with a running member is alive until that member exits.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_group_with_a_running_member_is_alive() {
        use std::os::unix::process::CommandExt;
        let mut child = std::process::Command::new("sh")
            .args(["-c", "sleep 30"])
            .process_group(0)
            .spawn()
            .expect("spawn");
        let pgid = child.id();
        assert!(group_alive(pgid), "the running member is seen");
        // SAFETY: killpg(2) with a group id and a signal number has no memory effects.
        unsafe {
            libc::killpg(libc::pid_t::try_from(pgid).expect("pid"), libc::SIGKILL);
        }
        child.wait().expect("reap");
        assert!(!group_alive(pgid), "the reaped group is gone");
    }
}

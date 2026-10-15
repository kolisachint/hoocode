//! Background shell jobs: the registry behind `Shell` with `run_in_background`,
//! `ShellOutput` and `ShellKill` (docs/design/tui-activity.md, T5.1).
//!
//! A job is a shell command that runs detached from the turn. Its stdout and
//! stderr go to `<dir>/<id>.log`, where `dir` is the session's `jobs/` folder
//! (the caller picks it). The registry keeps every job's state, a kill request
//! per job, and the jobs that finished but were not yet announced to the model
//! ([`JobRegistry::take_completions`]). Jobs run on the `hoocode-io` runtime, so
//! a job outlives the tool call that started it.

use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use hoocode_code_tool_api::{node_fs_error, ToolError};
use hoocode_runtime::io_handle;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::task::JoinHandle;

use crate::operations::{build_command, LocalBashOperations};
use crate::shell::{
    get_shell_config, track_detached_child_pid, untrack_detached_child_pid, ShellEnv,
};

/// Poll interval for exit and kill.
const POLL: Duration = Duration::from_millis(10);
/// How long a job's pipes may stay open after its shell exits (a detached
/// descendant can hold them forever).
const EXIT_STDIO_GRACE: Duration = Duration::from_millis(100);
const READ_BUF: usize = 16 * 1024;
/// How long [`JobRegistry::kill`] waits for the exit to be recorded.
const KILL_WAIT: Duration = Duration::from_secs(2);
/// How often a finished job's group is checked for processes that are still running.
const GROUP_POLL: Duration = Duration::from_millis(250);
/// The most bytes one [`JobRegistry::read`] returns.
pub const OUTPUT_READ_MAX: usize = 32 * 1024;

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
    /// `job-<n>`, unique within the registry.
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
    next: u64,
    jobs: Vec<Entry>,
    /// Ids of jobs that finished and are not announced yet, in finish order.
    announce: Vec<String>,
}

struct Shared {
    dir: PathBuf,
    ops: LocalBashOperations,
    state: Mutex<State>,
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
        Self {
            shared: Arc::new(Shared {
                dir: dir.into(),
                ops,
                state: Mutex::new(State::default()),
            }),
        }
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

        let id = {
            let mut st = lock(&self.shared.state);
            st.next += 1;
            format!("job-{}", st.next)
        };
        let log_path = self.shared.dir.join(format!("{id}.log"));
        let open_log = || {
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
                .map_err(|e| format!("Cannot open {}: {e}", log_path.display()))
        };
        // Both pipes append to the same file; O_APPEND keeps each write whole.
        let stdout_log = open_log()?;
        let stderr_log = open_log()?;

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
                entry.kill.store(true, Ordering::SeqCst);
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
                entry.kill.store(true, Ordering::SeqCst);
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
        let unreadable =
            |e: std::io::Error| format!("Cannot read {}: {e}", info.log_path.display());
        let mut file = std::fs::File::open(&info.log_path).map_err(unreadable)?;
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
            Err(e) if e.error_len().is_none() && info.state.is_running() => e.valid_up_to(),
            Err(_) => buf.len(),
        };
        Ok(JobRead {
            text: String::from_utf8_lossy(&buf[..whole]).into_owned(),
            from,
            next: from + whole as u64,
            size,
            state: info.state,
        })
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

/// Starts the job's shell, copies its output to the log, and records its exit.
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
    let mut wrap = build_command(&shell, &args, &command, &cwd, &env, nice);
    let mut child = match wrap.spawn() {
        Ok(child) => child,
        Err(e) => {
            let code = node_fs_error(e, "spawn", None).code;
            let _ = started.send(Err(format!("spawn {shell} {code}").into()));
            return;
        }
    };
    let pid = child.id().unwrap_or_default();
    set_group(&shared, &id, Some(pid));
    let _ = started.send(Ok(()));
    track_detached_child_pid(pid);
    let mut readers: Vec<JoinHandle<()>> = Vec::with_capacity(2);
    if let Some(stdout) = child.stdout().take() {
        readers.push(tokio::spawn(copy_to_log(
            stdout,
            tokio::fs::File::from_std(stdout_log),
        )));
    }
    if let Some(stderr) = child.stderr().take() {
        readers.push(tokio::spawn(copy_to_log(
            stderr,
            tokio::fs::File::from_std(stderr_log),
        )));
    }

    let mut killed = false;
    let code = loop {
        if !killed && kill.load(Ordering::SeqCst) {
            let _ = child.start_kill();
            killed = true;
        }
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) => {}
            Err(_) => break None,
        }
        tokio::time::sleep(POLL).await;
    };

    let _ = tokio::time::timeout(EXIT_STDIO_GRACE, async {
        for reader in &mut readers {
            let _ = reader.await;
        }
    })
    .await;
    readers.iter().for_each(JoinHandle::abort);
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

    // The shell is gone, but a background descendant may still run in its group.
    // Keep the group id until the group is empty, so ShellKill can still reach it.
    while group_alive(pid) {
        tokio::time::sleep(GROUP_POLL).await;
    }
    set_group(&shared, &id, None);
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

/// Whether any process is still in the group `pgid`.
#[cfg(unix)]
fn group_alive(pgid: u32) -> bool {
    let Ok(pgid) = libc::pid_t::try_from(pgid) else {
        return false;
    };
    // SAFETY: signal 0 only checks that the group exists.
    let rc = unsafe { libc::killpg(pgid, 0) };
    rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

// Job groups are a unix notion; a Windows job object ends with its shell.
#[cfg(not(unix))]
fn kill_group(_pgid: u32) {}

#[cfg(not(unix))]
fn group_alive(_pgid: u32) -> bool {
    false
}

/// Copies one pipe into the log until the pipe closes.
async fn copy_to_log<R>(mut pipe: R, mut log: tokio::fs::File)
where
    R: AsyncRead + Unpin,
{
    let mut buf = vec![0u8; READ_BUF];
    loop {
        match pipe.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if log.write_all(&buf[..n]).await.is_err() {
                    break;
                }
            }
        }
    }
    let _ = log.flush().await;
}

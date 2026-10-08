//! `BashOperations` and `createLocalBashOperations` (core/tools/bash.ts).
//!
//! The local shell runs on the `hoocode-io` runtime: its pipes are read by async
//! tasks there (no thread per pipe). The caller is sync (a tool body) and waits
//! on a bounded channel with a 10 ms timeout; each timeout is an idle tick for
//! the caller's throttle timer, so no thread is needed for that either.

use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::sync::mpsc::{self, RecvTimeoutError, TrySendError};
use std::time::Duration;

use hoocode_ai_types::AbortSignal;
use hoocode_code_tool_api::{js_number, node_fs_error, ToolError};
use hoocode_runtime::io_handle;
use process_wrap::tokio::{CommandWrap, ProcessGroup};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::task::JoinHandle;

use crate::shell::{
    get_shell_config, get_shell_env, track_detached_child_pid, untrack_detached_child_pid, ShellEnv,
};

/// Options for [`BashOperations::exec`].
pub struct BashExecOptions<'a> {
    /// Receives stdout and stderr chunks as they arrive.
    pub on_data: &'a mut dyn FnMut(&[u8]),
    /// Called about every 10 ms while the command runs and no output arrives
    /// (the tool's output throttle timer runs here, not on a thread of its own).
    pub on_idle: Option<&'a mut dyn FnMut()>,
    pub signal: Option<AbortSignal>,
    /// Seconds; `None` or <= 0 means no timeout.
    pub timeout: Option<f64>,
    /// The environment; `None` means [`get_shell_env`].
    pub env: Option<ShellEnv>,
}

/// `BashOperations`: how a command runs. Override to run commands elsewhere
/// (for example over SSH).
///
/// `exec` returns the exit code (`None` when the process was killed by a
/// signal). It fails with `aborted` when the signal fired and
/// `timeout:<seconds>` when the timeout hit; the tool words those for the
/// model.
pub trait BashOperations: Send + Sync {
    fn exec(
        &self,
        command: &str,
        cwd: &Path,
        options: BashExecOptions<'_>,
    ) -> Result<Option<i32>, ToolError>;
}

/// `createLocalBashOperations`: the local shell.
#[derive(Debug, Clone, Default)]
pub struct LocalBashOperations {
    pub shell_path: Option<String>,
}

impl LocalBashOperations {
    pub fn new(shell_path: Option<String>) -> Self {
        Self { shell_path }
    }
}

/// How long to wait for stdout/stderr to close after the shell exits. A
/// backgrounded descendant can hold the pipes open forever
/// (`waitForChildProcess`'s grace period).
const EXIT_STDIO_GRACE: Duration = Duration::from_millis(100);
/// Poll interval for exit, abort and timeout, and the caller's idle tick.
const POLL: Duration = Duration::from_millis(10);
/// Chunks queued between the pipe readers and the caller.
const CHUNK_QUEUE: usize = 256;
/// Retry delay for a reader when the caller's queue is full.
const QUEUE_FULL_WAIT: Duration = Duration::from_millis(1);
const READ_BUF: usize = 16 * 1024;

type Outcome = Result<Option<i32>, ToolError>;

/// Everything the async side needs to start one command.
struct Launch {
    shell: String,
    args: Vec<String>,
    command: String,
    cwd: PathBuf,
    env: ShellEnv,
    signal: Option<AbortSignal>,
    timeout: Option<f64>,
}

impl BashOperations for LocalBashOperations {
    fn exec(
        &self,
        command: &str,
        cwd: &Path,
        options: BashExecOptions<'_>,
    ) -> Result<Option<i32>, ToolError> {
        let BashExecOptions {
            on_data,
            mut on_idle,
            signal,
            timeout,
            env,
        } = options;
        let config = get_shell_config(self.shell_path.as_deref())?;
        if !cwd.exists() {
            return Err(format!(
                "Working directory does not exist: {}\nCannot execute bash commands.",
                cwd.display()
            )
            .into());
        }

        let launch = Launch {
            shell: config.shell,
            args: config.args,
            command: command.to_owned(),
            cwd: cwd.to_path_buf(),
            env: env.unwrap_or_else(get_shell_env),
            signal,
            timeout,
        };
        let (chunks_tx, chunks_rx) = mpsc::sync_channel::<Vec<u8>>(CHUNK_QUEUE);
        let (outcome_tx, outcome_rx) = mpsc::sync_channel::<Outcome>(1);
        // Detached on purpose: the task reports through `outcome_tx`.
        drop(io_handle().spawn(run_launch(launch, chunks_tx, outcome_tx)));

        let outcome = loop {
            match chunks_rx.recv_timeout(POLL) {
                Ok(chunk) => on_data(&chunk),
                Err(RecvTimeoutError::Timeout) => {}
                // Every pipe is closed but the process may still run: wait for
                // the outcome. Abort and timeout are applied by the task.
                Err(RecvTimeoutError::Disconnected) => match outcome_rx.recv_timeout(POLL) {
                    Ok(outcome) => break outcome,
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => {
                        break Err("shell task ended without a result".into())
                    }
                },
            }
            if let Some(idle) = on_idle.as_mut() {
                idle();
            }
        };
        outcome
    }
}

/// Runs one command on the `hoocode-io` runtime and sends its outcome.
async fn run_launch(
    launch: Launch,
    chunks: mpsc::SyncSender<Vec<u8>>,
    outcome: mpsc::SyncSender<Outcome>,
) {
    let result = run_child(launch, chunks).await;
    let _ = outcome.send(result);
}

async fn run_child(launch: Launch, chunks: mpsc::SyncSender<Vec<u8>>) -> Outcome {
    let Launch {
        shell,
        args,
        command,
        cwd,
        env,
        signal,
        timeout,
    } = launch;
    let mut wrap = CommandWrap::with_new(&shell, |cmd| {
        cmd.args(&args)
            .arg(&command)
            .current_dir(&cwd)
            .env_clear()
            .envs(&env)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
    });
    #[cfg(unix)]
    wrap.wrap(ProcessGroup::leader());
    #[cfg(windows)]
    wrap.wrap(process_wrap::tokio::JobObject);
    let mut child = wrap.spawn().map_err(|e| {
        let code = node_fs_error(e, "spawn", None).code;
        format!("spawn {shell} {code}")
    })?;
    let pid = child.id().unwrap_or_default();
    track_detached_child_pid(pid);

    let mut readers: Vec<JoinHandle<()>> = Vec::with_capacity(2);
    if let Some(stdout) = child.stdout().take() {
        readers.push(tokio::spawn(read_pipe(stdout, chunks.clone())));
    }
    if let Some(stderr) = child.stderr().take() {
        readers.push(tokio::spawn(read_pipe(stderr, chunks.clone())));
    }
    drop(chunks);

    let deadline = timeout
        .filter(|t| *t > 0.0)
        .map(|t| tokio::time::Instant::now() + Duration::from_secs_f64(t));
    let mut timed_out = false;
    let mut killed = false;
    let status: ExitStatus = loop {
        if !killed {
            let aborted = signal.as_ref().is_some_and(AbortSignal::aborted);
            let expired = deadline.is_some_and(|d| tokio::time::Instant::now() >= d);
            if aborted || expired {
                timed_out = expired && !aborted;
                let _ = child.start_kill();
                killed = true;
            }
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(e) => {
                untrack_detached_child_pid(pid);
                readers.iter().for_each(JoinHandle::abort);
                return Err(e.into());
            }
        }
        tokio::time::sleep(POLL).await;
    };

    // The shell has exited. Give its pipes a grace period to close, then stop
    // reading (a background descendant may still hold them open).
    let _ = tokio::time::timeout(EXIT_STDIO_GRACE, async {
        for reader in &mut readers {
            let _ = reader.await;
        }
    })
    .await;
    readers.iter().for_each(JoinHandle::abort);
    untrack_detached_child_pid(pid);

    if signal.as_ref().is_some_and(AbortSignal::aborted) {
        return Err("aborted".into());
    }
    if timed_out {
        return Err(format!("timeout:{}", js_number(timeout.unwrap_or_default())).into());
    }
    Ok(status.code())
}

/// Reads one pipe to its end and queues each chunk for the caller.
async fn read_pipe<R>(mut pipe: R, chunks: mpsc::SyncSender<Vec<u8>>)
where
    R: AsyncRead + Unpin,
{
    let mut buf = vec![0u8; READ_BUF];
    loop {
        match pipe.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                if !queue_chunk(&chunks, buf[..n].to_vec()).await {
                    return;
                }
            }
        }
    }
}

/// Queues a chunk without blocking the runtime. Returns false when the caller
/// has gone.
async fn queue_chunk(chunks: &mpsc::SyncSender<Vec<u8>>, mut chunk: Vec<u8>) -> bool {
    loop {
        match chunks.try_send(chunk) {
            Ok(()) => return true,
            Err(TrySendError::Full(back)) => {
                chunk = back;
                tokio::time::sleep(QUEUE_FULL_WAIT).await;
            }
            Err(TrySendError::Disconnected(_)) => return false,
        }
    }
}

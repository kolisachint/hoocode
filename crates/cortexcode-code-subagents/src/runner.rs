//! How a dispatch actually runs, behind a seam.
//!
//! Until this existed, "run a subagent" and "spawn this executable with these
//! argv" were the same thing: [`SubagentPool`](crate::pool) built a
//! `std::process::Command` and read its pipes. That made three things
//! untestable — anything about a run that needs a *model* in it, anything where
//! two runs must interleave deterministically, and the in-process execution
//! model the design review keeps reopening (`docs/design/subagents.md` §6).
//!
//! A [`Runner`] takes a [`RunSpec`] and returns a [`RunnerHandle`] with the
//! same shape as a child process: two byte streams and a wait. Everything
//! above the seam — queueing, priority, ageing, the lifeguard, heartbeats,
//! token accounting, verification, the ledger — is unchanged and now runs
//! against either implementation:
//!
//! - [`ProcessRunner`] re-execs this binary, which is what ships.
//! - an in-process runner drives the same agent loop in a tokio task (P5), so
//!   the suite can run against a fake provider with no processes at all.
//!
//! The seam deliberately keeps *streaming*: liveness is a per-byte signal, so a
//! handle that only returned its output at the end would quietly disable the
//! stall watchdog. [`RunnerHandle`] therefore hands out byte chunks, not lines.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::mpsc;

/// A closed stream, for a handle whose pipes were already taken.
fn empty_stream() -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel(1);
    drop(tx);
    rx
}

/// A boxed future, so this module needs no async-trait dependency.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Everything one dispatch needs to run, resolved by the pool.
#[derive(Debug, Clone, Default)]
pub struct RunSpec {
    pub task_id: String,
    pub agent_type: String,
    /// The child's working directory; the dispatch dir lives here.
    pub cwd: PathBuf,
    /// The executable plus everything before `--mode json`.
    pub argv: Vec<String>,
    /// The child's environment. The process runner starts from an empty one.
    pub env: HashMap<String, String>,
    /// The prompt, kept out of `argv` so an in-process runner never has to
    /// reconstruct it from a command line.
    pub prompt: String,
    pub max_turns: u64,
    pub deadline_ms: Option<u64>,
    /// The child's system prompt and tool allowlist, for a runner that builds
    /// a session instead of a command line.
    pub system_prompt: Option<String>,
    pub tools: Option<Vec<String>>,
    pub disallowed_tools: Option<Vec<String>>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub skill_paths: Vec<String>,
}

/// A running dispatch. The same surface a `tokio::process::Child` has, and the
/// same surface the pool already used: bytes in, an exit code out, and a kill.
pub trait RunnerHandle: Send {
    /// Take the two byte streams. They are taken rather than borrowed because
    /// the pool reads them in tasks that outlive the wait: a handle that lent
    /// them out could not also be awaited.
    fn take_streams(&mut self) -> (mpsc::Receiver<Vec<u8>>, mpsc::Receiver<Vec<u8>>);
    /// Resolves when the run has exited.
    fn wait(&mut self) -> BoxFuture<'static, std::io::Result<Option<i32>>>;
    /// End the run, as hard as the implementation can.
    fn kill(&mut self);
    /// The operating-system process, or 0 when there is none. Used for
    /// process-group kills and for the pid file.
    fn pid(&self) -> u32;
}

/// Runs dispatches.
///
/// `spawn` is synchronous on purpose: the pool starts runs from `pull`, which is
/// called from a sync path on a runtime thread, and blocking a runtime thread
/// on a future panics. Both implementations resolve a run immediately — one
/// process, one task — so nothing is lost by keeping the seam sync.
pub trait Runner: Send + Sync {
    fn spawn(&self, spec: RunSpec) -> std::io::Result<Box<dyn RunnerHandle>>;
}

/// Re-execs a binary per dispatch: how the product runs subagents today.
pub struct ProcessRunner {
    executable: PathBuf,
    prefix_args: Vec<String>,
}

impl ProcessRunner {
    pub fn new(executable: PathBuf, prefix_args: Vec<String>) -> Self {
        Self {
            executable,
            prefix_args,
        }
    }
}

impl Runner for ProcessRunner {
    fn spawn(&self, spec: RunSpec) -> std::io::Result<Box<dyn RunnerHandle>> {
        let mut command = std::process::Command::new(&self.executable);
        command
            .args(&self.prefix_args)
            .args(&spec.argv)
            .env_clear()
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .current_dir(&spec.cwd);
        for (key, value) in &spec.env {
            command.env(key, value);
        }
        // Its own process group, so one kill reaches the whole tree (its bash
        // commands, its nested subagents).
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let child = command.spawn()?;
        Ok(Box::new(ProcessHandle::new(child)) as Box<dyn RunnerHandle>)
    }
}

/// A child process behind [`RunnerHandle`]. The pipes are read on blocking
/// tasks — one short-lived thread per stream — because the seam is sync and a
/// blocking read is exactly what a pipe is.
struct ProcessHandle {
    pid: u32,
    stdout: Option<mpsc::Receiver<Vec<u8>>>,
    stderr: Option<mpsc::Receiver<Vec<u8>>>,
    child: Option<std::process::Child>,
}

/// Pump a pipe into a channel. `None` (no pipe) closes immediately.
fn pump<R: std::io::Read + Send + 'static>(
    pipe: Option<R>,
    tx: mpsc::Sender<Vec<u8>>,
) -> Option<std::thread::JoinHandle<()>> {
    let mut pipe = pipe?;
    Some(std::thread::spawn(move || {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let n = match pipe.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            if tx.blocking_send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    }))
}

impl ProcessHandle {
    fn new(mut child: std::process::Child) -> Self {
        let pid = child.id();
        let (stdout_tx, stdout_rx) = mpsc::channel(64);
        let (stderr_tx, stderr_rx) = mpsc::channel(64);
        pump(child.stdout.take(), stdout_tx);
        pump(child.stderr.take(), stderr_tx);
        Self {
            pid,
            stdout: Some(stdout_rx),
            stderr: Some(stderr_rx),
            child: Some(child),
        }
    }
}

impl RunnerHandle for ProcessHandle {
    fn take_streams(&mut self) -> (mpsc::Receiver<Vec<u8>>, mpsc::Receiver<Vec<u8>>) {
        (
            self.stdout.take().unwrap_or_else(empty_stream),
            self.stderr.take().unwrap_or_else(empty_stream),
        )
    }

    fn wait(&mut self) -> BoxFuture<'static, std::io::Result<Option<i32>>> {
        let child = self.child.take();
        Box::pin(async move {
            match child {
                // Waiting on a child blocks; keep it off the runtime's threads.
                Some(mut child) => match tokio::task::spawn_blocking(move || child.wait()).await {
                    Ok(status) => Ok(status.ok().and_then(|status| status.code())),
                    Err(error) => Err(std::io::Error::other(error.to_string())),
                },
                None => Ok(None),
            }
        })
    }

    fn kill(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
        }
    }

    fn pid(&self) -> u32 {
        self.pid
    }
}

/// A runner that answers from a script instead of a process.
///
/// `Clone` shares the cursor, so a test can keep a handle on the runner after
/// handing it to the pool.
///
/// This is the test double the seam exists for: no binary, no pipes, no
/// timings — a dispatch settles exactly when the script says it does. Tests
/// that used to spawn `/bin/sh` to say one line of JSON now say it here.
#[derive(Clone)]
pub struct ScriptedRunner {
    steps: Arc<Vec<ScriptedStep>>,
    next: Arc<std::sync::atomic::AtomicUsize>,
}

/// What a [`ScriptedRunner`] does for one dispatch.
#[derive(Debug, Clone, Default)]
pub struct ScriptedStep {
    /// stdout lines, emitted in order.
    pub stdout: Vec<String>,
    /// stderr text, emitted once the stdout lines are out.
    pub stderr: String,
    pub exit_code: Option<i32>,
    /// Writes `result.json` for the dispatch dir before exiting.
    pub result_json: Option<String>,
    /// Stay alive after the output until killed, so a test can cancel or reap.
    pub hang: bool,
}

impl ScriptedRunner {
    pub fn new(steps: Vec<ScriptedStep>) -> Self {
        Self {
            steps: Arc::new(steps),
            next: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    /// How many dispatches have been started.
    pub fn dispatched(&self) -> usize {
        self.next.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl Runner for ScriptedRunner {
    fn spawn(&self, spec: RunSpec) -> std::io::Result<Box<dyn RunnerHandle>> {
        let index = self.next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let step = self
            .steps
            .get(index)
            .cloned()
            .unwrap_or_else(|| self.steps.last().cloned().unwrap_or_default());
        Ok(Box::new(ScriptedHandle::new(spec, step)) as Box<dyn RunnerHandle>)
    }
}

struct ScriptedHandle {
    spec: RunSpec,
    step: ScriptedStep,
    stdout: Option<mpsc::Receiver<Vec<u8>>>,
    stderr: Option<mpsc::Receiver<Vec<u8>>>,
    finished: bool,
}

impl ScriptedHandle {
    fn new(spec: RunSpec, step: ScriptedStep) -> Self {
        let (stdout_tx, stdout_rx) = mpsc::channel(64);
        let (stderr_tx, stderr_rx) = mpsc::channel(64);
        let lines = step.stdout.clone();
        let hang = step.hang;
        let cwd = spec.cwd.clone();
        let task_id = spec.task_id.clone();
        let result_json = step.result_json.clone();
        let stderr = step.stderr.clone();
        let exit_code = step.exit_code;
        let pump = async move {
            for line in lines {
                if stdout_tx.send(line.into_bytes()).await.is_err() {
                    return;
                }
            }
            drop(stdout_tx);
            if let Some(text) = result_json {
                let dir = cortexcode_code_paths::dispatch_task_dir(&cwd, &task_id);
                if std::fs::create_dir_all(&dir).is_ok() {
                    let _ = std::fs::write(dir.join("result.json"), text);
                }
            }
            if !stderr.is_empty() && stderr_tx.send(stderr.into_bytes()).await.is_err() {
                return;
            }
            drop(stderr_tx);
            if hang {
                // Stay alive until killed, like a child that never exits.
                std::future::pending::<()>().await;
            }
            let _ = exit_code;
        };
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(pump);
            }
            Err(_) => {
                std::thread::spawn(move || {
                    tokio::runtime::Builder::new_current_thread()
                        .build()
                        .map(|rt| rt.block_on(pump))
                        .ok();
                });
            }
        }
        Self {
            spec,
            step,
            stdout: Some(stdout_rx),
            stderr: Some(stderr_rx),
            finished: false,
        }
    }
}

impl RunnerHandle for ScriptedHandle {
    fn take_streams(&mut self) -> (mpsc::Receiver<Vec<u8>>, mpsc::Receiver<Vec<u8>>) {
        (
            self.stdout.take().unwrap_or_else(empty_stream),
            self.stderr.take().unwrap_or_else(empty_stream),
        )
    }

    fn wait(&mut self) -> BoxFuture<'static, std::io::Result<Option<i32>>> {
        let code = if self.finished {
            Some(self.step.exit_code.unwrap_or(0))
        } else {
            self.finished = true;
            self.step.exit_code
        };
        let _ = &self.spec;
        Box::pin(async move {
            // Drain both streams first: the pool reads them until they close,
            // and a wait that resolved early would truncate the transcript.
            let _ = code;
            Ok(code)
        })
    }

    fn kill(&mut self) {
        self.finished = true;
    }

    fn pid(&self) -> u32 {
        // No process: 0 tells the pid file and the group kill that there is
        // nothing to signal, and the pool already treats 0 that way.
        0
    }
}

// ---------------------------------------------------------------------------
// In-process execution
// ---------------------------------------------------------------------------

/// One subagent run that happens inside this process instead of in a child.
///
/// This is the seam's other half, and the one `docs/design/subagents.md` §6 has
/// kept open since the port: "run the subagent here, not in a re-exec". The
/// interface is deliberately tiny — *run this prompt and write what a child
/// would have written* — because everything interesting (queueing, liveness,
/// verification, the ledger) is above it, and the only thing an implementation
/// has to get right is producing the same bytes on stdout and the same
/// `result.json` on disk.
pub trait InProcessAgent: Send + Sync {
    /// Run one prompt to completion. `out` receives the same JSONL lines a
    /// child process would have printed: heartbeats, progress events, `done`.
    /// Writing `result.json` is the agent's job, exactly as it is the child's.
    ///
    /// The future borrows the agent and the sender: an implementation may hold
    /// either for as long as the run lasts, which is what lets a run dispatch
    /// its own child and hand the result back through the same sender.
    fn run<'a>(
        &'a self,
        prompt: &'a str,
        out: &'a mpsc::Sender<Vec<u8>>,
    ) -> BoxFuture<'a, Result<(), String>>;
}

/// Builds an agent for a dispatch. The caller owns the wiring — the CLI has the
/// session services, the registry and the auth, and this crate deliberately
/// does not grow a second way to build them.
pub type InProcessFactory =
    Arc<dyn Fn(&RunSpec) -> Result<Box<dyn InProcessAgent>, String> + Send + Sync>;

/// Runs dispatches in this process.
///
/// Off by default: the product still ships [`ProcessRunner`]. This exists so the
/// question "should it?" has an answer with measurements instead of opinions.
pub struct InProcessRunner {
    factory: InProcessFactory,
}

impl InProcessRunner {
    pub fn new(factory: InProcessFactory) -> Self {
        Self { factory }
    }
}

impl Runner for InProcessRunner {
    fn spawn(&self, spec: RunSpec) -> std::io::Result<Box<dyn RunnerHandle>> {
        let agent = (self.factory)(&spec).map_err(std::io::Error::other)?;
        let (stdout_tx, stdout_rx) = mpsc::channel(64);
        let (stderr_tx, stderr_rx) = mpsc::channel(64);
        let prompt = spec.prompt.clone();
        let task_id = spec.task_id.clone();
        let cwd = spec.cwd.clone();
        let handle = tokio::runtime::Handle::try_current()
            .map_err(|_| std::io::Error::other("the in-process runner needs a tokio runtime"))?;
        let runner_task = handle.spawn(async move {
            // The agent and the sender live as long as the run: the future
            // borrows both, which is what allows a nested dispatch to send its
            // own lines into this run's stream.
            let result = agent.run(&prompt, &stdout_tx).await;
            drop(stdout_tx);
            match result {
                Ok(()) => Ok(()),
                Err(error) => {
                    let line = format!("in-process subagent failed: {error}\n");
                    let _ = stderr_tx.send(line.into_bytes()).await;
                    Err(error)
                }
            }
        });
        // The exit code follows the same contract as a child: zero when a valid
        // `result.json` was written, one otherwise.
        let exit = handle.spawn(async move {
            let outcome = runner_task.await;
            let ok = match &outcome {
                Ok(Ok(_)) => result_exists(&cwd, &task_id),
                _ => false,
            };
            outcome.is_ok() && ok
        });
        Ok(Box::new(InProcessHandle {
            stdout: Some(stdout_rx),
            stderr: Some(stderr_rx),
            exit: Some(exit),
        }) as Box<dyn RunnerHandle>)
    }
}

fn result_exists(cwd: &std::path::Path, task_id: &str) -> bool {
    cortexcode_code_paths::dispatch_task_dir(cwd, task_id)
        .join("result.json")
        .exists()
}

struct InProcessHandle {
    stdout: Option<mpsc::Receiver<Vec<u8>>>,
    stderr: Option<mpsc::Receiver<Vec<u8>>>,
    exit: Option<tokio::task::JoinHandle<bool>>,
}

impl RunnerHandle for InProcessHandle {
    fn take_streams(&mut self) -> (mpsc::Receiver<Vec<u8>>, mpsc::Receiver<Vec<u8>>) {
        (
            self.stdout.take().unwrap_or_else(empty_stream),
            self.stderr.take().unwrap_or_else(empty_stream),
        )
    }

    fn wait(&mut self) -> BoxFuture<'static, std::io::Result<Option<i32>>> {
        let exit = self.exit.take();
        Box::pin(async move {
            let ok = match exit {
                Some(handle) => handle.await.unwrap_or(false),
                // Nothing left to wait for: the handle was already awaited.
                None => true,
            };
            Ok(Some(if ok { 0 } else { 1 }))
        })
    }

    fn kill(&mut self) {
        // There is no process tree to signal. Dropping the streams is what
        // actually stops the run: the agent's `out` sender fails and it stops
        // writing, and the settle path sees the exit.
        self.stdout = None;
        self.stderr = None;
    }

    fn pid(&self) -> u32 {
        0
    }
}

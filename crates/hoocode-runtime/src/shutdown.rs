//! The ordered shutdown of the process (`docs/design/concurrency.md` section 4):
//! abort the agent, kill the tool process groups, flush the session, close MCP,
//! release the session, then restore the terminal.
//!
//! Each step runs on its own named thread and has its own deadline. A step that
//! runs past its deadline is abandoned: its thread keeps running, nothing waits
//! for it, and the next step runs. A panicking step is reported as failed. The
//! terminal restore runs on the calling thread after the steps, whatever they
//! did, because the terminal handle is not `Send`.

use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use crate::threads::{spawn_named_thread, sync_bounded_channel};

/// Name of the thread each step runs on.
pub const SHUTDOWN_THREAD: &str = "hoocode-shutdown";
/// Most time the whole shutdown may take, terminal restore included.
pub const SHUTDOWN_BUDGET: Duration = Duration::from_secs(3);
/// Time kept back from the budget for the terminal restore.
pub const RESTORE_RESERVE: Duration = Duration::from_millis(200);

/// One timed step of the shutdown.
pub struct ShutdownStep {
    name: &'static str,
    deadline: Duration,
    run: Box<dyn FnOnce() + Send>,
}

impl ShutdownStep {
    pub fn new(
        name: &'static str,
        deadline: Duration,
        run: impl FnOnce() + Send + 'static,
    ) -> Self {
        Self {
            name,
            deadline,
            run: Box::new(run),
        }
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn deadline(&self) -> Duration {
        self.deadline
    }
}

/// How a step ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepOutcome {
    /// The step returned before its deadline.
    Done,
    /// The deadline passed first. The step was abandoned, not stopped.
    TimedOut,
    /// The step panicked, or its thread could not start.
    Failed,
}

/// The result of one step, in the order the steps ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepReport {
    pub name: &'static str,
    pub outcome: StepOutcome,
    pub elapsed: Duration,
}

/// Whether the steps' deadlines, plus the restore reserve, fit the budget.
pub fn fits_budget(steps: &[ShutdownStep]) -> bool {
    let total: Duration = steps.iter().map(ShutdownStep::deadline).sum();
    total + RESTORE_RESERVE <= SHUTDOWN_BUDGET
}

/// Runs `steps` in order, then `restore` on the calling thread. Returns one
/// report per step. Never waits past a step's deadline.
pub fn run_ordered(steps: Vec<ShutdownStep>, restore: impl FnOnce()) -> Vec<StepReport> {
    debug_assert!(fits_budget(&steps), "shutdown steps exceed the budget");
    let reports = steps.into_iter().map(run_step).collect();
    restore();
    reports
}

fn run_step(step: ShutdownStep) -> StepReport {
    let started = Instant::now();
    let ShutdownStep {
        name,
        deadline,
        run,
    } = step;
    // A panic in `run` drops `done` without a send, which reads as a failure.
    let (done, finished) = sync_bounded_channel::<()>(1);
    let spawned = spawn_named_thread(SHUTDOWN_THREAD, move || {
        run();
        let _ = done.send(());
    });
    let outcome = match spawned {
        Err(_) => StepOutcome::Failed,
        Ok(_worker) => match finished.recv_timeout(deadline) {
            Ok(()) => StepOutcome::Done,
            Err(RecvTimeoutError::Timeout) => StepOutcome::TimedOut,
            Err(RecvTimeoutError::Disconnected) => StepOutcome::Failed,
        },
    };
    StepReport {
        name,
        outcome,
        elapsed: started.elapsed(),
    }
}

//! The ordered shutdown of the process (`docs/design/concurrency.md` section 4):
//! abort the agent, kill the tool process groups, flush the session, close MCP,
//! release the session, then restore the terminal.
//!
//! Each step runs on its own named thread and has its own deadline. A step that
//! runs past its deadline is abandoned: its thread keeps running, nothing waits
//! for it, and the next step runs. A panicking step is reported as failed. The
//! terminal restore runs on the calling thread after the steps, whatever they
//! did, because the terminal handle is not `Send`.

use std::io;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex};
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
/// report per step. Never waits past a step's deadline. `restore` runs even if
/// something before it panics, so the terminal is never left raw.
pub fn run_ordered(steps: Vec<ShutdownStep>, restore: impl FnOnce()) -> Vec<StepReport> {
    let _restore = RestoreGuard(Some(restore));
    debug_assert!(fits_budget(&steps), "shutdown steps exceed the budget");
    steps.into_iter().map(run_step).collect()
}

/// Calls its closure once when dropped: on the normal path after the steps, and
/// during an unwind if anything before the restore panics.
struct RestoreGuard<F: FnOnce()>(Option<F>);

impl<F: FnOnce()> Drop for RestoreGuard<F> {
    fn drop(&mut self) {
        let Some(restore) = self.0.take() else {
            return;
        };
        if std::thread::panicking() {
            // A second panic while unwinding would abort; the restore is best effort then.
            let _ = catch_unwind(AssertUnwindSafe(restore));
        } else {
            restore();
        }
    }
}

/// A step's closure, kept where the caller can take it back if its thread never starts.
type Job = Box<dyn FnOnce() + Send>;

fn run_step(step: ShutdownStep) -> StepReport {
    run_step_on(step, |job| {
        spawn_named_thread(SHUTDOWN_THREAD, job).map(drop)
    })
}

/// [`run_step`] with the thread start injected, so a failed start can be tested.
fn run_step_on(step: ShutdownStep, spawn: impl FnOnce(Job) -> io::Result<()>) -> StepReport {
    let started = Instant::now();
    let ShutdownStep {
        name,
        deadline,
        run,
    } = step;
    let slot: Arc<Mutex<Option<Job>>> = Arc::new(Mutex::new(Some(run)));
    // A panic in the job drops `done` without a send, which reads as a failure.
    let (done, finished) = sync_bounded_channel::<()>(1);
    let worker_slot = Arc::clone(&slot);
    let spawned = spawn(Box::new(move || {
        if let Some(job) = take_job(&worker_slot) {
            job();
        }
        let _ = done.send(());
    }));
    let outcome = match spawned {
        // The thread never ran, so the job is still in the slot: run it here.
        Err(_) => match take_job(&slot) {
            Some(job) => run_inline(job),
            None => StepOutcome::Failed,
        },
        Ok(()) => match finished.recv_timeout(deadline) {
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

fn take_job(slot: &Mutex<Option<Job>>) -> Option<Job> {
    slot.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// Runs a step on the calling thread after its thread failed to start. Has no
/// deadline (nothing can cut it short here), so a hanging step hangs the
/// shutdown; that is the price of still flushing the session. A panic is `Failed`.
fn run_inline(job: Job) -> StepOutcome {
    match catch_unwind(AssertUnwindSafe(job)) {
        Ok(()) => StepOutcome::Done,
        Err(_) => StepOutcome::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn flag_step(name: &'static str, flag: &Arc<AtomicBool>) -> ShutdownStep {
        let flag = Arc::clone(flag);
        ShutdownStep::new(name, Duration::from_secs(1), move || {
            flag.store(true, Ordering::SeqCst);
        })
    }

    #[test]
    fn a_failed_thread_start_runs_the_step_inline() {
        let ran = Arc::new(AtomicBool::new(false));
        let report = run_step_on(flag_step("flush session", &ran), |_job| {
            Err(io::Error::other("no threads"))
        });
        assert!(ran.load(Ordering::SeqCst), "the step ran on the caller");
        assert_eq!(report.name, "flush session");
        assert_eq!(report.outcome, StepOutcome::Done);
    }

    #[test]
    fn a_panicking_inline_step_is_failed_not_propagated() {
        let step = ShutdownStep::new("close MCP", Duration::from_secs(1), || panic!("boom"));
        let report = run_step_on(step, |_job| Err(io::Error::other("no threads")));
        assert_eq!(report.outcome, StepOutcome::Failed);
    }

    #[test]
    fn a_started_step_reports_done() {
        let ran = Arc::new(AtomicBool::new(false));
        let report = run_step_on(flag_step("kill tool process groups", &ran), |job| {
            spawn_named_thread(SHUTDOWN_THREAD, job).map(drop)
        });
        assert!(ran.load(Ordering::SeqCst));
        assert_eq!(report.outcome, StepOutcome::Done);
    }

    #[test]
    fn the_restore_runs_after_a_panic_before_it() {
        let restored = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&restored);
        let result = catch_unwind(AssertUnwindSafe(move || {
            let _guard = RestoreGuard(Some(move || flag.store(true, Ordering::SeqCst)));
            panic!("step collection failed");
        }));
        assert!(result.is_err());
        assert!(restored.load(Ordering::SeqCst), "restore ran during unwind");
    }

    #[test]
    fn the_restore_runs_once_on_the_normal_path() {
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = Arc::clone(&count);
        let reports = run_ordered(Vec::new(), move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        assert!(reports.is_empty());
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
}

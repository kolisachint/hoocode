//! Ctrl+Z: suspend to the background (`InteractiveMode.handleCtrlZ`).
//!
//! The TUI gives the terminal back, SIGINT is ignored while stopped (so a
//! Ctrl+C typed at the shell does not kill the backgrounded process), and
//! SIGTSTP goes to the process group. hoocode restores the TUI from a SIGCONT
//! handler; here `kill` returns once the process has been continued, so the
//! restore follows it directly.
//!
//! The steps go through [`SuspendOps`] so tests can run the sequence without
//! stopping the test process.
//!
//! A stop hoocode did not ask for (SIGTTIN/SIGTTOU when it lost the terminal's
//! foreground, a SIGTSTP from outside) leaves raw mode and the Kitty keyboard
//! flags on while the shell has the terminal, and the screen stale after
//! `fg`. A SIGCONT watch ([`note_sigcont`]) marks such a resume; the UI loop
//! takes it ([`take_external_resume`]) and restarts the UI the way Ctrl+Z
//! does ([`resume_after_external_stop`]).

use std::sync::atomic::{AtomicBool, Ordering};

/// Set just before our own SIGTSTP; the SIGCONT that ends that stop clears it.
static SELF_SUSPEND: AtomicBool = AtomicBool::new(false);
/// A SIGCONT arrived that our own Ctrl+Z did not cause.
static EXTERNAL_RESUME: AtomicBool = AtomicBool::new(false);

/// Called on each SIGCONT. The one that ends our own Ctrl+Z stop is ignored:
/// [`suspend_to_background`] restarts the UI itself.
pub fn note_sigcont() {
    if !SELF_SUSPEND.swap(false, Ordering::SeqCst) {
        EXTERNAL_RESUME.store(true, Ordering::SeqCst);
    }
}

/// Whether the process was continued after a stop it did not ask for, since
/// the last call.
pub fn take_external_resume() -> bool {
    EXTERNAL_RESUME.swap(false, Ordering::SeqCst)
}

/// Forget a pending external resume: the UI was just restarted anyway (after
/// an external editor, which may have been stopped and continued itself).
pub fn clear_external_resume() {
    EXTERNAL_RESUME.store(false, Ordering::SeqCst);
}

/// Bring the UI back after a stop it did not ask for: tear it down (the Kitty
/// flags popped, raw mode off, so nothing is pushed twice) and start it again
/// with a full redraw, as after Ctrl+Z.
pub fn resume_after_external_stop(ops: &mut dyn SuspendOps) {
    ops.stop_ui();
    ops.restart_ui();
}

/// What suspending touches.
pub trait SuspendOps {
    fn stop_ui(&mut self);
    /// Restart the UI and force a full redraw.
    fn restart_ui(&mut self);
    fn ignore_sigint(&mut self);
    fn restore_sigint(&mut self);
    /// `process.kill(0, "SIGTSTP")`: returns after the process is continued.
    fn stop_process_group(&mut self) -> Result<(), String>;
}

/// What happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SuspendOutcome {
    /// Suspended and resumed; the UI is back.
    Resumed,
    /// Not supported here; show this status instead.
    Unsupported(&'static str),
    /// The signal could not be sent. The temporary SIGINT handling is undone
    /// and the UI is still stopped.
    Failed(String),
}

/// `handleCtrlZ`.
pub fn suspend_to_background(ops: &mut dyn SuspendOps, windows: bool) -> SuspendOutcome {
    if windows {
        return SuspendOutcome::Unsupported("Suspend to background is not supported on Windows");
    }
    ops.ignore_sigint();
    ops.stop_ui();
    if let Err(error) = ops.stop_process_group() {
        ops.restore_sigint();
        return SuspendOutcome::Failed(error);
    }
    ops.restore_sigint();
    ops.restart_ui();
    SuspendOutcome::Resumed
}

/// The process half of [`SuspendOps`] on Unix: SIGINT ignored and restored
/// with `sigaction`, SIGTSTP to the process group.
#[cfg(unix)]
#[derive(Default)]
pub struct ProcessSignals {
    previous_sigint: Option<libc::sigaction>,
}

#[cfg(unix)]
impl ProcessSignals {
    pub fn ignore_sigint(&mut self) {
        // SAFETY: plain sigaction calls with zeroed, then filled, structs.
        unsafe {
            let mut ignore: libc::sigaction = std::mem::zeroed();
            ignore.sa_sigaction = libc::SIG_IGN;
            libc::sigemptyset(&mut ignore.sa_mask);
            let mut previous: libc::sigaction = std::mem::zeroed();
            if libc::sigaction(libc::SIGINT, &ignore, &mut previous) == 0 {
                self.previous_sigint = Some(previous);
            }
        }
    }

    pub fn restore_sigint(&mut self) {
        if let Some(previous) = self.previous_sigint.take() {
            // SAFETY: restores the action saved by `ignore_sigint`.
            unsafe {
                libc::sigaction(libc::SIGINT, &previous, std::ptr::null_mut());
            }
        }
    }

    pub fn stop_process_group(&mut self) -> Result<(), String> {
        // The SIGCONT that ends this stop is ours (see `note_sigcont`).
        SELF_SUSPEND.store(true, Ordering::SeqCst);
        // SAFETY: kill(2) on our own process group.
        if unsafe { libc::kill(0, libc::SIGTSTP) } == 0 {
            Ok(())
        } else {
            SELF_SUSPEND.store(false, Ordering::SeqCst);
            Err(std::io::Error::last_os_error().to_string())
        }
    }
}

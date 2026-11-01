//! Process signals for the TUI: `SIGWINCH` (terminal resize), the termination
//! signals (`SIGTERM`, `SIGHUP`) and `SIGCONT` (continued after a stop), each
//! on its own watch thread.
//!
//! The watcher runs on its own named thread that blocks on the signal
//! iterator, so an idle process spends no CPU on it. Dropping the
//! [`SignalWatch`] closes the iterator and the thread exits.

use std::io;
use std::thread::JoinHandle;

use signal_hook::consts::{SIGCONT, SIGHUP, SIGTERM, SIGWINCH};
use signal_hook::iterator::{Handle, Signals};

use crate::threads::spawn_named_thread;

/// A running `SIGWINCH` watch. Dropping it stops the watch.
pub struct SignalWatch {
    handle: Handle,
    _thread: JoinHandle<()>,
}

impl Drop for SignalWatch {
    fn drop(&mut self) {
        self.handle.close();
    }
}

/// Calls `on_signal` on the thread `name` each time the process gets `SIGWINCH`.
/// Signals may coalesce; the callback should look at the current state, not count calls.
pub fn watch_sigwinch<F>(name: &str, mut on_signal: F) -> io::Result<SignalWatch>
where
    F: FnMut() + Send + 'static,
{
    let mut signals = Signals::new([SIGWINCH])?;
    let handle = signals.handle();
    let thread = spawn_named_thread(name, move || {
        for _ in signals.forever() {
            on_signal();
        }
    })?;
    Ok(SignalWatch {
        handle,
        _thread: thread,
    })
}

/// Calls `on_signal` with the signal number on the thread `name` for each
/// `SIGTERM` or `SIGHUP`. The first call decides what happens next (the TUI
/// restores the terminal and exits); the watch sees no later signal then.
#[cfg(unix)]
pub fn watch_terminate<F>(name: &str, mut on_signal: F) -> io::Result<SignalWatch>
where
    F: FnMut(i32) + Send + 'static,
{
    let mut signals = Signals::new([SIGTERM, SIGHUP])?;
    let handle = signals.handle();
    let thread = spawn_named_thread(name, move || {
        for signal in signals.forever() {
            on_signal(signal);
        }
    })?;
    Ok(SignalWatch {
        handle,
        _thread: thread,
    })
}

/// Calls `on_signal` on the thread `name` each time the process is continued
/// (`SIGCONT`) after a stop: the TUI's own Ctrl+Z, or a stop it did not ask
/// for (SIGTTIN/SIGTTOU, a SIGTSTP from outside) ended by `fg`. Watching does
/// not change what SIGCONT does: the process is continued either way.
#[cfg(unix)]
pub fn watch_sigcont<F>(name: &str, mut on_signal: F) -> io::Result<SignalWatch>
where
    F: FnMut() + Send + 'static,
{
    let mut signals = Signals::new([SIGCONT])?;
    let handle = signals.handle();
    let thread = spawn_named_thread(name, move || {
        for _ in signals.forever() {
            on_signal();
        }
    })?;
    Ok(SignalWatch {
        handle,
        _thread: thread,
    })
}

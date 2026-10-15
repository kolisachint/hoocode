//! The TUI's panic hook (`docs/design/concurrency.md` section 4, Panics).
//!
//! A panic on the UI thread restores the terminal (once), flushes the session
//! (bounded) and is logged with a backtrace. A panic on any other thread is
//! logged with its thread name and leaves the terminal alone, because the UI is
//! still running. Every panic then goes to the hook that was installed before,
//! so the usual message still prints (on the restored terminal for the UI thread).

use std::backtrace::Backtrace;
use std::panic::{self, PanicHookInfo};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::{self, ThreadId};
use std::time::Duration;

use hoocode_runtime::{session_io, spawn_named_thread};

/// How long the session flush may take after a UI-thread panic.
pub const PANIC_FLUSH_DEADLINE: Duration = Duration::from_secs(1);

/// Set by the first [`install`]; later calls change nothing.
static INSTALLED: AtomicBool = AtomicBool::new(false);

/// What the hook does, as seams: production passes the terminal restore, the
/// session flush and the log file; tests pass counters.
pub struct Effects {
    /// Restores the terminal. Runs at most once per installed hook, on the UI thread.
    pub restore: Box<dyn Fn() + Send + Sync>,
    /// Flushes the session. Runs once, after `restore`, on the UI thread.
    pub flush: Box<dyn Fn() + Send + Sync>,
    /// Writes one panic report (message, location, backtrace).
    pub log: Box<dyn Fn(&str) + Send + Sync>,
}

struct Hook {
    ui_thread: ThreadId,
    effects: Effects,
    restored: AtomicBool,
    previous: Box<dyn Fn(&PanicHookInfo<'_>) + Send + Sync>,
}

impl Hook {
    fn on_panic(&self, info: &PanicHookInfo<'_>) {
        let current = thread::current();
        let report = describe(info, current.name().unwrap_or("<unnamed>"));
        self.handle(current.id(), &report);
        (self.previous)(info);
    }

    /// The part of the hook that does not depend on the panic payload.
    ///
    /// The hook runs before unwinding, so it cannot tell a caught panic
    /// (`catch_unwind`) from an uncaught one. A UI-thread panic that is caught
    /// therefore still restores the terminal and flushes the session. So
    /// `catch_unwind` must not be used on the UI thread during a live session.
    /// Check any existing `catch_unwind` (for example in `hoocode-ai-registry`'s
    /// `session_resources`) against this before it runs on the UI thread.
    fn handle(&self, thread: ThreadId, report: &str) {
        if thread == self.ui_thread && !self.restored.swap(true, Ordering::SeqCst) {
            (self.effects.restore)();
            (self.effects.flush)();
        }
        (self.effects.log)(report);
    }
}

/// Installs the hook. `ui_thread` is the thread that runs the UI loop: call this
/// there, before the terminal starts. Returns `false` (and changes nothing) if a
/// hook was already installed in this process.
pub fn install(ui_thread: ThreadId, effects: Effects) -> bool {
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return false;
    }
    let hook = Hook {
        ui_thread,
        effects,
        restored: AtomicBool::new(false),
        previous: panic::take_hook(),
    };
    panic::set_hook(Box::new(move |info| hook.on_panic(info)));
    true
}

/// Flushes the session on a helper thread and waits at most `deadline`. The hook
/// can run while the panicking thread holds the session writer's lock, so the
/// flush is never awaited without a bound.
pub fn flush_session(deadline: Duration) {
    let (done, finished) = mpsc::channel();
    let spawned = spawn_named_thread("hoocode-panic-flush", move || {
        let _ = session_io().flush_blocking(deadline);
        let _ = done.send(());
    });
    if spawned.is_ok() {
        let _ = finished.recv_timeout(deadline);
    }
}

/// The report for one panic: thread, location, message and a forced backtrace.
fn describe(info: &PanicHookInfo<'_>, thread: &str) -> String {
    let payload = info.payload();
    let message = payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "<non-string panic payload>".to_owned());
    let location = info.location().map_or_else(
        || "<unknown location>".to_owned(),
        |l| format!("{}:{}:{}", l.file(), l.line(), l.column()),
    );
    let backtrace = Backtrace::force_capture();
    format!("panic on thread '{thread}' at {location}: {message}\nbacktrace:\n{backtrace}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;
    use std::sync::Mutex;

    #[test]
    fn ui_thread_restores_once_and_other_threads_do_not() {
        let restores = Arc::new(AtomicUsize::new(0));
        let flushes = Arc::new(AtomicUsize::new(0));
        let logged = Arc::new(Mutex::new(Vec::<String>::new()));
        let hook = Hook {
            ui_thread: thread::current().id(),
            effects: Effects {
                restore: {
                    let restores = restores.clone();
                    Box::new(move || {
                        restores.fetch_add(1, Ordering::SeqCst);
                    })
                },
                flush: {
                    let flushes = flushes.clone();
                    Box::new(move || {
                        flushes.fetch_add(1, Ordering::SeqCst);
                    })
                },
                log: {
                    let logged = logged.clone();
                    Box::new(move |line: &str| logged.lock().unwrap().push(line.to_owned()))
                },
            },
            restored: AtomicBool::new(false),
            previous: Box::new(|_| {}),
        };
        let ui = thread::current().id();
        hook.handle(ui, "first");
        hook.handle(ui, "second");
        assert_eq!(restores.load(Ordering::SeqCst), 1, "restore runs once");
        assert_eq!(flushes.load(Ordering::SeqCst), 1, "flush runs once");
        assert_eq!(logged.lock().unwrap().len(), 2, "every panic is logged");

        let other =
            hoocode_runtime::spawn_named_thread("panic-test-other", || thread::current().id())
                .unwrap()
                .join()
                .unwrap();
        hook.handle(other, "worker");
        assert_eq!(
            restores.load(Ordering::SeqCst),
            1,
            "a worker panic leaves the terminal alone"
        );
        assert_eq!(flushes.load(Ordering::SeqCst), 1);
        assert_eq!(
            logged.lock().unwrap().last().map(String::as_str),
            Some("worker")
        );
    }
}

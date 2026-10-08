//! Named OS threads and bounded channels.

use std::io;
use std::thread::JoinHandle;

use tokio::sync::mpsc;

/// Starts an OS thread called `name` (it shows in logs and panic reports).
///
/// Use this for the few dedicated threads (input, terminal output, session
/// writer, watchdog). Work that can run on a pool goes through
/// [`run_blocking`](crate::run_blocking) instead.
pub fn spawn_named_thread<F, T>(name: &str, f: F) -> io::Result<JoinHandle<T>>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    // The one place a hoocode thread is started (`std::thread::spawn` is disallowed).
    #[allow(clippy::disallowed_methods)] // this helper is the named-thread spawn
    std::thread::Builder::new().name(name.to_owned()).spawn(f)
}

/// [`spawn_named_thread`] for callers that cannot go on without the thread: a
/// failure to start one panics, as `std::thread::spawn` does.
pub fn spawn_thread<F, T>(name: &str, f: F) -> JoinHandle<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    spawn_named_thread(name, f).unwrap_or_else(|e| panic!("failed to start thread {name}: {e}"))
}

/// A bounded async channel with `cap` slots (at least 1). A full channel makes
/// `send().await` wait: senders get backpressure, nothing is dropped.
pub fn bounded_channel<T>(cap: usize) -> (mpsc::Sender<T>, mpsc::Receiver<T>) {
    mpsc::channel(cap.max(1))
}

/// A bounded channel for sync threads, with `cap` slots. `send` blocks while
/// the channel is full.
pub fn sync_bounded_channel<T>(
    cap: usize,
) -> (std::sync::mpsc::SyncSender<T>, std::sync::mpsc::Receiver<T>) {
    std::sync::mpsc::sync_channel(cap)
}

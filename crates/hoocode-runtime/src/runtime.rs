//! The two tokio runtimes of the process: the `hoocode-io` runtime (async work)
//! and the `hoocode-tools` blocking pool (sync tool bodies).

use std::future::Future;
use std::io;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::thread::JoinHandle;

use tokio::runtime::{Builder, Handle, Runtime};
use tokio::task::JoinError;

use crate::lanes::{apply_current_thread_lane, Lane};

/// Name prefix of `hoocode-io` worker threads (`hoocode-io-0`, `hoocode-io-1`, ...).
pub const IO_THREAD_PREFIX: &str = "hoocode-io";
/// Name prefix of `hoocode-tools` pool threads (`hoocode-tools-0`, ...).
pub const TOOLS_THREAD_PREFIX: &str = "hoocode-tools";
/// Most `hoocode-io` workers in the main process.
pub const IO_MAX_WORKERS: usize = 4;
/// `hoocode-io` workers in a subagent child process.
pub const IO_CHILD_WORKERS: usize = 2;
/// Most threads in the `hoocode-tools` pool.
pub const TOOLS_MAX_THREADS: usize = 16;

/// Env var the subagent runner sets on every child (`HOOCODE_` prefix, depth
/// of the child; the main session has none or 0).
const SUBAGENT_DEPTH_VAR: &str = "HOOCODE_SUBAGENT_DEPTH";

static IO_RUNTIME: OnceLock<Runtime> = OnceLock::new();
static TOOLS_RUNTIME: OnceLock<Runtime> = OnceLock::new();
static IO_NAMES: AtomicUsize = AtomicUsize::new(0);
static TOOLS_NAMES: AtomicUsize = AtomicUsize::new(0);

/// Worker count for the `hoocode-io` runtime: `min(4, cores)`, or 2 in a
/// subagent child.
pub fn io_worker_count(cores: usize, subagent_child: bool) -> usize {
    if subagent_child {
        IO_CHILD_WORKERS
    } else {
        cores.clamp(1, IO_MAX_WORKERS)
    }
}

/// True when this process is a subagent child (the runner set its depth to 1 or more).
pub fn is_subagent_child() -> bool {
    std::env::var(SUBAGENT_DEPTH_VAR)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .is_some_and(|depth| depth > 0)
}

fn available_cores() -> usize {
    std::thread::available_parallelism()
        .map(NonZeroUsize::get)
        .unwrap_or(1)
}

fn io_runtime() -> &'static Runtime {
    IO_RUNTIME.get_or_init(|| {
        let workers = io_worker_count(available_cores(), is_subagent_child());
        #[allow(clippy::disallowed_methods)] // the one multi-thread runtime of the process
        let runtime = Builder::new_multi_thread()
            .worker_threads(workers)
            .thread_name_fn(|| {
                let n = IO_NAMES.fetch_add(1, Ordering::Relaxed);
                format!("{IO_THREAD_PREFIX}-{n}")
            })
            .on_thread_start(|| apply_current_thread_lane(Lane::High))
            .enable_all()
            .build();
        runtime.unwrap_or_else(|e| panic!("hoocode-io runtime failed to start: {e}"))
    })
}

fn tools_runtime() -> &'static Runtime {
    TOOLS_RUNTIME.get_or_init(|| {
        // Only its blocking pool is used: a current-thread runtime has no worker
        // thread of its own, so the pool holds at most TOOLS_MAX_THREADS threads.
        #[allow(clippy::disallowed_methods)] // hosts the capped blocking pool only
        let runtime = Builder::new_current_thread()
            .max_blocking_threads(TOOLS_MAX_THREADS)
            .thread_name_fn(|| {
                let n = TOOLS_NAMES.fetch_add(1, Ordering::Relaxed);
                format!("{TOOLS_THREAD_PREFIX}-{n}")
            })
            .on_thread_start(|| apply_current_thread_lane(Lane::Medium))
            .enable_all()
            .build();
        runtime.unwrap_or_else(|e| panic!("hoocode-tools pool failed to start: {e}"))
    })
}

/// Handle to the `hoocode-io` runtime. Spawn async work here.
pub fn io_handle() -> Handle {
    io_runtime().handle().clone()
}

/// Runs `fut` to completion on the `hoocode-io` runtime, blocking the caller.
///
/// For entry points only (`main`, tests). Calling it from inside a runtime
/// panics, as tokio's own `block_on` does.
pub fn block_on_entry<F: Future>(fut: F) -> F::Output {
    io_runtime().block_on(fut)
}

/// Runs sync `f` on the `hoocode-tools` pool (at most [`TOOLS_MAX_THREADS`]
/// threads; further calls queue) and resolves to its result.
///
/// The job is submitted when this function is called, not when the future is
/// first polled. A panic in `f` resolves to `Err`, not a panic in the caller.
pub fn run_blocking<F, T>(f: F) -> impl Future<Output = Result<T, JoinError>> + Send
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    tools_runtime().handle().spawn_blocking(f)
}

/// A private current-thread runtime for one sync bridge call. Not a pool: each
/// bridge builds its own, as the code it replaces did.
#[allow(clippy::disallowed_methods)] // the one place a private runtime is built
fn private_runtime() -> Runtime {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap_or_else(|e| panic!("failed to start a private tokio runtime: {e}"))
}

/// Sync bridge: runs `fut` to completion on the calling thread, on a private
/// current-thread runtime. Only for sync code with no runtime on this thread.
/// Do not call it from a runtime's worker thread (tokio refuses to nest).
pub fn block_on_current_thread<F: Future>(fut: F) -> F::Output {
    private_runtime().block_on(fut)
}

/// Sync bridge: runs `fut` on a new thread with a private runtime and waits for
/// it. Safe to call from inside any runtime, because the caller blocks on a
/// thread that is not an io worker. A panic in `fut` comes back as `Err`.
pub fn block_on_isolated<F>(fut: F) -> std::thread::Result<F::Output>
where
    F: Future + Send,
    F::Output: Send,
{
    std::thread::scope(|scope| {
        let handle = std::thread::Builder::new()
            .name("hoocode-block-on".to_owned())
            .spawn_scoped(scope, move || private_runtime().block_on(fut))
            .unwrap_or_else(|e| panic!("failed to start a thread: {e}"));
        handle.join()
    })
}

/// Runs `fut` to completion on a new named thread with a private runtime. The
/// caller may join the handle or drop it (the thread then runs detached).
pub fn spawn_isolated<F>(name: &str, fut: F) -> io::Result<JoinHandle<F::Output>>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    crate::threads::spawn_named_thread(name, move || private_runtime().block_on(fut))
}

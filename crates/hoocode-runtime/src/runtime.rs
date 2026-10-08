//! The two tokio runtimes of the process: the `hoocode-io` runtime (async work)
//! and the `hoocode-tools` blocking pool (sync tool bodies).

use std::future::Future;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

use tokio::runtime::{Builder, Handle, Runtime};
use tokio::task::JoinError;

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

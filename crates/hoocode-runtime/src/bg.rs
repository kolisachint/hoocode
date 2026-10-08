//! `hoocode-bg`: housekeeping on one Low lane thread (`docs/design/concurrency.md`
//! sections 1 and 2). It runs cleanup, file watchers and the like, so none of it
//! runs on the UI thread, the `hoocode-io` workers or the tools pool.

use std::future::Future;
use std::sync::OnceLock;

use tokio::runtime::{Builder, Handle};
use tokio::task::JoinHandle;

use crate::lanes::{spawn_lane_thread, Lane};

/// Name of the background thread.
pub const BG_THREAD_NAME: &str = "hoocode-bg";

static BG_HANDLE: OnceLock<Handle> = OnceLock::new();

/// Starts the `hoocode-bg` thread on first use and returns its runtime handle.
fn bg_handle() -> &'static Handle {
    BG_HANDLE.get_or_init(|| {
        let (ready_tx, ready_rx) = crate::threads::sync_bounded_channel::<Handle>(1);
        let started = spawn_lane_thread(BG_THREAD_NAME, Lane::Low, move || {
            #[allow(clippy::disallowed_methods)] // the one current-thread runtime of the bg lane
            let runtime = Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap_or_else(|e| panic!("hoocode-bg runtime failed to start: {e}"));
            let _ = ready_tx.send(runtime.handle().clone());
            // Spawned tasks run while this block_on drives the runtime. The
            // thread lives for the whole process.
            runtime.block_on(std::future::pending::<()>());
        });
        if let Err(e) = started {
            panic!("hoocode-bg thread failed to start: {e}");
        }
        ready_rx
            .recv()
            .unwrap_or_else(|_| panic!("hoocode-bg thread exited before it started"))
    })
}

/// Runs `fut` on the `hoocode-bg` thread and returns a handle to its output.
///
/// Use it for housekeeping only. Dropping the handle does not cancel the task.
/// Work here must not block for long: it is one thread, and it is Low priority.
pub fn spawn_bg<F>(fut: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    bg_handle().spawn(fut)
}

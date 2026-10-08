//! Threads, runtimes, blocking work and channels for the hoocode process.
//!
//! This crate is the only place that builds a tokio runtime, a blocking pool,
//! an OS thread or a channel for hoocode. Other crates use the functions here,
//! so the thread count, names and caps stay fixed in one place
//! (`docs/design/concurrency.md` sections 1 and 4).
//!
//! - [`io_handle`]: the `hoocode-io` runtime, `min(4, cores)` workers (2 in
//!   subagent children).
//! - [`run_blocking`]: sync work on the `hoocode-tools` pool, at most 16 threads.
//! - [`block_on_entry`]: entry points (`main`, tests) only.
//! - [`spawn_named_thread`], [`bounded_channel`], [`sync_bounded_channel`].
//! - [`Lane`], [`spawn_lane_thread`], [`apply_current_thread_lane`]: the three
//!   scheduling lanes and their OS priority (High normal, Medium and Low lowered).
//! - [`spawn_bg`]: housekeeping on the one Low lane thread `hoocode-bg`.
//! - [`lower_child_priority`]: niceness of a child process, set at spawn.
//! - [`ParallelToolLimit`]: the per-turn cap on parallel tool calls.
//! - [`session_io`]: the `hoocode-session-io` thread, the only writer of session
//!   files, with a bounded queue and flush barriers.

mod bg;
mod child;
mod lanes;
mod limits;
mod runtime;
mod session_io;
mod threads;

pub use bg::{spawn_bg, BG_THREAD_NAME};
pub use child::{lower_child_priority, BELOW_NORMAL_PRIORITY_CLASS, SUBAGENT_CHILD_NICE};
pub use lanes::{apply_current_thread_lane, spawn_lane_thread, Lane};
pub use limits::{
    total_memory_bytes, ParallelToolLimit, MAX_BASH_NICE, MAX_PARALLEL_TOOLS, MIN_PARALLEL_TOOLS,
};
pub use runtime::{
    block_on_current_thread, block_on_entry, block_on_isolated, io_handle, io_worker_count,
    is_subagent_child, run_blocking, spawn_isolated, IO_CHILD_WORKERS, IO_MAX_WORKERS,
    IO_THREAD_PREFIX, TOOLS_MAX_THREADS, TOOLS_THREAD_PREFIX,
};
pub use session_io::{
    session_io, FileWriter, FlushError, FlushTicket, SESSION_IO_THREAD, SESSION_QUEUE_MAX_BYTES,
    SESSION_QUEUE_MAX_ENTRIES,
};
pub use threads::{bounded_channel, spawn_named_thread, sync_bounded_channel};
pub use tokio::task::JoinError;

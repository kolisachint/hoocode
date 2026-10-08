//! The `hoocode-session-io` thread: the only writer of session files
//! (`docs/design/concurrency.md` sections 1, 3 and 4).
//!
//! Producers queue whole lines or whole-file contents; one thread writes them in
//! order. The queue is bounded (4096 entries or 64 MiB). A producer that finds it
//! full waits, and nothing is dropped. The thread keeps the current file open,
//! writes every line of a batch with one `write` per file, and answers flush
//! barriers: [`FileWriter::barrier`] returns once every earlier operation is
//! written to the OS. A barrier does not fsync; a crash of the OS can still lose
//! the newest writes.

use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use crate::run_blocking;
use crate::spawn_named_thread;

/// Name of the session writer thread.
pub const SESSION_IO_THREAD: &str = "hoocode-session-io";
/// Most queued session operations before producers wait.
pub const SESSION_QUEUE_MAX_ENTRIES: usize = 4096;
/// Most queued session bytes before producers wait.
pub const SESSION_QUEUE_MAX_BYTES: usize = 64 * 1024 * 1024;

/// The process-wide session writer. Built on first use.
pub fn session_io() -> &'static FileWriter {
    static SESSION_IO: OnceLock<FileWriter> = OnceLock::new();
    SESSION_IO.get_or_init(|| {
        FileWriter::start(
            SESSION_IO_THREAD,
            SESSION_QUEUE_MAX_ENTRIES,
            SESSION_QUEUE_MAX_BYTES,
        )
        .unwrap_or_else(|e| panic!("failed to start {SESSION_IO_THREAD}: {e}"))
    })
}

/// Why a flush barrier did not complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlushError {
    /// The deadline passed before the writer got to the barrier.
    Timeout,
    /// A write since the previous barrier failed, or the writer stopped.
    Io(String),
}

impl std::fmt::Display for FlushError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FlushError::Timeout => write!(f, "session flush timed out"),
            FlushError::Io(message) => write!(f, "session write failed: {message}"),
        }
    }
}

impl std::error::Error for FlushError {}

/// One-shot result slot a barrier completes on the writer thread.
struct Latch {
    result: Mutex<Option<Result<(), String>>>,
    ready: Condvar,
}

impl Latch {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            result: Mutex::new(None),
            ready: Condvar::new(),
        })
    }

    fn complete(&self, result: Result<(), String>) {
        *lock(&self.result) = Some(result);
        self.ready.notify_all();
    }
}

/// A flush barrier. [`wait_blocking`](Self::wait_blocking) or
/// [`wait`](Self::wait) returns once every operation queued before the barrier
/// is written.
pub struct FlushTicket {
    latch: Arc<Latch>,
}

impl FlushTicket {
    /// Waits up to `deadline` on the calling thread. Do not call it on a UI
    /// thread or a `hoocode-io` worker; use [`wait`](Self::wait) there.
    pub fn wait_blocking(&self, deadline: Duration) -> Result<(), FlushError> {
        let guard = lock(&self.latch.result);
        let (guard, _) = self
            .latch
            .ready
            .wait_timeout_while(guard, deadline, |result| result.is_none())
            .unwrap_or_else(|e| e.into_inner());
        match &*guard {
            Some(Ok(())) => Ok(()),
            Some(Err(message)) => Err(FlushError::Io(message.clone())),
            None => Err(FlushError::Timeout),
        }
    }

    /// Waits up to `deadline` without blocking the async runtime: the wait runs
    /// on the `hoocode-tools` pool.
    pub async fn wait(self, deadline: Duration) -> Result<(), FlushError> {
        match run_blocking(move || self.wait_blocking(deadline)).await {
            Ok(result) => result,
            Err(e) => Err(FlushError::Io(e.to_string())),
        }
    }
}

enum Op {
    /// Append `line` and a newline to `path`. The file must already exist.
    Append { path: PathBuf, line: String },
    /// Replace the whole of `path` with `contents`, creating parent directories.
    Replace { path: PathBuf, contents: String },
    /// Complete `latch` once every earlier operation is written.
    Barrier(Arc<Latch>),
}

#[derive(Default)]
struct Queue {
    ops: VecDeque<Op>,
    /// Entries (`Append` and `Replace`) waiting in `ops`.
    entries: usize,
    /// Bytes waiting in `ops`.
    bytes: usize,
    /// True while the thread is writing a batch it took from `ops`.
    busy: bool,
    /// Set by `Drop`: the thread finishes the queue, then exits.
    closed: bool,
    /// Set when the thread has stopped (after a panic or a normal exit).
    dead: bool,
    /// First write error since the last barrier.
    error: Option<String>,
}

struct Shared {
    state: Mutex<Queue>,
    not_empty: Condvar,
    not_full: Condvar,
    max_entries: usize,
    max_bytes: usize,
    /// Set by [`FileWriter::abandon`]: the thread stops without writing the queue.
    abandoned: AtomicBool,
}

/// The single writer of one set of files, on one named thread.
///
/// Use [`session_io`] in the product. [`FileWriter::start`] is for tests and
/// other uses that need their own writer.
pub struct FileWriter {
    shared: Arc<Shared>,
}

impl FileWriter {
    /// Starts a writer thread called `name` with a queue of at most
    /// `max_entries` operations or `max_bytes` bytes. A single operation larger
    /// than `max_bytes` is accepted when the queue is empty.
    pub fn start(name: &str, max_entries: usize, max_bytes: usize) -> io::Result<FileWriter> {
        let shared = Arc::new(Shared {
            state: Mutex::new(Queue::default()),
            not_empty: Condvar::new(),
            not_full: Condvar::new(),
            max_entries: max_entries.max(1),
            max_bytes: max_bytes.max(1),
            abandoned: AtomicBool::new(false),
        });
        let thread_shared = Arc::clone(&shared);
        // The thread is detached: its handle is dropped, and `Drop` closes the queue.
        spawn_named_thread(name, move || run(thread_shared))?;
        Ok(FileWriter { shared })
    }

    /// Queues one line for `path`. Waits while the queue is full.
    pub fn append_line(&self, path: impl Into<PathBuf>, line: String) {
        let bytes = line.len() + 1;
        self.push(
            Op::Append {
                path: path.into(),
                line,
            },
            1,
            bytes,
        );
    }

    /// Queues a whole-file write of `path`. Waits while the queue is full.
    pub fn replace(&self, path: impl Into<PathBuf>, contents: String) {
        let bytes = contents.len();
        self.push(
            Op::Replace {
                path: path.into(),
                contents,
            },
            1,
            bytes,
        );
    }

    /// Queues a flush barrier. It takes no queue space, so it never waits.
    /// When nothing is queued it completes at once.
    pub fn barrier(&self) -> FlushTicket {
        let latch = Latch::new();
        let mut q = lock(&self.shared.state);
        if q.dead {
            latch.complete(Err("session writer stopped".to_owned()));
        } else if q.ops.is_empty() && !q.busy {
            let result = match q.error.take() {
                Some(message) => Err(message),
                None => Ok(()),
            };
            latch.complete(result);
        } else {
            q.ops.push_back(Op::Barrier(Arc::clone(&latch)));
            self.shared.not_empty.notify_one();
        }
        FlushTicket { latch }
    }

    /// [`barrier`](Self::barrier), waited on for up to `deadline` on this thread.
    pub fn flush_blocking(&self, deadline: Duration) -> Result<(), FlushError> {
        self.barrier().wait_blocking(deadline)
    }

    /// Simulates a crash for tests: the thread stops after the write it is in
    /// the middle of, and queued operations are never written.
    #[doc(hidden)]
    pub fn abandon(self) {
        self.shared.abandoned.store(true, Ordering::SeqCst);
        self.shared.not_empty.notify_all();
        // `self` drops here; `Drop` closes the queue as well.
    }

    fn push(&self, op: Op, entries: usize, bytes: usize) {
        let shared = &*self.shared;
        let mut q = lock(&shared.state);
        while !q.dead
            && !q.ops.is_empty()
            && (q.entries + entries > shared.max_entries || q.bytes + bytes > shared.max_bytes)
        {
            q = shared.not_full.wait(q).unwrap_or_else(|e| e.into_inner());
        }
        if q.dead {
            // Only after the writer thread has panicked; the panic is already logged.
            return;
        }
        q.entries += entries;
        q.bytes += bytes;
        q.ops.push_back(op);
        shared.not_empty.notify_one();
    }
}

impl Drop for FileWriter {
    fn drop(&mut self) {
        lock(&self.shared.state).closed = true;
        self.shared.not_empty.notify_all();
    }
}

/// Marks the writer dead when its thread exits, normally or by a panic, and
/// completes any barrier still queued so no waiter is left without an answer.
struct DeathWatch(Arc<Shared>);

impl Drop for DeathWatch {
    fn drop(&mut self) {
        let shared = &self.0;
        let mut q = lock(&shared.state);
        q.dead = true;
        q.busy = false;
        q.entries = 0;
        q.bytes = 0;
        for op in q.ops.drain(..) {
            if let Op::Barrier(latch) = op {
                latch.complete(Err("session writer stopped".to_owned()));
            }
        }
        drop(q);
        shared.not_full.notify_all();
        shared.not_empty.notify_all();
    }
}

fn run(shared: Arc<Shared>) {
    let _watch = DeathWatch(Arc::clone(&shared));
    let mut out = Output::default();
    loop {
        let batch = {
            let mut q = lock(&shared.state);
            while q.ops.is_empty() && !q.closed && !shared.abandoned.load(Ordering::SeqCst) {
                q = shared.not_empty.wait(q).unwrap_or_else(|e| e.into_inner());
            }
            if shared.abandoned.load(Ordering::SeqCst) {
                return;
            }
            if q.ops.is_empty() {
                // Closed and drained.
                return;
            }
            q.busy = true;
            q.entries = 0;
            q.bytes = 0;
            shared.not_full.notify_all();
            std::mem::take(&mut q.ops)
        };
        for op in batch {
            if shared.abandoned.load(Ordering::SeqCst) {
                return;
            }
            out.apply(op, &shared);
        }
        out.flush_pending();
        out.publish_error(&shared);
        lock(&shared.state).busy = false;
        shared.not_full.notify_all();
    }
}

/// Writer-thread state: the open file and the bytes waiting for it.
#[derive(Default)]
struct Output {
    /// The file `Append` writes go to, kept open between batches.
    open: Option<(PathBuf, File)>,
    /// Lines for `pending_path`, written with one `write_all`.
    pending: Vec<u8>,
    pending_path: Option<PathBuf>,
    /// First error this batch, moved to the shared queue by `publish_error`.
    error: Option<String>,
}

impl Output {
    fn apply(&mut self, op: Op, shared: &Shared) {
        match op {
            Op::Append { path, line } => {
                if self.pending_path.as_deref() != Some(path.as_path()) {
                    self.flush_pending();
                    self.pending_path = Some(path);
                }
                self.pending.extend_from_slice(line.as_bytes());
                self.pending.push(b'\n');
            }
            Op::Replace { path, contents } => {
                self.flush_pending();
                self.publish_error(shared);
                self.open = None;
                let result = write_whole(&path, contents.as_bytes());
                self.record(result);
                self.publish_error(shared);
            }
            Op::Barrier(latch) => {
                self.flush_pending();
                self.publish_error(shared);
                let result = lock(&shared.state).error.take();
                latch.complete(match result {
                    Some(message) => Err(message),
                    None => Ok(()),
                });
            }
        }
    }

    fn flush_pending(&mut self) {
        let Some(path) = self.pending_path.take() else {
            return;
        };
        let bytes = std::mem::take(&mut self.pending);
        let result = self.write_append(&path, &bytes);
        self.record(result);
    }

    fn write_append(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        let same_file = matches!(&self.open, Some((open, _)) if open == path);
        let mut prefix_newline = false;
        if !same_file {
            self.open = None;
            let mut file = OpenOptions::new().append(true).read(true).open(path)?;
            // A crash can leave a torn last line. Start on a fresh line so the
            // next entry is not glued onto it.
            prefix_newline = ends_without_newline(&mut file)?;
            self.open = Some((path.to_path_buf(), file));
        }
        let Some((_, file)) = self.open.as_mut() else {
            return Ok(());
        };
        let result = if prefix_newline {
            file.write_all(b"\n").and_then(|()| file.write_all(bytes))
        } else {
            file.write_all(bytes)
        };
        if result.is_err() {
            // Reopen on the next write; the handle may be stale.
            self.open = None;
        }
        result
    }

    fn record(&mut self, result: io::Result<()>) {
        if let Err(e) = result {
            if self.error.is_none() {
                self.error = Some(e.to_string());
            }
        }
    }

    fn publish_error(&mut self, shared: &Shared) {
        if let Some(message) = self.error.take() {
            lock(&shared.state).error.get_or_insert(message);
        }
    }
}

/// True when the file has content and its last byte is not a newline.
fn ends_without_newline(file: &mut File) -> io::Result<bool> {
    use std::io::{Read, Seek, SeekFrom};
    if file.metadata()?.len() == 0 {
        return Ok(false);
    }
    file.seek(SeekFrom::End(-1))?;
    let mut last = [0u8; 1];
    file.read_exact(&mut last)?;
    Ok(last[0] != b'\n')
}

fn write_whole(path: &Path, contents: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, contents)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

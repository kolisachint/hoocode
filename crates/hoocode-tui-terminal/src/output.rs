//! The `hoocode-term-out` thread: the only writer of terminal output.
//!
//! Every byte the TUI sends to the terminal goes through this thread, in the
//! order it was sent: frames, cursor and mode escapes, and the escapes that the
//! input and progress threads send. Writes keep their order, so an escape sent
//! between two frames lands between them on screen.
//!
//! Queue policy (`docs/design/concurrency.md` section 3):
//! - **Frames: one pending slot.** A frame is a diff against the previous one, so
//!   a frame cannot be dropped or replaced without the screen going wrong. The
//!   TUI therefore builds the next frame only after this thread has taken the
//!   previous one ([`OutputHandle::frame_pending`]). The latest state is painted
//!   as soon as the terminal takes the frame.
//! - **Other writes: wait.** Control writes are small and never dropped. The
//!   channel holds [`OUTPUT_QUEUE_CAP`] of them; a full channel makes the sender
//!   wait, which only happens if the terminal has stopped reading for a long time.
//! - **Shutdown: deadline.** [`OutputThread::finish`] writes what it can within
//!   the deadline and then drops the rest, so a stalled terminal cannot hang exit.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use hoocode_runtime::{spawn_named_thread, sync_bounded_channel};

/// Name of the output thread (it shows in logs and panic reports).
pub const OUTPUT_THREAD_NAME: &str = "hoocode-term-out";
/// Most control writes queued ahead of the writer.
pub const OUTPUT_QUEUE_CAP: usize = 1024;

enum Command {
    /// Escapes and text outside a frame; written in order.
    Bytes(String),
    /// One frame; the only write the TUI waits on.
    Frame(String),
    /// Writes nothing more; the thread exits.
    Stop,
}

/// Callback the writer runs when a frame the TUI asked about is written.
pub type DrainedWake = Box<dyn FnMut() + Send>;

/// The sending side. Cheap to clone; every thread that writes to the terminal
/// holds one.
#[derive(Clone)]
pub struct OutputHandle {
    tx: SyncSender<Command>,
    frame_pending: Arc<AtomicBool>,
    wake_wanted: Arc<AtomicBool>,
}

impl OutputHandle {
    /// Queues escapes or text outside a frame. Waits only when the queue is full.
    pub fn write(&self, data: &str) {
        if data.is_empty() {
            return;
        }
        let _ = self.tx.send(Command::Bytes(data.to_owned()));
    }

    /// Queues one frame. The caller must not queue another frame until
    /// [`frame_pending`](Self::frame_pending) is false.
    pub fn write_frame(&self, data: &str) {
        self.frame_pending.store(true, Ordering::SeqCst);
        if self.tx.send(Command::Frame(data.to_owned())).is_err() {
            // The writer is gone; nothing would clear the flag otherwise.
            self.frame_pending.store(false, Ordering::SeqCst);
        }
    }

    /// True from [`write_frame`](Self::write_frame) until the writer has written that frame.
    pub fn frame_pending(&self) -> bool {
        self.frame_pending.load(Ordering::SeqCst)
    }

    /// Asks the writer to run its drained callback after the pending frame is written.
    /// Call it before checking `frame_pending`, so a frame written in between is not missed.
    pub fn notify_when_drained(&self) {
        self.wake_wanted.store(true, Ordering::SeqCst);
    }
}

/// The writer thread and its handle.
pub struct OutputThread {
    handle: OutputHandle,
    join: Option<JoinHandle<()>>,
}

impl OutputThread {
    /// Starts the writer on `sink`. `log` gets a copy of every write (the
    /// `HOOCODE_TUI_WRITE_LOG` file). `on_drained` runs after each frame the
    /// TUI asked about (see [`OutputHandle::notify_when_drained`]).
    pub fn spawn(
        sink: Box<dyn Write + Send>,
        log: Option<PathBuf>,
        on_drained: Option<DrainedWake>,
    ) -> io::Result<Self> {
        let (tx, rx) = sync_bounded_channel(OUTPUT_QUEUE_CAP);
        let frame_pending = Arc::new(AtomicBool::new(false));
        let wake_wanted = Arc::new(AtomicBool::new(false));
        let handle = OutputHandle {
            tx,
            frame_pending: frame_pending.clone(),
            wake_wanted: wake_wanted.clone(),
        };
        let join = spawn_named_thread(OUTPUT_THREAD_NAME, move || {
            run(rx, sink, log, &frame_pending, &wake_wanted, on_drained)
        })?;
        Ok(Self {
            handle,
            join: Some(join),
        })
    }

    /// A handle for another thread to write with.
    pub fn handle(&self) -> OutputHandle {
        self.handle.clone()
    }

    /// Writes what is queued, then stops the thread. Waits at most `deadline`;
    /// output still queued after that is dropped. Returns false when it dropped output.
    pub fn finish(mut self, deadline: Duration) -> bool {
        let started = Instant::now();
        let mut stop = Command::Stop;
        loop {
            match self.handle.tx.try_send(stop) {
                Ok(()) => break,
                // The writer already exited.
                Err(TrySendError::Disconnected(_)) => break,
                Err(TrySendError::Full(back)) => {
                    if started.elapsed() >= deadline {
                        return false;
                    }
                    stop = back;
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        }
        let Some(join) = self.join.take() else {
            return true;
        };
        while !join.is_finished() {
            if started.elapsed() >= deadline {
                // Still blocked in a write to the terminal. The thread is
                // detached; the process can exit without it.
                return false;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let _ = join.join();
        true
    }
}

fn run(
    rx: Receiver<Command>,
    mut sink: Box<dyn Write + Send>,
    log: Option<PathBuf>,
    frame_pending: &AtomicBool,
    wake_wanted: &AtomicBool,
    mut on_drained: Option<DrainedWake>,
) {
    while let Ok(command) = rx.recv() {
        match command {
            Command::Bytes(data) => emit(sink.as_mut(), log.as_deref(), &data),
            Command::Frame(data) => {
                emit(sink.as_mut(), log.as_deref(), &data);
                // Clear the flag before reading `wake_wanted`: a UI thread that
                // sets the wish first and then reads the flag cannot miss this.
                frame_pending.store(false, Ordering::SeqCst);
                if wake_wanted.swap(false, Ordering::SeqCst) {
                    if let Some(wake) = on_drained.as_mut() {
                        wake();
                    }
                }
            }
            Command::Stop => break,
        }
    }
}

/// Writes `data` to the sink and the log, then flushes the sink.
pub(crate) fn emit(sink: &mut dyn Write, log: Option<&Path>, data: &str) {
    let _ = sink.write_all(data.as_bytes());
    let _ = sink.flush();
    if let Some(path) = log {
        write_log(path, data);
    }
}

/// Appends `data` to the write log. Errors are ignored: logging must not break the screen.
pub(crate) fn write_log(path: &Path, data: &str) {
    use std::fs::OpenOptions;
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = f.write_all(data.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Condvar, Mutex};

    /// A sink that appends to a shared buffer, and can be held shut.
    #[derive(Clone)]
    struct Sink {
        out: Arc<Mutex<Vec<u8>>>,
        gate: Arc<(Mutex<bool>, Condvar)>,
    }

    impl Sink {
        fn open_gate() -> Self {
            Self {
                out: Arc::default(),
                gate: Arc::new((Mutex::new(true), Condvar::new())),
            }
        }
        fn closed_gate() -> Self {
            Self {
                out: Arc::default(),
                gate: Arc::new((Mutex::new(false), Condvar::new())),
            }
        }
        fn open(&self) {
            let (lock, cv) = &*self.gate;
            *lock.lock().unwrap() = true;
            cv.notify_all();
        }
        fn text(&self) -> String {
            String::from_utf8(self.out.lock().unwrap().clone()).unwrap()
        }
    }

    impl Write for Sink {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let (lock, cv) = &*self.gate;
            let mut open = lock.lock().unwrap();
            while !*open {
                open = cv.wait(open).unwrap();
            }
            self.out.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn writes_keep_their_order_across_frames() {
        let sink = Sink::open_gate();
        let thread = OutputThread::spawn(Box::new(sink.clone()), None, None).unwrap();
        let out = thread.handle();
        out.write("a");
        out.write_frame("[frame]");
        out.write("b");
        assert!(thread.finish(Duration::from_secs(5)));
        assert_eq!(sink.text(), "a[frame]b");
    }

    #[test]
    fn frame_is_pending_until_written_and_wakes_once() {
        let sink = Sink::closed_gate();
        let wakes = Arc::new(Mutex::new(0u32));
        let counter = wakes.clone();
        let thread = OutputThread::spawn(
            Box::new(sink.clone()),
            None,
            Some(Box::new(move || *counter.lock().unwrap() += 1)),
        )
        .unwrap();
        let out = thread.handle();
        out.write_frame("frame");
        assert!(out.frame_pending());
        out.notify_when_drained();
        sink.open();
        assert!(thread.finish(Duration::from_secs(5)));
        assert!(!out.frame_pending());
        assert_eq!(sink.text(), "frame");
        assert_eq!(*wakes.lock().unwrap(), 1);
    }

    #[test]
    fn finish_drops_output_a_stalled_terminal_never_takes() {
        let sink = Sink::closed_gate();
        let thread = OutputThread::spawn(Box::new(sink.clone()), None, None).unwrap();
        thread.handle().write_frame("never shown");
        let started = Instant::now();
        assert!(!thread.finish(Duration::from_millis(100)));
        assert!(started.elapsed() < Duration::from_secs(2));
        // Let the detached thread finish so the test does not leave it blocked.
        sink.open();
    }

    #[test]
    fn log_gets_every_write() {
        let dir = std::env::temp_dir().join(format!("hoocode-term-out-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("write.log");
        let _ = std::fs::remove_file(&log);
        let sink = Sink::open_gate();
        let thread = OutputThread::spawn(Box::new(sink.clone()), Some(log.clone()), None).unwrap();
        thread.handle().write("x");
        thread.handle().write_frame("y");
        assert!(thread.finish(Duration::from_secs(5)));
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "xy");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

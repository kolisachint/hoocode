//! The process-wide stdin reader (`hoocode-input`) and what it feeds.
//!
//! One thread reads stdin for the process and hands each chunk to the terminal
//! that is running; chunks wait while none is. The reader also owns the ESC
//! timer: an incomplete sequence (a lone ESC above all) is flushed once its
//! deadline passes with no more input. On Unix the reader waits in `poll(2)`
//! with that deadline as its timeout, so no thread sleeps for it. Elsewhere a
//! short-lived timer thread does the same.
//!
//! Every chunk goes through [`interrupt::observe`] before the terminal sees it,
//! so Ctrl+C aborts even when the UI loop is busy.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Once};
use std::time::Instant;

use hoocode_runtime::spawn_named_thread;

use crate::interrupt;
use crate::{parse_kitty_query_response, StdinBuffer, StdinBufferOptions, StdinEvent, Writer};

/// What a running terminal does with stdin.
pub(crate) struct Feed {
    forwarding: Arc<AtomicBool>,
    kitty_active: Arc<AtomicBool>,
    last_input_at: Arc<Mutex<Instant>>,
    buf: StdinBuffer,
    writer: Writer,
    on_input: Box<dyn FnMut(&str) + Send>,
    /// When the incomplete sequence in `buf` is flushed (the ESC timer).
    flush_at: Option<Instant>,
}

impl Feed {
    pub(crate) fn new(
        forwarding: Arc<AtomicBool>,
        kitty_active: Arc<AtomicBool>,
        last_input_at: Arc<Mutex<Instant>>,
        writer: Writer,
        on_input: Box<dyn FnMut(&str) + Send>,
    ) -> Self {
        Self {
            forwarding,
            kitty_active,
            last_input_at,
            buf: StdinBuffer::new(StdinBufferOptions::default()),
            writer,
            on_input,
            flush_at: None,
        }
    }

    /// One raw chunk of input, in order. It (re)arms the ESC timer when it
    /// leaves an incomplete sequence behind.
    fn chunk(&mut self, bytes: &[u8], now: Instant) {
        *lock(&self.last_input_at) = now;
        let text = if bytes.len() == 1 && bytes[0] > 127 {
            format!("\x1b{}", (bytes[0] - 128) as char)
        } else {
            String::from_utf8_lossy(bytes).into_owned()
        };
        let events = self.buf.process(&text);
        // Taken after `process`: the buffer stamps the start of the sequence
        // with its own clock, which is no later than this one, so the deadline
        // is never earlier than the buffer's own timeout.
        self.flush_at = self
            .buf
            .has_pending()
            .then(|| Instant::now() + self.buf.timeout());
        self.deliver(events);
    }

    /// Flushes the incomplete sequence once its deadline has passed.
    fn tick(&mut self, now: Instant) {
        if self.flush_at.is_some_and(|due| now >= due) {
            self.flush_at = None;
            let events = self.buf.poll_timeout(now);
            self.deliver(events);
            if self.buf.has_pending() {
                // Not yet old enough by the buffer's clock: check again later.
                self.flush_at = Some(Instant::now() + self.buf.timeout());
            }
        }
    }

    fn deliver(&mut self, events: Vec<StdinEvent>) {
        for event in events {
            if !self.forwarding.load(Ordering::SeqCst) {
                continue;
            }
            match event {
                StdinEvent::Data(seq) => {
                    // The Kitty protocol query response is ours, not the caller's.
                    if !self.kitty_active.load(Ordering::SeqCst) && parse_kitty_query_response(&seq)
                    {
                        self.kitty_active.store(true, Ordering::SeqCst);
                        self.writer.write("\x1b[>7u");
                        continue;
                    }
                    (self.on_input)(&seq);
                }
                StdinEvent::Paste(content) => {
                    (self.on_input)(&format!("\x1b[200~{content}\x1b[201~"));
                }
            }
        }
    }
}

/// Locks a mutex, ignoring poisoning (a panicked listener must not stop input).
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// The hub's state. Lock order: `HUB` before a `Feed`; the timer never takes `HUB`.
struct Hub {
    feed: Option<Arc<Mutex<Feed>>>,
    pending: Vec<Vec<u8>>,
}

static HUB: Mutex<Hub> = Mutex::new(Hub {
    feed: None,
    pending: Vec::new(),
});
static READER: Once = Once::new();

/// Makes `feed` the receiver of stdin, replaying what arrived while none was.
pub(crate) fn subscribe(feed: Arc<Mutex<Feed>>) {
    let mut hub = lock(&HUB);
    let now = Instant::now();
    let pending = std::mem::take(&mut hub.pending);
    {
        let mut receiver = lock(&feed);
        for chunk in pending {
            receiver.chunk(&chunk, now);
        }
    }
    hub.feed = Some(feed);
    drop(hub);
    READER.call_once(|| {
        let _ = spawn_named_thread("hoocode-input", reader);
    });
}

/// Stops delivering stdin; input waits for the next subscriber.
pub(crate) fn unsubscribe() {
    lock(&HUB).feed = None;
}

fn current_feed() -> Option<Arc<Mutex<Feed>>> {
    lock(&HUB).feed.clone()
}

/// The reader loop: wait for input or the ESC deadline, then deliver.
fn reader() {
    let mut chunk = [0u8; 4096];
    loop {
        let deadline = current_feed().and_then(|feed| lock(&feed).flush_at);
        match wait_readable(deadline) {
            Readiness::Readable => {
                let n = match read_stdin(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                };
                let now = Instant::now();
                let bytes = &chunk[..n];
                interrupt::observe(bytes, now);
                let mut hub = lock(&HUB);
                match hub.feed.clone() {
                    Some(feed) => {
                        lock(&feed).chunk(bytes, now);
                        drop(hub);
                        arm_flush_timer(&feed);
                    }
                    None => hub.pending.push(bytes.to_vec()),
                }
            }
            Readiness::TimedOut => {
                if let Some(feed) = current_feed() {
                    lock(&feed).tick(Instant::now());
                }
            }
            Readiness::Interrupted => {}
            Readiness::Failed => break,
        }
    }
}

enum Readiness {
    Readable,
    TimedOut,
    Interrupted,
    Failed,
}

/// Waits until stdin is readable or `deadline` passes.
#[cfg(unix)]
fn wait_readable(deadline: Option<Instant>) -> Readiness {
    let timeout_ms = match deadline {
        None => -1,
        Some(due) => {
            let wait = due.saturating_duration_since(Instant::now());
            // Rounded up: an early wake-up costs one more pass, a late one is a late flush.
            libc::c_int::try_from(wait.as_micros().div_ceil(1000)).unwrap_or(libc::c_int::MAX)
        }
    };
    let mut fds = libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: one valid pollfd, and the count matches.
    let ready = unsafe { libc::poll(&mut fds, 1, timeout_ms) };
    match ready {
        r if r > 0 => Readiness::Readable,
        0 => Readiness::TimedOut,
        _ => match io::Error::last_os_error().kind() {
            io::ErrorKind::Interrupted => Readiness::Interrupted,
            _ => Readiness::Failed,
        },
    }
}

/// Elsewhere the read itself blocks; the ESC timer is a thread (see
/// [`arm_flush_timer`]), so there is nothing to wait for here.
#[cfg(not(unix))]
fn wait_readable(_deadline: Option<Instant>) -> Readiness {
    Readiness::Readable
}

/// Reads stdin without std's buffer, so `poll` sees exactly what is unread.
#[cfg(unix)]
fn read_stdin(buf: &mut [u8]) -> io::Result<usize> {
    // SAFETY: `buf` is valid for `buf.len()` bytes.
    let n = unsafe { libc::read(libc::STDIN_FILENO, buf.as_mut_ptr().cast(), buf.len()) };
    usize::try_from(n).map_err(|_| io::Error::last_os_error())
}

#[cfg(not(unix))]
fn read_stdin(buf: &mut [u8]) -> io::Result<usize> {
    use std::io::Read;
    io::stdin().read(buf)
}

#[cfg(unix)]
fn arm_flush_timer(_feed: &Arc<Mutex<Feed>>) {}

/// Elsewhere: a timer thread for the ESC flush, armed after each chunk that
/// leaves an incomplete sequence.
#[cfg(not(unix))]
fn arm_flush_timer(feed: &Arc<Mutex<Feed>>) {
    let Some(due) = lock(feed).flush_at else {
        return;
    };
    let feed = feed.clone();
    let _ = spawn_named_thread("hoocode-input-timer", move || {
        std::thread::sleep(due.saturating_duration_since(Instant::now()));
        lock(&feed).tick(Instant::now());
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ESC timer: a lone ESC is held back, then flushed at its deadline,
    /// and input that completes it first keeps it from flushing.
    #[test]
    fn lone_escape_flushes_at_its_deadline_only() {
        let delivered = Arc::new(Mutex::new(Vec::<String>::new()));
        let sink = delivered.clone();
        let mut feed = Feed::new(
            Arc::new(AtomicBool::new(true)),
            Arc::new(AtomicBool::new(true)),
            Arc::new(Mutex::new(Instant::now())),
            Writer {
                handle: None,
                log: None,
            },
            Box::new(move |seq: &str| lock(&sink).push(seq.to_owned())),
        );
        let t0 = Instant::now();
        feed.chunk(b"\x1b", t0);
        assert!(
            lock(&delivered).is_empty(),
            "a lone ESC waits for more input"
        );
        let due = feed.flush_at.expect("ESC arms the timer");
        feed.tick(t0);
        assert!(lock(&delivered).is_empty(), "no flush before the deadline");
        feed.tick(due);
        assert_eq!(lock(&delivered).as_slice(), ["\x1b".to_owned()]);
        assert!(feed.flush_at.is_none());

        // Input that completes the sequence cancels the wait.
        feed.chunk(b"\x1b", Instant::now());
        feed.chunk(b"[A", Instant::now());
        assert!(feed.flush_at.is_none());
        assert_eq!(lock(&delivered).last().map(String::as_str), Some("\x1b[A"));
    }
}

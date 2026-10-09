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

use hoocode_tui_keys::{is_kitty_protocol_active, set_kitty_protocol_active};

use crate::interrupt;
use crate::{parse_kitty_query_response, StdinBuffer, StdinBufferOptions, StdinEvent, Writer};

/// What a running terminal does with stdin.
pub(crate) struct Feed {
    forwarding: Arc<AtomicBool>,
    last_input_at: Arc<Mutex<Instant>>,
    buf: StdinBuffer,
    writer: Writer,
    on_input: Box<dyn FnMut(&str) + Send>,
    /// When the incomplete sequence in `buf` is flushed (the ESC timer).
    flush_at: Option<Instant>,
    /// An incomplete UTF-8 sequence at the end of the last read, waiting for
    /// its continuation bytes.
    utf8_tail: Vec<u8>,
    /// True when `utf8_tail` is one lead byte that arrived alone. If its
    /// continuation never comes it is Alt+key (Meta), not text.
    tail_lone: bool,
}

impl Feed {
    pub(crate) fn new(
        forwarding: Arc<AtomicBool>,
        last_input_at: Arc<Mutex<Instant>>,
        writer: Writer,
        on_input: Box<dyn FnMut(&str) + Send>,
    ) -> Self {
        Self {
            forwarding,
            last_input_at,
            buf: StdinBuffer::new(StdinBufferOptions::default()),
            writer,
            on_input,
            flush_at: None,
            utf8_tail: Vec::new(),
            tail_lone: false,
        }
    }

    /// One raw chunk of input, in order. It (re)arms the ESC timer when it
    /// leaves an incomplete sequence behind.
    pub(crate) fn chunk(&mut self, bytes: &[u8], now: Instant) {
        *lock(&self.last_input_at) = now;
        let mut text = String::new();
        let mut data = std::mem::take(&mut self.utf8_tail);
        if self.tail_lone && bytes.first().is_some_and(|b| !is_continuation(*b)) {
            // The lone lead byte never got its continuation: it is Meta.
            text.push_str(&meta_text(data[0]));
            data.clear();
        }
        self.tail_lone = false;
        if data.is_empty() && bytes.len() == 1 && bytes[0] > 127 && !is_utf8_lead(bytes[0]) {
            text.push_str(&meta_text(bytes[0]));
        } else {
            data.extend_from_slice(bytes);
            let split = utf8_complete_len(&data);
            text.push_str(&String::from_utf8_lossy(&data[..split]));
            self.tail_lone = data.len() == 1 && split == 0;
            self.utf8_tail = data.split_off(split);
        }
        let events = if text.is_empty() {
            Vec::new()
        } else {
            self.buf.process(&text)
        };
        // Taken after `process`: the buffer stamps the start of the sequence
        // with its own clock, which is no later than this one, so the deadline
        // is never earlier than the buffer's own timeout.
        self.arm_flush();
        self.deliver(events);
    }

    /// Sets the ESC-timer deadline while an incomplete sequence is held.
    fn arm_flush(&mut self) {
        self.flush_at = (self.buf.has_pending() || !self.utf8_tail.is_empty())
            .then(|| Instant::now() + self.buf.timeout());
    }

    /// Flushes the incomplete sequence once its deadline has passed.
    fn tick(&mut self, now: Instant) {
        if self.flush_at.is_some_and(|due| now >= due) {
            self.flush_at = None;
            let mut events = self.buf.poll_timeout(now);
            if !self.utf8_tail.is_empty() {
                // The continuation did not come in time: a lone lead byte is
                // Meta, a broken sequence is replacement text.
                let tail = std::mem::take(&mut self.utf8_tail);
                let text = if std::mem::take(&mut self.tail_lone) {
                    meta_text(tail[0])
                } else {
                    String::from_utf8_lossy(&tail).into_owned()
                };
                events.extend(self.buf.process(&text));
            }
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
                    if !is_kitty_protocol_active() && parse_kitty_query_response(&seq) {
                        set_kitty_protocol_active(true);
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

/// A high byte that is Alt+key: ESC then the key with the high bit cleared.
fn meta_text(byte: u8) -> String {
    format!("\x1b{}", (byte - 128) as char)
}

/// True for a UTF-8 continuation byte.
fn is_continuation(byte: u8) -> bool {
    (0x80..=0xBF).contains(&byte)
}

/// True for a byte that starts a multi-byte UTF-8 sequence.
fn is_utf8_lead(byte: u8) -> bool {
    (0xC2..=0xF4).contains(&byte)
}

/// The length of the prefix of `data` that is complete: everything up to a
/// trailing incomplete UTF-8 sequence. Invalid bytes count as complete (they
/// decode to replacement characters).
fn utf8_complete_len(data: &[u8]) -> usize {
    let mut offset = 0;
    let mut rest = data;
    loop {
        match std::str::from_utf8(rest) {
            Ok(_) => return data.len(),
            Err(e) => match e.error_len() {
                None => return offset + e.valid_up_to(),
                Some(invalid) => {
                    let skip = e.valid_up_to() + invalid;
                    offset += skip;
                    rest = &rest[skip..];
                }
            },
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

    /// A feed that records every delivered sequence.
    fn recording_feed() -> (Feed, Arc<Mutex<Vec<String>>>) {
        let delivered = Arc::new(Mutex::new(Vec::<String>::new()));
        let sink = delivered.clone();
        let feed = Feed::new(
            Arc::new(AtomicBool::new(true)),
            Arc::new(Mutex::new(Instant::now())),
            Writer {
                handle: None,
                log: None,
            },
            Box::new(move |seq: &str| lock(&sink).push(seq.to_owned())),
        );
        (feed, delivered)
    }

    /// Feeds `chunks` as separate reads, lets the ESC timer run out, and
    /// returns everything delivered, concatenated.
    fn feed_reads(chunks: &[&[u8]]) -> String {
        let (mut feed, delivered) = recording_feed();
        let now = Instant::now();
        for chunk in chunks {
            feed.chunk(chunk, now);
        }
        if let Some(due) = feed.flush_at {
            feed.tick(due);
        }
        let got = lock(&delivered).concat();
        got
    }

    /// Every split point of every sample, as two reads, and one byte per read.
    #[test]
    fn utf8_text_survives_every_split_point() {
        for text in [
            "h\u{e9}llo \u{4e16}\u{754c} \u{1f389} end",
            "\u{e9}",
            "\u{1f389}",
        ] {
            let bytes = text.as_bytes();
            for split in 0..=bytes.len() {
                let got = feed_reads(&[&bytes[..split], &bytes[split..]]);
                assert_eq!(got, text, "two reads, split at {split}");
            }
            let one_by_one: Vec<&[u8]> = bytes.chunks(1).collect();
            assert_eq!(feed_reads(&one_by_one), text, "one byte per read");
        }
    }

    /// A lead byte alone at the end of a read is the start of a UTF-8
    /// sequence, not a Meta key, when the rest arrives in the next read.
    #[test]
    fn lone_utf8_lead_byte_waits_for_its_continuation() {
        assert_eq!(feed_reads(&[&[0xC3], &[0xA9]]), "\u{e9}");
        assert_eq!(feed_reads(&[&[0xE4], &[0xB8], &[0x96]]), "\u{4e16}");
        assert_eq!(
            feed_reads(&[&[0xF0], &[0x9F], &[0x8E], &[0x89]]),
            "\u{1f389}"
        );
    }

    /// A high byte that cannot start UTF-8 is still Alt+key (ESC prefix).
    #[test]
    fn high_byte_that_cannot_lead_utf8_is_meta() {
        assert_eq!(feed_reads(&[&[0xC1]]), "\x1bA");
        assert_eq!(feed_reads(&[&[0xFF]]), "\x1b\u{7f}");
    }

    /// A lone lead byte that never gets its continuation is flushed at the
    /// ESC deadline as Meta, and not before.
    #[test]
    fn lone_lead_byte_flushes_as_meta_at_its_deadline() {
        let (mut feed, delivered) = recording_feed();
        let t0 = Instant::now();
        feed.chunk(&[0xE9], t0);
        assert!(lock(&delivered).is_empty(), "the lead byte waits");
        let due = feed.flush_at.expect("a waiting lead byte arms the timer");
        feed.tick(t0);
        assert!(lock(&delivered).is_empty(), "no flush before the deadline");
        feed.tick(due);
        assert_eq!(lock(&delivered).concat(), "\x1bi");
        assert!(feed.flush_at.is_none());
    }

    /// A lone lead byte followed by a key that is not a continuation is Meta
    /// for the lead byte, and the key is read as typed.
    #[test]
    fn lone_lead_byte_then_ascii_is_meta_then_key() {
        assert_eq!(feed_reads(&[&[0xE9], b"x"]), "\x1bix");
    }
}

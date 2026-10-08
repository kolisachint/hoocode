//! Ctrl+C on the input thread, the UI heartbeat and the emergency exit
//! (`docs/design/concurrency.md` section 4).
//!
//! The `hoocode-input` reader looks for Ctrl+C in every raw chunk, before the UI
//! sees it, and fires the interrupt hook (the current turn's abort) at once. The
//! UI loop still gets the key for its own handling; an abort is idempotent, so
//! a second one does no harm.
//!
//! The UI loop calls [`ui_beat`] once per iteration. When Ctrl+C arrives twice
//! within a second and the UI has not beaten for 500 ms, the input thread
//! restores the terminal, flushes the session (1 s deadline) and exits 130.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// The UI counts as stalled after this long without a beat.
pub const UI_STALL: Duration = Duration::from_millis(500);
/// Two Ctrl+C presses this close together, with the UI stalled, exit the process.
pub const EMERGENCY_WINDOW: Duration = Duration::from_secs(1);
/// How long the emergency exit waits for the session to flush.
pub const EMERGENCY_FLUSH: Duration = Duration::from_secs(1);
/// Exit status of the emergency exit (the conventional SIGINT status).
pub const EMERGENCY_EXIT_CODE: i32 = 130;

/// Written to the terminal on the emergency exit, bypassing the output thread:
/// leave the alternate screen, show the cursor, turn off bracketed paste,
/// mouse reporting, the Kitty protocol and modifyOtherKeys.
#[cfg(unix)]
const RESTORE_SEQUENCE: &[u8] =
    b"\x1b[?1049l\x1b[?25h\x1b[?2004l\x1b[<u\x1b[>4;0m\x1b[?1006l\x1b[?1000l";

/// Kitty Ctrl+C (press, and the press event form), and modifyOtherKeys Ctrl+C.
const CTRL_C_SEQUENCES: [&[u8]; 3] = [b"\x1b[99;5u", b"\x1b[99;5:1u", b"\x1b[27;5;99~"];
const PASTE_START: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";

type Hook = Arc<dyn Fn() + Send + Sync>;

static HOOK: Mutex<Option<Hook>> = Mutex::new(None);
static EPOCH: OnceLock<Instant> = OnceLock::new();
/// Nanoseconds after [`EPOCH`] of the last UI beat, plus one (0 = never beat).
static LAST_BEAT: AtomicU64 = AtomicU64::new(0);
/// The input thread is inside a bracketed paste (Ctrl+C in a paste is text).
static IN_PASTE: AtomicBool = AtomicBool::new(false);
static LAST_PRESS: Mutex<Option<Instant>> = Mutex::new(None);

fn epoch() -> Instant {
    *EPOCH.get_or_init(Instant::now)
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// Sets the Ctrl+C hook: the current turn's abort. It runs on the input thread,
/// so it must return at once. `None` clears it.
pub fn set_hook(hook: Option<Hook>) {
    *lock(&HOOK) = hook;
}

/// The UI loop reports progress. Call it once per loop iteration.
pub fn ui_beat() {
    let nanos = epoch().elapsed().as_nanos() as u64;
    LAST_BEAT.store(nanos.saturating_add(1), Ordering::SeqCst);
}

/// Time since the UI last beat, or `None` if it never has.
fn ui_silence(now: Instant) -> Option<Duration> {
    let beat = LAST_BEAT.load(Ordering::SeqCst);
    if beat == 0 {
        return None;
    }
    let since_epoch = now.saturating_duration_since(epoch()).as_nanos() as u64;
    Some(Duration::from_nanos(since_epoch.saturating_sub(beat - 1)))
}

/// Counts the Ctrl+C presses in a raw input chunk, outside bracketed pastes.
fn count_presses(bytes: &[u8]) -> usize {
    let mut presses = 0;
    let mut in_paste = IN_PASTE.load(Ordering::SeqCst);
    let mut i = 0;
    while i < bytes.len() {
        let rest = &bytes[i..];
        if rest.starts_with(PASTE_START) {
            in_paste = true;
            i += PASTE_START.len();
            continue;
        }
        if rest.starts_with(PASTE_END) {
            in_paste = false;
            i += PASTE_END.len();
            continue;
        }
        if !in_paste {
            if let Some(seq) = CTRL_C_SEQUENCES.iter().find(|seq| rest.starts_with(seq)) {
                presses += 1;
                i += seq.len();
                continue;
            }
            if bytes[i] == 0x03 {
                presses += 1;
            }
        }
        i += 1;
    }
    IN_PASTE.store(in_paste, Ordering::SeqCst);
    presses
}

/// Looks at a raw input chunk, as the input thread receives it. Every Ctrl+C in
/// it fires the hook; a double press with the UI stalled exits the process.
pub fn observe(bytes: &[u8], now: Instant) {
    for _ in 0..count_presses(bytes) {
        press(now);
    }
}

fn press(now: Instant) {
    let previous = lock(&LAST_PRESS).replace(now);
    let hook = lock(&HOOK).clone();
    if let Some(hook) = hook {
        hook();
    }
    let stalled = ui_silence(now).is_some_and(|silence| silence >= UI_STALL);
    if emergency(previous, now, stalled) {
        emergency_exit();
    }
}

/// The emergency decision: a press within the window of the previous one, while
/// the UI is stalled.
fn emergency(previous: Option<Instant>, now: Instant, ui_stalled: bool) -> bool {
    ui_stalled
        && previous.is_some_and(|prev| now.saturating_duration_since(prev) <= EMERGENCY_WINDOW)
}

/// Restores the terminal, flushes the session (bounded) and exits 130.
fn emergency_exit() -> ! {
    restore_terminal();
    let _ = hoocode_runtime::session_io().flush_blocking(EMERGENCY_FLUSH);
    std::process::exit(EMERGENCY_EXIT_CODE);
}

#[cfg(unix)]
fn restore_terminal() {
    // Straight to the fd: the output thread may be stuck holding std's stdout lock.
    // SAFETY: the buffer is valid for its length; a failed write is ignored.
    unsafe {
        let _ = libc::write(
            libc::STDOUT_FILENO,
            RESTORE_SEQUENCE.as_ptr().cast(),
            RESTORE_SEQUENCE.len(),
        );
    }
    let _ = crossterm::terminal::disable_raw_mode();
}

#[cfg(not(unix))]
fn restore_terminal() {
    let _ = crossterm::terminal::disable_raw_mode();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The statics are process-wide: the tests that touch them run one at a time.
    static SERIAL: Mutex<()> = Mutex::new(());

    #[test]
    fn counts_ctrl_c_in_each_encoding() {
        let _guard = lock(&SERIAL);
        IN_PASTE.store(false, Ordering::SeqCst);
        assert_eq!(count_presses(b"\x03"), 1);
        assert_eq!(count_presses(b"a\x03\x03"), 2);
        assert_eq!(count_presses(b"\x1b[99;5u"), 1);
        assert_eq!(count_presses(b"\x1b[27;5;99~"), 1);
        assert_eq!(
            count_presses(b"\x1b[99;6u"),
            0,
            "Ctrl+Shift+C is not Ctrl+C"
        );
        assert_eq!(count_presses(b"hello"), 0);
    }

    #[test]
    fn ctrl_c_inside_a_bracketed_paste_is_text() {
        let _guard = lock(&SERIAL);
        IN_PASTE.store(false, Ordering::SeqCst);
        assert_eq!(count_presses(b"\x1b[200~a\x03b\x1b[201~"), 0);
        assert_eq!(count_presses(b"\x1b[200~a"), 0);
        assert_eq!(
            count_presses(b"\x03"),
            0,
            "paste state carries across chunks"
        );
        assert_eq!(count_presses(b"\x1b[201~\x03"), 1);
    }

    #[test]
    fn emergency_needs_a_stalled_ui_and_a_second_press_within_the_window() {
        let t0 = Instant::now();
        let soon = t0 + Duration::from_millis(900);
        let late = t0 + Duration::from_millis(1500);
        assert!(
            !emergency(None, t0, true),
            "first press is never an emergency"
        );
        assert!(emergency(Some(t0), soon, true));
        assert!(
            !emergency(Some(t0), soon, false),
            "a responsive UI handles it"
        );
        assert!(!emergency(Some(t0), late, true), "outside the 1 s window");
    }

    /// The fast path: the hook fires from the input side while the UI thread is
    /// blocked, well inside 500 ms.
    #[test]
    fn ctrl_c_fires_the_hook_while_the_ui_is_blocked() {
        let _guard = lock(&SERIAL);
        IN_PASTE.store(false, Ordering::SeqCst);
        let fired = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = fired.clone();
        set_hook(Some(Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        })));
        // No `ui_beat` here: the UI loop is not running, so it is stalled.
        let started = Instant::now();
        std::thread::scope(|scope| {
            scope.spawn(|| observe(b"\x03", Instant::now()));
        });
        assert_eq!(fired.load(Ordering::SeqCst), 1);
        assert!(started.elapsed() < UI_STALL);
        set_hook(None);
    }
}

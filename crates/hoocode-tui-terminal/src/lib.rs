//! Terminal abstraction for the hoocode TUI.
//!
//! Ported from TypeScript `@kolisachint/hoocode-tui` -> `terminal.ts` /
//! `stdin-buffer.ts`. Raw-mode toggling and dimension queries are delegated
//! to `crossterm`; escape sequences that `crossterm` has no dedicated API
//! for (bracketed paste, Kitty keyboard protocol negotiation, OSC progress /
//! title) are written directly, matching the byte sequences hoocode used.

pub mod interrupt;
pub mod mouse;
mod output;
mod stdin_buffer;
mod stdin_hub;

pub use mouse::{
    is_mouse_sequence, mouse_sequence_length, parse_mouse_event, MouseEvent, MouseEventKind,
    MOUSE_DISABLE, MOUSE_ENABLE,
};
pub use output::{DrainedWake, OutputHandle, OutputThread, OUTPUT_QUEUE_CAP, OUTPUT_THREAD_NAME};
pub use stdin_buffer::{StdinBuffer, StdinBufferOptions, StdinEvent};

use std::env;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use hoocode_runtime::spawn_named_thread;
use hoocode_tui_keys::{is_kitty_protocol_active, set_kitty_protocol_active};

const TERMINAL_PROGRESS_ACTIVE_SEQUENCE: &str = "\x1b]9;4;3\x07";
const TERMINAL_PROGRESS_CLEAR_SEQUENCE: &str = "\x1b]9;4;0;\x07";
const TERMINAL_PROGRESS_KEEPALIVE: Duration = Duration::from_millis(1000);
const KITTY_QUERY_FALLBACK_DELAY: Duration = Duration::from_millis(150);
/// How long shutdown waits for queued terminal output before dropping it.
const OUTPUT_SHUTDOWN_DEADLINE: Duration = Duration::from_secs(1);
/// Resize polling interval where there is no SIGWINCH (Windows, or if the listener fails).
const RESIZE_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Minimal terminal interface for the TUI.
///
/// Input/resize handling uses callbacks (rather than returning a stream)
/// to mirror the original event-driven `start(onInput, onResize)` API.
pub trait Terminal: Send {
    fn start(&mut self, on_input: Box<dyn FnMut(&str) + Send>, on_resize: Box<dyn FnMut() + Send>);
    fn stop(&mut self);
    fn drain_input(&mut self, max: Duration, idle: Duration);
    fn write(&mut self, data: &str);

    /// Writes one frame. The TUI does not build the next frame until
    /// [`frame_pending`](Terminal::frame_pending) is false, so a frame is never
    /// dropped. Default: an ordinary write.
    fn write_frame(&mut self, data: &str) {
        self.write(data);
    }

    /// True from [`write_frame`](Terminal::write_frame) until the terminal has the frame.
    fn frame_pending(&self) -> bool {
        false
    }

    /// Asks for `on_output_drained` (set with [`on_output_drained`](Terminal::on_output_drained))
    /// once the frame that is pending now has been written.
    fn notify_when_drained(&self) {}

    /// Sets the callback run on the output thread after a frame the TUI waits on
    /// is written. Call before [`start`](Terminal::start).
    fn on_output_drained(&mut self, _wake: DrainedWake) {}

    fn columns(&self) -> u16;
    fn rows(&self) -> u16;
    fn kitty_protocol_active(&self) -> bool;
    fn move_by(&mut self, lines: i32);
    fn hide_cursor(&mut self);
    fn show_cursor(&mut self);
    fn clear_line(&mut self);
    fn clear_from_cursor(&mut self);
    fn clear_screen(&mut self);
    fn set_title(&mut self, title: &str);
    fn set_progress(&mut self, active: bool);

    /// Whether mouse reporting is on, so the wheel arrives as input rather
    /// than scrolling the terminal's own scrollback.
    fn mouse_reporting(&self) -> bool {
        false
    }

    /// Enter or leave the alternate screen (`?1049`), a fixed grid with no
    /// scrollback of its own; leaving restores the normal screen as it was.
    fn set_alternate_screen(&mut self, _active: bool) {}
}

/// Resolves terminal dimensions the same way hoocode's `columns`/`rows`
/// getters do: measured size, then an env var override, then a default.
fn resolve_dimension(measured: Option<u16>, env_var: Option<&str>, default: u16) -> u16 {
    if let Some(m) = measured {
        if m != 0 {
            return m;
        }
    }
    if let Some(v) = env_var {
        if let Ok(parsed) = v.parse::<u16>() {
            if parsed != 0 {
                return parsed;
            }
        }
    }
    default
}

/// Matches a Kitty keyboard-protocol query response: `\x1b[?<flags>u`.
/// Clears the Kitty flag and reports whether it was set.
fn take_kitty_protocol_active() -> bool {
    let active = is_kitty_protocol_active();
    set_kitty_protocol_active(false);
    active
}

fn parse_kitty_query_response(sequence: &str) -> bool {
    let Some(rest) = sequence.strip_prefix("\x1b[?") else {
        return false;
    };
    let Some(digits) = rest.strip_suffix('u') else {
        return false;
    };
    !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
}

fn resolve_write_log_path() -> Option<PathBuf> {
    let env = env::var("HOOCODE_TUI_WRITE_LOG").ok()?;
    if env.is_empty() {
        return None;
    }
    let path = PathBuf::from(&env);
    if path.is_dir() {
        let now = std::time::SystemTime::now();
        let ts = humantime_like_timestamp(now);
        Some(path.join(format!("tui-{ts}-{}.log", std::process::id())))
    } else {
        Some(path)
    }
}

fn humantime_like_timestamp(_now: std::time::SystemTime) -> String {
    // Coarse, dependency-free timestamp (no chrono dependency): seconds since epoch.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    secs.to_string()
}

/// Where the terminal's own threads send output: the output thread while the
/// terminal runs, stdout directly before it starts and after it stops.
#[derive(Clone)]
struct Writer {
    handle: Option<OutputHandle>,
    log: Option<PathBuf>,
}

impl Writer {
    fn write(&self, data: &str) {
        match &self.handle {
            Some(handle) => handle.write(data),
            None => output::emit(&mut io::stdout(), self.log.as_deref(), data),
        }
    }
}

/// Resize watch: a `SIGWINCH` listener on Unix, size polling otherwise.
/// The payload is only held: dropping it stops the watch.
#[allow(dead_code)]
enum ResizeWatch {
    #[cfg(unix)]
    Signal(hoocode_runtime::SignalWatch),
    /// Stops when the terminal stops (`stop_signal`).
    Poll(JoinHandle<()>),
}

/// Calls `on_resize` when the terminal size differs from the last size seen.
#[derive(Clone)]
struct ResizeCheck {
    on_resize: Arc<Mutex<Box<dyn FnMut() + Send>>>,
    last_cols: Arc<AtomicU16>,
    last_rows: Arc<AtomicU16>,
}

impl ResizeCheck {
    fn run(&self) {
        let Ok((cols, rows)) = crossterm::terminal::size() else {
            return;
        };
        if cols != self.last_cols.load(Ordering::SeqCst)
            || rows != self.last_rows.load(Ordering::SeqCst)
        {
            self.last_cols.store(cols, Ordering::SeqCst);
            self.last_rows.store(rows, Ordering::SeqCst);
            (self.on_resize.lock().unwrap_or_else(|e| e.into_inner()))();
        }
    }
}

fn start_resize_watch(check: ResizeCheck, stop_signal: Arc<AtomicBool>) -> Option<ResizeWatch> {
    #[cfg(unix)]
    {
        let signal_check = check.clone();
        if let Ok(watch) =
            hoocode_runtime::watch_sigwinch("hoocode-resize", move || signal_check.run())
        {
            return Some(ResizeWatch::Signal(watch));
        }
    }
    let poll = spawn_named_thread("hoocode-resize", move || loop {
        if stop_signal.load(Ordering::SeqCst) {
            break;
        }
        thread::sleep(RESIZE_POLL_INTERVAL);
        check.run();
    })
    .ok()?;
    Some(ResizeWatch::Poll(poll))
}

/// Real terminal backed by process stdin/stdout, raw mode via `crossterm`.
pub struct ProcessTerminal {
    was_raw: bool,
    started: bool,
    modify_other_keys_active: Arc<AtomicBool>,
    progress_active: Arc<AtomicBool>,
    forwarding: Arc<AtomicBool>,
    last_input_at: Arc<Mutex<Instant>>,
    resize: Option<ResizeWatch>,
    stop_signal: Arc<AtomicBool>,
    progress_thread: Option<JoinHandle<()>>,
    last_cols: Arc<AtomicU16>,
    last_rows: Arc<AtomicU16>,
    write_log_path: Option<PathBuf>,
    mouse_reporting: bool,
    alternate_screen: bool,
    /// The `hoocode-term-out` thread, while the terminal runs.
    out: Option<OutputThread>,
    /// Run by the output thread once a frame is written; set before `start`.
    drained_wake: Option<DrainedWake>,
}

impl Default for ProcessTerminal {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessTerminal {
    pub fn new() -> Self {
        Self {
            was_raw: false,
            started: false,
            modify_other_keys_active: Arc::new(AtomicBool::new(false)),
            progress_active: Arc::new(AtomicBool::new(false)),
            forwarding: Arc::new(AtomicBool::new(true)),
            last_input_at: Arc::new(Mutex::new(Instant::now())),
            resize: None,
            stop_signal: Arc::new(AtomicBool::new(false)),
            progress_thread: None,
            last_cols: Arc::new(AtomicU16::new(0)),
            last_rows: Arc::new(AtomicU16::new(0)),
            write_log_path: resolve_write_log_path(),
            mouse_reporting: false,
            alternate_screen: false,
            out: None,
            drained_wake: None,
        }
    }

    /// `restoreOnExit`: leave the alternate screen and mouse reporting.
    fn restore_screen_modes(&mut self) {
        if self.alternate_screen {
            self.raw_write("\x1b[?1049l");
            self.alternate_screen = false;
        }
        if self.mouse_reporting {
            self.raw_write(mouse::MOUSE_DISABLE);
            self.mouse_reporting = false;
        }
    }

    fn writer(&self) -> Writer {
        Writer {
            handle: self.out.as_ref().map(OutputThread::handle),
            log: self.write_log_path.clone(),
        }
    }

    fn raw_write(&self, data: &str) {
        self.writer().write(data);
    }
}

impl Terminal for ProcessTerminal {
    fn start(&mut self, on_input: Box<dyn FnMut(&str) + Send>, on_resize: Box<dyn FnMut() + Send>) {
        if self.started {
            return;
        }
        self.started = true;
        // Every write from here on goes through the output thread, in order.
        // If the thread cannot start, writes go to stdout directly.
        self.out = OutputThread::spawn(
            Box::new(io::stdout()),
            self.write_log_path.clone(),
            self.drained_wake.take(),
        )
        .ok();
        self.was_raw = crossterm::terminal::is_raw_mode_enabled().unwrap_or(false);
        let _ = crossterm::terminal::enable_raw_mode();

        // Windows: Enable virtual terminal input processing.
        // This allows Windows to process escape sequences properly,
        // including mouse events, bracketed paste, and Kitty keyboard protocol.
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::System::Console::{
                GetConsoleMode, SetConsoleMode, ENABLE_VIRTUAL_TERMINAL_INPUT,
            };

            let stdin = io::stdin();
            let handle = stdin.as_raw_handle();
            let mut mode: u32 = 0;
            unsafe {
                if GetConsoleMode(handle, &mut mode) != 0 {
                    mode |= ENABLE_VIRTUAL_TERMINAL_INPUT;
                    SetConsoleMode(handle, mode);
                }
            }
        }

        // Bracketed paste mode.
        self.raw_write("\x1b[?2004h");

        // Take the wheel (see mouse.rs). A dumb terminal has nothing to report
        // with, and HOOCODE_MOUSE=0 hands the wheel back.
        if std::env::var("HOOCODE_MOUSE").as_deref() != Ok("0")
            && std::env::var("TERM").as_deref() != Ok("dumb")
            && std::io::IsTerminal::is_terminal(&io::stdout())
        {
            self.raw_write(mouse::MOUSE_ENABLE);
            self.mouse_reporting = true;
        }

        self.stop_signal.store(false, Ordering::SeqCst);
        self.forwarding.store(true, Ordering::SeqCst);

        // Query + (fallback) enable Kitty keyboard protocol / modifyOtherKeys.
        self.raw_write("\x1b[?u");
        {
            let modify_active = self.modify_other_keys_active.clone();
            let stop_signal = self.stop_signal.clone();
            let writer = self.writer();
            let _ = spawn_named_thread("hoocode-input-timer", move || {
                thread::sleep(KITTY_QUERY_FALLBACK_DELAY);
                if stop_signal.load(Ordering::SeqCst) {
                    return;
                }
                if !is_kitty_protocol_active() && !modify_active.load(Ordering::SeqCst) {
                    writer.write("\x1b[>4;2m");
                    modify_active.store(true, Ordering::SeqCst);
                }
            });
        }

        // Input: the process-wide stdin reader feeds this terminal while it is
        // started (see `stdin_hub`): Ctrl+C is seen first, then StdinBuffer
        // parses the chunks into sequences for `on_input`, and the ESC timer
        // runs on the reader's own deadline.
        let feed = stdin_hub::Feed::new(
            self.forwarding.clone(),
            self.last_input_at.clone(),
            self.writer(),
            on_input,
        );
        stdin_hub::subscribe(Arc::new(Mutex::new(feed)));

        // Resize: SIGWINCH on Unix (no polling); size polling on Windows.
        let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
        self.last_cols.store(cols, Ordering::SeqCst);
        self.last_rows.store(rows, Ordering::SeqCst);
        let check = ResizeCheck {
            on_resize: Arc::new(Mutex::new(on_resize)),
            last_cols: self.last_cols.clone(),
            last_rows: self.last_rows.clone(),
        };
        self.resize = start_resize_watch(check, self.stop_signal.clone());
    }

    fn stop(&mut self) {
        if !self.started {
            return;
        }
        self.started = false;
        self.stop_signal.store(true, Ordering::SeqCst);
        self.forwarding.store(false, Ordering::SeqCst);

        if self.progress_active.swap(false, Ordering::SeqCst) {
            self.raw_write(TERMINAL_PROGRESS_CLEAR_SEQUENCE);
        }
        if let Some(handle) = self.progress_thread.take() {
            let _ = handle.join();
        }

        // Back to the normal screen and off the wheel first, so the rest of
        // the teardown lands where the user will see it.
        self.restore_screen_modes();

        self.raw_write("\x1b[?2004l");

        if take_kitty_protocol_active() {
            self.raw_write("\x1b[<u");
        }
        if self.modify_other_keys_active.swap(false, Ordering::SeqCst) {
            self.raw_write("\x1b[>4;0m");
        }

        // Stop listening; input that arrives before the next terminal starts
        // waits for it, as it does in a paused Node stdin.
        stdin_hub::unsubscribe();
        self.resize = None;

        // Everything queued (the frame included) reaches the terminal before
        // raw mode changes. A terminal that stopped reading drops it after the deadline.
        if let Some(out) = self.out.take() {
            let _ = out.finish(OUTPUT_SHUTDOWN_DEADLINE);
        }

        let _ = crossterm::terminal::disable_raw_mode();
        if self.was_raw {
            let _ = crossterm::terminal::enable_raw_mode();
        }
    }

    fn drain_input(&mut self, max: Duration, idle: Duration) {
        if take_kitty_protocol_active() {
            self.raw_write("\x1b[<u");
        }
        if self.modify_other_keys_active.swap(false, Ordering::SeqCst) {
            self.raw_write("\x1b[>4;0m");
        }

        self.forwarding.store(false, Ordering::SeqCst);
        let start = Instant::now();
        loop {
            let last = *self.last_input_at.lock().unwrap();
            if start.elapsed() >= max {
                break;
            }
            if last.elapsed() >= idle {
                break;
            }
            thread::sleep(idle.min(Duration::from_millis(10)));
        }
        self.forwarding.store(true, Ordering::SeqCst);
    }

    fn write(&mut self, data: &str) {
        self.raw_write(data);
    }

    fn write_frame(&mut self, data: &str) {
        match &self.out {
            Some(out) => out.handle().write_frame(data),
            None => self.raw_write(data),
        }
    }

    fn frame_pending(&self) -> bool {
        self.out
            .as_ref()
            .is_some_and(|out| out.handle().frame_pending())
    }

    fn notify_when_drained(&self) {
        if let Some(out) = &self.out {
            out.handle().notify_when_drained();
        }
    }

    fn on_output_drained(&mut self, wake: DrainedWake) {
        self.drained_wake = Some(wake);
    }

    fn columns(&self) -> u16 {
        let measured = crossterm::terminal::size().ok().map(|(c, _)| c);
        resolve_dimension(measured, env::var("COLUMNS").ok().as_deref(), 80)
    }

    fn rows(&self) -> u16 {
        let measured = crossterm::terminal::size().ok().map(|(_, r)| r);
        resolve_dimension(measured, env::var("LINES").ok().as_deref(), 24)
    }

    fn kitty_protocol_active(&self) -> bool {
        is_kitty_protocol_active()
    }

    fn move_by(&mut self, lines: i32) {
        match lines.cmp(&0) {
            std::cmp::Ordering::Greater => self.raw_write(&format!("\x1b[{lines}B")),
            std::cmp::Ordering::Less => self.raw_write(&format!("\x1b[{}A", -lines)),
            std::cmp::Ordering::Equal => {}
        }
    }

    fn hide_cursor(&mut self) {
        self.raw_write("\x1b[?25l");
    }

    fn show_cursor(&mut self) {
        self.raw_write("\x1b[?25h");
    }

    fn clear_line(&mut self) {
        self.raw_write("\x1b[K");
    }

    fn clear_from_cursor(&mut self) {
        self.raw_write("\x1b[J");
    }

    fn clear_screen(&mut self) {
        self.raw_write("\x1b[2J\x1b[H");
    }

    fn set_title(&mut self, title: &str) {
        self.raw_write(&format!("\x1b]0;{title}\x07"));
    }

    fn set_progress(&mut self, active: bool) {
        if active {
            self.raw_write(TERMINAL_PROGRESS_ACTIVE_SEQUENCE);
            if !self.progress_active.swap(true, Ordering::SeqCst) {
                let stop_signal = self.stop_signal.clone();
                let progress_active = self.progress_active.clone();
                let writer = self.writer();
                self.progress_thread = spawn_named_thread("hoocode-progress", move || loop {
                    thread::sleep(TERMINAL_PROGRESS_KEEPALIVE);
                    if stop_signal.load(Ordering::SeqCst) || !progress_active.load(Ordering::SeqCst)
                    {
                        break;
                    }
                    writer.write(TERMINAL_PROGRESS_ACTIVE_SEQUENCE);
                })
                .ok();
            }
        } else {
            self.progress_active.store(false, Ordering::SeqCst);
            if let Some(handle) = self.progress_thread.take() {
                let _ = handle.join();
            }
            self.raw_write(TERMINAL_PROGRESS_CLEAR_SEQUENCE);
        }
    }

    fn mouse_reporting(&self) -> bool {
        self.mouse_reporting
    }

    fn set_alternate_screen(&mut self, active: bool) {
        if self.alternate_screen == active {
            return;
        }
        self.alternate_screen = active;
        // ?1049 saves and restores the cursor along with the screen.
        self.write(if active { "\x1b[?1049h" } else { "\x1b[?1049l" });
    }
}

impl Drop for ProcessTerminal {
    fn drop(&mut self) {
        if self.started {
            self.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_dimension_prefers_measured_value() {
        assert_eq!(resolve_dimension(Some(120), Some("999"), 80), 120);
    }

    // Port of `packages/tui/test/terminal.test.ts`: COLUMNS/LINES are used when stdout has no size.
    #[test]
    fn resolve_dimension_falls_back_to_env_var() {
        assert_eq!(resolve_dimension(None, Some("123"), 80), 123);
        assert_eq!(resolve_dimension(Some(0), Some("45"), 24), 45);
    }

    #[test]
    fn resolve_dimension_falls_back_to_default() {
        assert_eq!(resolve_dimension(None, None, 80), 80);
        assert_eq!(resolve_dimension(None, Some("not-a-number"), 80), 80);
    }

    #[test]
    fn kitty_query_response_matching() {
        assert!(parse_kitty_query_response("\x1b[?7u"));
        assert!(parse_kitty_query_response("\x1b[?0u"));
        assert!(!parse_kitty_query_response("\x1b[?u"));
        assert!(!parse_kitty_query_response("\x1b[97u"));
        assert!(!parse_kitty_query_response("\x1b[A"));
    }

    /// The Kitty flag is one flag, shared by the terminal (which detects it)
    /// and key matching (which reads it). Also: no flag before detection.
    #[test]
    fn kitty_detection_reaches_key_matching() {
        set_kitty_protocol_active(false);
        assert!(!ProcessTerminal::new().kitty_protocol_active());
        assert!(!hoocode_tui_keys::matches_key("\x1b\r", "shift+enter"));
        assert!(hoocode_tui_keys::matches_key("\x1b\r", "alt+enter"));

        let mut feed = stdin_hub::Feed::new(
            Arc::new(AtomicBool::new(true)),
            Arc::new(Mutex::new(Instant::now())),
            Writer {
                handle: None,
                log: None,
            },
            Box::new(|_: &str| {}),
        );
        feed.chunk(b"\x1b[?7u", Instant::now());
        assert!(ProcessTerminal::new().kitty_protocol_active());
        assert!(hoocode_tui_keys::matches_key("\x1b\r", "shift+enter"));
        assert!(!hoocode_tui_keys::matches_key("\x1b\r", "alt+enter"));
        set_kitty_protocol_active(false);
    }
}

//! The TUI differential renderer, ported from `tui.ts`'s `TUI` class.
//!
//! Two behavioral simplifications relative to the TypeScript original are
//! made deliberately (documented at each site below), since they are pure
//! performance optimizations that do not change what gets written to the
//! terminal:
//!
//! 1. No reference-identity flatten memoization ([`crate::Container`] always
//!    re-renders and re-concatenates children; the differential writer
//!    below still only rewrites lines whose *content* changed).
//! 2. No `requestAnimationFrame`-style 16ms render coalescing. Input and
//!    resizes passed to [`Tui::accept_event`] only schedule a frame, which
//!    [`Tui::flush_scheduled_render`] paints once per loop iteration. Other
//!    `request_render()` calls paint at once. A frame is also held back while
//!    the output thread still writes the previous one (see below).
//!
//! Threading: `hoocode_tui_terminal::Terminal::start` requires `Send`
//! callbacks because it reads stdin on a background thread, but this
//! module's component tree uses `Rc<RefCell<dyn Component>>` (matching the
//! original's mutable object graph) which is not `Send`. So unlike the
//! TypeScript original — where `terminal.start()`'s callbacks call
//! straight into `TUI.handleInput`/`requestRender` — [`Tui::start`] here
//! forwards raw input/resize notifications through an `mpsc` channel to a
//! single owning thread, which drives them into [`Tui::process_event`].
//!
//! Output: frames and the escapes around them go through the terminal's
//! output thread in the order they were sent. A frame is a diff against the
//! one before it, so the next frame is built only after the terminal has taken
//! the previous one; the output thread then sends [`TuiEvent::OutputDrained`].

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hoocode_tui_images::{
    allocate_image_id, delete_kitty_image, get_capabilities, set_cell_dimensions, CellDimensions,
};
use hoocode_tui_keys::is_key_release;
use hoocode_tui_terminal::{move_rows, parse_mouse_report, MouseEvent, MouseEventKind, Terminal};
use hoocode_tui_util::{
    bare_url_at, hyperlink_at, normalize_terminal_output, slice_by_column,
    strip_vt_control_characters, truncate_to_width, visible_width,
};

use crate::component::{Component, ComponentHandle, Container, FlexSpacer};
use crate::fullscreen;
use crate::scrollbar::{scrollbar_glyphs, SCROLLBAR_THUMB};

/// Cursor position marker: a zero-width APC escape sequence terminals
/// ignore. Components emit this at the cursor position when focused; the
/// TUI finds and strips it, then positions the hardware cursor there.
pub const CURSOR_MARKER: &str = "\x1b_pi:c\x07";

const KITTY_SEQUENCE_PREFIX: &str = "\x1b_G";
const SEGMENT_RESET: &str = "\x1b[0m\x1b]8;;\x07";

fn extract_kitty_image_ids(line: &str) -> Vec<u32> {
    let Some(sequence_start) = line.find(KITTY_SEQUENCE_PREFIX) else {
        return Vec::new();
    };
    let params_start = sequence_start + KITTY_SEQUENCE_PREFIX.len();
    let Some(params_end_rel) =
        hoocode_tui_util::text_slice::suffix_from(line, params_start).find(';')
    else {
        return Vec::new();
    };
    let params =
        hoocode_tui_util::text_slice::range(line, params_start, params_start + params_end_rel);
    for param in params.split(',') {
        let mut parts = param.splitn(2, '=');
        let key = parts.next().unwrap_or("");
        let Some(value) = parts.next() else { continue };
        if key != "i" {
            continue;
        }
        if let Ok(id) = value.parse::<u64>() {
            if id > 0 && id <= 0xffff_ffff {
                return vec![id as u32];
            }
        }
    }
    Vec::new()
}

/// Transmit the same kitty image under a different id (`retagKittyImageId`):
/// only the first chunk carries the parameter list.
fn retag_kitty_image_id(line: &str, id: u32) -> Option<String> {
    let sequence_start = line.find(KITTY_SEQUENCE_PREFIX)?;
    let params_start = sequence_start + KITTY_SEQUENCE_PREFIX.len();
    let params_end =
        params_start + hoocode_tui_util::text_slice::suffix_from(line, params_start).find(';')?;
    let params = hoocode_tui_util::text_slice::range(line, params_start, params_end);
    // `/(^|,)i=\d+/`
    let mut search_from = 0;
    while let Some(rel) = hoocode_tui_util::text_slice::suffix_from(params, search_from).find("i=")
    {
        let at = search_from + rel;
        let digits = hoocode_tui_util::text_slice::suffix_from(params, at + 2)
            .bytes()
            .take_while(u8::is_ascii_digit)
            .count();
        if (at == 0 || params.as_bytes()[at - 1] == b',') && digits > 0 {
            let retagged = format!(
                "{}i={id}{}",
                hoocode_tui_util::text_slice::prefix(params, at),
                hoocode_tui_util::text_slice::suffix_from(params, at + 2 + digits)
            );
            return Some(format!(
                "{}{retagged}{}",
                hoocode_tui_util::text_slice::prefix(line, params_start),
                hoocode_tui_util::text_slice::suffix_from(line, params_end)
            ));
        }
        search_from = at + 2;
    }
    None
}

/// How far above its own row an image line's picture reaches: the leading
/// `CSI <n> A` of a multi-row image (`imageRowOffset`).
fn image_row_offset(line: &str) -> i64 {
    let Some(rest) = line.strip_prefix("\x1b[") else {
        return 0;
    };
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 && hoocode_tui_util::text_slice::suffix_from(rest, digits).starts_with('A') {
        hoocode_tui_util::text_slice::prefix(rest, digits)
            .parse()
            .unwrap_or(0)
    } else {
        0
    }
}

/// DECTCEM cursor visibility, folded into a frame's synchronized buffer.
const HIDE_CURSOR: &str = "\x1b[?25l";
const SHOW_CURSOR: &str = "\x1b[?25h";

/// How far one wheel notch moves the pinned view.
const WHEEL_LINES: i64 = 3;

const IMAGE_PLACEHOLDER: &str = "\x1b[2m[image]\x1b[0m";

/// What the scroll indicator is told about the pinned view (`ScrollStatus`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrollStatus {
    /// 1-based transcript row at the top of the view.
    pub top: i64,
    /// 1-based transcript row at the bottom of the view.
    pub bottom: i64,
    /// Rows in the whole transcript.
    pub total: i64,
    /// Rows the view shows at once.
    pub view_height: i64,
    pub at_top: bool,
    /// True only when the very last row is in view.
    pub at_bottom: bool,
    /// Columns the indicator may fill.
    pub width: i64,
    /// Present while a search is running.
    pub search: Option<ScrollSearchStatus>,
    /// A picker (not the prompt) has focus: the arrows and escape go to it,
    /// so the indicator must not advertise them.
    pub picker: bool,
    /// Lines that arrived since the view last pinned or scrolled (`↓ N new`).
    /// Zero off the full-screen view.
    pub new_lines: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrollSearchStatus {
    pub query: String,
    /// Rows containing a match.
    pub count: usize,
    /// 1-based position among the matches, or 0 when there are none.
    pub index: usize,
    /// True while the query is still being typed.
    pub typing: bool,
}

pub type ScrollStatusFormatter = Box<dyn Fn(&ScrollStatus) -> String>;
/// Mission Control transmission toast lines (M8e4): the app draws the toast
/// with ratatui and the frame only floats its pre-rendered lines top-right
/// over the body. The provider gets the body width; empty means no toast.
pub type ToastProvider = Box<dyn Fn(usize) -> Vec<String>>;

/// Opens a clicked URL (`onHyperlink`).
pub type HyperlinkHandler = Box<dyn FnMut(&str)>;

/// The indicator drawn when the app has not supplied its own
/// (`defaultScrollStatus`): reverse video, position first, keys if they fit.
/// One shared case-insensitive matcher for the scan count and the
/// highlight: byte ranges of `needle_folded` in the ORIGINAL line.
/// Folding is per character, so a fold that changes byte length (the Turkish
/// İ folds to `i` plus a combining dot) or a multi-byte query (emoji) still
/// maps home, where whole-line lowercasing would not.
fn fold_match_ranges(plain: &str, needle_folded: &str) -> Vec<(usize, usize)> {
    // The folded chars with each char's origin byte range in the line.
    let mut folded: Vec<char> = Vec::new();
    let mut origins: Vec<(usize, usize)> = Vec::new();
    for (at, c) in plain.char_indices() {
        for folded_char in c.to_lowercase() {
            folded.push(folded_char);
            origins.push((at, at + c.len_utf8()));
        }
    }
    let needle: Vec<char> = needle_folded.chars().collect();
    if needle.is_empty() {
        return Vec::new();
    }
    // Non-overlapping scan, like `str::find` in a loop; every index lands
    // on a char, so no byte slicing can split one.
    let mut ranges = Vec::new();
    let mut i = 0;
    while i + needle.len() <= folded.len() {
        if folded[i..i + needle.len()] == needle[..] {
            ranges.push((origins[i].0, origins[i + needle.len() - 1].1));
            i += needle.len();
        } else {
            i += 1;
        }
    }
    ranges
}

pub fn default_scroll_status(status: &ScrollStatus) -> String {
    let position = format!("{}\u{2013}{}/{}", status.top, status.bottom, status.total);
    let where_ = if status.at_top { " top" } else { "" };
    let keys = if status.picker {
        "PgUp/PgDn page"
    } else {
        "\u{2191}\u{2193} line \u{b7} PgUp/PgDn page \u{b7} esc live"
    };
    let left = format!(" {position}{where_} ");
    let width = status.width.max(0) as usize;
    let body = if visible_width(&left) + visible_width(keys) < width {
        format!("{left}{keys} ")
    } else {
        left
    };
    format!(
        "\x1b[7m{}\x1b[0m",
        truncate_to_width(&body, width, "", true)
    )
}

struct ScrollSearch {
    query: String,
    matches: Vec<i64>,
    index: i64,
    /// Buffer length the matches were measured at; `None` for an empty query.
    measured_at: Option<i64>,
    typing: bool,
}

/// Result an [`InputListener`] can return to consume or transform input.
#[derive(Default)]
pub struct InputListenerResult {
    pub consume: bool,
    pub data: Option<String>,
}

pub type InputListener = Box<dyn FnMut(&str) -> Option<InputListenerResult>>;

/// See [`Tui::can_pin_scroll`].
pub type CanPinScroll = Box<dyn Fn(&Tui) -> bool>;

/// See [`Tui::set_input_interceptor`].
pub type InputInterceptor = Box<dyn FnMut(&mut Tui, &str) -> bool>;

/// Event delivered from the terminal's background threads to the single
/// thread driving the `Tui` (see module docs).
pub enum TuiEvent {
    /// Raw input, stamped when the terminal's reader thread received it (the
    /// start of keystroke-to-frame latency, see `hoocode-code-tui-app`'s `perf`).
    Input(String, Instant),
    Resize,
    /// The output thread wrote the frame the TUI was waiting to paint its
    /// next one after (see [`Tui::flush_scheduled_render`]).
    OutputDrained,
    /// Something outside the terminal (an agent event) is waiting for the loop;
    /// see [`WakeHandle`].
    Wake,
}

/// Wakes the loop that reads the [`Tui::start`] channel from any thread.
///
/// A wake is sent only when none is pending, so a flood of callers puts one
/// [`TuiEvent::Wake`] in the channel. The loop calls [`WakeHandle::clear`] when
/// it takes the event, before it reads whatever the wake was for. The handle
/// follows [`Tui::start`] when it opens a new channel.
#[derive(Clone)]
pub struct WakeHandle {
    pending: Arc<AtomicBool>,
    sender: Arc<Mutex<Option<Sender<TuiEvent>>>>,
}

impl WakeHandle {
    fn new() -> Self {
        Self {
            pending: Arc::new(AtomicBool::new(false)),
            sender: Arc::new(Mutex::new(None)),
        }
    }

    /// Asks the loop to run a pass. Cheap when a wake is already pending.
    pub fn wake(&self) {
        if self.pending.swap(true, Ordering::AcqRel) {
            return;
        }
        let sender = self.sender.lock().unwrap_or_else(|e| e.into_inner());
        match sender.as_ref() {
            Some(tx) if tx.send(TuiEvent::Wake).is_ok() => {}
            // No loop is listening (not started, or the channel is gone).
            _ => self.pending.store(false, Ordering::Release),
        }
    }

    /// The loop took a [`TuiEvent::Wake`]: the next `wake` sends a new one.
    pub fn clear(&self) {
        self.pending.store(false, Ordering::Release);
    }
}

impl TuiEvent {
    /// Input that arrived now.
    pub fn input(data: impl Into<String>) -> Self {
        Self::Input(data.into(), Instant::now())
    }
}

/// How long one frame took: building its lines and writing them to the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameTiming {
    pub build: Duration,
    pub write: Duration,
}

/// See [`Tui::set_frame_observer`].
pub type FrameObserver = Box<dyn FnMut(FrameTiming)>;

/// The frame-level numbers the paint steps share.
#[derive(Clone, Copy)]
struct FrameGeometry {
    width: i64,
    height: i64,
    /// The buffer row at the top of the screen before this frame.
    prev_viewport_top: i64,
    /// Lines in the frame this one replaces.
    prev_line_count: i64,
}

#[derive(Clone, Copy)]
struct CursorPos {
    row: i64,
    col: i64,
}

const MIN_LINES_RENDERED_START: i64 = 0;

/// A frame size as the baseline stores it. A zero size has always meant
/// "nothing painted yet", so it maps to `None`.
fn known_size(size: i64) -> Option<i64> {
    (size != 0).then_some(size)
}

pub struct Tui {
    pub terminal: Box<dyn Terminal>,
    root: Container,

    previous_lines: Vec<String>,
    /// The size of the last live frame; `None` until one is painted.
    previous_width: Option<i64>,
    previous_height: Option<i64>,
    /// Set by `request_render(true)`: the next live paint clears and redraws
    /// the whole screen. Cleared by that paint.
    force_redraw: bool,
    previous_kitty_image_ids: HashSet<u32>,
    saw_image_line: bool,

    focused_component: Option<ComponentHandle>,
    input_listeners: Vec<InputListener>,

    frame_observer: Option<FrameObserver>,
    /// Terminal write time accumulated by the frame being painted.
    frame_write_time: Duration,
    /// A render was asked for and has not been painted yet: input and resizes
    /// only mark the frame dirty; the loop paints once per iteration.
    render_scheduled: bool,
    /// Set while [`Tui::accept_event`] handles input, so the renders the input
    /// asks for are scheduled instead of painted one by one.
    in_input: bool,

    hardware_cursor_row: i64,
    show_hardware_cursor: bool,
    clear_on_shrink: bool,
    max_lines_rendered: i64,
    previous_viewport_top: i64,
    full_redraw_count: u64,
    stopped: bool,
    wake: WakeHandle,

    last_cursor_pos: Option<CursorPos>,

    /// The filler that keeps the app the size of the screen (`flexSpacer`):
    /// the buffer is never shorter than the terminal, so the header stays at
    /// the top and the prompt on the bottom row. `None` keeps the old
    /// append-only behaviour.
    flex_spacer: Option<Rc<RefCell<FlexSpacer>>>,
    /// The last flattened frame (`flatLines`).
    flat_lines: Vec<String>,
    /// Where the left button went down, for telling a click from a drag.
    pressed_cell: Option<(i64, i64)>,
    /// What the pinned window painted last, so a click on it can be placed.
    scroll_view_lines: Option<Vec<String>>,
    /// Open the URL behind a clicked hyperlink. Unset, clicks do nothing.
    pub on_hyperlink: Option<HyperlinkHandler>,
    /// The pinned viewport: the transcript row at the top of the screen, or
    /// `None` to follow the tail. While pinned, the TUI paints a window of
    /// the buffer on the alternate screen.
    scroll_offset: Option<i64>,
    /// Transcript length measured by the last pinned paint.
    scroll_total_lines: i64,
    scroll_status_formatter: ScrollStatusFormatter,
    toast_provider: Option<ToastProvider>,
    scroll_search: Option<ScrollSearch>,
    /// Whether the view may pin right now (unset means always). Given the
    /// TUI so it can ask what holds focus.
    pub can_pin_scroll: Option<CanPinScroll>,
    /// The prompt. While the view is pinned, a focused component that is not
    /// the prompt is a picker: its lines paint at the bottom of the view.
    /// Unset, every focused component counts as a picker.
    pub scroll_prompt: Option<ComponentHandle>,
    /// Sees input after mouse reports and before the input listeners, with
    /// the TUI in hand; `true` consumes it. The app's scroll view keys.
    input_interceptor: Option<InputInterceptor>,
    /// Live kitty image id -> the id the pinned window transmits its copy
    /// under, and the copies currently placed on the alternate screen.
    scroll_image_ids: HashMap<u32, u32>,
    scroll_placed_images: HashSet<u32>,

    /// Full-screen mode (M1): the alternate screen for the whole run, and a
    /// frame composed from regions. Off by default; set before `start`.
    fullscreen: bool,
    /// The transcript component in full-screen mode. Root children before it
    /// are the top chrome, those after it the bottom.
    fullscreen_body: Option<ComponentHandle>,
    /// The rows of the last full-screen frame, as written, for the per-row diff.
    previous_screen: Vec<String>,
    /// The size of the last full-screen frame; `None` until one is painted.
    previous_screen_size: Option<(usize, usize)>,
    /// Body rows on screen in the last full-screen frame: the first screen
    /// row and the lines, for link clicks.
    fullscreen_body_window: Option<(i64, Vec<String>)>,
    /// Body rows the last full-screen frame showed (the view height).
    fullscreen_view: i64,
    /// Between `start` and `stop`: a full-screen mode change would desync the frame.
    started: bool,
    /// The body's line count when the view last pinned or scrolled; the `N new`
    /// count is what has grown since. A shorter transcript resets it.
    fullscreen_seen_total: i64,
    /// Set by [`Self::reset_new_lines_count`]: the next full-screen frame starts
    /// the `N new` count at the transcript as it is then.
    fullscreen_seen_reset: bool,
    /// Width-diagnosis log (`HOOCODE_WIDTH_DEBUG`): resolved once at
    /// construction, so per-frame painting does no environment I/O.
    width_log: Option<std::path::PathBuf>,
    /// The last `(paint site, columns, rows)` logged, so a steady screen
    /// does not append a line per frame.
    last_paint_log: Option<(&'static str, u16, u16)>,
}

/// One composed full-screen frame: the rows as they are written, and the
/// cursor (row, column), 0-based, when it is shown.
struct Screen {
    rows: Vec<String>,
    cursor: Option<(usize, usize)>,
}

impl Tui {
    pub fn new(terminal: Box<dyn Terminal>, show_hardware_cursor: Option<bool>) -> Self {
        Self {
            terminal,
            root: Container::new(),
            previous_lines: Vec::new(),
            previous_width: None,
            previous_height: None,
            force_redraw: false,
            previous_kitty_image_ids: HashSet::new(),
            saw_image_line: false,
            focused_component: None,
            input_listeners: Vec::new(),
            frame_observer: None,
            frame_write_time: Duration::ZERO,
            render_scheduled: false,
            in_input: false,
            hardware_cursor_row: 0,
            show_hardware_cursor: show_hardware_cursor.unwrap_or(false),
            clear_on_shrink: false,
            max_lines_rendered: MIN_LINES_RENDERED_START,
            previous_viewport_top: 0,
            full_redraw_count: 0,
            stopped: false,
            wake: WakeHandle::new(),
            last_cursor_pos: None,
            flex_spacer: None,
            flat_lines: Vec::new(),
            pressed_cell: None,
            scroll_view_lines: None,
            on_hyperlink: None,
            scroll_offset: None,
            scroll_total_lines: 0,
            scroll_status_formatter: Box::new(default_scroll_status),
            toast_provider: None,
            scroll_search: None,
            can_pin_scroll: None,
            scroll_prompt: None,
            input_interceptor: None,
            scroll_image_ids: HashMap::new(),
            scroll_placed_images: HashSet::new(),
            fullscreen: false,
            fullscreen_body: None,
            previous_screen: Vec::new(),
            previous_screen_size: None,
            fullscreen_body_window: None,
            fullscreen_view: 0,
            started: false,
            fullscreen_seen_total: 0,
            fullscreen_seen_reset: false,
            width_log: std::env::var("HOOCODE_WIDTH_DEBUG")
                .ok()
                .filter(|v| !v.is_empty())
                .map(std::path::PathBuf::from),
            last_paint_log: None,
        }
    }

    /// Width diagnosis (`HOOCODE_WIDTH_DEBUG`): one appended line per
    /// distinct `(site, size)` shape: what `terminal.columns()` reports at
    /// paint time, and the `$COLUMNS` fallback it would use if measuring
    /// failed. Compare with the assistant-message lines: a narrow paint
    /// size here means size detection (tty measure, `$COLUMNS`,
    /// multiplexer pane) handed down a narrow viewport; a wide size here
    /// with narrow source text there means pre-wrapped upstream text.
    /// Never fails painting: I/O errors are dropped.
    fn log_paint(&mut self, site: &'static str) {
        let Some(path) = self.width_log.clone() else {
            return;
        };
        let columns = self.terminal.columns();
        let rows = self.terminal.rows();
        let key = (site, columns, rows);
        if self.last_paint_log == Some(key) {
            return;
        }
        self.last_paint_log = Some(key);
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            use std::io::Write;
            let _ = writeln!(
                file,
                "tui paint site={site} columns={columns} rows={rows} \
                 COLUMNS={} fullscreen={} pinned={}",
                std::env::var("COLUMNS").unwrap_or_default(),
                u8::from(self.fullscreen),
                u8::from(self.scroll_offset.is_some()),
            );
        }
    }

    /// Test-only: route every frame through the image path (full flatten +
    /// full diff), the oracle of the pin's `tui-flatcache-stress.test.ts`.
    #[doc(hidden)]
    pub fn force_image_path_for_tests(&mut self) {
        self.saw_image_line = true;
    }

    pub fn full_redraws(&self) -> u64 {
        self.full_redraw_count
    }

    // ── The pinned viewport ─────────────────────────────────────────────

    /// True while the view is pinned rather than following the tail.
    pub fn scroll_pinned(&self) -> bool {
        self.scroll_offset.is_some()
    }

    /// The width the transcript is laid out at: one column less while pinned,
    /// the last one being the scrollbar's.
    pub fn transcript_width(&self) -> u16 {
        let columns = self.terminal.columns();
        if self.scroll_offset.is_some() || self.fullscreen {
            columns.saturating_sub(1).max(1)
        } else {
            columns
        }
    }

    /// Where the pinned window sits `(top, total, view_height)`, or `None`
    /// while live.
    pub fn get_scroll_position(&self) -> Option<(i64, i64, i64)> {
        self.scroll_offset
            .map(|top| (top, self.scroll_total_lines, self.scroll_view_height()))
    }

    /// Puts the view back where [`Self::get_scroll_position`] said it was: the
    /// top row of a pinned window, or the tail (`None`). Used when the app
    /// swaps one transcript for another and back (the attached subagent view).
    pub fn restore_scroll_top(&mut self, top: Option<i64>) {
        match top {
            Some(top) => self.set_scroll_offset(top),
            None => {
                self.scroll_to_live();
            }
        }
    }

    /// The transcript was rebuilt or the screen resized: its line count is no
    /// longer comparable with the last one, so the `N new` count starts again
    /// from the next frame's transcript. Only growth after that is counted.
    pub fn reset_new_lines_count(&mut self) {
        self.fullscreen_seen_reset = true;
    }

    /// Let the app paint the indicator in its own theme.
    pub fn set_scroll_status_formatter(&mut self, formatter: ScrollStatusFormatter) {
        self.scroll_status_formatter = formatter;
    }

    /// Float the app's transmission toast lines top-right over the body.
    /// Only the fullscreen frame floats them; the pinned painter leaves the
    /// band to the app.
    pub fn set_toast_provider(&mut self, provider: ToastProvider) {
        self.toast_provider = Some(provider);
    }

    /// Nominate the child that absorbs the leftover rows. It must already be
    /// a child of the root, at the *top* of the tree.
    pub fn set_flex_spacer(&mut self, spacer: Option<Rc<RefCell<FlexSpacer>>>) {
        self.flex_spacer = spacer;
    }

    fn flex_height(&self) -> i64 {
        self.flex_spacer
            .as_ref()
            .map_or(0, |f| f.borrow().current_height() as i64)
    }

    /// Give the flex child whatever the frame did not use (`fitFlexSpacer`);
    /// true when the height moved and the frame must be flattened again.
    fn fit_flex_spacer(&self, lines: &[String], height: i64) -> bool {
        let Some(spacer) = &self.flex_spacer else {
            return false;
        };
        let content = lines.len() as i64 - spacer.borrow().current_height() as i64;
        spacer.borrow_mut().set_height((height - content).max(0))
    }

    /// The focused picker, when the view is pinned on a non-prompt focus.
    fn scroll_pane(&self) -> Option<ComponentHandle> {
        let focused = self.focused_component.clone()?;
        match &self.scroll_prompt {
            Some(prompt) if Rc::ptr_eq(prompt, &focused) => None,
            _ => Some(focused),
        }
    }

    /// The picker's lines at `width`, or none when the prompt has focus.
    fn scroll_pane_lines(&self, width: u16) -> Vec<String> {
        match self.scroll_pane() {
            Some(pane) => pane.borrow_mut().render(width),
            None => Vec::new(),
        }
    }

    /// `(transcript rows, picker rows)` of a pinned screen whose picker is
    /// `pane_len` lines tall. The status row is the last row; the transcript
    /// keeps at least one row. A picker that cannot fit whole is never shown
    /// pinned: `paint` lets go of the pin first (see `scroll_pane_fits`).
    fn pinned_split(&self, pane_len: i64) -> (i64, i64) {
        let room = self.terminal.rows() as i64 - 1;
        let pane = pane_len.min((room - 1).max(0));
        ((room - pane).max(1), pane)
    }

    /// Whether the focused picker, at the current width, fits whole beside
    /// the transcript. A picker taller than that would hide its top rows.
    fn scroll_pane_fits(&self) -> bool {
        let pane_len = self.scroll_pane_lines(self.terminal.columns()).len() as i64;
        self.pinned_split(pane_len).1 == pane_len
    }

    /// The screen rows a pinned window shows for the transcript; the status
    /// row is the last, and a picker takes its share at the bottom.
    fn scroll_view_height(&self) -> i64 {
        if self.fullscreen {
            return if self.previous_screen_size.is_some() {
                self.fullscreen_view
            } else {
                self.fullscreen_measure().1
            };
        }
        let pane_len = self.scroll_pane_lines(self.terminal.columns()).len() as i64;
        self.pinned_split(pane_len).0
    }

    /// The body's `(lines, rows shown)` as the next frame would lay them out.
    /// Before the first frame nothing has been measured, so scrolling asks here.
    fn fullscreen_measure(&self) -> (i64, i64) {
        let width = self.terminal.columns() as usize;
        let height = self.terminal.rows() as usize;
        if width == 0 || height == 0 {
            return (0, 0);
        }
        let (top, body, bottom) = self.fullscreen_regions();
        let top_natural = render_children(&top, width as u16).len();
        let bottom_natural = render_children(&bottom, width as u16).len();
        let total = body.map_or(0, |body| {
            body.borrow_mut()
                .render(fullscreen::body_width(width) as u16)
                .len()
        });
        let rows = fullscreen::split_rows(
            height,
            top_natural,
            self.scroll_offset.is_some(),
            bottom_natural,
        );
        (total as i64, rows.body as i64)
    }

    /// The full-screen children, split at the body: the top chrome, the body
    /// (when it is a child), and the bottom. Without a body every child is bottom.
    fn fullscreen_regions(
        &self,
    ) -> (
        Vec<ComponentHandle>,
        Option<ComponentHandle>,
        Vec<ComponentHandle>,
    ) {
        let children = &self.root.children;
        let index = self
            .fullscreen_body
            .as_ref()
            .and_then(|body| children.iter().position(|child| Rc::ptr_eq(child, body)));
        match index {
            Some(index) => (
                children[..index].to_vec(),
                Some(children[index].clone()),
                children[index + 1..].to_vec(),
            ),
            None => (Vec::new(), None, children.clone()),
        }
    }

    /// Rows available to scroll through; the filler is not transcript.
    fn transcript_length(&self) -> i64 {
        if self.fullscreen && self.previous_screen_size.is_none() {
            return self.fullscreen_measure().0;
        }
        if self.scroll_offset.is_some() || self.fullscreen {
            return self.scroll_total_lines;
        }
        self.previous_lines.len() as i64 - self.flex_height()
    }

    /// Move the view by `delta` rows (negative is towards the start). Returns
    /// whether anything moved.
    pub fn scroll_by_lines(&mut self, delta: i64) -> bool {
        if delta == 0 {
            return false;
        }
        if self.scroll_offset.is_none() && delta > 0 {
            return false;
        }
        let view_height = self.scroll_view_height();
        let max_offset = (self.transcript_length() - view_height).max(0);
        if max_offset == 0 {
            return false;
        }
        let next = self.scroll_offset.unwrap_or(max_offset) + delta;
        // Reaching the end is how the pin lets go.
        if next >= max_offset {
            return self.scroll_to_live();
        }
        self.set_scroll_offset(next);
        true
    }

    /// Move by pages, keeping two rows of overlap.
    pub fn scroll_by_pages(&mut self, delta: i64) -> bool {
        let page = (self.scroll_view_height() - 2).max(1);
        self.scroll_by_lines(delta * page)
    }

    /// Pin the view to the very start of the transcript.
    pub fn scroll_to_top(&mut self) -> bool {
        let max_offset = (self.transcript_length() - self.scroll_view_height()).max(0);
        if max_offset == 0 || self.scroll_offset == Some(0) {
            return false;
        }
        self.set_scroll_offset(0);
        true
    }

    /// Release the pin and follow the tail again. Leaving the alternate
    /// screen restores the normal screen and its scrollback, so the next
    /// frame is an ordinary differential one.
    pub fn scroll_to_live(&mut self) -> bool {
        if self.scroll_offset.is_none() {
            return false;
        }
        self.scroll_offset = None;
        self.scroll_search = None;
        self.release_scroll_images();
        // A full-screen run stays on the alternate screen for its whole life.
        if !self.fullscreen {
            self.terminal.set_alternate_screen(false);
        }
        self.last_cursor_pos = None;
        self.scroll_view_lines = None;
        self.request_render(false);
        true
    }

    /// Pin the view so `row` is on screen, with `context` rows above it
    /// (default `min(3, view_height - 1)`); a row already comfortably in view
    /// is left where it is.
    pub fn scroll_to_row(&mut self, row: i64, context: Option<i64>) -> bool {
        let view_height = self.scroll_view_height();
        let max_offset = (self.transcript_length() - view_height).max(0);
        if max_offset == 0 {
            return false;
        }
        let context = context.unwrap_or_else(|| 3.min((view_height - 1).max(0)));
        let target = (row - context).clamp(0, max_offset);
        if let Some(top) = self.scroll_offset {
            if row >= top + context && row < top + view_height {
                return false;
            }
        }
        self.set_scroll_offset(target);
        self.scroll_offset == Some(target)
    }

    fn set_scroll_offset(&mut self, offset: i64) {
        let entering = self.scroll_offset.is_none();
        // Only entry is gated, so a pinned view never strands the reader.
        if entering && self.can_pin_scroll.as_ref().is_some_and(|f| !f(self)) {
            return;
        }
        self.scroll_offset = Some(offset.max(0));
        if self.fullscreen {
            // Reading or moving: the `N new` count starts again from here.
            self.fullscreen_seen_total = self.scroll_total_lines;
        }
        if entering {
            if !self.fullscreen {
                self.terminal.set_alternate_screen(true);
            }
            self.terminal.hide_cursor();
        }
        self.request_render(false);
    }

    // ── Searching the pinned view ───────────────────────────────────────

    /// Find rows containing `query` (case-insensitive, over visible text)
    /// and pin the view to the nearest one at or above the eye. Returns how
    /// many rows matched.
    pub fn set_scroll_search(&mut self, query: &str, typing: bool) -> usize {
        if query.is_empty() {
            self.scroll_search = Some(ScrollSearch {
                query: String::new(),
                matches: Vec::new(),
                index: -1,
                measured_at: None,
                typing,
            });
            self.request_render(false);
            return 0;
        }
        let from = self
            .scroll_offset
            .unwrap_or_else(|| (self.transcript_length() - 1).max(0));
        let matches = self.find_scroll_matches(query);
        let mut index = matches
            .iter()
            .rposition(|&m| m <= from)
            .map_or(-1, |i| i as i64);
        if index == -1 && !matches.is_empty() {
            index = matches.len() as i64 - 1;
        }
        let count = matches.len();
        let target = (index >= 0).then(|| matches[index as usize]);
        self.scroll_search = Some(ScrollSearch {
            query: query.to_string(),
            matches,
            index,
            measured_at: Some(self.search_buffer().len() as i64),
            typing,
        });
        if let Some(row) = target {
            self.scroll_to_row(row, None);
        }
        self.request_render(false);
        count
    }

    /// Step to the next match (`-1` is further back); wraps.
    pub fn scroll_search_step(&mut self, direction: i64) -> bool {
        let Some(search) = &mut self.scroll_search else {
            return false;
        };
        if search.matches.is_empty() {
            return false;
        }
        let len = search.matches.len() as i64;
        search.index = (search.index + direction + len).rem_euclid(len);
        search.typing = false;
        let row = search.matches[search.index as usize];
        self.scroll_to_row(row, None);
        self.request_render(false);
        true
    }

    /// Stop typing the query but keep the matches.
    pub fn commit_scroll_search(&mut self) {
        if let Some(search) = &mut self.scroll_search {
            search.typing = false;
            self.request_render(false);
        }
    }

    /// Drop the search, leaving the view where it is.
    pub fn clear_scroll_search(&mut self) {
        if self.scroll_search.take().is_some() {
            self.request_render(false);
        }
    }

    pub fn scroll_search_active(&self) -> bool {
        self.scroll_search.is_some()
    }

    /// The query being searched for, or "" when there is no search.
    pub fn scroll_search_query(&self) -> &str {
        self.scroll_search.as_ref().map_or("", |s| s.query.as_str())
    }

    fn scroll_search_status(&self) -> Option<ScrollSearchStatus> {
        self.scroll_search.as_ref().map(|s| ScrollSearchStatus {
            query: s.query.clone(),
            count: s.matches.len(),
            index: if s.index >= 0 {
                s.index as usize + 1
            } else {
                0
            },
            typing: s.typing,
        })
    }

    fn search_buffer(&self) -> &[String] {
        if self.flat_lines.is_empty() {
            &self.previous_lines
        } else {
            &self.flat_lines
        }
    }

    fn find_scroll_matches(&self, query: &str) -> Vec<i64> {
        let needle = query.to_lowercase();
        self.search_buffer()
            .iter()
            .enumerate()
            .filter(|(_, line)| {
                !line.is_empty()
                    && !fold_match_ranges(&strip_vt_control_characters(line), &needle).is_empty()
            })
            .map(|(row, _)| row as i64)
            .collect()
    }

    /// Re-run the search if the buffer changed under it. `rescan_all`
    /// (a resize re-wrapped the transcript, or a rebuild replaced it)
    /// re-scans the whole buffer even at the same total. Otherwise only
    /// plain streaming growth re-scans, and only its tail: earlier rows keep
    /// their matches. Shrinks always re-scan whole.
    fn refresh_scroll_search(&mut self, total: i64, rescan_all: bool) {
        let query = match self.scroll_search.as_ref() {
            Some(search) if !search.query.is_empty() => search.query.clone(),
            _ => return,
        };
        let measured = self.scroll_search.as_ref().and_then(|s| s.measured_at);
        if !rescan_all && measured == Some(total) {
            return;
        }
        let needle = query.to_lowercase();
        let buffer_len = self.search_buffer().len();
        let tail = match measured {
            Some(prev) if !rescan_all && total > prev && prev >= 0 => {
                (prev as usize).min(buffer_len)
            }
            _ => 0,
        };
        // Rows below the tail are new: earlier matches stand, the tail is
        // scanned with the shared matcher and its rows offset home.
        let mut matches = if tail == 0 {
            Vec::new()
        } else {
            self.scroll_search.as_ref().unwrap().matches.clone()
        };
        for (i, line) in self.search_buffer()[tail..].iter().enumerate() {
            if !line.is_empty()
                && !fold_match_ranges(&strip_vt_control_characters(line), &needle).is_empty()
            {
                matches.push((tail + i) as i64);
            }
        }
        let search = self.scroll_search.as_mut().unwrap();
        search.matches = matches;
        search.measured_at = Some(total);
        if search.index >= search.matches.len() as i64 {
            search.index = search.matches.len() as i64 - 1;
        }
    }

    /// Mark the query where it appears in a row about to be painted, slicing
    /// by display column so the row's own styling survives. A matched row
    /// also carries the Mission Control contact marker (M8e3): `◂ contact`.
    /// The marker is plain text (a shape, not a colour, per the NO_COLOR
    /// rules); the frame cuts the row to the body width, so long rows keep
    /// the highlight and drop the marker first.
    fn highlight_scroll_matches(line: &str, query: &str) -> String {
        let plain = strip_vt_control_characters(line);
        let needle = query.to_lowercase();
        let ranges = fold_match_ranges(&plain, &needle);
        if ranges.is_empty() {
            return line.to_string();
        }
        let mut out = String::new();
        let mut cursor_col = 0;
        for (start, end) in ranges {
            let start_col = visible_width(hoocode_tui_util::text_slice::prefix(&plain, start));
            let end_col = visible_width(hoocode_tui_util::text_slice::prefix(&plain, end));
            out.push_str(&slice_by_column(
                line,
                cursor_col,
                start_col - cursor_col,
                false,
            ));
            out.push_str(&format!(
                "\x1b[7m{}\x1b[27m",
                slice_by_column(line, start_col, end_col - start_col, false)
            ));
            cursor_col = end_col;
        }
        out.push_str(&slice_by_column(line, cursor_col, usize::MAX / 2, false));
        out.push_str(" \u{25c2} contact");
        out
    }

    pub fn set_show_hardware_cursor(&mut self, enabled: bool) {
        if self.show_hardware_cursor == enabled {
            return;
        }
        self.show_hardware_cursor = enabled;
        if !enabled {
            self.terminal.hide_cursor();
        }
        self.request_render(false);
    }

    pub fn set_clear_on_shrink(&mut self, enabled: bool) {
        self.clear_on_shrink = enabled;
    }

    pub fn add_child(&mut self, component: ComponentHandle) {
        self.root.add_child(component);
    }

    pub fn remove_child(&mut self, component: &ComponentHandle) {
        self.root.remove_child(component);
    }

    pub fn set_focus(&mut self, component: Option<ComponentHandle>) {
        if let Some(old) = &self.focused_component {
            if old.borrow().is_focusable() {
                old.borrow_mut().set_focused(false);
            }
        }
        if let Some(new) = &component {
            if new.borrow().is_focusable() {
                new.borrow_mut().set_focused(true);
            }
        }
        self.focused_component = component;
    }

    /// The component keystrokes are currently going to.
    pub fn focused(&self) -> Option<ComponentHandle> {
        self.focused_component.clone()
    }

    /// Where each root child's output starts, in rows, from the last render.
    pub fn child_row_offsets(&self, width: u16) -> Option<Vec<usize>> {
        self.root.child_row_offsets(width)
    }

    pub fn invalidate(&mut self) {
        self.root.invalidate();
    }

    /// Start the terminal and return a channel of raw input/resize events.
    /// The caller must drive these into [`Tui::process_event`] on the
    /// thread that owns this `Tui` (see module docs on the `Send` split).
    pub fn start(&mut self) -> Receiver<TuiEvent> {
        self.stopped = false;
        self.started = true;
        if self.fullscreen {
            // The first frame is a full paint on the alternate screen.
            self.previous_screen.clear();
            self.previous_screen_size = None;
            self.terminal.set_alternate_screen(true);
        }
        let (tx, rx) = mpsc::channel();
        *self.wake.sender.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx.clone());
        self.wake.clear();
        let tx_drained = tx.clone();
        self.terminal.on_output_drained(Box::new(move || {
            let _ = tx_drained.send(TuiEvent::OutputDrained);
        }));
        let tx_input = tx.clone();
        self.terminal.start(
            Box::new(move |data: &str| {
                let _ = tx_input.send(TuiEvent::input(data));
            }),
            Box::new(move || {
                let _ = tx.send(TuiEvent::Resize);
            }),
        );
        self.terminal.hide_cursor();
        self.query_cell_size();
        self.request_render(false);
        rx
    }

    /// A handle other threads use to wake the loop reading [`Tui::start`]'s channel.
    pub fn wake_handle(&self) -> WakeHandle {
        self.wake.clone()
    }

    /// Handles one event and paints if it asked for a frame.
    pub fn process_event(&mut self, event: TuiEvent) {
        self.accept_event(event);
        self.flush_scheduled_render();
    }

    /// Handles one event without painting. A frame it asks for waits for
    /// [`flush_scheduled_render`](Self::flush_scheduled_render), so a loop can
    /// take several keys and paint once.
    pub fn accept_event(&mut self, event: TuiEvent) {
        match event {
            TuiEvent::Input(data, _) => {
                self.in_input = true;
                self.handle_input(&data);
                self.in_input = false;
            }
            TuiEvent::Resize => self.schedule_render(),
            TuiEvent::OutputDrained => {}
            TuiEvent::Wake => self.wake.clear(),
        }
    }

    /// Marks the frame dirty without painting it (see [`flush_scheduled_render`](Self::flush_scheduled_render)).
    pub fn schedule_render(&mut self) {
        if !self.stopped {
            self.render_scheduled = true;
        }
    }

    /// Paints the frame if one was scheduled. A frame waits while the terminal
    /// is still writing the previous one; the output thread then wakes the loop
    /// with [`TuiEvent::OutputDrained`].
    pub fn flush_scheduled_render(&mut self) {
        if self.render_scheduled {
            self.do_render();
        }
    }

    /// Install the input interceptor: it sees each input after mouse reports
    /// are taken and before the input listeners, and may drive the TUI
    /// (scroll, search). hoocode's scroll view is an input listener closing
    /// over the TUI; a Rust listener cannot hold the TUI that owns it.
    pub fn set_input_interceptor(&mut self, interceptor: Option<InputInterceptor>) {
        self.input_interceptor = interceptor;
    }

    /// The root's children, in order (`ui.children`).
    pub fn children(&self) -> &[ComponentHandle] {
        &self.root.children
    }

    pub fn add_input_listener(&mut self, listener: InputListener) {
        self.input_listeners.push(listener);
    }

    fn query_cell_size(&mut self) {
        if get_capabilities().images.is_none() {
            return;
        }
        // Query terminal for cell size in pixels: CSI 16 t.
        // Response: CSI 6 ; height ; width t
        self.terminal.write("\x1b[16t");
    }

    /// Turns the full-screen mode on or off (M1). Off by default. It is read by
    /// [`Tui::start`], which enters the alternate screen when it is on. While
    /// the TUI runs the call is ignored: switching screens under a live frame
    /// would desync the diff. Between `stop` and `start` it takes effect.
    pub fn set_fullscreen(&mut self, on: bool) {
        if self.started {
            return;
        }
        self.fullscreen = on;
    }

    /// Whether the full-screen mode is on.
    pub fn fullscreen(&self) -> bool {
        self.fullscreen
    }

    /// The transcript in full-screen mode. Root children before it are the top
    /// chrome (HUD, gap, attach), those after it the bottom (prompt and below).
    /// Unset, the body is empty and every child is bottom.
    pub fn set_fullscreen_body(&mut self, body: Option<ComponentHandle>) {
        self.fullscreen_body = body;
    }

    pub fn stop(&mut self) {
        self.started = false;
        if self.fullscreen {
            // Leaving the alternate screen restores the normal one, as it was
            // before launch. Nothing is written below the content: that screen
            // was ours, and a newline here would scroll the terminal. Kitty
            // images are freed first, as the off path frees its pinned copies.
            self.stopped = true;
            let ids: Vec<u32> = self.previous_kitty_image_ids.drain().collect();
            let buffer = self.delete_kitty_images(ids);
            if !buffer.is_empty() {
                self.terminal.write(&buffer);
            }
            self.previous_screen.clear();
            self.previous_screen_size = None;
            self.terminal.set_alternate_screen(false);
            self.terminal.show_cursor();
            self.terminal.stop();
            return;
        }
        // Off the alternate screen before the exit bookkeeping below, which
        // moves the cursor relative to content on the normal screen.
        if self.scroll_offset.is_some() {
            self.scroll_offset = None;
            self.release_scroll_images();
            self.terminal.set_alternate_screen(false);
        }
        self.stopped = true;
        if !self.previous_lines.is_empty() {
            let target_row = self.previous_lines.len() as i64;
            let line_diff = target_row - self.hardware_cursor_row;
            let moves = move_rows(line_diff);
            if !moves.is_empty() {
                self.terminal.write(&moves);
            }
            self.terminal.write("\r\n");
        }
        self.terminal.show_cursor();
        self.terminal.stop();
    }

    /// Request a render. `force` clears all cached state for a full
    /// redraw (matching `requestRender(true)`). The frame is painted now,
    /// unless it was requested while handling input (painted once per loop
    /// iteration) or the terminal is still writing the previous frame.
    pub fn request_render(&mut self, force: bool) {
        if force {
            self.previous_lines.clear();
            self.force_redraw = true;
            self.hardware_cursor_row = 0;
            self.max_lines_rendered = 0;
            self.previous_viewport_top = 0;
        }
        if self.stopped {
            return;
        }
        if self.in_input {
            self.render_scheduled = true;
            return;
        }
        self.do_render();
    }

    fn handle_input(&mut self, data: &str) {
        let mut data = data.to_string();
        // Ahead of the listeners: a mouse report that reaches a text field is
        // typed into it.
        if self.terminal.mouse_reporting() {
            match self.consume_mouse_reports(&data) {
                None => return,
                Some(rest) => data = rest,
            }
        }
        if let Some(mut interceptor) = self.input_interceptor.take() {
            let consumed = interceptor(self, &data);
            if self.input_interceptor.is_none() {
                self.input_interceptor = Some(interceptor);
            }
            if consumed {
                return;
            }
        }
        if !self.input_listeners.is_empty() {
            let mut current = data.clone();
            let mut consumed = false;
            for listener in &mut self.input_listeners {
                if let Some(result) = listener(&current) {
                    if result.consume {
                        consumed = true;
                        break;
                    }
                    if let Some(new_data) = result.data {
                        current = new_data;
                    }
                }
            }
            if consumed {
                return;
            }
            if current.is_empty() {
                return;
            }
            data = current;
        }

        if self.consume_cell_size_response(&data) {
            return;
        }

        if let Some(focused) = self.focused_component.clone() {
            if is_key_release(&data) {
                return;
            }
            focused.borrow_mut().handle_input(&data);
            self.request_render(false);
        }
    }

    /// Act on every mouse report in `data` and return what is left of it;
    /// `None` when the chunk was nothing but reports.
    fn consume_mouse_reports(&mut self, data: &str) -> Option<String> {
        if !data.contains("\x1b[<") && !data.contains("\x1b[M") {
            return Some(data.to_string());
        }
        let mut rest = data;
        let mut out = String::new();
        let mut saw_report = false;
        while !rest.is_empty() {
            let Some((event, length)) = parse_mouse_report(rest) else {
                let ch = rest.chars().next().unwrap();
                out.push(ch);
                rest = hoocode_tui_util::text_slice::suffix_from(rest, ch.len_utf8());
                continue;
            };
            if let Some(event) = event {
                self.handle_mouse_event(event);
            }
            saw_report = true;
            rest = hoocode_tui_util::text_slice::suffix_from(rest, length);
        }
        if !saw_report {
            return Some(data.to_string());
        }
        (!out.is_empty()).then_some(out)
    }

    /// The wheel scrolls; a click (press and release on the same cell) opens
    /// the link under it. Anything else is swallowed.
    fn handle_mouse_event(&mut self, event: MouseEvent) {
        match event.kind {
            MouseEventKind::WheelUp => {
                self.scroll_by_lines(-WHEEL_LINES);
            }
            MouseEventKind::WheelDown => {
                self.scroll_by_lines(WHEEL_LINES);
            }
            MouseEventKind::Press => {
                if event.button == 0 {
                    self.press_scrollbar(event.row, event.column);
                }
                self.pressed_cell = (event.button == 0).then_some((event.row, event.column));
            }
            MouseEventKind::Release => {
                let pressed = self.pressed_cell.take();
                if self.on_hyperlink.is_none() {
                    return;
                }
                if pressed != Some((event.row, event.column)) {
                    return;
                }
                if let Some(url) = self.hyperlink_at_screen_cell(event.row, event.column) {
                    if let Some(cb) = &mut self.on_hyperlink {
                        cb(&url);
                    }
                }
            }
            _ => {}
        }
    }

    /// A press in the scrollbar column of a pinned view pages towards it:
    /// above the thumb pages up, below it pages down, on it does nothing.
    fn press_scrollbar(&mut self, row: i64, column: i64) {
        let Some(top) = self.scroll_offset else {
            return;
        };
        if column != self.terminal.columns() as i64 {
            return;
        }
        let view_height = self.scroll_view_height();
        // The view starts on screen row 1, or below the full-screen top chrome.
        let first = if self.fullscreen {
            self.fullscreen_body_window
                .as_ref()
                .map_or(0, |window| window.0)
        } else {
            0
        };
        let index = row - 1 - first;
        if index < 0 || index >= view_height {
            return;
        }
        let glyphs = scrollbar_glyphs(view_height, self.scroll_total_lines, top);
        let thumb: Vec<i64> = glyphs
            .iter()
            .enumerate()
            .filter(|(_, glyph)| **glyph == SCROLLBAR_THUMB)
            .map(|(i, _)| i as i64)
            .collect();
        let (Some(&first), Some(&last)) = (thumb.first(), thumb.last()) else {
            return;
        };
        if index < first {
            self.scroll_by_pages(-1);
        } else if index > last {
            self.scroll_by_pages(1);
        }
    }

    /// The link on the screen cell a mouse report names, through the window
    /// the last frame painted. A live buffer shorter than the screen is
    /// declined rather than guessed at. A picker's rows link to nothing.
    fn hyperlink_at_screen_cell(&self, row: i64, column: i64) -> Option<String> {
        if self.fullscreen {
            // Only body rows carry the transcript's links; `row` is 1-based.
            let (first_row, lines) = self.fullscreen_body_window.as_ref()?;
            let index = usize::try_from(row - 1 - first_row).ok()?;
            let line = lines.get(index)?;
            return hyperlink_at(line, column - 1).or_else(|| bare_url_at(line, column - 1));
        }
        let pinned = self.scroll_offset.is_some();
        let lines: &[String] = if pinned {
            self.scroll_view_lines.as_deref()?
        } else {
            &self.previous_lines
        };
        if lines.is_empty() {
            return None;
        }
        if pinned && row > self.scroll_view_height() {
            return None;
        }
        if !pinned && (lines.len() as i64) < self.terminal.rows() as i64 {
            return None;
        }
        let top = if pinned {
            self.scroll_offset.unwrap_or(0)
        } else {
            self.previous_viewport_top
        };
        let index = top + row - 1;
        let line = lines.get(usize::try_from(index).ok()?)?;
        hyperlink_at(line, column - 1).or_else(|| bare_url_at(line, column - 1))
    }

    fn consume_cell_size_response(&mut self, data: &str) -> bool {
        // Response format: ESC [ 6 ; height ; width t
        let Some(rest) = data.strip_prefix("\x1b[6;") else {
            return false;
        };
        let Some(rest) = rest.strip_suffix('t') else {
            return false;
        };
        let mut parts = rest.splitn(2, ';');
        let (Some(h), Some(w)) = (parts.next(), parts.next()) else {
            return false;
        };
        let (Ok(height_px), Ok(width_px)) = (h.parse::<u32>(), w.parse::<u32>()) else {
            return false;
        };
        if height_px == 0 || width_px == 0 {
            return true;
        }
        set_cell_dimensions(CellDimensions {
            width_px,
            height_px,
        });
        self.invalidate();
        self.request_render(false);
        true
    }

    /// Bytes for one row. Image rows pass through; text rows are cut to `width`
    /// columns so an over-wide component line can never overrun the terminal.
    fn emit_line(&mut self, line: &str, width: i64) -> String {
        if hoocode_tui_images::is_image_line(line) {
            self.saw_image_line = true;
            return line.to_string();
        }
        let text = slice_by_column(
            &normalize_terminal_output(line),
            0,
            width.max(0) as usize,
            true,
        );
        format!("{text}{SEGMENT_RESET}")
    }

    fn collect_kitty_image_ids(&self, lines: &[String]) -> HashSet<u32> {
        if !self.saw_image_line {
            return HashSet::new();
        }
        let mut ids = HashSet::new();
        for line in lines {
            for id in extract_kitty_image_ids(line) {
                ids.insert(id);
            }
        }
        ids
    }

    fn delete_kitty_images(&self, ids: impl IntoIterator<Item = u32>) -> String {
        let mut buffer = String::new();
        for id in ids {
            buffer.push_str(&delete_kitty_image(id));
        }
        buffer
    }

    fn expand_last_changed_for_kitty_images(&self, first_changed: i64, last_changed: i64) -> i64 {
        if !self.saw_image_line {
            return last_changed;
        }
        let mut expanded = last_changed;
        let start = first_changed.max(0) as usize;
        for (i, line) in self.previous_lines.iter().enumerate().skip(start) {
            if !extract_kitty_image_ids(line).is_empty() {
                expanded = expanded.max(i as i64);
            }
        }
        expanded
    }

    fn delete_changed_kitty_images(&self, first_changed: i64, last_changed: i64) -> String {
        if first_changed < 0 || last_changed < first_changed {
            return String::new();
        }
        let mut ids = HashSet::new();
        let max_line = last_changed.min(self.previous_lines.len() as i64 - 1);
        if max_line < 0 {
            return String::new();
        }
        for i in first_changed as usize..=max_line as usize {
            if let Some(line) = self.previous_lines.get(i) {
                for id in extract_kitty_image_ids(line) {
                    ids.insert(id);
                }
            }
        }
        self.delete_kitty_images(ids)
    }

    /// Find and strip the cursor marker from rendered lines, returning its position.
    fn extract_cursor_position(&self, lines: &mut [String], height: i64) -> Option<CursorPos> {
        let viewport_top = (lines.len() as i64 - height).max(0);
        for row in (viewport_top..lines.len() as i64).rev() {
            let line = &lines[row as usize];
            if let Some(marker_index) = line.find(CURSOR_MARKER) {
                let before_marker = hoocode_tui_util::text_slice::prefix(line, marker_index);
                let col = visible_width(before_marker) as i64;
                let after = hoocode_tui_util::text_slice::suffix_from(
                    line,
                    marker_index + CURSOR_MARKER.len(),
                )
                .to_string();
                let mut new_line =
                    hoocode_tui_util::text_slice::prefix(line, marker_index).to_string();
                new_line.push_str(&after);
                lines[row as usize] = new_line;
                return Some(CursorPos { row, col });
            }
        }
        None
    }

    /// Flatten the component tree. Simplified relative to the TypeScript
    /// original: always fully re-renders (see module docs on flatten memoization).
    pub fn render(&mut self, width: u16) -> Vec<String> {
        self.root.render(width)
    }

    /// Install (or remove) the observer that gets the timing of every frame.
    pub fn set_frame_observer(&mut self, observer: Option<FrameObserver>) {
        self.frame_observer = observer;
    }

    /// Paint one frame, then report how long it took to build and to write.
    fn do_render(&mut self) {
        if self.stopped {
            self.render_scheduled = false;
            return;
        }
        // A frame is a diff against the one before it, so a new frame must not be
        // built while the terminal still lacks the previous one. The state the
        // frame is built from is read when it is finally built, so nothing is lost.
        if self.terminal.frame_pending() {
            self.render_scheduled = true;
            self.terminal.notify_when_drained();
            return;
        }
        self.render_scheduled = false;
        let started = Instant::now();
        self.frame_write_time = Duration::ZERO;
        self.paint();
        if let Some(observer) = self.frame_observer.as_mut() {
            let write = self.frame_write_time;
            let build = started.elapsed().saturating_sub(write);
            observer(FrameTiming { build, write });
        }
    }

    /// The frame's bytes to the terminal, timed.
    fn write_frame(&mut self, buffer: &str) {
        let started = Instant::now();
        self.terminal.write_frame(buffer);
        self.frame_write_time += started.elapsed();
    }

    fn paint(&mut self) {
        if self.stopped {
            return;
        }
        if self.fullscreen {
            self.paint_fullscreen();
            return;
        }
        if self.scroll_offset.is_some() && !self.scroll_pane_fits() {
            // The picker would clip its own top rows: let go of the pin. The
            // release paints the live frame itself (or schedules it).
            self.scroll_to_live();
            return;
        }
        if self.scroll_offset.is_some() {
            self.render_scroll_view();
            return;
        }
        self.paint_live();
    }

    /// A live paint: a full redraw when the screen cannot be diffed, else the
    /// rows that changed since the last frame.
    fn paint_live(&mut self) {
        self.log_paint("live");
        let forced = std::mem::take(&mut self.force_redraw);
        let width = self.terminal.columns() as i64;
        let height = self.terminal.rows() as i64;
        let width_changed = self.previous_width.is_some_and(|w| w != width);
        let height_changed = self.previous_height.is_some_and(|h| h != height);
        let previous_buffer_length = self
            .previous_height
            .map_or(height, |h| self.previous_viewport_top + h);
        let prev_viewport_top = if height_changed {
            (previous_buffer_length - height).max(0)
        } else {
            self.previous_viewport_top
        };

        let (new_lines, cursor_pos) = self.build_live_frame(width, height);
        let line_count = new_lines.len() as i64;
        if let Some(clear) =
            self.full_redraw_clear(forced, width_changed, height_changed, line_count)
        {
            self.full_render(new_lines, width, height, clear);
            return;
        }

        let geo = FrameGeometry {
            width,
            height,
            prev_viewport_top,
            prev_line_count: self.previous_lines.len() as i64,
        };
        let (changed, append_start) = self.changed_rows(&new_lines);

        // --- The window has to move *back* over the buffer. ---
        // A buffer taller than the screen shrank, so rows the reader scrolled
        // past come back at the top; the append-only path cannot express that,
        // so the visible window is painted in place. Only trees with a flex
        // spacer take this path.
        let window_top = (line_count - height).max(0);
        if self.flex_spacer.is_some()
            && line_count < geo.prev_line_count
            && window_top < prev_viewport_top
        {
            if line_count < height || self.saw_image_line {
                self.full_render(new_lines, width, height, true);
                return;
            }
            self.paint_window_in_place(new_lines, geo, cursor_pos);
            return;
        }

        // --- No changes: still may need to move the hardware cursor. ---
        let Some(changed) = changed else {
            self.position_hardware_cursor(&cursor_pos, line_count);
            self.previous_viewport_top = prev_viewport_top;
            self.previous_height = known_size(height);
            return;
        };
        let (first_changed, _) = changed;

        // --- All changes are deleted lines: nothing to render, just clear. ---
        if first_changed >= line_count {
            self.paint_deleted_lines(new_lines, geo, cursor_pos, changed);
            return;
        }

        // --- Changes above the previous viewport require a full redraw. ---
        if first_changed < prev_viewport_top {
            self.full_render(new_lines, width, height, true);
            return;
        }

        self.paint_differential(new_lines, geo, cursor_pos, changed, append_start);
    }

    /// Flattens the tree for this frame, fits the filler to the screen and
    /// takes the cursor marker out. Returns the frame and the cursor position.
    fn build_live_frame(&mut self, width: i64, height: i64) -> (Vec<String>, Option<CursorPos>) {
        let mut new_lines = self.render(width.max(0) as u16);
        // The frame has to exist before its leftover rows can be counted, so
        // the fill is settled by flattening again.
        if self.fit_flex_spacer(&new_lines, height) {
            new_lines = self.render(width.max(0) as u16);
        }
        self.flat_lines = new_lines.clone();

        let cursor_pos = self.extract_cursor_position(&mut new_lines, height);
        self.last_cursor_pos = cursor_pos;
        (new_lines, cursor_pos)
    }

    /// Whether this frame needs a full redraw, and if so whether it clears the
    /// screen first. `None` leaves the frame to the differential paths.
    fn full_redraw_clear(
        &self,
        forced: bool,
        width_changed: bool,
        height_changed: bool,
        line_count: i64,
    ) -> Option<bool> {
        // The first frame writes everything without clearing.
        if self.previous_lines.is_empty() && !forced && !width_changed && !height_changed {
            return Some(false);
        }
        // Forced, or width change: a full re-render (wrapping changes).
        if forced || width_changed {
            return Some(true);
        }
        // --- Height change: full re-render to keep the viewport aligned, except
        // on Termux, whose height changes with the software keyboard (a full
        // redraw there replays the whole history on every toggle). ---
        if height_changed && std::env::var_os("TERMUX_VERSION").is_none_or(|v| v.is_empty()) {
            return Some(true);
        }
        // --- Content shrunk below the working area: clear empty rows. ---
        if self.clear_on_shrink && line_count < self.max_lines_rendered {
            return Some(true);
        }
        None
    }

    /// Full-buffer diff: the `(first, last)` rows that differ from the last
    /// frame (`None` when none do), and whether the frame only appends.
    fn changed_rows(&self, new_lines: &[String]) -> (Option<(i64, i64)>, bool) {
        let prev_line_count = self.previous_lines.len() as i64;
        // (first, last) changed row; `None` when the frames match.
        let mut changed: Option<(i64, i64)> = None;
        let max_lines = (new_lines.len() as i64).max(prev_line_count);
        for i in 0..max_lines {
            let old_line = if i < prev_line_count {
                self.previous_lines[i as usize].as_str()
            } else {
                ""
            };
            let new_line = if (i as usize) < new_lines.len() {
                new_lines[i as usize].as_str()
            } else {
                ""
            };
            if old_line != new_line {
                changed = Some(changed.map_or((i, i), |(first, _)| (first, i)));
            }
        }

        let appended_lines = (new_lines.len() as i64) > prev_line_count;
        if appended_lines {
            let first = changed.map_or(prev_line_count, |(first, _)| first);
            changed = Some((first, new_lines.len() as i64 - 1));
        }
        if let Some((first, last)) = changed {
            let last = self.expand_last_changed_for_kitty_images(first, last);
            changed = Some((first, last));
        }
        let append_start = appended_lines
            && changed.is_some_and(|(first, _)| first == prev_line_count && first > 0);
        (changed, append_start)
    }

    /// Paints the window of a shrunken buffer in place: the whole screen, since
    /// the append-only path cannot move the window back over the buffer.
    fn paint_window_in_place(
        &mut self,
        new_lines: Vec<String>,
        geo: FrameGeometry,
        cursor_pos: Option<CursorPos>,
    ) {
        let FrameGeometry { width, height, .. } = geo;
        let line_count = new_lines.len() as i64;
        let window_top = (line_count - height).max(0);
        let mut buffer = String::from("\x1b[?2026h");
        // Autowrap off: the bottom row must not wrap into a scroll.
        buffer.push_str("\x1b[?7l");
        for row in 0..height {
            buffer.push_str(&format!("\x1b[{};1H\x1b[2K", row + 1));
            let line = new_lines
                .get((window_top + row) as usize)
                .cloned()
                .unwrap_or_default();
            buffer.push_str(&self.emit_line(&line, width));
        }
        buffer.push_str("\x1b[?7h");
        self.hardware_cursor_row = line_count - 1;
        buffer.push_str(&self.build_hardware_cursor_move(&cursor_pos, line_count));
        buffer.push_str("\x1b[?2026l");
        self.write_frame(&buffer);
        self.previous_kitty_image_ids = self.collect_kitty_image_ids(&new_lines);
        self.previous_lines = new_lines;
        self.previous_width = known_size(width);
        self.previous_height = known_size(height);
        self.previous_viewport_top = window_top;
        self.max_lines_rendered = self
            .max_lines_rendered
            .max(self.previous_lines.len() as i64);
    }

    /// The frame only shortened: clear the rows past its end.
    fn paint_deleted_lines(
        &mut self,
        new_lines: Vec<String>,
        geo: FrameGeometry,
        cursor_pos: Option<CursorPos>,
        changed: (i64, i64),
    ) {
        let FrameGeometry {
            width,
            height,
            prev_viewport_top,
            prev_line_count,
        } = geo;
        let (first_changed, last_changed) = changed;
        let line_count = new_lines.len() as i64;
        if prev_line_count > line_count {
            let mut buffer = String::from("\x1b[?2026h");
            buffer.push_str(&self.delete_changed_kitty_images(first_changed, last_changed));
            let target_row = (line_count - 1).max(0);
            if target_row < prev_viewport_top {
                self.full_render(new_lines, width, height, true);
                return;
            }
            let line_diff = {
                let current_screen_row = self.hardware_cursor_row - prev_viewport_top;
                let target_screen_row = target_row - prev_viewport_top;
                target_screen_row - current_screen_row
            };
            buffer.push_str(&move_rows(line_diff));
            buffer.push('\r');
            let extra_lines = prev_line_count - line_count;
            if extra_lines > height {
                self.full_render(new_lines, width, height, true);
                return;
            }
            if extra_lines > 0 {
                buffer.push_str("\x1b[1B");
            }
            for i in 0..extra_lines {
                buffer.push_str("\r\x1b[2K");
                if i < extra_lines - 1 {
                    buffer.push_str("\x1b[1B");
                }
            }
            if extra_lines > 0 {
                buffer.push_str(&format!("\x1b[{extra_lines}A"));
            }
            self.hardware_cursor_row = target_row;
            buffer.push_str(&self.build_hardware_cursor_move(&cursor_pos, line_count));
            buffer.push_str("\x1b[?2026l");
            self.write_frame(&buffer);
        } else {
            self.position_hardware_cursor(&cursor_pos, line_count);
        }
        self.previous_kitty_image_ids = self.collect_kitty_image_ids(&new_lines);
        self.previous_lines = new_lines;
        self.previous_width = known_size(width);
        self.previous_height = known_size(height);
        self.previous_viewport_top = prev_viewport_top;
    }

    /// The differential paint: rewrite the changed rows from the first one to
    /// the end, scrolling the screen first when the change lies below it.
    fn paint_differential(
        &mut self,
        new_lines: Vec<String>,
        geo: FrameGeometry,
        cursor_pos: Option<CursorPos>,
        changed: (i64, i64),
        append_start: bool,
    ) {
        let (width, height, prev_line_count) = (geo.width, geo.height, geo.prev_line_count);
        let (first_changed, last_changed) = changed;
        let line_count = new_lines.len() as i64;
        let mut prev_viewport_top = geo.prev_viewport_top;
        let mut viewport_top = prev_viewport_top;
        let mut hardware_cursor_row = self.hardware_cursor_row;

        let mut buffer = String::from("\x1b[?2026h");
        buffer.push_str(&self.delete_changed_kitty_images(first_changed, last_changed));
        let prev_viewport_bottom = prev_viewport_top + height - 1;
        let move_target_row = if append_start {
            first_changed - 1
        } else {
            first_changed
        };
        if move_target_row > prev_viewport_bottom {
            let current_screen_row = (hardware_cursor_row - prev_viewport_top).clamp(0, height - 1);
            let move_to_bottom = height - 1 - current_screen_row;
            if move_to_bottom > 0 {
                buffer.push_str(&format!("\x1b[{move_to_bottom}B"));
            }
            let scroll = move_target_row - prev_viewport_bottom;
            buffer.push_str(&"\r\n".repeat(scroll.max(0) as usize));
            prev_viewport_top += scroll;
            viewport_top += scroll;
            hardware_cursor_row = move_target_row;
        }

        let line_diff = {
            let current_screen_row = hardware_cursor_row - prev_viewport_top;
            let target_screen_row = move_target_row - viewport_top;
            target_screen_row - current_screen_row
        };
        buffer.push_str(&move_rows(line_diff));

        buffer.push_str(if append_start { "\r\n" } else { "\r" });

        let render_end = last_changed.min(line_count - 1);
        for i in first_changed..=render_end {
            if i > first_changed {
                buffer.push_str("\r\n");
            }
            buffer.push_str("\x1b[2K");
            buffer.push_str(&self.emit_line(&new_lines[i as usize], width));
        }

        let mut final_cursor_row = render_end;
        if prev_line_count > line_count {
            if render_end < line_count - 1 {
                let move_down = line_count - 1 - render_end;
                buffer.push_str(&format!("\x1b[{move_down}B"));
                final_cursor_row = line_count - 1;
            }
            let extra_lines = prev_line_count - line_count;
            for _ in line_count..prev_line_count {
                buffer.push_str("\r\n\x1b[2K");
            }
            buffer.push_str(&format!("\x1b[{extra_lines}A"));
        }

        self.hardware_cursor_row = final_cursor_row;
        // Inside the synchronized block, so the frame is never presented with
        // the cursor still parked at the end of the last redrawn line.
        buffer.push_str(&self.build_hardware_cursor_move(&cursor_pos, line_count));

        buffer.push_str("\x1b[?2026l");
        self.write_frame(&buffer);

        self.max_lines_rendered = self.max_lines_rendered.max(line_count);
        self.previous_viewport_top = prev_viewport_top.max(final_cursor_row - height + 1);

        self.previous_kitty_image_ids = self.collect_kitty_image_ids(&new_lines);
        self.previous_lines = new_lines;
        self.previous_width = known_size(width);
        self.previous_height = known_size(height);
    }

    /// Paints `new_lines` whole and keeps them as the differential baseline.
    /// Takes the frame by value so the baseline is a move, not a copy.
    fn full_render(&mut self, new_lines: Vec<String>, width: i64, height: i64, clear: bool) {
        self.full_redraw_count += 1;
        let mut buffer = String::from("\x1b[?2026h");
        if clear {
            let ids: Vec<u32> = self.previous_kitty_image_ids.iter().copied().collect();
            buffer.push_str(&self.delete_kitty_images(ids));
            buffer.push_str("\x1b[2J\x1b[H\x1b[3J");
        }
        for (i, line) in new_lines.iter().enumerate() {
            if i > 0 {
                buffer.push_str("\r\n");
            }
            buffer.push_str(&self.emit_line(line, width));
        }
        self.hardware_cursor_row = (new_lines.len() as i64 - 1).max(0);
        let cp = self.last_cursor_pos;
        buffer.push_str(&self.build_hardware_cursor_move(&cp, new_lines.len() as i64));
        buffer.push_str("\x1b[?2026l");
        self.write_frame(&buffer);

        let line_count = new_lines.len() as i64;
        if clear {
            self.max_lines_rendered = line_count;
        } else {
            self.max_lines_rendered = self.max_lines_rendered.max(line_count);
        }
        let buffer_length = height.max(line_count);
        self.previous_viewport_top = (buffer_length - height).max(0);

        self.previous_kitty_image_ids = self.collect_kitty_image_ids(&new_lines);
        self.previous_lines = new_lines;
        self.previous_width = known_size(width);
        self.previous_height = known_size(height);
    }

    /// The escape sequence that parks the hardware cursor for this frame
    /// (`buildHardwareCursorMove`); callers fold it into the frame buffer
    /// inside the synchronized block. Updates `hardware_cursor_row`.
    fn build_hardware_cursor_move(
        &mut self,
        cursor_pos: &Option<CursorPos>,
        total_lines: i64,
    ) -> String {
        let visibility = if self.show_hardware_cursor {
            SHOW_CURSOR
        } else {
            HIDE_CURSOR
        };
        let Some(cursor_pos) = cursor_pos.filter(|_| total_lines > 0) else {
            // Nothing focused: return to column 0 so a frame ending in the
            // pending-wrap state keeps the cursor on the row we think it is on.
            return format!("\r{HIDE_CURSOR}");
        };
        let target_row = cursor_pos.row.clamp(0, total_lines - 1);
        let target_col = cursor_pos.col.max(0);
        let row_delta = target_row - self.hardware_cursor_row;
        let mut buffer = String::new();
        buffer.push_str(&move_rows(row_delta));
        buffer.push_str(&format!("\x1b[{}G", target_col + 1));
        self.hardware_cursor_row = target_row;
        buffer.push_str(visibility);
        buffer
    }

    /// Position the hardware cursor as a standalone write, for frames that
    /// emit no content of their own.
    fn position_hardware_cursor(&mut self, cursor_pos: &Option<CursorPos>, total_lines: i64) {
        let buffer = self.build_hardware_cursor_move(cursor_pos, total_lines);
        self.write_frame(&buffer);
    }

    /// Paint the pinned window onto the alternate screen, whole (not
    /// differential): at most a screenful inside one synchronized block. The
    /// differential state is left as the last live frame left it.
    fn render_scroll_view(&mut self) {
        self.log_paint("scroll");
        let width = self.terminal.columns() as i64;
        let height = self.terminal.rows() as i64;
        // The transcript gives up its last column to the scrollbar.
        let transcript_width = self.transcript_width() as i64;

        // No fill while pinned: blank rows would be transcript to scroll past.
        if let Some(spacer) = &self.flex_spacer {
            spacer.borrow_mut().set_height(0);
        }
        let lines = self.render(transcript_width as u16);
        self.flat_lines = lines.clone();

        let pane = self.scroll_pane_lines(width.max(0) as u16);
        let (view_height, pane_rows) = self.pinned_split(pane.len() as i64);
        self.scroll_total_lines = lines.len() as i64;
        let max_offset = (lines.len() as i64 - view_height).max(0);
        // Re-clamped every frame: the transcript can shrink under the view.
        let top = self.scroll_offset.unwrap_or(0).clamp(0, max_offset);
        self.scroll_offset = Some(top);

        let mut buffer = String::from("\x1b[?2026h");
        buffer.push_str(HIDE_CURSOR);
        // Autowrap off: a full-width row would wrap and shift the window.
        buffer.push_str("\x1b[?7l");
        // Kitty placements are not text; last frame's come off first.
        buffer.push_str(&self.clear_scroll_images());

        self.refresh_scroll_search(lines.len() as i64, false);
        let query = self
            .scroll_search
            .as_ref()
            .map(|s| s.query.clone())
            .unwrap_or_default();
        for row in 0..view_height {
            buffer.push_str(&format!("\x1b[{};1H\x1b[2K", row + 1));
            if let Some(line) = lines.get((top + row) as usize) {
                let emitted = self.emit_scroll_line(line, row, &query);
                buffer.push_str(&emitted);
            }
        }

        // One glyph per transcript row, last column, only when there is more to scroll.
        for (row, glyph) in scrollbar_glyphs(view_height, lines.len() as i64, top)
            .iter()
            .enumerate()
        {
            buffer.push_str(&format!("\x1b[{};{width}H{glyph}", row + 1));
        }
        // The picker, bottom-aligned above the status row: its options stay in view.
        let skip = pane.len() - pane_rows as usize;
        for (i, line) in pane.iter().skip(skip).enumerate() {
            buffer.push_str(&format!("\x1b[{};1H\x1b[2K", view_height + i as i64 + 1));
            let text = line.replacen(CURSOR_MARKER, "", 1);
            buffer.push_str(&normalize_terminal_output(&text));
            buffer.push_str(SEGMENT_RESET);
        }

        buffer.push_str(&format!("\x1b[{height};1H\x1b[2K"));
        let status = ScrollStatus {
            top: top + 1,
            bottom: (top + view_height).min(lines.len() as i64),
            total: lines.len() as i64,
            view_height,
            at_top: top == 0,
            at_bottom: top >= max_offset,
            width,
            search: self.scroll_search_status(),
            picker: self.scroll_pane().is_some(),
            new_lines: 0,
        };
        buffer.push_str(&(self.scroll_status_formatter)(&status));
        buffer.push_str("\x1b[?7h");
        buffer.push_str("\x1b[?2026l");
        self.write_frame(&buffer);
        // Kept so a click on the pinned window can be placed.
        self.scroll_view_lines = Some(lines);
    }

    // ── Full-screen mode (M1b) ──────────────────────────────────────────

    /// Paints one full-screen frame. A size change or a forced render clears the
    /// screen and repaints every row; otherwise only the rows that changed since
    /// the last frame are written. Nothing is written below the last row, and
    /// autowrap is off, so no row scrolls into the terminal's scrollback.
    fn paint_fullscreen(&mut self) {
        self.log_paint("fullscreen");
        let width = self.terminal.columns() as usize;
        let height = self.terminal.rows() as usize;
        let forced = std::mem::take(&mut self.force_redraw);
        if width == 0 || height == 0 {
            return;
        }
        let resized = self
            .previous_screen_size
            .is_some_and(|size| size != (width, height));
        let full = forced || resized || self.previous_screen_size.is_none();
        let screen = self.compose_fullscreen(width, height, resized);

        let mut buffer = String::from("\x1b[?2026h\x1b[?7l");
        buffer.push_str(HIDE_CURSOR);
        if full {
            let ids: Vec<u32> = self.previous_kitty_image_ids.iter().copied().collect();
            buffer.push_str(&self.delete_kitty_images(ids));
            buffer.push_str("\x1b[2J\x1b[H");
            for (row, text) in screen.rows.iter().enumerate() {
                buffer.push_str(&format!("\x1b[{};1H\x1b[2K{text}", row + 1));
            }
        } else {
            let changed: Vec<usize> = (0..screen.rows.len())
                .filter(|&row| self.previous_screen.get(row) != Some(&screen.rows[row]))
                .collect();
            // A kitty image on a row that changes goes first; the new row places its own copy.
            let stale: Vec<u32> = changed
                .iter()
                .filter_map(|&row| self.previous_screen.get(row))
                .flat_map(|text| extract_kitty_image_ids(text))
                .collect();
            buffer.push_str(&self.delete_kitty_images(stale));
            for row in changed {
                buffer.push_str(&format!("\x1b[{};1H\x1b[2K{}", row + 1, screen.rows[row]));
            }
        }
        match screen.cursor {
            Some((row, col)) => {
                buffer.push_str(&format!("\x1b[{};{}H", row + 1, col + 1));
                buffer.push_str(if self.show_hardware_cursor {
                    SHOW_CURSOR
                } else {
                    HIDE_CURSOR
                });
            }
            None => buffer.push_str(HIDE_CURSOR),
        }
        buffer.push_str("\x1b[?7h\x1b[?2026l");
        self.write_frame(&buffer);

        self.previous_kitty_image_ids = self.collect_kitty_image_ids(&screen.rows);
        self.previous_screen = screen.rows;
        self.previous_screen_size = Some((width, height));
    }

    /// Composes a full-screen frame of `height` rows: the top chrome, the body
    /// window, the scroll status row while pinned, then the bottom. Each row is
    /// already cut to its region's width and in the form it is written. While
    /// pinned, a focused picker (not the prompt) takes the bottom rows, and the
    /// body gets its scrollbar column and the search highlights.
    fn compose_fullscreen(&mut self, width: usize, height: usize, resized: bool) -> Screen {
        let (top_children, body, bottom_children) = self.fullscreen_regions();
        let top_lines: Vec<String> = render_children(&top_children, width as u16)
            .iter()
            .map(|line| strip_cursor_marker(line))
            .collect();
        // The cursor marker is stripped where a line is drawn (the window), or on
        // every line while pinned, where the search reads the whole body. A live
        // frame does not copy the whole transcript a second time.
        let mut body_lines: Vec<String> = match &body {
            Some(body) => body
                .borrow_mut()
                .render(fullscreen::body_width(width) as u16),
            None => Vec::new(),
        };
        let total = body_lines.len();
        let pinned = self.scroll_offset.is_some();
        // While pinned a picker takes the bottom; the prompt keeps its place.
        let pane = if pinned {
            self.scroll_pane_lines(width as u16)
        } else {
            Vec::new()
        };
        let mut bottom_lines = if pane.is_empty() {
            render_children(&bottom_children, width as u16)
        } else {
            pane
        };
        let rows = fullscreen::split_rows(height, top_lines.len(), pinned, bottom_lines.len());

        // A pinned view keeps its share of the transcript through a resize.
        let mut pinned_top = self.scroll_offset.map(|top| top.max(0) as usize);
        if let Some(top) = pinned_top.filter(|_| resized) {
            let old_total = self.scroll_total_lines.max(0) as usize;
            pinned_top = Some(fullscreen::anchor_on_resize(
                top, old_total, total, rows.body,
            ));
        }
        let start = fullscreen::body_start(total, rows.body, pinned_top);
        if pinned_top.is_some() {
            self.scroll_offset = Some(start as i64);
        }
        self.scroll_total_lines = total as i64;
        self.fullscreen_view = rows.body as i64;
        // A resize re-wraps the transcript and a rebuild replaces it: neither is
        // output that arrived. A transcript that shrank (a clear) also starts over.
        // Otherwise the growth is streaming output, and the search only
        // re-scans that tail.
        let rebuilt = resized || std::mem::take(&mut self.fullscreen_seen_reset);
        if rebuilt {
            self.fullscreen_seen_total = total as i64;
        }
        self.fullscreen_seen_total = self.fullscreen_seen_total.min(total as i64);
        // The search runs over the body lines, as the pinned painter's does.
        let query = if pinned {
            for line in &mut body_lines {
                *line = strip_cursor_marker(line);
            }
            self.flat_lines = body_lines.clone();
            self.refresh_scroll_search(total as i64, rebuilt);
            self.scroll_search
                .as_ref()
                .map(|search| search.query.clone())
                .unwrap_or_default()
        } else {
            String::new()
        };

        let bottom_height = bottom_lines.len() as i64;
        let cursor = self.extract_cursor_position(&mut bottom_lines, bottom_height);
        let window = fullscreen::bottom_window(
            bottom_lines.len(),
            rows.bottom,
            cursor.map(|c| c.row.max(0) as usize),
        );
        let bottom_start = rows.top + rows.body + rows.bar;
        // The cursor shows on the live view only, and only when its row is on screen.
        let cursor_at = cursor
            .filter(|c| !pinned && window.contains(&(c.row.max(0) as usize)))
            .map(|c| {
                let row = bottom_start + c.row.max(0) as usize - window.start;
                (row, (c.col.max(0) as usize).min(width.saturating_sub(1)))
            });

        let mut window_lines: Vec<String> = body_lines
            .iter()
            .skip(start)
            .take(rows.body)
            .map(|line| strip_cursor_marker(line))
            .collect();
        // Live with a short transcript: bottom-anchor the tail just above the
        // bottom chrome, padding the blank rows above it (M1 top-alignment
        // revisited for this case only). Pinned keeps the top-held window, and
        // a transcript that fills the body is already tail-aligned.
        if !pinned && window_lines.len() < rows.body {
            let mut padded = vec![String::new(); rows.body - window_lines.len()];
            padded.append(&mut window_lines);
            window_lines = padded;
        }
        // The scrollbar shows while pinned only: the column is blank while live.
        let glyphs = if pinned {
            scrollbar_glyphs(rows.body as i64, total as i64, start as i64)
        } else {
            Vec::new()
        };
        let mut out: Vec<String> = Vec::with_capacity(height);
        for (row, text) in top_lines.iter().take(rows.top).enumerate() {
            out.push(self.region_row(text, width, row));
        }
        // Mission Control transmissions (M8e4): the newest toast floats
        // top-right over the body, starting on its first row. The lines are
        // drawn by the app; the frame only splices them over the body rows.
        let toast = self
            .toast_provider
            .as_ref()
            .map(|provide| provide(fullscreen::body_width(width)))
            .unwrap_or_default();
        for row in 0..rows.body {
            let text = window_lines.get(row).map_or("", String::as_str);
            let marked = if query.is_empty() || hoocode_tui_images::is_image_line(text) {
                text.to_string()
            } else {
                Self::highlight_scroll_matches(text, &query)
            };
            let glyph = glyphs.get(row).copied();
            let mut line = self.body_row(&marked, width, row, glyph);
            if let Some(floated) = toast.get(row) {
                line = Self::float_toast_line(&line, floated, width, glyph);
            }
            out.push(line);
        }
        if rows.bar == 1 {
            let status = ScrollStatus {
                top: start as i64 + 1,
                bottom: (start + rows.body).min(total) as i64,
                total: total as i64,
                view_height: rows.body as i64,
                at_top: start == 0,
                at_bottom: start >= total.saturating_sub(rows.body),
                width: width as i64,
                search: self.scroll_search_status(),
                picker: false,
                new_lines: fullscreen::new_lines_since(self.fullscreen_seen_total as usize, total)
                    as i64,
            };
            let text = (self.scroll_status_formatter)(&status);
            out.push(self.emit_line(&text, width as i64));
        }
        for (row, text) in bottom_lines
            .iter()
            .skip(window.start)
            .take(window.len())
            .enumerate()
        {
            out.push(self.region_row(text, width, row));
        }
        // Screens under four rows show the bottom alone, which may leave rows blank.
        out.resize(height, String::new());
        self.fullscreen_body_window = Some((rows.top as i64, window_lines));
        Screen {
            rows: out,
            cursor: cursor_at,
        }
    }

    /// One row of a full-screen region, cut to `width` columns. An image line
    /// whose picture would reach above the region (more rows up than `row`)
    /// is named in its place, so no picture overlays the chrome or the prompt.
    fn region_row(&mut self, text: &str, width: usize, row: usize) -> String {
        if hoocode_tui_images::is_image_line(text) && image_row_offset(text) > row as i64 {
            return self.emit_line(IMAGE_PLACEHOLDER, width as i64);
        }
        self.emit_line(text, width as i64)
    }

    /// One body row with the toast's line spliced over its right end: the
    /// body keeps its left cells, the toast its own width, and the reserved
    /// scrollbar column keeps its glyph. `width` is the full row including
    /// that column. The row ends reset so no style leaks into the next row.
    fn float_toast_line(line: &str, toast: &str, width: usize, glyph: Option<char>) -> String {
        let toast_width = visible_width(toast);
        if toast_width == 0 || toast_width + 1 > width {
            return line.to_string();
        }
        let keep = width - 1 - toast_width;
        let kept = slice_by_column(line, 0, keep, true);
        let pad = keep.saturating_sub(visible_width(&kept));
        let suffix: String = glyph.map_or_else(|| " ".to_string(), |g| g.to_string());
        format!("{kept}{}{toast}{SEGMENT_RESET}{suffix}", " ".repeat(pad))
    }

    /// One body row. With a scrollbar glyph, the row is padded to the body
    /// width so the glyph lands in the reserved last column.
    fn body_row(&mut self, text: &str, width: usize, row: usize, glyph: Option<char>) -> String {
        let body_width = fullscreen::body_width(width);
        let glyph = match glyph {
            Some(glyph) if !hoocode_tui_images::is_image_line(text) => glyph,
            _ => return self.region_row(text, body_width, row),
        };
        let shown = self.region_row(text, body_width, row);
        let content = shown.strip_suffix(SEGMENT_RESET).unwrap_or(&shown);
        let pad = body_width.saturating_sub(visible_width(content));
        format!("{content}{}{SEGMENT_RESET}{glyph}", " ".repeat(pad))
    }

    /// Take the pinned window's own copies of the images off the screen.
    fn clear_scroll_images(&mut self) -> String {
        if self.scroll_placed_images.is_empty() {
            return String::new();
        }
        let ids: Vec<u32> = self.scroll_placed_images.drain().collect();
        self.delete_kitty_images(ids)
    }

    /// On the way off the alternate screen: free the copies and their ids.
    fn release_scroll_images(&mut self) {
        let buffer = self.clear_scroll_images();
        if !buffer.is_empty() {
            self.write_frame(&buffer);
        }
        self.scroll_image_ids.clear();
    }

    /// One transcript row for the pinned window (`emitScrollLine`).
    fn emit_scroll_line(&mut self, line: &str, row: i64, query: &str) -> String {
        if hoocode_tui_images::is_image_line(line) {
            return self.emit_scroll_image(line, row);
        }
        let mut text = line.replacen(CURSOR_MARKER, "", 1);
        if !query.is_empty() {
            text = Self::highlight_scroll_matches(&text, query);
        }
        format!("{}{SEGMENT_RESET}", normalize_terminal_output(&text))
    }

    /// An image line in the pinned window: drawn if all of it fits, named if
    /// not; kitty images are drawn as a separately deletable copy.
    fn emit_scroll_image(&mut self, line: &str, row: i64) -> String {
        if image_row_offset(line) > row {
            return IMAGE_PLACEHOLDER.to_string();
        }
        if !line.contains(KITTY_SEQUENCE_PREFIX) {
            return line.to_string();
        }
        let Some(&live_id) = extract_kitty_image_ids(line).first() else {
            return IMAGE_PLACEHOLDER.to_string();
        };
        let pinned_id = *self
            .scroll_image_ids
            .entry(live_id)
            .or_insert_with(allocate_image_id);
        let Some(retagged) = retag_kitty_image_id(line, pinned_id) else {
            return IMAGE_PLACEHOLDER.to_string();
        };
        self.scroll_placed_images.insert(pinned_id);
        self.saw_image_line = true;
        retagged
    }
}

/// The cursor marker removed from a line. Only the bottom carries the cursor,
/// so every other line is stripped of it and drawn without it.
fn strip_cursor_marker(line: &str) -> String {
    line.replacen(CURSOR_MARKER, "", 1)
}

/// Renders each child in order and joins the lines (full-screen regions).
fn render_children(children: &[ComponentHandle], width: u16) -> Vec<String> {
    let mut lines = Vec::new();
    for child in children {
        lines.extend(child.borrow_mut().render(width));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scrollbar::SCROLLBAR_TRACK;
    use std::cell::RefCell as StdRefCell;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[derive(Clone, Default)]
    struct SharedLog(Arc<Mutex<Vec<String>>>);

    impl SharedLog {
        fn push(&self, s: &str) {
            self.0.lock().unwrap().push(s.to_string());
        }
        fn joined(&self) -> String {
            self.0.lock().unwrap().join("")
        }
        fn clear(&self) {
            self.0.lock().unwrap().clear();
        }
    }

    struct MockTerminal {
        writes: SharedLog,
        cols: Arc<Mutex<u16>>,
        rows: Arc<Mutex<u16>>,
        hide_cursor_calls: Arc<Mutex<u32>>,
        show_cursor_calls: Arc<Mutex<u32>>,
        /// Stands in for the output thread still writing the last frame.
        frame_busy: Arc<Mutex<bool>>,
        /// Every `set_alternate_screen` call, in order.
        alternate_screen: Arc<Mutex<Vec<bool>>>,
    }

    impl MockTerminal {
        fn new(cols: u16, rows: u16) -> Self {
            Self {
                writes: SharedLog::default(),
                cols: Arc::new(Mutex::new(cols)),
                rows: Arc::new(Mutex::new(rows)),
                hide_cursor_calls: Arc::new(Mutex::new(0)),
                show_cursor_calls: Arc::new(Mutex::new(0)),
                frame_busy: Arc::new(Mutex::new(false)),
                alternate_screen: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl Terminal for MockTerminal {
        fn start(
            &mut self,
            _on_input: Box<dyn FnMut(&str) + Send>,
            _on_resize: Box<dyn FnMut() + Send>,
        ) {
        }
        fn stop(&mut self) {}
        fn drain_input(&mut self, _max: Duration, _idle: Duration) {}
        fn write(&mut self, data: &str) {
            self.writes.push(data);
        }
        fn columns(&self) -> u16 {
            *self.cols.lock().unwrap()
        }
        fn rows(&self) -> u16 {
            *self.rows.lock().unwrap()
        }
        fn kitty_protocol_active(&self) -> bool {
            false
        }
        fn move_by(&mut self, _lines: i32) {}
        fn hide_cursor(&mut self) {
            *self.hide_cursor_calls.lock().unwrap() += 1;
        }
        fn show_cursor(&mut self) {
            *self.show_cursor_calls.lock().unwrap() += 1;
        }
        fn clear_line(&mut self) {}
        fn clear_from_cursor(&mut self) {}
        fn clear_screen(&mut self) {}
        fn set_title(&mut self, _title: &str) {}
        fn set_progress(&mut self, _active: bool) {}
        fn frame_pending(&self) -> bool {
            *self.frame_busy.lock().unwrap()
        }
        fn set_alternate_screen(&mut self, active: bool) {
            self.alternate_screen.lock().unwrap().push(active);
        }
    }

    struct TestComponent {
        lines: Vec<String>,
        focused: bool,
        focusable: bool,
    }

    impl TestComponent {
        fn new() -> Rc<StdRefCell<Self>> {
            Rc::new(StdRefCell::new(Self {
                lines: Vec::new(),
                focused: false,
                focusable: false,
            }))
        }
    }

    impl Component for TestComponent {
        fn render(&mut self, _width: u16) -> Vec<String> {
            self.lines.clone()
        }
        fn is_focusable(&self) -> bool {
            self.focusable
        }
        fn set_focused(&mut self, focused: bool) {
            self.focused = focused;
        }
    }

    struct MockHandles {
        writes: SharedLog,
        cols: Arc<Mutex<u16>>,
        rows: Arc<Mutex<u16>>,
        frame_busy: Arc<Mutex<bool>>,
        alternate_screen: Arc<Mutex<Vec<bool>>>,
    }

    impl MockHandles {
        fn set_size(&self, cols: u16, rows: u16) {
            *self.cols.lock().unwrap() = cols;
            *self.rows.lock().unwrap() = rows;
        }
    }

    fn new_tui(cols: u16, rows: u16) -> (Tui, MockHandles) {
        let terminal = MockTerminal::new(cols, rows);
        let handles = MockHandles {
            writes: terminal.writes.clone(),
            cols: terminal.cols.clone(),
            rows: terminal.rows.clone(),
            frame_busy: terminal.frame_busy.clone(),
            alternate_screen: terminal.alternate_screen.clone(),
        };
        (Tui::new(Box::new(terminal), Some(false)), handles)
    }

    #[test]
    fn keys_accepted_in_one_pass_paint_one_frame() {
        let (mut tui, _handles) = new_tui(40, 10);
        let component = TestComponent::new();
        component.borrow_mut().lines = vec!["a".to_string()];
        component.borrow_mut().focusable = true;
        tui.add_child(component.clone());
        tui.set_focus(Some(component.clone()));
        tui.request_render(false);

        let frames = Rc::new(std::cell::Cell::new(0u32));
        let counter = frames.clone();
        tui.set_frame_observer(Some(Box::new(move |_| counter.set(counter.get() + 1))));
        for key in ["x", "y", "z"] {
            tui.accept_event(TuiEvent::input(key));
        }
        assert_eq!(frames.get(), 0, "keys alone must not paint");
        tui.flush_scheduled_render();
        assert_eq!(frames.get(), 1, "three keys, one frame");
        tui.flush_scheduled_render();
        assert_eq!(frames.get(), 1, "nothing scheduled, nothing painted");
    }

    #[test]
    fn frame_waits_while_terminal_is_still_writing_the_last_one() {
        let (mut tui, handles) = new_tui(40, 10);
        let component = TestComponent::new();
        component.borrow_mut().lines = vec!["a".to_string()];
        tui.add_child(component.clone());
        tui.request_render(false);
        assert!(handles.writes.joined().contains('a'));

        *handles.frame_busy.lock().unwrap() = true;
        handles.writes.clear();
        component.borrow_mut().lines = vec!["b".to_string()];
        tui.request_render(false);
        assert!(
            handles.writes.joined().is_empty(),
            "no frame while one is pending"
        );

        *handles.frame_busy.lock().unwrap() = false;
        tui.flush_scheduled_render();
        assert!(
            handles.writes.joined().contains('b'),
            "the latest state is painted"
        );
    }

    #[test]
    fn first_render_writes_all_lines_without_clearing() {
        let (mut tui, handles) = new_tui(40, 10);
        let component = TestComponent::new();
        component.borrow_mut().lines = vec!["a".to_string(), "b".to_string()];
        tui.add_child(component);

        tui.request_render(false);

        let out = handles.writes.joined();
        assert!(out.contains("a"));
        assert!(out.contains("b"));
        assert!(
            !out.contains("\x1b[2J"),
            "first render should not clear the screen"
        );
        assert!(out.starts_with("\x1b[?2026h"));
        assert!(out.ends_with("\x1b[?2026l"));
    }

    #[test]
    fn no_change_produces_no_writes() {
        let (mut tui, handles) = new_tui(40, 10);
        let component = TestComponent::new();
        component.borrow_mut().lines = vec!["a".to_string()];
        tui.add_child(component);

        tui.request_render(false);
        handles.writes.clear();

        tui.request_render(false);
        // Only the cursor parking the pin writes on every frame.
        assert_eq!(handles.writes.joined(), "\r\x1b[?25l");
    }

    #[test]
    fn width_change_forces_full_clear() {
        let (mut tui, handles) = new_tui(40, 10);
        let component = TestComponent::new();
        component.borrow_mut().lines = vec!["a".to_string()];
        tui.add_child(component);
        tui.request_render(false);
        handles.writes.clear();

        handles.set_size(80, 10);
        tui.request_render(false);
        assert!(
            handles.writes.joined().contains("\x1b[2J"),
            "width change should force full clear"
        );
    }

    #[test]
    fn appended_lines_are_written_without_full_clear() {
        let (mut tui, handles) = new_tui(40, 10);
        let component = TestComponent::new();
        component.borrow_mut().lines = vec!["a".to_string()];
        tui.add_child(component.clone());
        tui.request_render(false);
        handles.writes.clear();

        component.borrow_mut().lines = vec!["a".to_string(), "b".to_string()];
        tui.request_render(false);

        let out = handles.writes.joined();
        assert!(out.contains('b'));
        assert!(!out.contains("\x1b[2J"));
    }

    #[test]
    fn shrinking_content_clears_deleted_lines() {
        let (mut tui, handles) = new_tui(40, 10);
        let component = TestComponent::new();
        component.borrow_mut().lines = vec!["a".to_string(), "b".to_string()];
        tui.add_child(component.clone());
        tui.request_render(false);
        handles.writes.clear();

        component.borrow_mut().lines = vec!["a".to_string()];
        tui.request_render(false);

        let out = handles.writes.joined();
        assert!(out.contains("\x1b[2K"), "deleted line should be cleared");
        assert!(
            !out.contains("\x1b[2J"),
            "shrink-by-default should not force a full screen clear"
        );
    }

    #[test]
    fn deletes_changed_kitty_image_before_drawing_new_placement() {
        use hoocode_tui_images::{delete_kitty_image, encode_kitty, KittyEncodeOptions};

        let (mut tui, handles) = new_tui(40, 10);
        let component = TestComponent::new();
        let old_image = encode_kitty(
            "AAAA",
            &KittyEncodeOptions {
                columns: Some(2),
                rows: Some(2),
                image_id: Some(42),
                move_cursor: Some(false),
            },
        );
        component.borrow_mut().lines = vec!["top".to_string(), old_image];
        tui.add_child(component.clone());
        tui.request_render(false);
        handles.writes.clear();

        let new_image = encode_kitty(
            "BBBB",
            &KittyEncodeOptions {
                columns: Some(2),
                rows: Some(1),
                image_id: Some(42),
                move_cursor: Some(false),
            },
        );
        component.borrow_mut().lines = vec![new_image.clone(), String::new()];
        tui.request_render(false);

        let out = handles.writes.joined();
        let delete_seq = delete_kitty_image(42);
        let delete_idx = out.find(&delete_seq);
        let draw_idx = out.find(&new_image);
        assert!(delete_idx.is_some(), "changed old image should be deleted");
        assert!(draw_idx.is_some(), "new image should be drawn");
        assert!(
            delete_idx.unwrap() < draw_idx.unwrap(),
            "old image must be deleted before the new placement is drawn"
        );
    }

    #[test]
    fn cursor_marker_is_stripped_and_positions_hardware_cursor() {
        let (mut tui, handles) = new_tui(40, 10);
        tui.set_show_hardware_cursor(true);
        let component = TestComponent::new();
        component.borrow_mut().lines = vec![format!("hello{CURSOR_MARKER}")];
        tui.add_child(component);

        tui.request_render(false);

        let out = handles.writes.joined();
        assert!(
            !out.contains(CURSOR_MARKER),
            "marker should be stripped from output"
        );
        assert!(out.contains("hello"));
        // Column move to col 6 (1-indexed) after "hello".
        assert!(out.contains("\x1b[6G"));
    }

    #[test]
    fn forced_redraw_clears_once_even_when_content_is_unchanged() {
        let (mut tui, handles) = new_tui(40, 10);
        let component = TestComponent::new();
        component.borrow_mut().lines = vec!["a".to_string()];
        tui.add_child(component);
        tui.request_render(false);
        handles.writes.clear();

        tui.request_render(true);
        assert!(
            handles.writes.joined().contains("\x1b[2J"),
            "forced redraw should clear the screen"
        );
        handles.writes.clear();

        tui.request_render(false);
        assert!(
            !handles.writes.joined().contains("\x1b[2J"),
            "the force is spent after one paint"
        );
    }

    #[test]
    fn full_redraw_count_increments_only_on_full_renders() {
        let (mut tui, _writes) = new_tui(40, 10);
        let component = TestComponent::new();
        component.borrow_mut().lines = vec!["a".to_string()];
        tui.add_child(component.clone());
        tui.request_render(false);
        assert_eq!(tui.full_redraws(), 1);

        component.borrow_mut().lines = vec!["a".to_string(), "b".to_string()];
        tui.request_render(false);
        assert_eq!(
            tui.full_redraws(),
            1,
            "appending lines should not trigger a full redraw"
        );
    }

    #[test]
    fn over_width_line_on_diff_path_is_truncated_not_panicked() {
        let (mut tui, handles) = new_tui(40, 10);
        let component = TestComponent::new();
        component.borrow_mut().lines = vec!["a".to_string(), "b".to_string()];
        tui.add_child(component.clone());
        tui.request_render(false);
        handles.writes.clear();

        component.borrow_mut().lines = vec!["a".to_string(), "x".repeat(50)];
        tui.request_render(false);

        let out = handles.writes.joined();
        assert!(out.contains(&"x".repeat(40)), "line is cut at the width");
        assert!(!out.contains(&"x".repeat(41)), "nothing past the width");
    }

    #[test]
    fn over_width_line_on_full_path_is_truncated() {
        let (mut tui, handles) = new_tui(40, 10);
        let component = TestComponent::new();
        component.borrow_mut().lines = vec!["x".repeat(50)];
        tui.add_child(component);
        tui.request_render(false);

        let out = handles.writes.joined();
        assert!(out.contains(&"x".repeat(40)), "line is cut at the width");
        assert!(!out.contains(&"x".repeat(41)), "nothing past the width");
    }

    // ── Full-screen mode (M1b) ──────────────────────────────────────────

    /// What `emit_line` appends after every row's text.
    const ROW_END: &str = "\x1b[0m\x1b]8;;\x07";

    /// The text written for screen row `row`, if the frame wrote that row.
    fn written_row(writes: &str, row: usize) -> Option<String> {
        let marker = format!("\x1b[{};1H\x1b[2K", row + 1);
        let start = writes.find(&marker)? + marker.len();
        let rest = writes.get(start..)?;
        let end = rest.find(ROW_END)?;
        rest.get(..end).map(str::to_string)
    }

    fn assert_screen(writes: &str, expected: &[String]) {
        for (row, text) in expected.iter().enumerate() {
            assert_eq!(
                written_row(writes, row).as_deref(),
                Some(text.as_str()),
                "screen row {row}"
            );
        }
    }

    fn text_component(lines: Vec<String>) -> ComponentHandle {
        let component = TestComponent::new();
        component.borrow_mut().lines = lines;
        component
    }

    /// The bottom: seven rows, the cursor on the third.
    fn bottom_lines() -> Vec<String> {
        vec![
            "p0".to_string(),
            "p1".to_string(),
            format!("p2{CURSOR_MARKER}"),
            "p3".to_string(),
            "p4".to_string(),
            "p5".to_string(),
            "p6".to_string(),
        ]
    }

    /// The bottom rows as they show on screen: the cursor marker is stripped.
    fn bottom_rows() -> Vec<String> {
        ["p0", "p1", "p2", "p3", "p4", "p5", "p6"]
            .iter()
            .map(|row| row.to_string())
            .collect()
    }

    struct FullscreenFixture {
        tui: Tui,
        handles: MockHandles,
        body: Rc<StdRefCell<TestComponent>>,
        bottom: Rc<StdRefCell<TestComponent>>,
    }

    /// The tree the app will build (M1c), with fixed content: a HUD, a gap, an
    /// optional attach row, a 40-row body and a 7-row bottom. Full-screen is on.
    fn fullscreen_fixture(cols: u16, rows: u16, attach: bool) -> FullscreenFixture {
        let (mut tui, handles) = new_tui(cols, rows);
        tui.set_fullscreen(true);
        tui.add_child(text_component(vec!["HUD".to_string()]));
        tui.add_child(text_component(vec![String::new()]));
        if attach {
            tui.add_child(text_component(vec!["ATTACH".to_string()]));
        }
        let body = TestComponent::new();
        body.borrow_mut().lines = (0..40).map(|i| format!("b{i:02}")).collect();
        tui.add_child(body.clone());
        let body_handle: ComponentHandle = body.clone();
        tui.set_fullscreen_body(Some(body_handle));
        let bottom = TestComponent::new();
        bottom.borrow_mut().lines = bottom_lines();
        tui.add_child(bottom.clone());
        FullscreenFixture {
            tui,
            handles,
            body,
            bottom,
        }
    }

    #[test]
    fn fullscreen_start_enters_alternate_screen_stop_leaves_it() {
        let mut f = fullscreen_fixture(100, 30, false);
        let _events = f.tui.start();
        assert_eq!(*f.handles.alternate_screen.lock().unwrap(), vec![true]);
        f.tui.stop();
        assert_eq!(
            *f.handles.alternate_screen.lock().unwrap(),
            vec![true, false]
        );
    }

    #[test]
    fn fullscreen_stop_writes_nothing_below_content() {
        let mut f = fullscreen_fixture(100, 30, false);
        let _events = f.tui.start();
        f.handles.writes.clear();
        f.tui.stop();
        assert_eq!(
            f.handles.writes.joined(),
            "",
            "no newline or cursor move below the frame"
        );
    }

    #[test]
    fn fullscreen_off_never_touches_the_alternate_screen() {
        let (mut tui, handles) = new_tui(100, 30);
        assert!(!tui.fullscreen(), "off by default");
        let _events = tui.start();
        tui.stop();
        assert!(handles.alternate_screen.lock().unwrap().is_empty());
    }

    #[test]
    fn fullscreen_frame_100x30_composes_regions() {
        let mut f = fullscreen_fixture(100, 30, false);
        let _events = f.tui.start();
        let mut expected = vec!["HUD".to_string(), String::new()];
        // Live: the last 21 body rows. Then the 7 bottom rows.
        expected.extend((19..40).map(|i| format!("b{i:02}")));
        expected.extend(bottom_rows());
        let writes = f.handles.writes.joined();
        assert_screen(&writes, &expected);
        // The cursor is on bottom row 2, column 2, which is screen row 25 (1-based 26).
        assert!(writes.contains("\x1b[26;3H"), "cursor in the prompt");
    }

    #[test]
    fn fullscreen_frame_80x24_with_attach_row() {
        let mut f = fullscreen_fixture(80, 24, true);
        let _events = f.tui.start();
        let mut expected = vec!["HUD".to_string(), String::new(), "ATTACH".to_string()];
        // 24 - 3 top - 7 bottom = 14 body rows.
        expected.extend((26..40).map(|i| format!("b{i:02}")));
        expected.extend(bottom_rows());
        let writes = f.handles.writes.joined();
        assert_screen(&writes, &expected);
        assert!(writes.contains("\x1b[20;3H"), "cursor in the prompt");
    }

    #[test]
    fn fullscreen_repaints_only_changed_rows() {
        let mut f = fullscreen_fixture(100, 30, false);
        let _events = f.tui.start();
        f.handles.writes.clear();
        f.bottom.borrow_mut().lines[4] = "p4 changed".to_string();
        f.tui.request_render(false);
        let writes = f.handles.writes.joined();
        assert_eq!(writes.matches("\x1b[2K").count(), 1, "one row rewritten");
        // Bottom row 4 is screen row 27 (23 + 4).
        assert_eq!(written_row(&writes, 27).as_deref(), Some("p4 changed"));
        assert!(!writes.contains("\x1b[2J"), "a diff frame does not clear");
    }

    #[test]
    fn fullscreen_resize_clears_then_repaints_all_rows() {
        let mut f = fullscreen_fixture(100, 30, false);
        let _events = f.tui.start();
        f.handles.writes.clear();
        f.handles.set_size(100, 28);
        f.tui.request_render(false);
        let writes = f.handles.writes.joined();
        assert!(writes.contains("\x1b[2J\x1b[H"), "the screen is cleared");
        assert_eq!(
            writes.matches("\x1b[2K").count(),
            28,
            "every row of the new size"
        );
        // 28 rows: 2 top, 7 bottom, so 19 body rows from row 21.
        assert_eq!(written_row(&writes, 2).as_deref(), Some("b21"));
    }

    #[test]
    fn fullscreen_restart_repaints_everything() {
        let mut f = fullscreen_fixture(100, 30, false);
        let _events = f.tui.start();
        f.tui.stop();
        f.handles.writes.clear();
        let _events = f.tui.start();
        assert_eq!(
            *f.handles.alternate_screen.lock().unwrap(),
            vec![true, false, true],
            "suspend and the editor round trip re-enter the alternate screen"
        );
        assert_eq!(
            f.handles.writes.joined().matches("\x1b[2K").count(),
            30,
            "every row is painted again"
        );
    }

    #[test]
    fn fullscreen_session_writes_no_scrollback_sequences() {
        let mut f = fullscreen_fixture(100, 30, false);
        let _events = f.tui.start();
        f.bottom.borrow_mut().lines[0] = "changed".to_string();
        f.tui.request_render(false);
        f.handles.set_size(80, 24);
        f.tui.request_render(false);
        f.tui.stop();
        let writes = f.handles.writes.joined();
        assert!(
            !writes.contains("\x1b[3J"),
            "the scrollback is never cleared"
        );
        assert!(
            !writes.contains('\n') && !writes.contains('\r'),
            "no line feed or carriage return: nothing scrolls"
        );
    }

    #[test]
    fn fullscreen_pinned_view_holds_its_rows_and_shows_status() {
        let mut f = fullscreen_fixture(100, 30, false);
        let _events = f.tui.start();
        f.handles.writes.clear();
        assert!(f.tui.scroll_by_lines(-5), "moves up from the tail");
        assert!(f.tui.scroll_pinned());
        let writes = f.handles.writes.joined();
        // 40 body rows, pinned at row 14, 20 rows shown (one row is the status row).
        // Padded to the body width, so the scrollbar glyph lands in the last column.
        assert_eq!(
            written_row(&writes, 2)
                .map(|row| row.trim_end().to_string())
                .as_deref(),
            Some("b14")
        );
        assert_eq!(
            written_row(&writes, 21)
                .map(|row| row.trim_end().to_string())
                .as_deref(),
            Some("b33")
        );
        assert!(written_row(&writes, 22).is_some(), "the status row");
        assert!(!writes.contains("\x1b[26;3H"), "no cursor while pinned");
        assert_eq!(
            *f.handles.alternate_screen.lock().unwrap(),
            vec![true],
            "pinning does not toggle the alternate screen"
        );

        f.handles.writes.clear();
        assert!(f.tui.scroll_to_live());
        let writes = f.handles.writes.joined();
        assert_eq!(written_row(&writes, 2).as_deref(), Some("b19"));
        assert_eq!(*f.handles.alternate_screen.lock().unwrap(), vec![true]);
    }

    #[test]
    fn fullscreen_set_while_running_is_ignored() {
        let mut f = fullscreen_fixture(100, 30, false);
        let _events = f.tui.start();
        f.tui.set_fullscreen(false);
        assert!(
            f.tui.fullscreen(),
            "the mode does not change under a live frame"
        );
        assert_eq!(
            *f.handles.alternate_screen.lock().unwrap(),
            vec![true],
            "no screen switch"
        );
        f.tui.stop();
        f.tui.set_fullscreen(false);
        assert!(
            !f.tui.fullscreen(),
            "between stop and start it takes effect"
        );
    }

    #[test]
    fn fullscreen_short_screen_keeps_the_prompt_row() {
        // A top taller than the screen allows: the bottom keeps its row.
        let (mut tui, handles) = new_tui(40, 6);
        tui.set_fullscreen(true);
        for i in 0..5 {
            tui.add_child(text_component(vec![format!("top{i}")]));
        }
        let body = text_component(vec!["body".to_string()]);
        tui.add_child(body.clone());
        tui.set_fullscreen_body(Some(body));
        let bottom = TestComponent::new();
        bottom.borrow_mut().lines = bottom_lines();
        tui.add_child(bottom);
        let _events = tui.start();
        let writes = handles.writes.joined();
        assert_eq!(
            written_row(&writes, 5).as_deref(),
            Some("p2"),
            "the prompt row is painted"
        );
        assert!(writes.contains("\x1b[6;3H"), "cursor on the prompt row");
    }

    #[test]
    fn fullscreen_cursor_marker_is_stripped_from_body_rows() {
        let mut f = fullscreen_fixture(100, 30, false);
        f.body.borrow_mut().lines[25] = format!("b25{CURSOR_MARKER}");
        let _events = f.tui.start();
        let writes = f.handles.writes.joined();
        assert!(
            !writes.contains(CURSOR_MARKER),
            "no marker reaches the screen"
        );
        assert!(writes.contains("b25"), "the row keeps its text");
    }

    #[test]
    fn fullscreen_image_is_named_when_its_picture_reaches_above_the_body() {
        let mut f = fullscreen_fixture(100, 30, false);
        // Live: body row 0 is transcript line 19. A picture two rows up would
        // overlay the gap, so it is named; one row up stays inside the body.
        f.body.borrow_mut().lines[19] = "\x1b[2A\x1b_Ga=T,i=9;AAAA\x1b\\".to_string();
        f.body.borrow_mut().lines[20] = "\x1b[1A\x1b_Ga=T,i=10;AAAA\x1b\\".to_string();
        let _events = f.tui.start();
        let writes = f.handles.writes.joined();
        let row_0 = written_row(&writes, 2).expect("body row 0");
        assert!(row_0.contains("[image]"), "named: {row_0}");
        assert!(!row_0.contains("i=9"), "the picture is not drawn: {row_0}");
        let row_1 = written_row(&writes, 3).expect("body row 1");
        assert!(row_1.contains("i=10"), "drawn inside the body: {row_1}");
    }

    #[test]
    fn fullscreen_stop_frees_the_kitty_images_it_placed() {
        let mut f = fullscreen_fixture(100, 30, false);
        f.body.borrow_mut().lines[22] = "\x1b_Ga=T,i=7;AAAA\x1b\\".to_string();
        let _events = f.tui.start();
        f.handles.writes.clear();
        f.tui.stop();
        let writes = f.handles.writes.joined();
        let delete = hoocode_tui_images::delete_kitty_image(7);
        assert!(
            writes.contains(&delete),
            "image 7 is deleted before the screen is left"
        );
        assert_eq!(
            *f.handles.alternate_screen.lock().unwrap(),
            vec![true, false],
            "the alternate screen is left"
        );
    }

    #[test]
    fn fullscreen_scroll_before_the_first_frame_uses_the_body_height() {
        let mut f = fullscreen_fixture(100, 30, false);
        // No start and no frame yet: the body is measured, 21 rows live over 40 lines.
        assert!(f.tui.scroll_by_lines(-5), "moves up from the live tail");
        assert_eq!(
            f.tui
                .get_scroll_position()
                .map(|(top, total, _)| (top, total)),
            Some((14, 40)),
            "19 is the live top, less 5"
        );
    }

    #[test]
    fn fullscreen_live_view_follows_output() {
        let mut f = fullscreen_fixture(100, 30, false);
        let _events = f.tui.start();
        f.handles.writes.clear();
        f.body
            .borrow_mut()
            .lines
            .extend(["n0", "n1", "n2"].map(String::from));
        f.tui.request_render(false);
        let writes = f.handles.writes.joined();
        assert_eq!(
            written_row(&writes, 22).as_deref(),
            Some("n2"),
            "the tail follows"
        );
        assert_eq!(
            written_row(&writes, 2).as_deref(),
            Some("b22"),
            "the window moved down by three"
        );
    }

    #[test]
    fn fullscreen_pinned_view_holds_while_output_grows_and_counts_new_lines() {
        let mut f = fullscreen_fixture(100, 30, false);
        f.tui
            .set_scroll_status_formatter(Box::new(|status| format!("NEW{}", status.new_lines)));
        let _events = f.tui.start();
        assert!(f.tui.scroll_by_lines(-5));
        f.handles.writes.clear();
        f.body
            .borrow_mut()
            .lines
            .extend((0..8).map(|i| format!("n{i}")));
        // A forced frame writes every row, so the held rows are visible.
        f.tui.request_render(true);
        let writes = f.handles.writes.joined();
        assert_eq!(
            written_row(&writes, 2)
                .map(|row| row.trim_end().to_string())
                .as_deref(),
            Some("b14"),
            "the pinned window holds its top row"
        );
        assert_eq!(
            written_row(&writes, 22).as_deref(),
            Some("NEW8"),
            "the bar counts the new lines"
        );

        // Moving the view starts the count again from here: the move itself paints it.
        f.handles.writes.clear();
        assert!(f.tui.scroll_by_lines(-1));
        assert_eq!(
            written_row(&f.handles.writes.joined(), 22).as_deref(),
            Some("NEW0")
        );

        // A shorter transcript (a clear) starts the count over: a frame that sees it resets it.
        f.body.borrow_mut().lines.truncate(10);
        f.handles.writes.clear();
        f.tui.request_render(true);
        assert_eq!(
            written_row(&f.handles.writes.joined(), 22).as_deref(),
            Some("NEW0")
        );
        f.body
            .borrow_mut()
            .lines
            .extend((0..3).map(|i| format!("m{i}")));
        f.handles.writes.clear();
        f.tui.request_render(false);
        assert_eq!(
            written_row(&f.handles.writes.joined(), 22).as_deref(),
            Some("NEW3")
        );
    }

    #[test]
    fn fullscreen_new_count_ignores_a_rebuild_that_is_not_new_output() {
        let mut f = fullscreen_fixture(100, 30, false);
        f.tui
            .set_scroll_status_formatter(Box::new(|status| format!("NEW{}", status.new_lines)));
        let _events = f.tui.start();
        assert!(f.tui.scroll_by_lines(-5));
        f.body
            .borrow_mut()
            .lines
            .extend((0..8).map(|i| format!("n{i}")));
        f.tui.request_render(true);

        // A toggle rebuilds the rows: the transcript grows, but nothing new arrived.
        f.body.borrow_mut().lines = (0..50).map(|i| format!("r{i}")).collect();
        f.tui.reset_new_lines_count();
        f.handles.writes.clear();
        f.tui.request_render(true);
        assert_eq!(
            written_row(&f.handles.writes.joined(), 22).as_deref(),
            Some("NEW0"),
            "a rebuild is not output"
        );

        f.body
            .borrow_mut()
            .lines
            .extend((0..2).map(|i| format!("m{i}")));
        f.handles.writes.clear();
        f.tui.request_render(false);
        assert_eq!(
            written_row(&f.handles.writes.joined(), 22).as_deref(),
            Some("NEW2"),
            "only growth after the rebuild counts"
        );
    }

    #[test]
    fn fullscreen_pinned_scrollbar_draws_a_glyph_per_body_row() {
        let mut f = fullscreen_fixture(100, 30, false);
        let _events = f.tui.start();
        f.handles.writes.clear();
        f.tui.request_render(false);
        let live = f.handles.writes.joined();
        assert!(
            !live.contains(SCROLLBAR_THUMB) && !live.contains(SCROLLBAR_TRACK),
            "blank while live"
        );
        assert!(f.tui.scroll_by_lines(-5));
        let writes = f.handles.writes.joined();
        // 40 lines over 20 rows: a thumb of 10 and a track of 10, each after the row's text.
        let glyphs = writes.matches(SEGMENT_RESET).count();
        assert!(glyphs >= 20, "one reset per body row: {glyphs}");
        let thumbs = writes
            .matches(&format!("{SEGMENT_RESET}{SCROLLBAR_THUMB}"))
            .count();
        let tracks = writes
            .matches(&format!("{SEGMENT_RESET}{SCROLLBAR_TRACK}"))
            .count();
        assert_eq!((thumbs, tracks), (10, 10));
        let row = written_row(&writes, 2).expect("body row 0");
        assert!(row.starts_with("b14"), "{row}");
    }

    #[test]
    fn fullscreen_pinned_picker_takes_the_bottom_rows() {
        let mut f = fullscreen_fixture(100, 30, false);
        let picker = TestComponent::new();
        picker.borrow_mut().lines = vec!["PICK A".to_string(), "PICK B".to_string()];
        picker.borrow_mut().focusable = true;
        f.tui.add_child(picker.clone());
        let prompt: ComponentHandle = f.bottom.clone();
        f.tui.scroll_prompt = Some(prompt);
        let _events = f.tui.start();
        f.tui.set_focus(Some(picker.clone()));
        f.handles.writes.clear();
        assert!(f.tui.scroll_by_lines(-5));
        // The picker's rows were on screen already (it is a bottom child too), so a forced frame shows them.
        f.handles.writes.clear();
        f.tui.request_render(true);
        let writes = f.handles.writes.joined();
        // Pinned with a picker: the body keeps 25 rows, the status row is 27, the picker is last.
        assert_eq!(written_row(&writes, 28).as_deref(), Some("PICK A"));
        assert_eq!(written_row(&writes, 29).as_deref(), Some("PICK B"));
        assert!(
            !writes.contains("p0"),
            "the prompt rows are not drawn while the picker is up"
        );
    }

    #[test]
    fn fullscreen_search_highlights_body_rows() {
        let mut f = fullscreen_fixture(100, 30, false);
        let _events = f.tui.start();
        assert!(f.tui.scroll_by_lines(-5));
        f.handles.writes.clear();
        assert_eq!(f.tui.set_scroll_search("b20", false), 1);
        let writes = f.handles.writes.joined();
        assert!(
            writes.contains("\x1b[7mb20\x1b[27m"),
            "the match is marked in its row"
        );
        assert!(!writes.contains("\x1b[7mb21"), "other rows are not marked");
    }

    /// Frame time with a long transcript, full-screen and off. Timing only, so it is
    /// ignored by default:
    /// `cargo test -p hoocode-tui-render perf_long_transcript -- --ignored --nocapture`.
    #[test]
    #[ignore = "timing only: run by hand"]
    fn perf_long_transcript_frame_time() {
        const LINES: usize = 5000;
        const FRAMES: u32 = 300;
        let body_lines = |tail: &str| -> Vec<String> {
            let mut lines: Vec<String> = (0..LINES)
                .map(|i| format!("line {i:05} lorem ipsum dolor sit amet, consectetur"))
                .collect();
            lines.push(tail.to_string());
            lines
        };
        let body = TestComponent::new();
        body.borrow_mut().lines = body_lines("tail");
        let start_body = std::time::Instant::now();
        for _ in 0..FRAMES {
            let _ = body.borrow_mut().render(100);
        }
        let body_only = start_body.elapsed() / FRAMES;
        println!("component render of {LINES} lines: {body_only:?} per call");
        for fullscreen in [true, false] {
            let (mut tui, handles) = new_tui(100, 40);
            tui.set_fullscreen(fullscreen);
            tui.add_child(text_component(vec!["HUD".to_string()]));
            let body = TestComponent::new();
            body.borrow_mut().lines = body_lines("tail");
            tui.add_child(body.clone());
            if fullscreen {
                let body_handle: ComponentHandle = body.clone();
                tui.set_fullscreen_body(Some(body_handle));
            }
            let bottom = TestComponent::new();
            bottom.borrow_mut().lines = bottom_lines();
            tui.add_child(bottom.clone());
            let _events = tui.start();
            tui.request_render(true);
            handles.writes.clear();
            let mut bytes = 0usize;
            let started = std::time::Instant::now();
            for frame in 0..FRAMES {
                if let Some(last) = body.borrow_mut().lines.last_mut() {
                    *last = format!("tail {frame}");
                }
                tui.request_render(false);
                bytes += handles.writes.joined().len();
                handles.writes.clear();
            }
            let per_frame = started.elapsed() / FRAMES;
            println!(
                "{} {LINES} lines, 100x40: {per_frame:?} per frame, {} bytes per frame",
                if fullscreen {
                    "full-screen"
                } else {
                    "off        "
                },
                bytes / FRAMES as usize
            );
        }
    }
    #[test]
    fn fold_matcher_maps_folds_that_change_byte_length() {
        // İ (U+0130, 2 bytes) folds to `i` plus a combining dot.
        assert_eq!(fold_match_ranges("İSTANBUL", "i"), vec![(0, 2)]);
        assert_eq!(fold_match_ranges("Hello", "ell"), vec![(1, 4)]);
        // An emoji query is multi-byte on both sides.
        assert_eq!(fold_match_ranges("launch 🚀 now", "🚀"), vec![(7, 11)]);
        assert!(fold_match_ranges("plain", "z").is_empty());
        assert!(fold_match_ranges("plain", "").is_empty());
    }

    #[test]
    fn highlight_marks_folded_and_emoji_matches() {
        // The old whole-line fold bailed here and dropped the marker while
        // the count still found the row.
        let marked = Tui::highlight_scroll_matches("İSTANBUL", "i");
        assert!(marked.contains("\x1b[7m"), "{marked:?}");
        assert!(marked.contains("◂ contact"), "{marked:?}");
        let marked = Tui::highlight_scroll_matches("launch 🚀 now", "🚀");
        assert!(marked.contains("\x1b[7m"), "{marked:?}");
        assert!(marked.contains("◂ contact"), "{marked:?}");
        let plain = Tui::highlight_scroll_matches("nothing here", "z");
        assert!(!plain.contains("contact"), "{plain:?}");
    }

    #[test]
    fn refresh_scans_only_the_grown_tail_while_streaming() {
        let (mut tui, _handles) = new_tui(40, 10);
        tui.flat_lines = vec![
            "alpha one".to_string(),
            "beta two".to_string(),
            "zzz".to_string(),
            "yyy".to_string(),
        ];
        // Measured at four rows, matching row 0 (set directly: going
        // through set_scroll_search would paint and re-measure).
        tui.scroll_search = Some(ScrollSearch {
            query: "alp".to_string(),
            matches: vec![0],
            index: 0,
            measured_at: Some(4),
            typing: false,
        });
        // Streaming growth: two rows arrive, one matching.
        tui.flat_lines.push("gamma".to_string());
        tui.flat_lines.push("alpine summit".to_string());
        tui.refresh_scroll_search(6, true);
        assert_eq!(tui.scroll_search.as_ref().unwrap().matches, vec![0, 5]);
        // A rebuild re-scans everything and drops stale rows.
        tui.flat_lines = vec![
            "zzz".to_string(),
            "yyy".to_string(),
            "xxx".to_string(),
            "www".to_string(),
            "vvv".to_string(),
            "an alpine match".to_string(),
        ];
        tui.refresh_scroll_search(6, true);
        assert_eq!(tui.scroll_search.as_ref().unwrap().matches, vec![5]);
    }
}

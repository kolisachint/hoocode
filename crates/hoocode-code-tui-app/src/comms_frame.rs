//! The Mission Control comms frame (M2): the 3-row heavy frame at the bottom
//! of the screen. Its title carries the state (READY, IN FLIGHT, COMMS LOST,
//! LANDED, DID NOT LAND, ABORTED); the bottom border carries the foot (the
//! view dial and the token totals). It wraps the prompt editor, which renders
//! only its text content: no border, no hint, no session chip. The editor
//! shows a per-state placeholder while it is empty.
//!
//! The chrome (top and bottom edges) is drawn with ratatui
//! ([`RatatuiComponent`]) from the Mission Control [`Palette`]; the input
//! rows splice the editor's own lines (ANSI, with the cursor marker) between
//! painted `┃` edges. At rest the frame is 3 rows; it grows with multi-line
//! input and the autocomplete popup, which renders inside the frame.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use hoocode_code_agent_session::format::{format_duration_secs, format_tokens, js_to_fixed};
use hoocode_code_tui_keybindings::key_display_text;
use hoocode_code_tui_theme::session_color_token;
use hoocode_tui_glyphs::status;
use hoocode_tui_ratatui::core::buffer::Buffer;
use hoocode_tui_ratatui::core::layout::Rect;
use hoocode_tui_ratatui::core::style::{Color, Style};
use hoocode_tui_ratatui::{bold, paint, Palette, RatatuiComponent};
use hoocode_tui_render::{Component, ComponentHandle};
use hoocode_tui_util::{truncate_to_width, visible_width};

use crate::footer::{FooterSource, ToolOutputView};
use crate::footer_data::FooterDataProvider;
use crate::hull_paint::HullPaint;
use crate::session_chip::{render_session_chip, session_chip_fits};
use hoocode_code_tui_widgets::radar::FlightMark;

/// The READY placeholder: what the empty prompt invites. Short on purpose:
/// the `/`, `@` and `!` keys get their own suggestion row.
pub const READY_PLACEHOLDER: &str = "name your mission";
/// The IN FLIGHT placeholder: typed text queues behind the running mission.
pub const IN_FLIGHT_PLACEHOLDER: &str = "type to queue the next mission";
/// The COMMS LOST placeholder: enter hails again.
pub const COMMS_LOST_PLACEHOLDER: &str = "press enter to hail again";

/// The comms frame's state. Only READY and IN FLIGHT are wired to the
/// session in M2; the rest render a plausible title and frame so the enum
/// is complete for the cards and consoles that follow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommsState {
    Ready,
    /// A mission is running. `elapsed` runs from submit (`TurnTimer`); it is
    /// `None` when no prompt is open, and the title then omits the time.
    InFlight {
        elapsed: Option<Duration>,
        /// The running step's number (1-based, session-wide): the title's
        /// `step N`. `None` omits it, when the host does not know it.
        step: Option<usize>,
    },
    CommsLost,
    Landed {
        elapsed: Option<Duration>,
        steps: Option<usize>,
    },
    DidNotLand {
        elapsed: Option<Duration>,
        steps: Option<usize>,
    },
    Aborted,
}

/// How the last mission ended, read by the frame's title until the next
/// mission starts: `LANDED`, `DID NOT LAND` or `ABORTED`. Set when a turn
/// settles; cleared when a mission opens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MissionOutcome {
    pub mark: FlightMark,
    /// The mission's time, in seconds.
    pub secs: f64,
    pub steps: usize,
}

impl MissionOutcome {
    /// The comms state the title shows for this outcome. A flying mark has
    /// no outcome yet, so it reads READY.
    pub fn comms_state(&self) -> CommsState {
        let elapsed = Some(Duration::from_secs_f64(self.secs.max(0.0)));
        let steps = (self.steps > 0).then_some(self.steps);
        match self.mark {
            FlightMark::Landed => CommsState::Landed { elapsed, steps },
            FlightMark::DidNotLand => CommsState::DidNotLand { elapsed, steps },
            FlightMark::Aborted => CommsState::Aborted,
            FlightMark::InFlight => CommsState::Ready,
        }
    }
}

/// The input glyph's (`▸`) colour: accent during a mission, err after a lost
/// one, ink otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlyphStyle {
    Ink,
    Acc,
    Err,
}

impl GlyphStyle {
    /// The palette style for the glyph.
    pub fn style(&self) -> Style {
        let palette = Palette::current();
        match self {
            GlyphStyle::Ink => palette.ink(),
            GlyphStyle::Acc => palette.acc(),
            GlyphStyle::Err => palette.err(),
        }
    }
}

/// What the comms frame tells the editor each render: the placeholder and
/// the glyph colour for the state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputStyle {
    pub placeholder: String,
    pub glyph: GlyphStyle,
}

impl CommsState {
    /// The frame colour: faint at rest, acc in flight, err when comms is
    /// lost. The hull paint (`/color`, M8e5) takes the frame when set.
    fn frame_style(&self, palette: &Palette, paint: Option<Style>) -> Style {
        if let Some(paint) = paint {
            return paint;
        }
        match self {
            CommsState::InFlight { .. } => palette.acc(),
            CommsState::CommsLost => palette.err(),
            _ => palette.faint(),
        }
    }

    /// The placeholder and glyph for the state.
    pub fn input_style(&self) -> InputStyle {
        match self {
            CommsState::InFlight { .. } => InputStyle {
                placeholder: IN_FLIGHT_PLACEHOLDER.to_string(),
                glyph: GlyphStyle::Acc,
            },
            CommsState::CommsLost => InputStyle {
                placeholder: COMMS_LOST_PLACEHOLDER.to_string(),
                glyph: GlyphStyle::Err,
            },
            _ => InputStyle {
                placeholder: READY_PLACEHOLDER.to_string(),
                glyph: GlyphStyle::Ink,
            },
        }
    }

    /// The title's styled runs, space-separated when drawn. The hull
    /// paint takes the state colour (the mark and name) when set; the keys
    /// stay ink on dim.
    fn title_segs(&self, palette: &Palette, paint: Option<Style>) -> Vec<Seg> {
        let dim = palette.dim();
        let faint = palette.faint();
        let ink = palette.ink();
        match self {
            // READY stays dim even under a hull paint (design e5): the frame
            // and the right label take the paint, the title does not.
            CommsState::Ready => vec![Seg::new(dim, "READY")],
            CommsState::InFlight { elapsed, step } => {
                let state = paint.unwrap_or_else(|| palette.acc());
                let mut segs = vec![
                    Seg::new(state, "◐"),
                    Seg::new(bold(state), "IN FLIGHT"),
                    Seg::new(faint, "·"),
                ];
                if let Some(step) = step {
                    segs.push(Seg::new(dim, &format!("step {step}")));
                    segs.push(Seg::new(faint, "·"));
                }
                if let Some(elapsed) = elapsed {
                    segs.push(Seg::new(dim, &format!("{}s", elapsed.as_secs())));
                    segs.push(Seg::new(faint, "·"));
                }
                segs.push(Seg::new(ink, "esc"));
                segs.push(Seg::new(dim, "abort"));
                segs
            }
            CommsState::CommsLost => vec![
                Seg::new(paint.unwrap_or_else(|| palette.err()), "⊗"),
                Seg::new(bold(paint.unwrap_or_else(|| palette.err())), "COMMS LOST"),
                Seg::new(faint, "·"),
                Seg::new(ink, "enter"),
                Seg::new(dim, "hail again"),
                Seg::new(faint, "·"),
                Seg::new(ink, "alt+m"),
                Seg::new(dim, "switch ship"),
            ],
            CommsState::Landed { elapsed, steps } => landed_title(
                paint.unwrap_or_else(|| palette.ok()),
                palette,
                *elapsed,
                *steps,
                "LANDED",
                status::DONE_STR,
            ),
            CommsState::DidNotLand { elapsed, steps } => landed_title(
                paint.unwrap_or_else(|| palette.err()),
                palette,
                *elapsed,
                *steps,
                "DID NOT LAND",
                "⊗",
            ),
            CommsState::Aborted => vec![Seg::new(paint.unwrap_or(dim), "⊘ ABORTED · READY")],
        }
    }
}

/// A LANDED / DID NOT LAND title: the coloured mark and name, then the dim
/// `time · N steps` when the debrief knows them.
fn landed_title(
    color: Style,
    palette: &Palette,
    elapsed: Option<Duration>,
    steps: Option<usize>,
    name: &str,
    mark: &str,
) -> Vec<Seg> {
    let mut segs = vec![Seg::new(color, mark), Seg::new(bold(color), name)];
    let mut tail = String::new();
    if let Some(elapsed) = elapsed {
        tail.push_str(&format_duration_secs(elapsed.as_secs_f64()));
    }
    if let Some(steps) = steps {
        if !tail.is_empty() {
            tail.push_str(" · ");
        }
        tail.push_str(&format_steps(steps));
    }
    if !tail.is_empty() {
        segs.push(Seg::new(palette.faint(), "·"));
        segs.push(Seg::new(palette.dim(), &tail));
    }
    segs
}

/// `1 step`, `N steps`.
fn format_steps(steps: usize) -> String {
    if steps == 1 {
        "1 step".to_string()
    } else {
        format!("{steps} steps")
    }
}

/// One styled run inside an edge row, with its width in cells.
#[derive(Debug, Clone)]
struct Seg {
    text: String,
    style: Style,
    width: usize,
}

impl Seg {
    fn new(style: Style, text: &str) -> Self {
        Self {
            text: text.to_string(),
            style,
            width: visible_width(text),
        }
    }

    fn space() -> Self {
        Self {
            text: " ".to_string(),
            style: Style::default(),
            width: 1,
        }
    }
}

/// `segs` with one plain space between runs.
fn spaced(mut segs: Vec<Seg>) -> Vec<Seg> {
    let mut out = Vec::with_capacity(segs.len() * 2);
    for (i, seg) in segs.drain(..).enumerate() {
        if i > 0 {
            out.push(Seg::space());
        }
        out.push(seg);
    }
    out
}

/// `segs` cut to `budget` cells, ending in `…` when cut. Returns the runs
/// and their width.
fn truncate_segs(segs: &[Seg], budget: usize) -> (Vec<Seg>, usize) {
    let mut out = Vec::new();
    let mut used = 0;
    for seg in segs {
        if used + seg.width <= budget {
            used += seg.width;
            out.push(seg.clone());
            continue;
        }
        let room = budget.saturating_sub(used);
        if room > 0 {
            let text = truncate_to_width(&seg.text, room, "…", false);
            let width = visible_width(&text);
            used += width;
            out.push(Seg {
                text,
                style: seg.style,
                width,
            });
        }
        break;
    }
    (out, used)
}

/// One drawn cell run: text, style, and width in cells.
#[derive(Debug, Clone)]
struct Run {
    text: String,
    style: Style,
    width: usize,
}

/// An edge row as drawn runs: `┏━ title ━…━ right ━┓`. The title goes
/// first when the row is short; past that both are cut.
fn edge_runs(
    frame: Style,
    corners: (char, char),
    left: &[Seg],
    right: &[Seg],
    width: usize,
) -> Vec<Run> {
    // Corners, two thick bars and four spaces are fixed; the fill is at
    // least one cell, like the simulator's `box`.
    let budget = width.saturating_sub(9);
    let (right, rw) = truncate_segs(right, budget);
    let (left, lw) = truncate_segs(left, budget.saturating_sub(rw));
    let fill = width.saturating_sub(8 + lw + rw).max(1);
    let mut runs = vec![
        Run::from_seg(&Seg::new(frame, &corners.0.to_string())),
        Run::from_seg(&Seg::new(frame, "━")),
        Run::from_seg(&Seg::space()),
    ];
    runs.extend(left.iter().map(Run::from_seg));
    runs.push(Run::from_seg(&Seg::space()));
    runs.push(Run::from_seg(&Seg::new(frame, &"━".repeat(fill))));
    runs.push(Run::from_seg(&Seg::space()));
    runs.extend(right.iter().map(Run::from_seg));
    runs.push(Run::from_seg(&Seg::space()));
    runs.push(Run::from_seg(&Seg::new(frame, "━")));
    runs.push(Run::from_seg(&Seg::new(frame, &corners.1.to_string())));
    runs
}

impl Run {
    fn from_seg(seg: &Seg) -> Self {
        Self {
            text: seg.text.clone(),
            style: seg.style,
            width: seg.width,
        }
    }
}

/// The chrome both edge rows draw: the frame style and the rows' runs.
#[derive(Debug, Clone, Default)]
struct ChromeData {
    top: Vec<Run>,
    bottom: Vec<Run>,
}

impl ChromeData {
    // The paint pushed the chrome config past seven arguments.
    #[allow(clippy::too_many_arguments)]
    fn build(
        state: &CommsState,
        palette: &Palette,
        hull: Option<HullPaint>,
        view: ToolOutputView,
        plan_mode: bool,
        input_total: u64,
        output_total: u64,
        session_name: &str,
        session_color_slot: u8,
        context: Option<(u64, Option<f64>)>,
        model_window: u64,
        has_queue: bool,
        width: usize,
    ) -> Self {
        // The hull paint (`/color`, M8e5) keeps the frame's right label;
        // otherwise the session chip rides top-right (the plain name when
        // the chip doesn't fit the width). The frame title and border take
        // the paint's colour either way.
        let paint = hull.map(|hull| palette.style(hull.token()));
        let frame = state.frame_style(palette, paint);
        let right: Vec<Seg> = match hull {
            Some(hull) => vec![Seg::new(
                paint.unwrap_or_else(|| palette.acc()),
                hull.name(),
            )],
            None => session_label(palette, session_name, session_color_slot, width),
        };
        let top = edge_runs(
            frame,
            ('┏', '┓'),
            &spaced(state.title_segs(palette, paint)),
            &right,
            width,
        );
        let mut foot = match view {
            ToolOutputView::Peek => vec![Seg::new(palette.dim(), "peek")],
            ToolOutputView::Radar => vec![Seg::new(palette.dim(), "radar · flight log")],
        };
        if plan_mode {
            foot.push(Seg::new(palette.faint(), "·"));
            foot.push(Seg::new(palette.vio(), "PLAN · nothing gets changed"));
        }
        // A queued note says how to take it back (spec §4 c4). The key is
        // the dequeue binding's own label, so a rebound key still reads true.
        if has_queue {
            foot.push(Seg::new(palette.faint(), "·"));
            foot.push(Seg::new(
                palette.dim(),
                &key_display_text("app.message.dequeue"),
            ));
            foot.push(Seg::new(palette.dim(), "edit queue"));
        }
        let footr = foot_totals(palette, input_total, output_total, context, model_window);
        let bottom = edge_runs(frame, ('┗', '┛'), &spaced(foot), &footr, width);
        Self { top, bottom }
    }
}

/// The top-right label: the session chip (` name `, filled with the
/// session's colour) when it fits, else the plain session name. An empty
/// name keeps the old `app-edit` fallback.
fn session_label(palette: &Palette, name: &str, color_slot: u8, width: usize) -> Vec<Seg> {
    if session_chip_fits(width) {
        if let Some(chip) = render_session_chip(name, i64::from(color_slot)) {
            return vec![Seg::new(
                session_chip_style(palette, color_slot),
                &chip.plain,
            )];
        }
    }
    let trimmed = name.trim();
    if trimmed.is_empty() {
        vec![Seg::new(palette.acc(), "app-edit")]
    } else {
        vec![Seg::new(palette.acc(), trimmed)]
    }
}

/// The chip's style: dark text on the session colour, bold — the same
/// reversed chip the HUD uses. Without a background fill, bold default.
fn session_chip_style(palette: &Palette, color_slot: u8) -> Style {
    let fill = palette.fill(session_color_token(i64::from(color_slot)));
    if fill.bg.is_some() {
        return bold(fill.fg(Color::Black));
    }
    bold(Style::default())
}

/// The bottom-right totals: `↑in ↓out · {pct}% of {window}`, reusing the
/// footer's formatters. The context tail draws last, so `truncate_segs`
/// cuts it before the totals on short rows. No window anywhere (no usage
/// and no model window): just the totals.
fn foot_totals(
    palette: &Palette,
    input_total: u64,
    output_total: u64,
    context: Option<(u64, Option<f64>)>,
    model_window: u64,
) -> Vec<Seg> {
    let mut segs = vec![Seg::new(
        palette.dim(),
        &format!(
            "↑{} ↓{}",
            format_tokens(input_total),
            format_tokens(output_total)
        ),
    )];
    let window = context
        .map(|(window, _)| window)
        .filter(|window| *window > 0)
        .unwrap_or(model_window);
    if window > 0 {
        let percent = match context {
            Some((_, Some(percent))) => format!("{}%", js_to_fixed(percent, 0)),
            Some((_, None)) => "?".to_string(),
            None => format!("{}%", js_to_fixed(0.0, 0)),
        };
        segs.push(Seg::new(palette.faint(), " · "));
        segs.push(Seg::new(
            palette.dim(),
            &format!("{percent} of {}", format_tokens(window)),
        ));
    }
    segs
}

/// Draws one edge row's runs left to right, clipping at the row's width.
fn draw_row(buf: &mut Buffer, x: u16, y: u16, width: u16, runs: &[Run]) {
    let mut x = x;
    let end = x.saturating_add(width);
    for run in runs {
        if x >= end || run.text.is_empty() {
            continue;
        }
        let room = end.saturating_sub(x) as usize;
        buf.set_stringn(x, y, &run.text, room, run.style);
        x = x.saturating_add(run.width.min(room) as u16);
    }
}

/// The chrome's draw closure, boxed so the wrapped component has a nameable type.
type ChromeDraw = Box<dyn FnMut(Rect, &mut Buffer)>;

/// The comms frame around the prompt editor. Reads the session on each
/// render (through `read_state`); the editor keeps its key handling and
/// focus, reached through this component.
pub struct CommsFrame {
    editor: ComponentHandle,
    read_state: Rc<dyn Fn() -> CommsState>,
    /// The messages queued while the agent works, as `(steering, follow-up)`.
    /// They draw inside the frame (spec §4 c4), above the input row.
    read_queue: Rc<dyn Fn() -> (Vec<String>, Vec<String>)>,
    set_input: Rc<dyn Fn(InputStyle)>,
    source: Box<dyn FooterSource>,
    data: FooterDataProvider,
    view: ToolOutputView,
    chrome_data: Rc<RefCell<ChromeData>>,
    chrome: RatatuiComponent<ChromeDraw>,
}

impl CommsFrame {
    /// The frame around `editor`. `read_state` maps the session to a state;
    /// `read_queue` reads the queued steering and follow-up messages, drawn
    /// inside the frame above the input row; `set_input` styles the editor's
    /// input row for it (the placeholder and the glyph colour). `source`
    /// feeds the foot's token totals; `data` tells the foot the view's plan note.
    pub fn new(
        editor: ComponentHandle,
        read_state: impl Fn() -> CommsState + 'static,
        read_queue: impl Fn() -> (Vec<String>, Vec<String>) + 'static,
        set_input: impl Fn(InputStyle) + 'static,
        source: Box<dyn FooterSource>,
        data: FooterDataProvider,
        view: ToolOutputView,
    ) -> Self {
        let chrome_data = Rc::new(RefCell::new(ChromeData::default()));
        let draw_data = chrome_data.clone();
        let draw: ChromeDraw = Box::new(move |area: Rect, buf: &mut Buffer| {
            let data = draw_data.borrow();
            draw_row(buf, area.x, area.y, area.width, &data.top);
            if area.height > 1 {
                draw_row(buf, area.x, area.y + 1, area.width, &data.bottom);
            }
        });
        Self {
            editor,
            read_state: Rc::new(read_state),
            read_queue: Rc::new(read_queue),
            set_input: Rc::new(set_input),
            source,
            data,
            view,
            chrome_data,
            chrome: RatatuiComponent::new(2, draw),
        }
    }

    /// The view dial's current stop (kept beside the footer's).
    pub fn set_tool_output_view(&mut self, view: ToolOutputView) {
        self.view = view;
    }
}

impl Component for CommsFrame {
    fn render(&mut self, width: u16) -> Vec<String> {
        if width == 0 {
            return Vec::new();
        }
        let state = (self.read_state)();
        (self.set_input)(state.input_style());
        let palette = Palette::current();
        let (input_total, output_total, _, _, _) = self.source.usage_totals();
        let session_name = self.source.display_name();
        let session_color_slot = self.source.session_color_slot();
        let context = self.source.context_usage();
        let model_window = self
            .source
            .model()
            .map(|model| model.context_window)
            .unwrap_or(0);
        let plan_mode = self.data.get_active_mode() == "plan";
        let hull = self.data.get_hull_paint();
        let frame = state.frame_style(&palette, hull.map(|hull| palette.style(hull.token())));
        let (steering, follow_up) = (self.read_queue)();
        *self.chrome_data.borrow_mut() = ChromeData::build(
            &state,
            &palette,
            hull,
            self.view,
            plan_mode,
            input_total,
            output_total,
            &session_name,
            session_color_slot,
            context,
            model_window,
            !(steering.is_empty() && follow_up.is_empty()),
            width as usize,
        );
        let chrome = self.chrome.render(width);
        let inner = (width as usize).saturating_sub(2).max(1);
        let mut editor_lines = self.editor.borrow_mut().render(inner as u16);
        if editor_lines.is_empty() {
            editor_lines.push(String::new());
        }
        let edge = paint(frame, "┃");
        let mut out = Vec::with_capacity(editor_lines.len() + 2);
        out.push(chrome.first().cloned().unwrap_or_default());
        for row in queue_rows(&steering, &follow_up, inner, &palette) {
            out.push(format!("{edge}{}{edge}", fit_line(&row, inner)));
        }
        for line in editor_lines {
            out.push(format!("{edge}{}{edge}", fit_line(&line, inner)));
        }
        out.push(chrome.get(1).cloned().unwrap_or_default());
        out
    }

    fn handle_input(&mut self, data: &str) {
        self.editor.borrow_mut().handle_input(data);
    }

    fn invalidate(&mut self) {
        self.editor.borrow_mut().invalidate();
        self.chrome.invalidate();
    }

    fn is_focusable(&self) -> bool {
        self.editor.borrow().is_focusable()
    }

    fn set_focused(&mut self, focused: bool) {
        self.editor.borrow_mut().set_focused(focused);
    }
}

/// The queued messages as frame rows: `● text queued · sends after this
/// step` for steering notes, `○ text queued · sends after landing` for
/// follow-ups (spec §4 c4). Each row is cut to `inner` cells; the frame's
/// `fit_line` pads it (never cuts: a cut could drop the editor's cursor).
fn queue_rows(
    steering: &[String],
    follow_up: &[String],
    inner: usize,
    palette: &Palette,
) -> Vec<String> {
    let mut rows = Vec::new();
    for (texts, bullet, bullet_style, disposition) in [
        (
            steering,
            status::DONE_STR,
            palette.acc(),
            "queued · sends after this step",
        ),
        (
            follow_up,
            status::TODO_STR,
            palette.dim(),
            "queued · sends after landing",
        ),
    ] {
        for text in texts {
            let tail = format!(" {disposition}");
            let room = inner.saturating_sub(2 + visible_width(&tail));
            let text = truncate_to_width(text, room, "…", false);
            rows.push(format!(
                "{} {}{}",
                paint(bullet_style, bullet),
                text,
                paint(palette.dim(), &tail)
            ));
        }
    }
    rows
}

/// The editor's line at exactly `width` cells. The bare editor pads its
/// lines there itself; this only pads short ones (never cuts: a cut could
/// drop the cursor marker).
fn fit_line(line: &str, width: usize) -> String {
    let w = visible_width(line);
    if w < width {
        format!("{line}{}", " ".repeat(width - w))
    } else {
        line.to_string()
    }
}

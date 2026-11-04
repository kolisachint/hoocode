//! The status line (declutter spec v2.1 to v2.9): ONE row under the prompt.
//!
//! Left slot, only while something happens: `◉ working · 4s · esc stop`, the
//! last turn's result, a notice, `scrolled 43–61 of 78 · ↓ 3 new`, or
//! `search  query  1 match`. Right slot, always, dim: view, session, model,
//! context, cost (`peek · amber-harbor · claude-haiku-5-5 · 42% context · $1.23`).
//!
//! The component is a pure function of [`StatusLineData`]: no session reads,
//! no clock. The caller owns the state (and the notice's age); it is not
//! wired into the screen yet.
//!
//! Colours come from the existing palette tokens (`ink`, `dim`, `ok`, `err`,
//! `warn`). With colour off (NO_COLOR) only shapes, bold and dim are used.

use std::time::Duration;

use hoocode_tui_ratatui::core::buffer::Buffer;
use hoocode_tui_ratatui::core::layout::Rect;
use hoocode_tui_ratatui::core::style::{Modifier, Style};
use hoocode_tui_ratatui::core::text::{Line, Span};
use hoocode_tui_ratatui::{bold, spans_width, truncate_spans, Palette, RatatuiComponent};

/// Info notices leave the status line after this long.
pub const INFO_TTL: Duration = Duration::from_secs(3);
/// Warning notices leave the status line after this long.
pub const WARNING_TTL: Duration = Duration::from_secs(8);

/// Columns between the left and the right slot.
const GAP: usize = 2;
/// Blank columns at the left and at the right edge.
const PAD: usize = 1;

/// How a notice looks and how long it stays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeLevel {
    /// Plain text; 3 s.
    Info,
    /// Yellow text; 8 s.
    Warning,
    /// Red `⊗`, dim text, the action key in normal text; never times out
    /// (`⊗ not signed in  /login`).
    Action,
}

/// A key and what it does, shown after a message: key normal, action dim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyHint {
    pub key: String,
    pub action: String,
}

impl KeyHint {
    pub fn new(key: impl Into<String>, action: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            action: action.into(),
        }
    }
}

/// A one-row notice that takes the left slot for its timeout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusNotice {
    pub level: NoticeLevel,
    pub message: String,
    /// Keys after the message (`shift+alt+m back`); empty for none.
    pub keys: Vec<KeyHint>,
    /// How long the notice has been up; the caller advances it.
    pub age: Duration,
}

impl StatusNotice {
    pub fn new(level: NoticeLevel, message: impl Into<String>) -> Self {
        Self {
            level,
            message: message.into(),
            keys: Vec::new(),
            age: Duration::ZERO,
        }
    }

    /// The notice that needs an action and never times out.
    pub fn action(message: impl Into<String>, key: impl Into<String>) -> Self {
        Self {
            level: NoticeLevel::Action,
            message: message.into(),
            keys: vec![KeyHint::new(key, "")],
            age: Duration::ZERO,
        }
    }

    /// The timeout; `None` never times out.
    pub fn ttl(&self) -> Option<Duration> {
        match self.level {
            NoticeLevel::Info => Some(INFO_TTL),
            NoticeLevel::Warning => Some(WARNING_TTL),
            NoticeLevel::Action => None,
        }
    }

    /// Whether the timeout has passed (an action notice never expires).
    pub fn expired(&self) -> bool {
        self.ttl().is_some_and(|ttl| self.age >= ttl)
    }
}

/// How the last turn ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnOutcome {
    Done,
    Failed,
    Stopped,
}

/// The last turn's result, shown until the next prompt is sent.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnResult {
    pub outcome: TurnOutcome,
    pub elapsed: Duration,
    /// Zero is hidden.
    pub tokens: u64,
    /// Failed tool calls in a turn that still finished; zero is hidden.
    pub failed_calls: usize,
}

/// The scrolled-back bar: `scrolled 43–61 of 78 · ↓ 3 new`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrollInfo {
    pub top: usize,
    pub bottom: usize,
    pub total: usize,
    /// Rows that arrived below the view; zero is hidden.
    pub new_below: usize,
    pub keys: Vec<KeyHint>,
}

/// The search bar: `search  query  1 match`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchInfo {
    pub query: String,
    /// Number of matches.
    pub count: usize,
    /// The current match, 1-based; 0 when none is selected yet.
    pub index: usize,
    pub keys: Vec<KeyHint>,
}

/// Everything the status line shows. Owned, plain data.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StatusLineData {
    /// Elapsed time of the running turn; `Some` while a turn runs.
    pub working: Option<Duration>,
    pub notice: Option<StatusNotice>,
    pub scroll: Option<ScrollInfo>,
    pub search: Option<SearchInfo>,
    pub last_turn: Option<TurnResult>,
    /// No working provider: left slot `⊗ not signed in  /login`, right side
    /// shows only view and session.
    pub signed_out: bool,
    /// The tool-output view: `peek`, `outline`, `full`.
    pub view: String,
    pub plan_mode: bool,
    pub session: Option<String>,
    pub model: Option<String>,
    /// Thinking level after the model; hidden when `None` or empty.
    pub effort: Option<String>,
    /// Context fill in percent; `None` or under 1 is hidden.
    pub context_percent: Option<f64>,
    /// Session cost in dollars; under one cent is hidden.
    pub cost: Option<f64>,
}

/// What the left slot shows after the priority rules.
#[derive(Debug, Clone, PartialEq)]
pub enum LeftSlot {
    Empty,
    Search(SearchInfo),
    Scroll(ScrollInfo),
    Working(Duration),
    Notice(StatusNotice),
    NotSignedIn,
    Turn(TurnResult),
}

impl StatusLineData {
    /// The left slot by priority: search, scrolled, working, a live notice,
    /// not signed in, the last turn's result. (Not signed in outranks the
    /// result and yields to working; a notice waits behind working, which
    /// keeps the slot.)
    pub fn left_slot(&self) -> LeftSlot {
        if let Some(search) = &self.search {
            return LeftSlot::Search(search.clone());
        }
        if let Some(scroll) = &self.scroll {
            return LeftSlot::Scroll(scroll.clone());
        }
        if let Some(elapsed) = self.working {
            return LeftSlot::Working(elapsed);
        }
        if let Some(notice) = self.notice.as_ref().filter(|n| !n.expired()) {
            return LeftSlot::Notice(notice.clone());
        }
        if self.signed_out {
            return LeftSlot::NotSignedIn;
        }
        match &self.last_turn {
            Some(turn) => LeftSlot::Turn(turn.clone()),
            None => LeftSlot::Empty,
        }
    }
}

/// `4s`, `1m 05s`: whole seconds for the running turn.
pub fn format_clock(elapsed: Duration) -> String {
    let secs = elapsed.as_secs();
    if secs >= 60 {
        format!("{}m {:02}s", secs / 60, secs % 60)
    } else {
        format!("{secs}s")
    }
}

/// `5.0s` under ten seconds, then whole seconds (`11s`, `1m 05s`).
pub fn format_took(elapsed: Duration) -> String {
    let secs = elapsed.as_secs_f64();
    if secs < 10.0 {
        format!("{secs:.1}s")
    } else {
        format_clock(Duration::from_secs(secs.round() as u64))
    }
}

/// `850 tokens`, `7.1k tokens`, `23k tokens`.
pub fn format_tokens(tokens: u64) -> String {
    if tokens < 1_000 {
        format!("{tokens} tokens")
    } else if tokens < 10_000 {
        format!("{:.1}k tokens", tokens as f64 / 1_000.0)
    } else {
        format!("{}k tokens", (tokens as f64 / 1_000.0).round() as u64)
    }
}

/// `$1.23`; `None` for under one cent.
pub fn format_cost(cost: f64) -> Option<String> {
    let cents = (cost * 100.0).round();
    (cents >= 1.0).then(|| format!("${:.2}", cents / 100.0))
}

/// The styles of one pass.
struct Styles {
    ink: Style,
    dim: Style,
    ok: Style,
    err: Style,
    warn: Style,
    plan: Style,
}

impl Styles {
    fn new(pal: &Palette) -> Self {
        if hoocode_code_tui_theme::chalk::enabled() {
            Self {
                ink: pal.ink(),
                dim: pal.dim(),
                ok: pal.ok(),
                err: pal.err(),
                warn: pal.warn(),
                plan: bold(pal.dim()),
            }
        } else {
            // NO_COLOR: shapes, bold and dim only.
            let plain = Style::default();
            Self {
                ink: plain,
                dim: plain.add_modifier(Modifier::DIM),
                ok: plain,
                err: plain,
                warn: plain,
                plan: bold(plain),
            }
        }
    }
}

fn span(text: impl Into<String>, style: Style) -> Span<'static> {
    Span::styled(text.into(), style)
}

fn keys_spans(keys: &[KeyHint], st: &Styles, out: &mut Vec<Span<'static>>) {
    for (i, hint) in keys.iter().enumerate() {
        out.push(span(if i == 0 { "  " } else { " · " }, st.dim));
        out.push(span(hint.key.clone(), st.ink));
        if !hint.action.is_empty() {
            out.push(span(format!(" {}", hint.action), st.dim));
        }
    }
}

fn left_spans(slot: &LeftSlot, st: &Styles) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    match slot {
        LeftSlot::Empty => {}
        LeftSlot::Working(elapsed) => {
            out.push(span("◉", st.warn));
            out.push(span(
                format!(" working · {} · esc stop", format_clock(*elapsed)),
                st.dim,
            ));
        }
        LeftSlot::Turn(turn) => {
            let (glyph, glyph_style, word) = match turn.outcome {
                TurnOutcome::Done => ("●", st.ok, "done"),
                TurnOutcome::Failed => ("⊗", st.err, "failed"),
                TurnOutcome::Stopped => ("⊗", st.dim, "stopped"),
            };
            out.push(span(glyph, glyph_style));
            let mut text = format!(" {word} · {}", format_took(turn.elapsed));
            if turn.tokens > 0 {
                text.push_str(" · ");
                text.push_str(&format_tokens(turn.tokens));
            }
            out.push(span(text, st.dim));
            if turn.failed_calls > 0 {
                out.push(span(" · ", st.dim));
                let noun = if turn.failed_calls == 1 {
                    "call"
                } else {
                    "calls"
                };
                out.push(span(format!("{} failed {noun}", turn.failed_calls), st.err));
            }
        }
        LeftSlot::NotSignedIn => {
            out.push(span("⊗", st.err));
            out.push(span(" not signed in", st.dim));
            out.push(span("  /login", st.ink));
        }
        LeftSlot::Notice(notice) => match notice.level {
            NoticeLevel::Info => {
                out.push(span(notice.message.clone(), st.ink));
                keys_spans(&notice.keys, st, &mut out);
            }
            NoticeLevel::Warning => {
                out.push(span(notice.message.clone(), st.warn));
                keys_spans(&notice.keys, st, &mut out);
            }
            NoticeLevel::Action => {
                out.push(span("⊗", st.err));
                out.push(span(format!(" {}", notice.message), st.dim));
                keys_spans(&notice.keys, st, &mut out);
            }
        },
        LeftSlot::Scroll(scroll) => {
            let mut text = format!(
                "scrolled {}–{} of {}",
                scroll.top, scroll.bottom, scroll.total
            );
            if scroll.new_below > 0 {
                text.push_str(&format!(" · ↓ {} new", scroll.new_below));
            }
            out.push(span(text, st.dim));
            keys_spans(&scroll.keys, st, &mut out);
        }
        LeftSlot::Search(search) => {
            out.push(span("search", st.dim));
            if !search.query.is_empty() {
                out.push(span("  ", st.dim));
                out.push(span(search.query.clone(), st.ink));
                let found = match (search.count, search.index) {
                    (0, _) => "no matches".to_string(),
                    (1, _) => "1 match".to_string(),
                    (n, 0) => format!("{n} matches"),
                    (n, i) => format!("{i}/{n} matches"),
                };
                out.push(span(format!("  {found}"), st.dim));
            }
            keys_spans(&search.keys, st, &mut out);
        }
    }
    out
}

/// The last right-slot drop stage. Stage 0 shows everything; each later stage
/// drops one more part, in the spec's order: cost (1), session name (2), the
/// word `context` with the percent kept (3), model effort (4); then, beyond
/// the spec so the row always fits, the model (5) and the context percent
/// (6). The view and the plan word never drop.
const LAST_STAGE: usize = 6;
/// The last stage the spec names (model effort dropped).
const LAST_BASIC_STAGE: usize = 4;
/// Columns the left slot keeps when the right slot has to drop past the spec.
const MIN_LEFT: usize = 10;

fn right_spans(data: &StatusLineData, stage: usize, st: &Styles) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let mut parts: Vec<String> = Vec::new();
    if data.plan_mode {
        out.push(span("plan", st.plan));
        out.push(span(" · ", st.dim));
    }
    if !data.view.is_empty() {
        parts.push(data.view.clone());
    }
    if stage < 2 {
        if let Some(session) = data.session.as_ref().filter(|s| !s.is_empty()) {
            parts.push(session.clone());
        }
    }
    if !data.signed_out {
        if stage < 5 {
            if let Some(model) = data.model.as_ref().filter(|m| !m.is_empty()) {
                let effort = data.effort.as_ref().filter(|e| !e.is_empty());
                match effort {
                    Some(effort) if stage < 4 => parts.push(format!("{model} {effort}")),
                    _ => parts.push(model.clone()),
                }
            }
        }
        if stage < 6 {
            let pct = data.context_percent.map(f64::round).filter(|p| *p >= 1.0);
            if let Some(pct) = pct {
                if stage < 3 {
                    parts.push(format!("{pct:.0}% context"));
                } else {
                    parts.push(format!("{pct:.0}%"));
                }
            }
        }
        if stage < 1 {
            if let Some(cost) = data.cost.and_then(format_cost) {
                parts.push(cost);
            }
        }
    }
    out.push(span(parts.join(" · "), st.dim));
    out
}

/// The status line as spans: one pad column at each edge, the right slot
/// flush right. When the slots do not fit with a two-column gap, the right
/// slot drops parts in the narrow order; at the last stage the left slot is
/// cut with `…`.
pub fn status_line_spans(data: &StatusLineData, width: usize, pal: &Palette) -> Vec<Span<'static>> {
    let st = Styles::new(pal);
    let left = left_spans(&data.left_slot(), &st);
    let left_width = spans_width(&left);
    let inner = width.saturating_sub(2 * PAD);
    let reserved = if left_width > 0 { left_width + GAP } else { 0 };

    let mut stage_used = LAST_STAGE;
    let mut fitted = false;
    for stage in 0..=LAST_STAGE {
        if spans_width(&right_spans(data, stage, &st)) + reserved <= inner {
            stage_used = stage;
            fitted = true;
            break;
        }
    }
    if !fitted {
        // Nothing fits whole: keep the left slot readable. Use the first of
        // the last stages that leaves it MIN_LEFT columns.
        stage_used = (LAST_BASIC_STAGE..=LAST_STAGE)
            .find(|stage| {
                let width = spans_width(&right_spans(data, *stage, &st));
                inner >= width + GAP + MIN_LEFT
            })
            .unwrap_or(LAST_STAGE);
    }
    let right = right_spans(data, stage_used, &st);
    let right_width = spans_width(&right);

    let mut row = vec![span(" ".repeat(PAD), Style::default())];
    if fitted {
        row.extend(left);
        let gap = inner - left_width - right_width;
        row.push(span(" ".repeat(gap), Style::default()));
        row.extend(right);
    } else if right_width + GAP < inner {
        row.extend(truncate_spans(left, inner - right_width - GAP, "…"));
        row.push(span(" ".repeat(GAP), Style::default()));
        row.extend(right);
    } else {
        row.extend(truncate_spans(right, inner, "…"));
    }
    row.push(span(" ".repeat(PAD), Style::default()));
    row
}

/// The status line as plain text, for tests and logs.
pub fn status_line_text(data: &StatusLineData, width: usize, pal: &Palette) -> String {
    status_line_spans(data, width, pal)
        .iter()
        .map(|s| s.content.as_ref())
        .collect()
}

/// The status line as a one-row component for a fixed snapshot of `data`.
pub fn status_line_component(
    data: StatusLineData,
) -> RatatuiComponent<impl FnMut(Rect, &mut Buffer)> {
    RatatuiComponent::new(1, move |area: Rect, buf: &mut Buffer| {
        let pal = Palette::current();
        let spans = status_line_spans(&data, usize::from(area.width), &pal);
        buf.set_line(area.x, area.y, &Line::from(spans), area.width);
    })
}

//! The footer (`components/footer.ts`). Two layouts (see `FooterDensity`):
//! detailed, two rows, and short, one row.
//!
//! Detailed row 1 starts with the mode chip and the view dial, then the folder
//! name (never dropped), the branch, the session name, the path; on the
//! right, subagents. Row 2 starts with the context gauge and its percent
//! (never dropped), then the window, model and effort, tokens, cache, cost.
//!
//! Short is one row: mode, gauge, branch, model, folder, cost, dial,
//! subagents.
//!
//! When the width is short, parts go in a fixed order (see
//! `line1_candidates`, `line2_candidates` and `short_candidates`). Transient messages (the
//! warning, extension statuses, startup progress) are not rows: read them with
//! [`FooterComponent::transient_lines`] and show them in the notification area.

use std::path::Path;

use hoocode_code_agent_session::format::{format_tokens, js_to_fixed};
use hoocode_code_task_store::{task_store, TaskSource, TaskStatus};
use hoocode_code_tui_theme::theme;
use hoocode_tui_render::Component;
use hoocode_tui_util::js_math::js_round;
use hoocode_tui_util::{truncate_to_width, visible_width};

use crate::brand::GIT_BRANCH_GLYPH;
use crate::footer_data::FooterDataProvider;
use crate::progress_bar::{render_download_progress, render_progress_bar};
use crate::session_chip::session_chip_fits;
use crate::startup_progress::{self, StartupProgress};

/// The model as the footer shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct FooterModel {
    pub id: String,
    pub provider: String,
    pub context_window: u64,
    pub reasoning: bool,
}

/// What the footer reads from the session (an `AgentSession` in the app, a
/// stub in tests).
pub trait FooterSource {
    /// Cumulative (input, output, cache read, cache write, cost) over all
    /// session entries.
    fn usage_totals(&self) -> (u64, u64, u64, u64, f64);
    /// Context usage: (context window, percent — `None` when unknown after a
    /// compaction). `None` when there is no usage at all.
    fn context_usage(&self) -> Option<(u64, Option<f64>)>;
    fn model(&self) -> Option<FooterModel>;
    fn thinking_level(&self) -> String;
    fn cwd(&self) -> String;
    fn display_name(&self) -> String;
    /// Compaction reserve tokens.
    fn reserve_tokens(&self) -> u64;
    fn is_using_oauth(&self) -> bool;
}

pub use hoocode_code_settings::ToolOutputView;

/// The view dial's glyph, filling as the view widens.
pub fn tool_output_view_glyph(view: ToolOutputView) -> &'static str {
    match view {
        ToolOutputView::Radar => "◌",
        ToolOutputView::Peek => "◍",
    }
}

/// Which layout the footer uses: detailed (two rows) or short (one row).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FooterDensity {
    #[default]
    Full,
    /// Short: one row, with the mode, the gauge and the folder name.
    Line,
}

/// Plain and styled text of one piece of a row.
type Piece = (String, String);

/// `assembleLine`-style right alignment: `left` flush left, `right` flush
/// right, with at least two columns between. `None` when they do not fit.
fn fit_left_right(width: usize, left: &Piece, right: Option<&Piece>) -> Option<String> {
    let lw = visible_width(&left.0);
    let Some((right_plain, right_styled)) = right else {
        return (lw <= width).then(|| left.1.clone());
    };
    let rw = visible_width(right_plain);
    (lw + 2 + rw <= width)
        .then(|| format!("{}{}{right_styled}", left.1, " ".repeat(width - lw - rw)))
}

/// Joins pieces with `sep`, plain and styled alike.
fn join_pieces(pieces: &[Piece], sep: &str) -> Piece {
    let plain: Vec<&str> = pieces.iter().map(|(p, _)| p.as_str()).collect();
    let styled: Vec<&str> = pieces.iter().map(|(_, s)| s.as_str()).collect();
    (plain.join(sep), styled.join(sep))
}

/// A compact context-fill gauge, coloured by proximity to the compaction
/// trip point.
fn context_gauge(percent: f64, error_level: f64, warn_level: f64) -> Piece {
    const CELLS: usize = 8;
    let filled = (js_round(percent / 100.0 * CELLS as f64)).clamp(0.0, CELLS as f64) as usize;
    let fill = "▰".repeat(filled);
    let track = "▱".repeat(CELLS - filled);
    let color = if percent >= error_level {
        "error"
    } else if percent >= warn_level {
        "warning"
    } else {
        "accent"
    };
    let t = theme();
    (
        format!("{fill}{track}"),
        t.fg(color, &fill) + &t.fg("halftone", &track),
    )
}

/// Running and queued subagent rows. Queued used to be invisible: with five
/// slots and twelve dispatches the footer said "5 running" and gave no hint
/// that seven more were waiting behind them.
fn subagent_counts() -> (usize, usize) {
    let store = task_store();
    let mut counts = (0, 0);
    for task in store.list().iter() {
        if task.source != Some(TaskSource::Subagent) {
            continue;
        }
        match task.status {
            TaskStatus::InProgress => counts.0 += 1,
            TaskStatus::Pending => counts.1 += 1,
            _ => {}
        }
    }
    counts
}

/// `◇2 running · 1 queued`. Queued is included: a pool with five slots and
/// twelve dispatches is five running and seven waiting.
fn subagent_piece(running: usize, queued: usize) -> Piece {
    let t = theme();
    if queued > 0 {
        (
            format!("◇{running} running · {queued} queued"),
            t.fg("accent", &format!("◇{running}"))
                + &t.fg("dim", &format!(" running · {queued} queued")),
        )
    } else {
        (
            format!("◇{running} running"),
            t.fg("accent", &format!("◇{running}")) + &t.fg("dim", " running"),
        )
    }
}

/// Newlines, tabs and carriage returns become spaces; runs of spaces collapse.
fn sanitize_status_text(text: &str) -> String {
    let replaced: String = text
        .chars()
        .map(|c| {
            if matches!(c, '\r' | '\n' | '\t') {
                ' '
            } else {
                c
            }
        })
        .collect();
    let mut out = String::new();
    for c in replaced.chars() {
        if c == ' ' && out.ends_with(' ') {
            continue;
        }
        out.push(c);
    }
    out.trim().to_string()
}

/// One footer line for a transient progress entry.
fn render_startup_line(entry: &StartupProgress) -> String {
    let t = theme();
    match entry {
        StartupProgress::Error { label, message, .. } => {
            t.fg("dim", &format!("{label}: {message}"))
        }
        StartupProgress::Download {
            label,
            received_bytes,
            total_bytes,
            ..
        } => format!(
            "{} {}",
            t.fg("text", label),
            render_download_progress(*received_bytes, *total_bytes, None)
        ),
        StartupProgress::Work {
            label,
            done,
            total,
            unit,
            ..
        } => {
            let ratio = if *total > 0 {
                *done as f64 / *total as f64
            } else {
                0.0
            };
            format!(
                "{} {}",
                t.fg("text", label),
                render_progress_bar(ratio, &format!("{done}/{total} {unit}"), None)
            )
        }
    }
}

/// `$HOME` (or `USERPROFILE`) replaced by `~`.
fn tilde(path: &str) -> String {
    let home = std::env::var("HOME")
        .ok()
        .filter(|h| !h.is_empty())
        .or_else(|| std::env::var("USERPROFILE").ok().filter(|h| !h.is_empty()));
    match home {
        Some(home) if path.starts_with(&home) => format!(
            "~{}",
            hoocode_tui_util::text_slice::suffix_from(path, home.len())
        ),
        _ => path.to_string(),
    }
}

/// Shorter forms of `path`, cut from the left at a `/`, longest first:
/// `~/github/hoocode` gives `…/hoocode`. Only forms narrower than `path`.
fn shortened_paths(path: &str) -> Vec<String> {
    let full = visible_width(path);
    path.match_indices('/')
        .filter_map(|(i, _)| path.get(i..))
        .map(|suffix| format!("…{suffix}"))
        .filter(|s| visible_width(s) < full)
        .collect()
}

/// `$USD` for a cost. Under a cent it keeps a third digit, so real spend
/// never reads `$0.00`.
fn format_cost(cost: f64) -> String {
    if cost > 0.0 && cost < 0.01 {
        js_to_fixed(cost, 3)
    } else {
        js_to_fixed(cost, 2)
    }
}

/// Width of the left column in both detailed rows: the mode chip and the dial
/// (row 1), the bar, percent, window and warning flag (row 2). Row 2's column
/// ends with the one-cell flag slot (`!` or a space), so both rows line up.
const LEFT_COLUMN: usize = 22;

/// The mode chip is padded to this width: the longest mode name.
const CHIP_WIDTH: usize = 5;

/// The window part, `of 1.0M`, padded to this width.
const WINDOW_WIDTH: usize = 7;

/// Provider prefixes dropped from a model id in the short layout.
const PROVIDER_PREFIXES: &[&str] = &[
    "claude-",
    "gpt-",
    "gemini-",
    "mistral-",
    "llama-",
    "deepseek-",
    "grok-",
    "qwen-",
];

/// The model id without a known provider prefix: `claude-opus-5-5` gives
/// `opus-5-5`.
fn short_model_name(id: &str) -> &str {
    PROVIDER_PREFIXES
        .iter()
        .find_map(|prefix| id.strip_prefix(prefix))
        .filter(|rest| !rest.is_empty())
        .unwrap_or(id)
}

/// Thinking levels in the short layout. `high`, `low` and `off` are unchanged.
fn short_level(level: &str) -> &str {
    match level {
        "medium" => "med",
        "minimal" => "min",
        other => other,
    }
}

/// `text` padded on the right with spaces to `width` cells.
fn pad_plain(text: &str, width: usize) -> String {
    format!(
        "{text}{}",
        " ".repeat(width.saturating_sub(visible_width(text)))
    )
}

/// `piece` padded on the right to `width` cells. The padding is unstyled.
fn pad_piece(piece: Piece, width: usize) -> Piece {
    let pad = " ".repeat(width.saturating_sub(visible_width(&piece.0)));
    (piece.0 + &pad, piece.1 + &pad)
}

/// Pieces with no gap between them.
fn cat_pieces(pieces: &[Piece]) -> Piece {
    join_pieces(pieces, "")
}

/// A single unstyled space: a gap or an empty flag slot.
fn space_piece() -> Piece {
    (" ".to_string(), " ".to_string())
}

/// The mode chip, padded to `CHIP_WIDTH`, in the chip style.
fn chip_piece(mode: &str) -> Piece {
    let t = theme();
    let text = pad_plain(mode, CHIP_WIDTH);
    let styled = if t.has_bg("brandBg") && t.has("brandText") {
        t.bg("brandBg", &t.bold(&t.fg("brandText", &text)))
    } else {
        t.bold(&t.fg("accent", &text))
    };
    (text, styled)
}

/// The percent, right-aligned in four cells: `  2%`, ` 88%`, `100%`.
fn percent_piece(text: &str, color: &str) -> Piece {
    let pad = " ".repeat(4usize.saturating_sub(visible_width(text)));
    (
        format!("{pad}{text}"),
        format!("{pad}{}", theme().fg(color, text)),
    )
}

/// What row 1 can show, before any dropping.
struct Line1Data {
    name: String,
    /// The path, `~`-shortened.
    path: String,
    branch: Option<String>,
    session: Option<String>,
    /// Detailed form: `◇2 running · 1 queued`.
    subagents: Option<Piece>,
    mode: String,
    dial: String,
}

/// Which optional parts of row 1 are on. The mode chip and the dial are
/// always on.
#[derive(Debug, Clone, PartialEq)]
struct Line1Parts {
    /// The path as shown (full or shortened); `None` drops it.
    path: Option<String>,
    session: bool,
    subagents: bool,
    branch: bool,
}

/// Row 1 candidates, most to least. Order of dropping: the path (shortened
/// from the left first), the session, subagents, the branch. The mode chip,
/// the dial and the folder name are never dropped.
fn line1_candidates(d: &Line1Data) -> Vec<Line1Parts> {
    let mut p = Line1Parts {
        path: None,
        session: d.session.is_some(),
        subagents: d.subagents.is_some(),
        branch: d.branch.is_some(),
    };
    let mut out = Vec::new();
    if !d.path.is_empty() {
        out.push(Line1Parts {
            path: Some(d.path.clone()),
            ..p.clone()
        });
        out.extend(shortened_paths(&d.path).into_iter().map(|s| Line1Parts {
            path: Some(s),
            ..p.clone()
        }));
    }
    out.push(p.clone());
    p.session = false;
    out.push(p.clone());
    p.subagents = false;
    out.push(p.clone());
    p.branch = false;
    out.push(p);
    out
}

/// Row 1 with the parts in `p`: the left part (mode chip, dial and the
/// identity, padded to `LEFT_COLUMN`) and the right part.
fn row1_parts(d: &Line1Data, p: &Line1Parts) -> (Piece, Option<Piece>) {
    let t = theme();
    let mut identity = (d.name.clone(), t.bold(&t.fg("text", &d.name)));
    if let Some(branch) = d.branch.as_ref().filter(|_| p.branch) {
        identity.0 += &format!("  {GIT_BRANCH_GLYPH} {branch}");
        identity.1 += &format!(
            "  {} {}",
            t.fg("dim", GIT_BRANCH_GLYPH),
            t.fg("muted", branch)
        );
    }
    if let Some(session) = d.session.as_ref().filter(|_| p.session) {
        identity.0 += &format!(" • {session}");
        identity.1 += &t.fg("dim", &format!(" • {session}"));
    }
    if let Some(path) = &p.path {
        identity.0 += &format!("  {path}");
        identity.1 += &format!("  {}", t.fg("dim", path));
    }
    let head = pad_piece(
        join_pieces(
            &[chip_piece(&d.mode), (d.dial.clone(), t.fg("dim", &d.dial))],
            " · ",
        ),
        LEFT_COLUMN,
    );
    let left = cat_pieces(&[head, identity]);
    let right = d.subagents.clone().filter(|_| p.subagents);
    (left, right)
}

/// Row 1 as text, if the candidate fits `width`.
fn fit_line1(width: usize, d: &Line1Data, p: &Line1Parts) -> Option<String> {
    let (left, right) = row1_parts(d, p);
    fit_left_right(width, &left, right.as_ref())
}

/// What row 2 can show, before any dropping.
struct Line2Data {
    /// `(provider) ` before the model, when more than one provider is set up.
    provider: Option<Piece>,
    /// Model id and effort.
    model: Piece,
    /// The bar and the percent: `▰▰▰▱▱▱▱▱ 34%`, no flag.
    gauge: Piece,
    /// `!` at the warning threshold.
    flag: Option<Piece>,
    /// `of 200k`, padded to `WINDOW_WIDTH`.
    window: Option<Piece>,
    /// `↑48k ↓3.1k`.
    io: Vec<Piece>,
    /// `cache 120k/50k`.
    cache: Option<Piece>,
    cost: Option<Piece>,
}

/// Which optional parts of row 2 are on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Line2Parts {
    provider: bool,
    window: bool,
    cache: bool,
    io: bool,
    cost: bool,
}

/// Row 2 candidates, most to least. Order of dropping: provider prefix,
/// cache, the ↑↓ tokens, cost, window. Model and gauge are never dropped.
fn line2_candidates(d: &Line2Data) -> Vec<Line2Parts> {
    let mut p = Line2Parts {
        provider: d.provider.is_some(),
        window: d.window.is_some(),
        cache: d.cache.is_some(),
        io: !d.io.is_empty(),
        cost: d.cost.is_some(),
    };
    let mut out = vec![p];
    p.provider = false;
    out.push(p);
    p.cache = false;
    out.push(p);
    p.io = false;
    out.push(p);
    p.cost = false;
    out.push(p);
    p.window = false;
    out.push(p);
    out
}

/// Row 2 with the parts in `p`, as plain and styled text.
///
/// With the window, the left column is `bar  NN% of 1.0M!` padded to
/// `LEFT_COLUMN`; its last cell is the flag slot. One space follows it, then
/// the model and the other groups, three spaces apart. Without the window,
/// the flag follows the percent, and the column is still padded to
/// `LEFT_COLUMN`, so the model starts in the same cell.
fn build_line2(d: &Line2Data, p: &Line2Parts) -> Piece {
    let left = match (&d.window, p.window) {
        (Some(window), true) => {
            let slot = d.flag.clone().unwrap_or_else(space_piece);
            pad_piece(
                cat_pieces(&[d.gauge.clone(), space_piece(), window.clone(), slot]),
                LEFT_COLUMN,
            )
        }
        _ => pad_piece(
            cat_pieces(&[d.gauge.clone(), d.flag.clone().unwrap_or_default()]),
            LEFT_COLUMN,
        ),
    };
    let model = match (&d.provider, p.provider) {
        (Some((pp, ps)), true) => (format!("{pp}{}", d.model.0), format!("{ps}{}", d.model.1)),
        _ => d.model.clone(),
    };
    let mut groups = vec![model];
    if p.io && !d.io.is_empty() {
        groups.push(join_pieces(&d.io, " "));
    }
    if p.cache {
        groups.extend(d.cache.clone());
    }
    if p.cost {
        groups.extend(d.cost.clone());
    }
    cat_pieces(&[left, space_piece(), join_pieces(&groups, "   ")])
}

/// Row 2 as text, if the candidate fits `width`.
fn fit_line2(width: usize, d: &Line2Data, p: &Line2Parts) -> Option<String> {
    let build = build_line2(d, p);
    (visible_width(&build.0) <= width).then_some(build.1)
}

/// The short layout's parts, before any dropping. One row:
/// `BUILD  ▱▱▱▱▱▱▱▱   2%  ⑂ main  opus-5-5 · med  hoocode  $0.30  ◌ radar  ◇2`.
struct ShortData {
    chip: Piece,
    gauge: Piece,
    branch: Option<Piece>,
    model: Piece,
    name: Piece,
    cost: Option<Piece>,
    dial: Piece,
    subagents: Option<Piece>,
}

/// Which optional parts of the short layout are on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ShortParts {
    subagents: bool,
    cost: bool,
    name: bool,
    model: bool,
    branch: bool,
    dial: bool,
}

/// Short candidates, most to least. Order of dropping: subagents, cost, the
/// folder name, the model, the branch, the dial. The mode chip and the gauge
/// are never dropped.
fn short_candidates(d: &ShortData) -> Vec<ShortParts> {
    let mut p = ShortParts {
        subagents: d.subagents.is_some(),
        cost: d.cost.is_some(),
        name: true,
        model: true,
        branch: d.branch.is_some(),
        dial: true,
    };
    let mut out = vec![p];
    p.subagents = false;
    out.push(p);
    p.cost = false;
    out.push(p);
    p.name = false;
    out.push(p);
    p.model = false;
    out.push(p);
    p.branch = false;
    out.push(p);
    p.dial = false;
    out.push(p);
    out
}

/// The short layout with the parts in `p`: two spaces between groups.
fn build_short(d: &ShortData, p: &ShortParts) -> Piece {
    let mut groups = vec![d.chip.clone(), d.gauge.clone()];
    if p.branch {
        groups.extend(d.branch.clone());
    }
    if p.model {
        groups.push(d.model.clone());
    }
    if p.name {
        groups.push(d.name.clone());
    }
    if p.cost {
        groups.extend(d.cost.clone());
    }
    if p.dial {
        groups.push(d.dial.clone());
    }
    if p.subagents {
        groups.extend(d.subagents.clone());
    }
    join_pieces(&groups, "  ")
}

/// `◇2` for the short layout, `◇2 · 1 queued` when some wait.
fn subagent_short_piece(running: usize, queued: usize) -> Piece {
    let t = theme();
    let head = format!("◇{running}");
    if queued > 0 {
        let tail = format!(" · {queued} queued");
        (
            format!("{head}{tail}"),
            t.fg("accent", &head) + &t.fg("dim", &tail),
        )
    } else {
        (head.clone(), t.fg("accent", &head))
    }
}

pub struct FooterComponent {
    source: Box<dyn FooterSource>,
    data: FooterDataProvider,
    auto_compact_enabled: bool,
    tool_output_view: ToolOutputView,
    session_chip_shown: bool,
    density: FooterDensity,
    /// A warning (memory shedding, UI stall). Not a row: see `transient_lines`.
    notice: Option<String>,
}

impl FooterComponent {
    pub fn new(source: Box<dyn FooterSource>, data: FooterDataProvider) -> Self {
        Self {
            source,
            data,
            auto_compact_enabled: true,
            tool_output_view: ToolOutputView::Peek,
            session_chip_shown: false,
            density: FooterDensity::Full,
            notice: None,
        }
    }

    /// Sets or clears the warning. `None` removes it.
    pub fn set_notice(&mut self, notice: Option<String>) {
        self.notice = notice;
    }

    pub fn set_source(&mut self, source: Box<dyn FooterSource>) {
        self.source = source;
    }

    pub fn set_auto_compact_enabled(&mut self, enabled: bool) {
        self.auto_compact_enabled = enabled;
    }

    /// The view dial's current stop.
    pub fn set_tool_output_view(&mut self, view: ToolOutputView) {
        self.tool_output_view = view;
    }

    /// `Line` is the short layout: one row.
    pub fn set_density(&mut self, density: FooterDensity) {
        self.density = density;
    }

    /// Whether the input box carries the session chip (row 1 then drops the
    /// name, when the chip fits).
    pub fn set_session_chip_shown(&mut self, shown: bool) {
        self.session_chip_shown = shown;
    }

    /// The transient messages, not part of the footer's two rows: the
    /// warning, then extension statuses, then startup progress. Each is at
    /// most `width` cells. Show them in the notification area.
    pub fn transient_lines(&self, width: usize) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(notice) = &self.notice {
            lines.push(truncate_to_width(
                &theme().fg("warning", notice),
                width,
                &theme().fg("dim", "..."),
                false,
            ));
        }
        let statuses = self.data.get_extension_statuses();
        if !statuses.is_empty() {
            let line = statuses
                .values()
                .map(|text| sanitize_status_text(text))
                .collect::<Vec<_>>()
                .join(" ");
            lines.push(truncate_to_width(
                &line,
                width,
                &theme().fg("dim", "..."),
                false,
            ));
        }
        for entry in startup_progress::list() {
            lines.push(truncate_to_width(
                &render_startup_line(&entry),
                width,
                &theme().fg("dim", "…"),
                false,
            ));
        }
        lines
    }

    fn render_lines(&self, width: usize) -> Vec<String> {
        let t = theme();
        let source = &self.source;
        let (total_input, total_output, total_cache_read, total_cache_write, total_cost) =
            source.usage_totals();
        let model = source.model();

        // ── Shared: identity & location
        let cwd = source.cwd();
        let name = Path::new(&cwd)
            .file_name()
            .map_or_else(|| cwd.clone(), |n| n.to_string_lossy().into_owned());
        let session_name = if self.session_chip_shown && session_chip_fits(width) {
            None
        } else {
            Some(source.display_name()).filter(|n| !n.is_empty())
        };
        let (n_sub, n_queued) = subagent_counts();
        let has_subagents = n_sub > 0 || n_queued > 0;
        let view = self.tool_output_view;
        let dial = format!("{} {}", tool_output_view_glyph(view), view.as_str());
        let mode = self.data.get_active_mode().to_uppercase();
        let branch = self.data.get_git_branch();

        // ── Shared: context gauge
        let context_usage = source.context_usage();
        let context_window = context_usage
            .map(|(w, _)| w)
            .or_else(|| model.as_ref().map(|m| m.context_window))
            .unwrap_or(0);
        let context_percent_value = context_usage.and_then(|(_, p)| p).unwrap_or(0.0);
        let mut threshold_percent: Option<f64> = None;
        if self.auto_compact_enabled && context_window > 0 {
            let effective = context_window as f64 - source.reserve_tokens() as f64;
            if effective > 0.0 {
                threshold_percent = Some(effective / context_window as f64 * 100.0);
            }
        }
        let error_level = threshold_percent.map_or(90.0, |p| p - 3.0);
        let warn_level = threshold_percent.map_or(70.0, |p| p - 10.0);

        let pct_text = match context_usage {
            Some((_, None)) => "?".to_string(),
            _ => format!("{}%", js_to_fixed(context_percent_value, 0)),
        };
        let pct_color = if context_percent_value >= error_level {
            "error"
        } else if context_percent_value >= warn_level {
            "warning"
        } else {
            "muted"
        };
        // The `!` is the warning cue that does not depend on colour. It shows
        // where the bar changes colour.
        let flag =
            (context_percent_value >= warn_level).then(|| ("!".to_string(), t.fg(pct_color, "!")));
        let (bar_plain, bar_styled) = context_gauge(context_percent_value, error_level, warn_level);
        let gauge = cat_pieces(&[
            (bar_plain, bar_styled),
            space_piece(),
            percent_piece(&pct_text, pct_color),
        ]);
        let window = (context_window > 0).then(|| {
            let text = pad_plain(
                &format!("of {}", format_tokens(context_window)),
                WINDOW_WIDTH,
            );
            (text.clone(), t.fg("dim", &text))
        });

        // ── Shared: usage
        let arrow = |a: &str, n: u64| {
            (
                format!("{a}{}", format_tokens(n)),
                t.fg("dim", a) + &t.fg("muted", &format_tokens(n)),
            )
        };
        let mut io = Vec::new();
        if total_input > 0 {
            io.push(arrow("↑", total_input));
        }
        if total_output > 0 {
            io.push(arrow("↓", total_output));
        }
        let cache = (total_cache_read > 0 || total_cache_write > 0).then(|| {
            let figures = format!(
                "{}/{}",
                format_tokens(total_cache_read),
                format_tokens(total_cache_write)
            );
            (
                format!("cache {figures}"),
                t.fg("dim", "cache ") + &t.fg("muted", &figures),
            )
        });
        let using_subscription = model.is_some() && source.is_using_oauth();
        let cost = (total_cost != 0.0 || using_subscription).then(|| {
            let text = format!(
                "${}{}",
                format_cost(total_cost),
                if using_subscription { " sub" } else { "" }
            );
            (text.clone(), t.fg("muted", &text))
        });

        // ── Shared: model
        let model_name = model
            .as_ref()
            .map(|m| m.id.clone())
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| "no-model".into());
        let level = model.as_ref().filter(|m| m.reasoning).map(|_| {
            let level = source.thinking_level();
            if level.is_empty() {
                "off".to_string()
            } else {
                level
            }
        });
        let provider = model
            .as_ref()
            .filter(|_| self.data.get_available_provider_count() > 1)
            .map(|m| {
                (
                    format!("({}) ", m.provider),
                    t.fg("dim", &format!("({}) ", m.provider)),
                )
            });

        match self.density {
            FooterDensity::Full => {
                // ── Row 1 — identity & location
                let d1 = Line1Data {
                    name: name.clone(),
                    path: tilde(&cwd),
                    branch,
                    session: session_name,
                    subagents: has_subagents.then(|| subagent_piece(n_sub, n_queued)),
                    mode: mode.clone(),
                    dial: dial.clone(),
                };
                let line1 = line1_candidates(&d1)
                    .iter()
                    .find_map(|p| fit_line1(width, &d1, p))
                    .unwrap_or_else(|| {
                        let (left, right) = row1_parts(
                            &d1,
                            &Line1Parts {
                                path: None,
                                session: false,
                                subagents: false,
                                branch: false,
                            },
                        );
                        let all = match right {
                            Some(r) => cat_pieces(&[left, ("  ".into(), "  ".into()), r]),
                            None => left,
                        };
                        truncate_to_width(&all.1, width, &t.fg("dim", "…"), false)
                    });

                // ── Row 2 — session vitals
                let model_piece = {
                    let mut piece = (model_name.clone(), t.fg("muted", &model_name));
                    if let Some(level) = &level {
                        let text = if level == "off" {
                            "thinking off".to_string()
                        } else {
                            level.clone()
                        };
                        piece.0 += &format!(" · {text}");
                        piece.1 += &t.fg("dim", &format!(" · {text}"));
                    }
                    piece
                };
                let d2 = Line2Data {
                    provider,
                    model: model_piece,
                    gauge: gauge.clone(),
                    flag: flag.clone(),
                    window,
                    io,
                    cache,
                    cost,
                };
                let candidates = line2_candidates(&d2);
                let line2 = candidates
                    .iter()
                    .find_map(|p| fit_line2(width, &d2, p))
                    .unwrap_or_else(|| {
                        let last = candidates.last().copied().unwrap_or(Line2Parts {
                            provider: false,
                            window: false,
                            cache: false,
                            io: false,
                            cost: false,
                        });
                        truncate_to_width(
                            &build_line2(&d2, &last).1,
                            width,
                            &t.fg("dim", "…"),
                            false,
                        )
                    });

                vec![line1, line2]
            }
            FooterDensity::Line => {
                let model_piece = {
                    let short = short_model_name(&model_name);
                    let mut piece = (short.to_string(), t.fg("muted", short));
                    if let Some(level) = &level {
                        let level = short_level(level);
                        piece.0 += &format!(" · {level}");
                        piece.1 += &t.fg("dim", &format!(" · {level}"));
                    }
                    piece
                };
                let cost_short = cost.clone();
                let d = ShortData {
                    chip: chip_piece(&mode),
                    gauge: cat_pieces(&[gauge, flag.clone().unwrap_or_default()]),
                    branch: branch.map(|b| {
                        (format!("{GIT_BRANCH_GLYPH} {b}"), {
                            t.fg("dim", GIT_BRANCH_GLYPH) + " " + &t.fg("muted", &b)
                        })
                    }),
                    model: model_piece,
                    name: (name.clone(), t.bold(&t.fg("text", &name))),
                    cost: cost_short,
                    dial: (dial.clone(), t.fg("dim", &dial)),
                    subagents: has_subagents.then(|| subagent_short_piece(n_sub, n_queued)),
                };
                let line = short_candidates(&d)
                    .iter()
                    .find_map(|p| {
                        let (plain, styled) = build_short(&d, p);
                        (visible_width(&plain) <= width).then_some(styled)
                    })
                    .unwrap_or_else(|| {
                        let last = short_candidates(&d).last().copied().unwrap_or(ShortParts {
                            subagents: false,
                            cost: false,
                            name: false,
                            model: false,
                            branch: false,
                            dial: false,
                        });
                        truncate_to_width(
                            &build_short(&d, &last).1,
                            width,
                            &t.fg("dim", "…"),
                            false,
                        )
                    });
                vec![line]
            }
        }
    }
}

impl Component for FooterComponent {
    fn render(&mut self, width: u16) -> Vec<String> {
        self.render_lines(width as usize)
    }
}

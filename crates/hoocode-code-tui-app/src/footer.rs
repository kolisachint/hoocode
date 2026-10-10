//! The footer (`components/footer.ts`): always two rows.
//!
//! Row 1 is identity and location: the folder name (never dropped), the
//! branch, the session name, the path; on the right, subagents, the mode chip
//! and the view dial. Row 2 is session vitals: model and effort, the context
//! gauge with its percent (never dropped), the window, tokens, cost.
//!
//! When the width is short, parts go in a fixed order (see
//! `line1_candidates` and `line2_candidates`). Transient messages (the
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

/// Which layout the footer uses. Both are two rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FooterDensity {
    #[default]
    Full,
    /// Compact: row 1 drops the path and session; row 2 keeps only the
    /// model, gauge and percent. The folder name stays on both layouts.
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

/// What row 1 can show, before any dropping.
struct Line1Data {
    name: String,
    /// The path, `~`-shortened.
    path: String,
    branch: Option<String>,
    session: Option<String>,
    subagents: Option<Piece>,
    mode: String,
    dial: String,
}

/// One candidate for row 1: which optional parts are on.
#[derive(Debug, Clone, PartialEq)]
struct Line1Parts {
    /// The path as shown (full or shortened); `None` drops it.
    path: Option<String>,
    session: bool,
    subagents: bool,
    branch: bool,
    dial: bool,
    chip: bool,
}

/// Row 1 candidates, most to least. Order of dropping: the path (shortened
/// from the left first), the session, subagents, the branch, the dial, the
/// mode chip. The folder name is always there; the last candidate is the
/// name alone.
fn line1_candidates(d: &Line1Data, compact: bool) -> Vec<Line1Parts> {
    let mut p = Line1Parts {
        path: None,
        session: d.session.is_some() && !compact,
        subagents: d.subagents.is_some(),
        branch: d.branch.is_some(),
        dial: true,
        chip: true,
    };
    let mut out = Vec::new();
    if !compact && !d.path.is_empty() {
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
    out.push(p.clone());
    p.dial = false;
    out.push(p.clone());
    p.chip = false;
    out.push(p);
    out
}

/// Row 1 as text, if the candidate fits `width`.
fn fit_line1(width: usize, d: &Line1Data, p: &Line1Parts) -> Option<String> {
    let t = theme();
    let mut left = (d.name.clone(), t.bold(&t.fg("text", &d.name)));
    if let Some(branch) = d.branch.as_ref().filter(|_| p.branch) {
        left.0 += &format!("  {GIT_BRANCH_GLYPH} {branch}");
        left.1 += &format!(
            "  {} {}",
            t.fg("dim", GIT_BRANCH_GLYPH),
            t.fg("muted", branch)
        );
    }
    if let Some(session) = d.session.as_ref().filter(|_| p.session) {
        left.0 += &format!(" • {session}");
        left.1 += &t.fg("dim", &format!(" • {session}"));
    }
    if let Some(path) = &p.path {
        left.0 += &format!("  {path}");
        left.1 += &format!("  {}", t.fg("dim", path));
    }

    let mut groups: Vec<Piece> = Vec::new();
    if let Some(sub) = d.subagents.as_ref().filter(|_| p.subagents) {
        groups.push(sub.clone());
    }
    let mut mode: Vec<Piece> = Vec::new();
    if p.chip {
        let chip_styled = if t.has_bg("brandBg") && t.has("brandText") {
            t.bg("brandBg", &t.bold(&t.fg("brandText", &d.mode)))
        } else {
            t.bold(&t.fg("accent", &d.mode))
        };
        mode.push((d.mode.clone(), chip_styled));
    }
    if p.dial {
        mode.push((d.dial.clone(), t.fg("dim", &d.dial)));
    }
    if !mode.is_empty() {
        groups.push(join_pieces(&mode, " · "));
    }
    let right = (!groups.is_empty()).then(|| join_pieces(&groups, "  "));
    fit_left_right(width, &left, right.as_ref())
}

/// What row 2 can show, before any dropping.
struct Line2Data {
    /// `(provider) ` before the model, when more than one provider is set up.
    provider: Option<Piece>,
    /// Model id and effort.
    model: Piece,
    /// The bar and the percent.
    gauge: Piece,
    /// ` of 200k`.
    window: Option<Piece>,
    /// `↑48k ↓3.1k`.
    io: Vec<Piece>,
    /// `R120k W50`.
    cache: Vec<Piece>,
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
/// cache (R/W), the ↑↓ tokens, cost, window. Model and percent are never
/// dropped.
fn line2_candidates(d: &Line2Data, compact: bool) -> Vec<Line2Parts> {
    if compact {
        return vec![Line2Parts {
            provider: false,
            window: false,
            cache: false,
            io: false,
            cost: false,
        }];
    }
    let mut p = Line2Parts {
        provider: d.provider.is_some(),
        window: d.window.is_some(),
        cache: !d.cache.is_empty(),
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

/// Row 2 as styled text, if the candidate fits `width`.
fn fit_line2(width: usize, d: &Line2Data, p: &Line2Parts) -> Option<String> {
    let build = build_line2(d, p);
    (visible_width(&build.0) <= width).then_some(build.1)
}

/// Row 2 with the parts in `p`: plain and styled.
fn build_line2(d: &Line2Data, p: &Line2Parts) -> Piece {
    let model = match (&d.provider, p.provider) {
        (Some((pp, ps)), true) => (format!("{pp}{}", d.model.0), format!("{ps}{}", d.model.1)),
        _ => d.model.clone(),
    };
    let gauge = match (&d.window, p.window) {
        (Some((wp, ws)), true) => (format!("{}{wp}", d.gauge.0), format!("{}{ws}", d.gauge.1)),
        _ => d.gauge.clone(),
    };
    let mut tokens: Vec<Piece> = Vec::new();
    if p.io {
        tokens.extend(d.io.iter().cloned());
    }
    if p.cache {
        tokens.extend(d.cache.iter().cloned());
    }
    let (mut plain, mut styled) = join_pieces(&[model, gauge], "   ");
    if !tokens.is_empty() {
        let (tp, ts) = join_pieces(&tokens, " ");
        plain += &format!("   {tp}");
        styled += &format!("   {ts}");
    }
    if let Some((cp, cs)) = d.cost.as_ref().filter(|_| p.cost) {
        plain += &format!("  {cp}");
        styled += &format!("  {cs}");
    }
    (plain, styled)
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

    /// `Line` is the compact layout: still two rows, with the path and session
    /// left out of row 1 and the tokens and cost left out of row 2.
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
        let compact = self.density == FooterDensity::Line;
        let (total_input, total_output, total_cache_read, total_cache_write, total_cost) =
            source.usage_totals();
        let model = source.model();

        // ── Row 1 — identity & location
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
        let view = self.tool_output_view;
        let d1 = Line1Data {
            name,
            path: tilde(&cwd),
            branch: self.data.get_git_branch(),
            session: session_name,
            subagents: (n_sub > 0 || n_queued > 0).then(|| subagent_piece(n_sub, n_queued)),
            mode: self.data.get_active_mode().to_uppercase(),
            dial: format!("{} {}", tool_output_view_glyph(view), view.as_str()),
        };
        let line1 = line1_candidates(&d1, compact)
            .iter()
            .find_map(|p| fit_line1(width, &d1, p))
            .unwrap_or_else(|| {
                truncate_to_width(
                    &t.bold(&t.fg("text", &d1.name)),
                    width,
                    &t.fg("dim", "…"),
                    false,
                )
            });

        // ── Row 2 — session vitals
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
        let (bar_plain, bar_styled) = context_gauge(context_percent_value, error_level, warn_level);
        let gauge = (
            format!("{bar_plain} {pct_text}"),
            format!("{bar_styled} {}", t.fg(pct_color, &pct_text)),
        );
        let window = (context_window > 0).then(|| {
            let text = format!(" of {}", format_tokens(context_window));
            (text.clone(), t.fg("dim", &text))
        });

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
        let mut cache = Vec::new();
        if total_cache_read > 0 {
            cache.push(arrow("R", total_cache_read));
        }
        if total_cache_write > 0 {
            cache.push(arrow("W", total_cache_write));
        }
        let using_subscription = model.is_some() && source.is_using_oauth();
        let cost = (total_cost != 0.0 || using_subscription).then(|| {
            let text = format!(
                "${}{}",
                format_cost(total_cost),
                if using_subscription { " (sub)" } else { "" }
            );
            (text.clone(), t.fg("muted", &text))
        });

        let model_name = model
            .as_ref()
            .map(|m| m.id.clone())
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| "no-model".into());
        let mut model_piece = (model_name.clone(), t.fg("muted", &model_name));
        if model.as_ref().is_some_and(|m| m.reasoning) {
            let level = source.thinking_level();
            let level = if level.is_empty() {
                "off".to_string()
            } else {
                level
            };
            let text = if level == "off" {
                "thinking off".to_string()
            } else {
                level
            };
            model_piece.0 += &format!(" • {text}");
            model_piece.1 += &t.fg("dim", &format!(" • {text}"));
        }
        let provider = model
            .as_ref()
            .filter(|_| self.data.get_available_provider_count() > 1)
            .map(|m| {
                (
                    format!("({}) ", m.provider),
                    t.fg("dim", &format!("({}) ", m.provider)),
                )
            });

        let d2 = Line2Data {
            provider,
            model: model_piece,
            gauge,
            window,
            io,
            cache,
            cost,
        };
        let candidates = line2_candidates(&d2, compact);
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
                truncate_to_width(&build_line2(&d2, &last).1, width, &t.fg("dim", "…"), false)
            });

        vec![line1, line2]
    }
}

impl Component for FooterComponent {
    fn render(&mut self, width: u16) -> Vec<String> {
        self.render_lines(width as usize)
    }
}

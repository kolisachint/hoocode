//! The Mission Control HUD (M3): the one-row HUD replacing the two-row footer.
//!
//! The row is drawn with ratatui (`RatatuiComponent`). The layout is built
//! from styled runs ([`hud_runs`], pure, tested here); [`hud_text`] is the
//! plain concatenation the width math is checked against, and
//! [`hud_component`] maps each run's palette token to a style.
//!
//! Left: `◭ HOOCODE`, the mode chip (`BUILD` accent, `PLAN` violet), the
//! folder in dim text. Right, in order: `◈ N agent` (violet, while agents
//! run), `⌁ N sat` (accent, while satellites are up), `SHIP` and the ship
//! name, `FUEL` with 8 cells and the percent, `CR` and the spend.
//!
//! Narrow (80 columns, spec r10): the folder goes first, then the ship. Mode,
//! fuel and credits stay. Past that the left part is cut first, with `…` at
//! its end; the right part is cut from the right.

use std::path::Path;

use hoocode_code_agent_session::format::js_to_fixed;
use hoocode_code_task_store::{task_store, TaskSource, TaskStatus};
use hoocode_tui_ratatui::core::buffer::Buffer;
use hoocode_tui_ratatui::core::layout::Rect;
use hoocode_tui_ratatui::core::text::{Line, Span};
use hoocode_tui_ratatui::{Palette, RatatuiComponent};
use hoocode_tui_util::visible_width;

use crate::footer::FooterSource;
use crate::footer_data::FooterDataProvider;
use crate::hull_paint::HullPaint;

/// The fields the HUD row shows.
#[derive(Debug, Clone, PartialEq)]
pub struct HudData {
    /// The active mode, upper case: `BUILD`, `PLAN`.
    pub mode: String,
    /// The working folder's name.
    pub folder: String,
    /// Running subagents (`◈ N agent`, hidden while none run).
    pub agents: usize,
    /// Active MCP servers (`⌁ N sat`, hidden while none are up).
    pub satellites: usize,
    /// The model id, or `none`.
    pub model: String,
    /// The context fill percent; `None` when unknown after a compaction.
    pub fuel: Option<f64>,
    /// The session cost, already formatted: `$1.23`.
    pub cost: String,
    /// The hull paint (`/color`, M8e5); `None` is unpainted.
    pub paint: Option<HullPaint>,
}

/// One styled run of the HUD row: plain text plus the palette token styling
/// it. The `""` token is the default style (the gaps between tokens).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HudRun {
    pub text: String,
    pub token: &'static str,
}

fn run(text: impl Into<String>, token: &'static str) -> HudRun {
    HudRun {
        text: text.into(),
        token,
    }
}

/// The two-space gap between tokens.
fn gap() -> HudRun {
    run("  ", "")
}

/// The palette token for the mark and the mode chip: accent for BUILD,
/// violet for PLAN.
fn mark_token(mode: &str) -> &'static str {
    if mode == "PLAN" {
        "vio"
    } else {
        "acc"
    }
}

/// The fuel cells: 8 wide, `▰` filled, `▱` track.
pub const FUEL_CELLS: usize = 8;

/// The fuel bar for `percent`: 8 cells. Unknown is an empty bar.
pub fn fuel_bar(percent: Option<f64>) -> String {
    let filled = percent.map_or(0, |p| {
        (p / 100.0 * FUEL_CELLS as f64)
            .round()
            .clamp(0.0, FUEL_CELLS as f64) as usize
    });
    format!("{}{}", "▰".repeat(filled), "▱".repeat(FUEL_CELLS - filled))
}

/// The palette token colouring the fuel bar and percent: accent below 70%,
/// warn below 90%, err from 90%. Unknown fuel is dim.
pub fn fuel_token(percent: Option<f64>) -> &'static str {
    match percent {
        Some(p) if p >= 90.0 => "err",
        Some(p) if p >= 70.0 => "warn",
        Some(_) => "acc",
        None => "dim",
    }
}

/// The fuel percent as text: `42%`, `?` when unknown.
pub fn fuel_text(percent: Option<f64>) -> String {
    match percent {
        Some(p) => format!("{}%", p.round() as i64),
        None => "?".to_string(),
    }
}

/// The left part: the mark, the mode chip, and the folder when `with_folder`.
fn left_runs(data: &HudData, with_folder: bool) -> Vec<HudRun> {
    let mark = mark_token(&data.mode);
    let mut runs = vec![run("◭ HOOCODE", mark), gap(), run(&data.mode, mark)];
    if with_folder {
        runs.push(gap());
        runs.push(run(&data.folder, "dim"));
    }
    runs
}

/// Pushes `text` with a two-space gap before it, unless `runs` is empty.
fn push_token(runs: &mut Vec<HudRun>, text: String, token: &'static str) {
    if !runs.is_empty() {
        runs.push(gap());
    }
    runs.push(run(text, token));
}

/// The right part: agents, satellites, and — when `with_ship` — the ship,
/// then fuel and credits.
fn right_runs(data: &HudData, with_ship: bool) -> Vec<HudRun> {
    let mut runs = Vec::new();
    if let Some(paint) = data.paint {
        push_token(&mut runs, paint.name().to_string(), paint.token());
    }
    if data.agents > 0 {
        push_token(&mut runs, format!("◈ {} agent", data.agents), "vio");
    }
    if data.satellites > 0 {
        push_token(&mut runs, format!("⌁ {} sat", data.satellites), "acc");
    }
    if with_ship {
        if !runs.is_empty() {
            runs.push(gap());
        }
        runs.push(run("SHIP ", "dim"));
        runs.push(run(data.model.clone(), "ink"));
    }
    push_token(&mut runs, "FUEL ".to_string(), "dim");
    let fuel = fuel_token(data.fuel);
    runs.push(run(fuel_bar(data.fuel), fuel));
    runs.push(run(" ".to_string(), ""));
    runs.push(run(fuel_text(data.fuel), fuel));
    push_token(&mut runs, "CR ".to_string(), "dim");
    runs.push(run(data.cost.clone(), "ink"));
    runs
}

/// The plain text of `runs`.
fn runs_text(runs: &[HudRun]) -> String {
    runs.iter().map(|r| r.text.as_str()).collect()
}

/// `left` flush left, `right` flush right, with the gap between. `None` when
/// they do not fit in `width`.
fn join_runs(left: &[HudRun], right: &[HudRun], width: usize) -> Option<Vec<HudRun>> {
    let (lw, rw) = (runs_width(left), runs_width(right));
    if lw + 2 + rw > width {
        return None;
    }
    let mut row = left.to_vec();
    row.push(run(" ".repeat(width - lw - rw), ""));
    row.extend_from_slice(right);
    Some(row)
}

fn runs_width(runs: &[HudRun]) -> usize {
    visible_width(&runs_text(runs))
}

/// `runs` cut to `width` columns from the right, with `…` at its end. Runs
/// that fit are kept whole; the run holding the cut keeps its token.
fn cut_runs_end(runs: &[HudRun], width: usize) -> Vec<HudRun> {
    if runs_width(runs) <= width {
        return runs.to_vec();
    }
    if width == 0 {
        return Vec::new();
    }
    let budget = width - 1; // One cell for the `…`.
    let mut out = Vec::new();
    let mut used = 0;
    for r in runs {
        if used + visible_width(&r.text) <= budget {
            used += visible_width(&r.text);
            out.push(r.clone());
            continue;
        }
        let mut piece = String::new();
        for c in r.text.chars() {
            let w = visible_width(&c.to_string());
            if used + w > budget {
                break;
            }
            used += w;
            piece.push(c);
        }
        if !piece.is_empty() {
            out.push(run(piece, r.token));
        }
        break;
    }
    out.push(run("…", ""));
    out
}

/// The HUD row as styled runs, exactly `width` columns when it fits. The
/// folder goes first when the row is short, then the ship; mode, fuel and
/// credits stay. Past that the left part is cut first, with `…` at its end;
/// the right part is cut from the right.
pub fn hud_runs(data: &HudData, width: usize) -> Vec<HudRun> {
    let right_full = right_runs(data, true);
    let right_slim = right_runs(data, false);
    let left_full = left_runs(data, true);
    let left_short = left_runs(data, false);
    if let Some(row) = join_runs(&left_full, &right_full, width) {
        return row;
    }
    if let Some(row) = join_runs(&left_short, &right_full, width) {
        return row;
    }
    if let Some(row) = join_runs(&left_short, &right_slim, width) {
        return row;
    }
    let right_width = runs_width(&right_slim);
    if right_width + 2 < width {
        // The right part fits on its own: cut the left part first, with `…`
        // at its end, and keep the two-space gap.
        let mut row = cut_runs_end(&left_short, width - 2 - right_width);
        row.push(gap());
        row.extend_from_slice(&right_slim);
        return row;
    }
    // Even the slim row does not fit: cut the whole row from the right.
    let mut row = left_short.clone();
    row.push(gap());
    row.extend_from_slice(&right_slim);
    cut_runs_end(&row, width)
}

/// The HUD row as text: [`hud_runs`] concatenated. Exactly `width` columns
/// when it fits (padded between the left and right parts), else cut with a
/// trailing `…`. Plain text only: no escape codes may reach the row.
pub fn hud_text(data: &HudData, width: usize) -> String {
    runs_text(&hud_runs(data, width))
}

/// Whether full-screen is on. `HOOCODE_FULLSCREEN` reads as a boolean
/// (`0`/`false`/`no`/`off` or `1`/`true`/`yes`/`on`, see `parse_env_bool`)
/// and wins over the `fullscreen` setting for one run. Any other value, or
/// none, takes the setting.
pub fn fullscreen_enabled(env: Option<&str>, setting: bool) -> bool {
    env.and_then(hoocode_code_paths::parse_env_bool)
        .unwrap_or(setting)
}

/// Running subagent rows. Queued rows are not shown: the HUD counts live
/// agents only.
fn running_agents() -> usize {
    task_store()
        .list()
        .iter()
        .filter(|t| t.source == Some(TaskSource::Subagent) && t.status == TaskStatus::InProgress)
        .count()
}

/// `$USD` for a cost. Under a cent it keeps a third digit, so real spend
/// never reads `$0.00`.
fn format_cost(cost: f64) -> String {
    if cost > 0.0 && cost < 0.01 {
        format!("${}", js_to_fixed(cost, 3))
    } else {
        format!("${}", js_to_fixed(cost, 2))
    }
}

/// The HUD's data, read from the session and the footer's data provider. The
/// satellite count comes from the session's MCP hub through [`FooterSource`]
/// (TODO(M3): none wired yet — the source default is 0).
pub fn read_hud(source: &dyn FooterSource, data: &FooterDataProvider) -> HudData {
    let (_, _, _, _, cost) = source.usage_totals();
    let cwd = source.cwd();
    let folder = Path::new(&cwd)
        .file_name()
        .map_or_else(|| cwd.clone(), |n| n.to_string_lossy().into_owned());
    // No usage at all reads as empty fuel; usage without a percent (after a
    // compaction) reads as unknown.
    let fuel = match source.context_usage() {
        None => Some(0.0),
        Some((_, percent)) => percent,
    };
    HudData {
        paint: data.get_hull_paint(),
        mode: data.get_active_mode().to_uppercase(),
        folder,
        agents: running_agents(),
        satellites: source.satellite_count(),
        model: source.model().map_or_else(|| "none".to_string(), |m| m.id),
        fuel,
        cost: format_cost(cost),
    }
}

/// The HUD as a one-row component. It reads the session on each render and
/// styles each run with the Mission Control palette.
pub fn hud_component(
    source: Box<dyn FooterSource>,
    data: FooterDataProvider,
) -> RatatuiComponent<impl FnMut(Rect, &mut Buffer)> {
    RatatuiComponent::new(1, move |area: Rect, buf: &mut Buffer| {
        let width = usize::from(area.width);
        let pal = Palette::current();
        let hud = read_hud(source.as_ref(), &data);
        // The hull tint replaces the row background while painted (M8e5).
        let tint = hud.paint.and_then(|paint| pal.hull_tint(paint.token()));
        let spans: Vec<Span<'static>> = hud_runs(&hud, width)
            .into_iter()
            .map(|r| {
                let style = pal.style(r.token);
                Span::styled(r.text, tint.map_or(style, |bg| style.bg(bg)))
            })
            .collect();
        buf.set_line(area.x, area.y, &Line::from(spans), area.width);
    })
}

/// The parts of the screen, in the order the app tree adds them. Full-screen
/// puts the HUD, the gap and the attach slot on top, the body (header and
/// chat) under them, and the rest below. The scroll bar has no part: `Tui`
/// draws it between the body and the bottom. Inline mode keeps the two-row
/// footer; full-screen replaces it with the HUD.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreePart {
    Hud,
    /// One blank row under the HUD.
    Gap,
    Header,
    /// A flex spacer that fills the screen (inline mode only).
    ScreenFill,
    Attach,
    Chat,
    /// Full-screen only: the body container, holding the header and the chat.
    Body,
    Pending,
    Status,
    WidgetAbove,
    Tasks,
    Notifications,
    FooterTransient,
    Editor,
    WidgetsBelow,
    /// Inline mode only: the two-row footer. Full-screen shows the HUD instead.
    Footer,
}

/// The app tree's children in order. With `fullscreen` off this is today's
/// inline tree, footer included. With `fullscreen` on the HUD replaces the
/// footer, so the plan leaves the footer slot out.
pub fn tree_plan(fullscreen: bool) -> Vec<TreePart> {
    use TreePart::*;
    let bottom = [
        Pending,
        Status,
        WidgetAbove,
        Tasks,
        Notifications,
        FooterTransient,
        Editor,
        WidgetsBelow,
    ];
    let mut plan = if fullscreen {
        vec![Hud, Gap, Attach, Body]
    } else {
        vec![Header, ScreenFill, Attach, Chat]
    };
    plan.extend(bottom);
    if !fullscreen {
        plan.push(Footer);
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> HudData {
        HudData {
            mode: "BUILD".into(),
            folder: "hoocode-workspace-checkout".into(),
            agents: 0,
            satellites: 0,
            model: "claude-haiku-5-5".into(),
            fuel: Some(42.0),
            cost: "$1.23".into(),
            paint: None,
        }
    }

    fn live() -> HudData {
        HudData {
            agents: 2,
            satellites: 1,
            ..data()
        }
    }

    #[test]
    fn env_zero_turns_fullscreen_off_whatever_the_setting() {
        assert!(!fullscreen_enabled(Some("0"), true));
        assert!(fullscreen_enabled(Some("1"), false));
        assert!(fullscreen_enabled(None, true));
        assert!(!fullscreen_enabled(None, false));
        assert!(fullscreen_enabled(Some("yes"), true));
    }

    #[test]
    fn env_reads_booleans_trimmed_and_any_case() {
        for off in ["0", "false", "No", " OFF "] {
            assert!(!fullscreen_enabled(Some(off), true), "{off:?}");
        }
        for on in ["1", "TRUE", "yes", "On "] {
            assert!(fullscreen_enabled(Some(on), false), "{on:?}");
        }
    }

    #[test]
    fn env_with_no_boolean_meaning_takes_the_setting() {
        assert!(fullscreen_enabled(Some("maybe"), true));
        assert!(!fullscreen_enabled(Some(""), false));
        assert!(fullscreen_enabled(Some("2"), true));
    }

    #[test]
    fn hud_100_cols_shows_all_fields() {
        let text = hud_text(&data(), 100);
        assert_eq!(visible_width(&text), 100);
        assert!(text.starts_with("◭ HOOCODE  BUILD  hoocode-workspace-checkout"));
        assert!(text.contains("SHIP claude-haiku-5-5"));
        assert!(text.contains("FUEL ▰▰▰▱▱▱▱▱ 42%"));
        assert!(text.ends_with("CR $1.23"));
    }

    #[test]
    fn hud_shows_running_agents_and_satellites_while_up() {
        let text = hud_text(&live(), 140);
        assert_eq!(visible_width(&text), 140);
        assert!(text.starts_with("◭ HOOCODE  BUILD  hoocode-workspace-checkout"));
        assert!(text.contains("◈ 2 agent"));
        assert!(text.contains("⌁ 1 sat"));
        assert!(text.ends_with("CR $1.23"));
    }

    #[test]
    fn hud_hides_agent_and_satellite_counts_while_none_run() {
        let text = hud_text(&data(), 100);
        assert!(!text.contains('◈'));
        assert!(!text.contains('⌁'));
    }

    #[test]
    fn hud_80_cols_drops_folder_first() {
        let text = hud_text(&data(), 80);
        assert_eq!(visible_width(&text), 80);
        assert!(!text.contains("hoocode-workspace-checkout"));
        assert!(text.starts_with("◭ HOOCODE  BUILD"));
        assert!(text.ends_with("SHIP claude-haiku-5-5  FUEL ▰▰▰▱▱▱▱▱ 42%  CR $1.23"));
    }

    #[test]
    fn hud_keeps_folder_when_it_fits_at_80_cols() {
        let mut short = data();
        short.folder = "hoocode".into();
        let text = hud_text(&short, 80);
        assert!(text.contains("hoocode"));
        assert_eq!(visible_width(&text), 80);
    }

    #[test]
    fn hud_narrow_drops_ship_but_keeps_mode_fuel_credits() {
        let text = hud_text(&data(), 60);
        assert_eq!(visible_width(&text), 60);
        assert!(!text.contains("SHIP"));
        assert!(text.starts_with("◭ HOOCODE  BUILD"));
        assert!(text.contains("FUEL ▰▰▰▱▱▱▱▱ 42%"));
        assert!(text.ends_with("CR $1.23"));
    }

    #[test]
    fn hud_cuts_the_left_part_first_with_ellipsis_at_its_end() {
        let text = hud_text(&data(), 30);
        assert_eq!(visible_width(&text), 30);
        assert!(text.starts_with('…'));
        assert!(text.contains("FUEL ▰▰▰▱▱▱▱▱ 42%"));
        assert!(text.ends_with("CR $1.23"));
    }

    #[test]
    fn hud_cuts_the_whole_row_from_the_right_when_nothing_fits() {
        let text = hud_text(&data(), 20);
        assert_eq!(visible_width(&text), 20);
        assert!(text.starts_with("◭ HOOCODE"));
        assert!(text.ends_with('…'));
    }

    #[test]
    fn hud_zero_width_is_empty() {
        assert_eq!(hud_text(&data(), 0), "");
    }

    #[test]
    fn fuel_bar_is_eight_cells() {
        assert_eq!(fuel_bar(Some(50.0)), "▰▰▰▰▱▱▱▱");
        assert_eq!(fuel_bar(Some(42.0)), "▰▰▰▱▱▱▱▱");
        assert_eq!(fuel_bar(Some(100.0)), "▰▰▰▰▰▰▰▰");
        assert_eq!(fuel_bar(Some(0.0)), "▱▱▱▱▱▱▱▱");
        assert_eq!(fuel_bar(None), "▱▱▱▱▱▱▱▱");
        assert_eq!(fuel_bar(Some(1000.0)), "▰▰▰▰▰▰▰▰");
    }

    #[test]
    fn fuel_token_colours_match_the_spec_bands() {
        assert_eq!(fuel_token(Some(50.0)), "acc");
        assert_eq!(fuel_token(Some(69.9)), "acc");
        assert_eq!(fuel_token(Some(70.0)), "warn");
        assert_eq!(fuel_token(Some(85.0)), "warn");
        assert_eq!(fuel_token(Some(89.9)), "warn");
        assert_eq!(fuel_token(Some(90.0)), "err");
        assert_eq!(fuel_token(Some(95.0)), "err");
        assert_eq!(fuel_token(None), "dim");
    }

    #[test]
    fn fuel_runs_carry_the_band_token() {
        let mut warm = data();
        warm.fuel = Some(85.0);
        let runs = hud_runs(&warm, 100);
        let bar = runs.iter().find(|r| r.text == "▰▰▰▰▰▰▰▱").unwrap();
        assert_eq!(bar.token, "warn");
        let pct = runs.iter().find(|r| r.text == "85%").unwrap();
        assert_eq!(pct.token, "warn");

        let mut hot = data();
        hot.fuel = Some(95.0);
        let runs = hud_runs(&hot, 100);
        assert!(runs.iter().any(|r| r.text == "95%" && r.token == "err"));

        let cool = data();
        let runs = hud_runs(&cool, 100);
        assert!(runs.iter().any(|r| r.text == "42%" && r.token == "acc"));
    }

    #[test]
    fn fuel_tokens_resolve_through_the_palette() {
        let pal = Palette::current();
        assert_eq!(pal.style(fuel_token(Some(50.0))), pal.acc());
        assert_eq!(pal.style(fuel_token(Some(85.0))), pal.warn());
        assert_eq!(pal.style(fuel_token(Some(95.0))), pal.err());
    }

    #[test]
    fn mode_chip_uses_accent_for_build_and_violet_for_plan() {
        let runs = hud_runs(&data(), 100);
        assert_eq!(runs[0].token, "acc");
        assert!(runs.iter().any(|r| r.text == "BUILD" && r.token == "acc"));

        let mut plan = data();
        plan.mode = "PLAN".into();
        let runs = hud_runs(&plan, 100);
        assert_eq!(runs[0].token, "vio");
        assert!(runs.iter().any(|r| r.text == "PLAN" && r.token == "vio"));
        assert!(hud_text(&plan, 100).contains("PLAN"));
    }

    #[test]
    fn right_tokens_carry_their_semantic_colours() {
        let runs = hud_runs(&live(), 100);
        assert!(runs
            .iter()
            .any(|r| r.text == "◈ 2 agent" && r.token == "vio"));
        assert!(runs.iter().any(|r| r.text == "⌁ 1 sat" && r.token == "acc"));
        assert!(runs.iter().any(|r| r.text == "SHIP " && r.token == "dim"));
        assert!(runs
            .iter()
            .any(|r| r.text == "claude-haiku-5-5" && r.token == "ink"));
    }
}

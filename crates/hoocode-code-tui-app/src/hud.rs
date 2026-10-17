//! The Mission Control HUD (M1c): the first row of the full-screen frame, and
//! the frame's tree plan. The HUD is a placeholder here: the real fields and
//! the narrow rules come in M3.
//!
//! The row is drawn with ratatui (`RatatuiComponent`). The text is built by
//! [`hud_text`], a pure function, so the layout is tested without a terminal.

use std::path::Path;

use hoocode_tui_ratatui::core::buffer::Buffer;
use hoocode_tui_ratatui::core::layout::Rect;
use hoocode_tui_ratatui::core::style::Style;
use hoocode_tui_ratatui::RatatuiComponent;
use hoocode_tui_util::visible_width;

use crate::footer::FooterSource;
use crate::footer_data::FooterDataProvider;

/// The fields the placeholder row shows. The footer already holds all of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HudData {
    /// The active mode, upper case: `BUILD`, `PLAN`.
    pub mode: String,
    /// The working folder's name.
    pub folder: String,
    /// The model id, or `none`.
    pub model: String,
    /// The context fill: `42%`, `?` when unknown.
    pub fuel: String,
    /// The session cost: `$1.23`.
    pub cost: String,
}

/// Whether full-screen is on. `HOOCODE_FULLSCREEN=0` turns it off, `=1` on, for
/// one run; any other value (or none) takes the `fullscreen` setting. The env
/// var wins over the setting.
pub fn fullscreen_enabled(env: Option<&str>, setting: bool) -> bool {
    match env {
        Some("0") => false,
        Some("1") => true,
        _ => setting,
    }
}

/// The HUD row as text, exactly `width` columns when it fits. The left part is
/// `◭ HOOCODE  <mode>  <folder>`, the right part `SHIP <model>  FUEL <fuel>  CR <cost>`.
/// The folder goes first when the row is short; past that the row is cut.
pub fn hud_text(data: &HudData, width: usize) -> String {
    let right = format!("SHIP {}  FUEL {}  CR {}", data.model, data.fuel, data.cost);
    let left_full = format!("◭ HOOCODE  {}  {}", data.mode, data.folder);
    let left_short = format!("◭ HOOCODE  {}", data.mode);
    let right_width = visible_width(&right);
    let fits = |left: &str| visible_width(left) + 2 + right_width <= width;
    let left = if fits(&left_full) {
        left_full
    } else {
        left_short
    };
    if !fits(&left) {
        return cut_plain(&format!("{left}  {right}"), width);
    }
    let gap = width - visible_width(&left) - right_width;
    format!("{left}{}{right}", " ".repeat(gap))
}

/// `text` cut to `width` columns with a trailing `…`. Plain text only: the
/// row is drawn cell by cell, so no escape codes may reach it.
fn cut_plain(text: &str, width: usize) -> String {
    if visible_width(text) <= width {
        return text.to_string();
    }
    let budget = width.saturating_sub(1);
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let piece = c.to_string();
        let w = visible_width(&piece);
        if used + w > budget {
            break;
        }
        used += w;
        out.push_str(&piece);
    }
    if width > 0 {
        out.push('…');
    }
    out
}

/// The HUD's data, read from the session and the footer's data provider.
pub fn read_hud(source: &dyn FooterSource, data: &FooterDataProvider) -> HudData {
    let (_, _, _, _, cost) = source.usage_totals();
    let cwd = source.cwd();
    let folder = Path::new(&cwd)
        .file_name()
        .map_or_else(|| cwd.clone(), |n| n.to_string_lossy().into_owned());
    let fuel = match source.context_usage() {
        None => "0%".to_string(),
        Some((_, None)) => "?".to_string(),
        Some((_, Some(percent))) => format!("{}%", percent.round() as i64),
    };
    HudData {
        mode: data.get_active_mode().to_uppercase(),
        folder,
        model: source.model().map_or_else(|| "none".to_string(), |m| m.id),
        fuel,
        cost: format!("${cost:.2}"),
    }
}

/// The HUD as a one-row component. It reads the session on each render.
pub fn hud_component(
    source: Box<dyn FooterSource>,
    data: FooterDataProvider,
) -> RatatuiComponent<impl FnMut(Rect, &mut Buffer)> {
    RatatuiComponent::new(1, move |area: Rect, buf: &mut Buffer| {
        let width = usize::from(area.width);
        let text = hud_text(&read_hud(source.as_ref(), &data), width);
        buf.set_stringn(area.x, area.y, &text, width, Style::default());
    })
}

/// The parts of the screen, in the order the app tree adds them. Full-screen
/// puts the HUD, the gap and the attach slot on top, the body (header and
/// chat) under them, and the rest below. The scroll bar has no part: `Tui`
/// draws it between the body and the bottom while pinned.
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
    Footer,
}

/// The app tree's children in order. With `fullscreen` off this is today's
/// inline tree, unchanged.
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
        Footer,
    ];
    let mut plan = if fullscreen {
        vec![Hud, Gap, Attach, Body]
    } else {
        vec![Header, ScreenFill, Attach, Chat]
    };
    plan.extend(bottom);
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(folder: &str) -> HudData {
        HudData {
            mode: "BUILD".into(),
            folder: folder.into(),
            model: "claude-haiku-5-5".into(),
            fuel: "42%".into(),
            cost: "$1.23".into(),
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
    fn hud_100_cols_shows_mode_folder_ship_fuel_cost() {
        let text = hud_text(&data("hoocode-workspace-checkout"), 100);
        assert_eq!(visible_width(&text), 100);
        assert!(text.starts_with("◭ HOOCODE  BUILD  hoocode-workspace-checkout"));
        assert!(text.ends_with("SHIP claude-haiku-5-5  FUEL 42%  CR $1.23"));
    }

    #[test]
    fn hud_80_cols_drops_folder_first() {
        let text = hud_text(&data("hoocode-workspace-checkout"), 80);
        assert_eq!(visible_width(&text), 80);
        assert!(!text.contains("hoocode-workspace-checkout"));
        assert!(text.starts_with("◭ HOOCODE  BUILD"));
        assert!(text.ends_with("SHIP claude-haiku-5-5  FUEL 42%  CR $1.23"));
    }

    #[test]
    fn hud_keeps_folder_when_it_fits_at_80_cols() {
        let text = hud_text(&data("hoocode"), 80);
        assert!(text.contains("hoocode"));
        assert_eq!(visible_width(&text), 80);
    }

    #[test]
    fn hud_cuts_the_row_when_even_the_short_left_does_not_fit() {
        let text = hud_text(&data("hoocode"), 30);
        assert_eq!(visible_width(&text), 30);
        assert!(text.starts_with("◭ HOOCODE  BUILD  SHIP"));
        assert!(text.ends_with('…'), "{text:?}");
    }

    #[test]
    fn hud_zero_width_is_empty() {
        assert_eq!(hud_text(&data("hoocode"), 0), "");
    }
}

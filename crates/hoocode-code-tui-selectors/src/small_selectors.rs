//! The one-list pickers: `thinking-selector.ts`, `theme-selector.ts`,
//! `show-images-selector.ts` and `session-color-selector.ts`. Each is a
//! [`FramedSelectList`]; the callbacks go on its list.

use hoocode_code_tui_theme::{
    get_available_themes, get_resolved_theme_colors, get_theme_description,
};
use hoocode_tui_components::SelectItem;

use crate::framed_list::FramedSelectList;

fn item(value: &str, label: &str, description: Option<String>) -> SelectItem {
    SelectItem {
        value: value.to_string(),
        label: label.to_string(),
        description,
    }
}

/// `LEVEL_DESCRIPTIONS`.
pub fn thinking_level_description(level: &str) -> Option<&'static str> {
    Some(match level {
        "off" => "No reasoning",
        "minimal" => "Very brief reasoning (~1k tokens)",
        "low" => "Light reasoning (~2k tokens)",
        "medium" => "Moderate reasoning (~8k tokens)",
        "high" => "Deep reasoning (~16k tokens)",
        "xhigh" => "Maximum reasoning (~32k tokens)",
        _ => return None,
    })
}

/// `ThinkingSelectorComponent`: the levels the model offers, the current one
/// selected. The selected item's value is the level.
pub fn thinking_selector(current_level: &str, available_levels: &[&str]) -> FramedSelectList {
    let items: Vec<SelectItem> = available_levels
        .iter()
        .map(|level| {
            item(
                level,
                level,
                thinking_level_description(level).map(str::to_string),
            )
        })
        .collect();
    let selected = items.iter().position(|i| i.value == current_level);
    let count = items.len();
    FramedSelectList::new("thinking", items, count, selected)
}

/// `ThemeSelectorComponent`: every available theme with its swatches and
/// description, the current one marked `● in use` and selected. Wire
/// `on_selection_change` for the live preview.
pub fn theme_selector(current_theme: &str) -> FramedSelectList {
    let themes = get_available_themes();
    let items = themes
        .iter()
        .map(|name| {
            let in_use = name == current_theme;
            let description = match (get_theme_description(name), in_use) {
                (Some(text), true) => Some(format!("{text} ● in use")),
                (Some(text), false) => Some(text),
                (None, true) => Some("● in use".to_string()),
                (None, false) => None,
            };
            item(name, name, description)
        })
        .collect();
    let selected = themes.iter().position(|t| t == current_theme);
    let swatches = themes
        .iter()
        .map(|name| (name.clone(), theme_swatches(name)))
        .collect();
    FramedSelectList::new("HULL · /theme", items, 10, selected)
        .with_count(if themes.len() == 1 {
            "1 theme".to_string()
        } else {
            format!("{} themes", themes.len())
        })
        .with_swatches(swatches)
}

/// Four swatches from a theme's own palette for its picker row: the
/// accent, success, warning and error tokens, skipping any the theme
/// does not resolve.
fn theme_swatches(name: &str) -> Vec<String> {
    let Ok(colors) = get_resolved_theme_colors(Some(name)) else {
        return Vec::new();
    };
    ["accent", "success", "warning", "error"]
        .iter()
        .filter_map(|token| colors.get(*token).cloned())
        .collect()
}

/// `ShowImagesSelectorComponent`: values `yes` / `no`.
pub fn show_images_selector(current_value: bool) -> FramedSelectList {
    let items = vec![
        item(
            "yes",
            "Yes",
            Some("Show images inline in terminal".to_string()),
        ),
        item(
            "no",
            "No",
            Some("Show text placeholder instead".to_string()),
        ),
    ];
    FramedSelectList::new("images", items, 5, Some(if current_value { 0 } else { 1 }))
}

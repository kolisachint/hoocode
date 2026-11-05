//! The one-list pickers: `thinking-selector.ts`, `theme-selector.ts`,
//! `show-images-selector.ts` and `session-color-selector.ts`. Each is a
//! [`FramedSelectList`]; the callbacks go on its list.

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
    FramedSelectList::new("Thinking", items, count, selected)
}

/// The themes a picker offers, in list order: `auto`, the built-ins, then the
/// user's own themes (see [`hoocode_code_tui_theme::picker_theme_list`]).
pub fn picker_themes() -> Vec<String> {
    hoocode_code_tui_theme::picker_theme_list()
        .into_iter()
        .map(|t| t.name)
        .collect()
}

/// `ThemeSelectorComponent`: the picker's themes, the current one marked
/// `in use` and selected; the detail shows each user theme's file path.
/// Wire `on_selection_change` for the live preview.
pub fn theme_selector(current_theme: &str) -> FramedSelectList {
    let themes = picker_themes();
    let items = themes
        .iter()
        .map(|name| {
            let in_use = name == current_theme;
            item(name, name, in_use.then(|| "in use".to_string()))
        })
        .collect();
    let selected = themes.iter().position(|t| t == current_theme);
    FramedSelectList::new("Theme · /theme", items, 10, selected)
        .with_count(format!("{} themes", themes.len()))
        .with_detail(crate::framed_list::theme_detail)
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
    FramedSelectList::new("Images", items, 5, Some(if current_value { 0 } else { 1 }))
}

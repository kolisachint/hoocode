//! Component goldens and keys for two settings sub-panes that draw on the
//! pane grammar: Thinking level (Behavior → Thinking level, a select pane
//! that does not preview) and the string-flag editor (Flags → a string flag).
//! Accept a deliberate change with
//! `UPDATE_GOLDENS=1 cargo nextest run -p hoocode-code-tui-selectors settings_subpane`.

use std::cell::RefCell;
use std::rc::Rc;

use hoocode_code_settings::FlagValue;
use hoocode_code_tui_selectors::settings_selector::{
    submenu_list, FlagInfo, SettingsChange, SettingsConfig, SettingsSelectorComponent,
    ToolToggleInfo,
};
use hoocode_tui_render::assert_golden;
use hoocode_tui_render::golden::render_golden;
use hoocode_tui_render::Component;

use crate::support::lock;

/// Raw input for moving down, the app's arrow sequence.
const DOWN: &str = "\x1b[B";

fn pane(changes: Rc<RefCell<Vec<SettingsChange>>>) -> SettingsSelectorComponent {
    let sink = changes.clone();
    SettingsSelectorComponent::new(config(), move |change| sink.borrow_mut().push(change))
}

fn config() -> SettingsConfig {
    SettingsConfig {
        tools: vec![ToolToggleInfo {
            name: "Read".into(),
            enabled: true,
            tokens: Some(162),
        }],
        current_theme: "dark".into(),
        available_themes: vec!["dark".into(), "light".into()],
        thinking_level: "medium".into(),
        available_thinking_levels: ["off", "minimal", "low", "medium", "high", "xhigh"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        flags: vec![
            FlagInfo {
                name: "fast-path".into(),
                description: Some("Use the fast path.".into()),
                value: FlagValue::Bool(true),
            },
            FlagInfo {
                name: "api-base".into(),
                description: Some("Override the API base URL.".into()),
                value: FlagValue::String("https://example.test".into()),
            },
        ],
        tips_enabled: false,
        ..SettingsConfig::default()
    }
}

/// Open the Behavior category, then its Thinking level pane, by keys.
fn open_thinking(pane: &mut SettingsSelectorComponent) {
    pane.settings_list().borrow_mut().apply_filter("Behavior");
    pane.handle_input("\r");
    let behavior = pane
        .settings_list()
        .borrow()
        .open_submenu()
        .expect("the Behavior submenu");
    submenu_list(&behavior)
        .expect("a settings-list submenu")
        .borrow_mut()
        .apply_filter("Thinking level");
    pane.handle_input("\r");
}

/// Open the Flags list, then the api-base string editor, by keys.
fn open_flag_editor(pane: &mut SettingsSelectorComponent) {
    pane.settings_list().borrow_mut().apply_filter("Flags");
    pane.handle_input("\r");
    pane.handle_input(DOWN);
    pane.handle_input("\r");
}

#[test]
fn settings_thinking_100() {
    let _g = lock();
    let mut pane = pane(Rc::default());
    open_thinking(&mut pane);
    let rendered = render_golden(&mut pane, 100);
    let text = rendered.split("--- styles ---").next().unwrap_or_default();
    assert!(
        text.contains("Thinking level  /settings  6 levels"),
        "missing title and count in\n{text}"
    );
    assert!(
        text.lines().any(|l| l.starts_with(" › ● medium")),
        "missing the selected in-use row in\n{text}"
    );
    assert!(
        text.contains("↑↓ move · enter set · esc back"),
        "missing hint row in\n{text}"
    );
    assert!(!text.contains("Enter to select"), "prose hint in\n{text}");
    // The rows carry the level only; the description is the detail's.
    assert!(
        !text.contains("Very brief reasoning"),
        "description crammed into a row in\n{text}"
    );
    assert!(
        text.contains("Moderate reasoning (~8k tokens)"),
        "missing the selected level's detail in\n{text}"
    );
    assert_golden!("settings_thinking_100", rendered);
}

#[test]
fn settings_flag_100() {
    let _g = lock();
    let mut pane = pane(Rc::default());
    open_flag_editor(&mut pane);
    let rendered = render_golden(&mut pane, 100);
    let text = rendered.split("--- styles ---").next().unwrap_or_default();
    assert!(
        text.contains("--api-base  /settings"),
        "missing title in\n{text}"
    );
    assert!(
        text.contains("https://example.test"),
        "missing the value in\n{text}"
    );
    assert!(
        text.contains("enter save · esc back"),
        "missing hint row in\n{text}"
    );
    assert!(!text.contains("Flag: --"), "old heading in\n{text}");
    assert_golden!("settings_flag_100", rendered);
}

/// Enter on a level sets it and closes the pane.
#[test]
fn settings_thinking_enter_sets_the_level() {
    let _g = lock();
    let changes: Rc<RefCell<Vec<SettingsChange>>> = Rc::default();
    let mut pane = pane(changes.clone());
    open_thinking(&mut pane);
    pane.handle_input(DOWN);
    pane.handle_input("\r");
    assert_eq!(
        *changes.borrow(),
        vec![SettingsChange::ThinkingLevel("high".to_string())]
    );
}

/// Esc backs out of the level pane without a change.
#[test]
fn settings_thinking_esc_changes_nothing() {
    let _g = lock();
    let changes: Rc<RefCell<Vec<SettingsChange>>> = Rc::default();
    let mut pane = pane(changes.clone());
    open_thinking(&mut pane);
    pane.handle_input(DOWN);
    pane.handle_input("\x1b");
    assert!(changes.borrow().is_empty(), "{:?}", changes.borrow());
}

/// Typing edits the value and Enter saves it to the flag.
#[test]
fn settings_flag_enter_saves_the_value() {
    let _g = lock();
    let changes: Rc<RefCell<Vec<SettingsChange>>> = Rc::default();
    let mut pane = pane(changes.clone());
    open_flag_editor(&mut pane);
    pane.handle_input("x");
    pane.handle_input("\r");
    assert_eq!(
        *changes.borrow(),
        vec![SettingsChange::Flag {
            name: "api-base".to_string(),
            value: FlagValue::String("xhttps://example.test".to_string()),
        }]
    );
}

/// Open a settings row by its label from the Settings pane, through the
/// category that holds it when `category` is set.
fn open_by_keys(pane: &mut SettingsSelectorComponent, category: Option<&str>, label: &str) {
    pane.settings_list()
        .borrow_mut()
        .apply_filter(category.unwrap_or(label));
    pane.handle_input("\r");
    if category.is_some() {
        let cat = pane
            .settings_list()
            .borrow()
            .open_submenu()
            .expect("the category submenu");
        submenu_list(&cat)
            .expect("a settings-list submenu")
            .borrow_mut()
            .apply_filter(label);
        pane.handle_input("\r");
    }
}

/// Each list sub-pane opens with its title chip row (`Title  /settings …`)
/// and marks its cursor row with the ` › ` gutter.
#[test]
fn settings_list_panes_draw_the_chip_row_and_gutter() {
    let _g = lock();
    for (category, label) in [
        (Some("Advanced"), "Warnings"),
        (None, "Tools"),
        (None, "Tool output"),
        (None, "Flags"),
    ] {
        let mut pane = pane(Rc::default());
        open_by_keys(&mut pane, category, label);
        let rendered = render_golden(&mut pane, 100);
        let text = rendered.split("--- styles ---").next().unwrap_or_default();
        let first = text.lines().next().unwrap_or_default().trim_start();
        assert!(
            first.starts_with(&format!("{label}  /settings")),
            "{label}: no title chip row first in\n{text}"
        );
        assert!(
            text.lines().any(|l| l.starts_with(" › ")),
            "{label}: no ` › ` cursor row in\n{text}"
        );
    }
}

/// The lists that share the sub-pane chrome draw the same title and hint.
#[test]
fn settings_list_kinds_share_the_sub_pane_chrome() {
    let _g = lock();
    for (category, label, title) in [
        (Some("Advanced"), "Warnings", "Warnings  /settings"),
        (Some("Plugins"), "Platform", "Platform  /settings"),
        (None, "Tools", "Tools  /settings"),
        (None, "Tool output", "Tool output  /settings"),
        (None, "Flags", "Flags  /settings"),
    ] {
        let mut pane = pane(Rc::default());
        open_by_keys(&mut pane, category, label);
        let text = render_golden(&mut pane, 100);
        assert!(text.contains(title), "{label}: missing {title} in\n{text}");
        assert!(
            text.contains("↑↓ move"),
            "{label}: missing the hint row in\n{text}"
        );
    }
}

/// A list with no search (Tool output) has no filter row; a searchable one
/// (Interface) has it.
#[test]
fn settings_list_without_search_has_no_filter_row() {
    let _g = lock();
    let mut pane = pane(Rc::default());
    open_by_keys(&mut pane, None, "Tool output");
    let text = render_golden(&mut pane, 100);
    assert!(
        !text.contains("▏ filter"),
        "Tool output has a filter row in\n{text}"
    );

    let mut pane = pane_with_interface();
    let text = render_golden(&mut pane, 100);
    assert!(
        text.contains("▏ filter"),
        "Interface has no filter row in\n{text}"
    );
}

/// Open the Interface category by keys (it is searchable).
fn pane_with_interface() -> SettingsSelectorComponent {
    let mut pane = pane(Rc::default());
    open_by_keys(&mut pane, None, "Interface");
    pane
}

/// The text row with the caret (`▏`) in a rendered pane.
fn caret_row(text: &str) -> &str {
    text.lines()
        .find(|l| l.contains('▏'))
        .unwrap_or_else(|| panic!("no caret row in\n{text}"))
}

/// A 200-character string value at width 80: with the cursor at the end the
/// row shows the last characters and the caret; at the start, the first.
#[test]
fn settings_flag_long_value_keeps_the_caret_in_view() {
    let _g = lock();
    // Fifty distinct four-digit numbers: no ten-character run repeats.
    let long: String = (0..50).map(|i| format!("{i:04}")).collect();
    let mut cfg = config();
    cfg.flags[1].value = FlagValue::String(long.clone());
    let mut pane = SettingsSelectorComponent::new(cfg, |_| {});
    open_flag_editor(&mut pane);

    pane.handle_input("\x05");
    let text = render_golden(&mut pane, 80);
    let row = caret_row(&text);
    assert!(
        row.contains(&long[190..]),
        "no tail of the value in {row:?}"
    );
    assert!(
        !row.contains(&long[..10]),
        "head shown at the end in {row:?}"
    );
    assert!(row.contains('…'), "no cut edge in {row:?}");
    assert!(
        row.trim_end().ends_with('▏'),
        "caret not at the end in {row:?}"
    );

    pane.handle_input("\x01");
    let text = render_golden(&mut pane, 80);
    let row = caret_row(&text);
    assert!(row.contains(&long[..10]), "no head of the value in {row:?}");
    assert!(
        !row.contains(&long[190..]),
        "tail shown at the start in {row:?}"
    );
    assert!(row.contains('▏'), "no caret in {row:?}");
}

/// A list whose filter matches nothing has no row to toggle or change, so
/// its hint drops `space toggle` and `enter change`.
#[test]
fn settings_list_hint_with_no_match_has_only_move_and_back() {
    let _g = lock();
    let mut pane = pane(Rc::default());
    open_by_keys(&mut pane, None, "Flags");
    let flags = pane
        .settings_list()
        .borrow()
        .open_submenu()
        .expect("the Flags submenu");
    submenu_list(&flags)
        .expect("a settings-list submenu")
        .borrow_mut()
        .apply_filter("zzz");
    let text = render_golden(&mut pane, 100);
    assert!(
        text.contains("↑↓ move · esc back"),
        "missing move and back in\n{text}"
    );
    assert!(!text.contains("space toggle"), "toggle hint in\n{text}");
    assert!(!text.contains("enter change"), "change hint in\n{text}");
}

/// Reopening Thinking level after a pick puts the cursor on the level now in use.
#[test]
fn settings_thinking_reopens_on_the_level_in_use() {
    let _g = lock();
    let mut pane = pane(Rc::default());
    open_thinking(&mut pane);
    pane.handle_input(DOWN);
    pane.handle_input("\r");
    // The pick closed the level pane; Behavior is open again.
    let behavior = pane
        .settings_list()
        .borrow()
        .open_submenu()
        .expect("the Behavior submenu");
    submenu_list(&behavior)
        .expect("a settings-list submenu")
        .borrow_mut()
        .apply_filter("Thinking level");
    pane.handle_input("\r");
    let text = render_golden(&mut pane, 100);
    assert!(
        text.lines().any(|l| l.starts_with(" › ● high")),
        "cursor not on the level in use in\n{text}"
    );
}

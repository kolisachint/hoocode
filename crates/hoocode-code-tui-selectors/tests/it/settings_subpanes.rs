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

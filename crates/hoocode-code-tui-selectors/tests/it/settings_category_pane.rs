//! Component goldens: a settings category sub-pane (the Interface category
//! opened from `/settings`) at 100 and 80 columns, dark, and light at 100.
//! The sub-pane draws on the same pane chrome as the `/settings` pane.
//! Accept a deliberate change with
//! `UPDATE_GOLDENS=1 cargo nextest run -p hoocode-code-tui-selectors settings_interface`.

use std::cell::RefCell;
use std::rc::Rc;

use hoocode_code_tools::light::PromptSurface;
use hoocode_code_tui_selectors::settings_selector::{
    SettingsChange, SettingsConfig, SettingsSelectorComponent, ToolToggleInfo,
};
use hoocode_tui_render::golden::render_golden;
use hoocode_tui_render::{assert_golden, Component};

use crate::support::{has_title, lock, look, Look};

/// Raw input for moving down, the app's arrow sequence.
const DOWN: &str = "\x1b[B";

fn surface() -> PromptSurface {
    PromptSurface {
        system_prompt_tokens: 1430,
        tool_schema_tokens: 2710,
        total_tokens: 4140,
        tools: vec![("Read".into(), 162), ("Shell".into(), 128)],
    }
}

fn pane(changes: Rc<RefCell<Vec<SettingsChange>>>) -> SettingsSelectorComponent {
    let sink = changes.clone();
    SettingsSelectorComponent::new(
        SettingsConfig {
            tools: vec![ToolToggleInfo {
                name: "Read".into(),
                enabled: true,
                tokens: Some(162),
            }],
            current_theme: "dark".into(),
            available_themes: vec!["dark".into(), "light".into()],
            measure_token_surface: Some(Rc::new(surface)),
            tips_enabled: false,
            ..SettingsConfig::default()
        },
        move |change| sink.borrow_mut().push(change),
    )
}

/// Open the Interface category from the `/settings` pane.
fn open_interface(pane: &mut SettingsSelectorComponent) {
    pane.settings_list().borrow_mut().apply_filter("Interface");
    pane.handle_input("\r");
}

fn check_interface(name: &str, width: u16, look_: Look) {
    let _g = lock();
    look(look_);
    let mut pane = pane(Rc::default());
    open_interface(&mut pane);
    let rendered = render_golden(&mut pane, width);
    let text = rendered.split("--- styles ---").next().unwrap_or_default();
    assert!(
        has_title(text, "Interface  /settings  8 settings"),
        "{name}: missing title and count in\n{text}"
    );
    assert!(
        text.lines().any(|l| l.starts_with(" › Theme")),
        "{name}: missing selected row in\n{text}"
    );
    assert!(
        text.lines()
            .any(|l| l.contains("▏") && l.contains("filter")),
        "{name}: missing filter row in\n{text}"
    );
    assert!(
        text.contains("↑↓ move · enter open · esc back"),
        "{name}: missing hint row in\n{text}"
    );
    assert!(!text.contains("..."), "{name}: ASCII ellipsis in\n{text}");
    assert_golden!(name, rendered);
}

#[test]
fn settings_interface_100() {
    check_interface("settings_interface_100", 100, Look::Dark);
}

#[test]
fn settings_interface_80() {
    check_interface("settings_interface_80", 80, Look::Dark);
}

#[test]
fn settings_interface_light_100() {
    check_interface("settings_interface_light_100", 100, Look::Light);
}

/// A row that changes a value offers `space toggle` and `enter change`; a row
/// that opens a submenu offers `enter open` only.
#[test]
fn settings_interface_hint_follows_the_selected_row() {
    let _g = lock();
    let mut pane = pane(Rc::default());
    open_interface(&mut pane);
    pane.handle_input(DOWN);
    let text = render_golden(&mut pane, 100);
    assert!(
        text.contains("↑↓ move · space toggle · enter change · esc back"),
        "missing the value hint in\n{text}"
    );
}

/// Esc closes the sub-pane and the Settings pane is back.
#[test]
fn settings_interface_esc_returns_to_settings() {
    let _g = lock();
    let mut pane = pane(Rc::default());
    open_interface(&mut pane);
    pane.handle_input("\x1b");
    let text = render_golden(&mut pane, 100);
    assert!(
        text.contains("Settings  /settings  4,140 tokens per turn"),
        "missing the Settings pane in\n{text}"
    );
}

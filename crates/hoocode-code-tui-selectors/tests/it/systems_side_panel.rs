//! Behavior: `/settings` draws the launch weight as spec d4's right-side
//! `│` panel on wide screens and keeps the stacked bar under 100 columns.

use std::cell::RefCell;
use std::rc::Rc;

use hoocode_code_tools::light::PromptSurface;
use hoocode_code_tui_selectors::settings_selector::{
    SettingsChange, SettingsConfig, SettingsSelectorComponent, ToolToggleInfo,
};
use hoocode_tui_render::Component;

use crate::support::{lock, strip};

fn surface() -> PromptSurface {
    PromptSurface {
        system_prompt_tokens: 1430,
        tool_schema_tokens: 2710,
        total_tokens: 4140,
        tools: vec![("Read".into(), 162), ("Shell".into(), 128)],
    }
}

fn pane() -> SettingsSelectorComponent {
    let sink: Rc<RefCell<Vec<SettingsChange>>> = Rc::default();
    SettingsSelectorComponent::new(
        SettingsConfig {
            tools: vec![ToolToggleInfo {
                name: "Read".into(),
                enabled: true,
                tokens: Some(162),
            }],
            measure_token_surface: Some(Rc::new(surface)),
            tips_enabled: false,
            ..SettingsConfig::default()
        },
        move |change| sink.borrow_mut().push(change),
    )
}

#[test]
fn wide_screens_pair_rows_with_the_launch_panel() {
    let _g = lock();
    let shown = strip(&pane().render(100));
    assert!(
        shown.contains("LAUNCH WEIGHT every turn carries 4,140 tokens"),
        "missing panel title in\n{shown}"
    );
    assert!(
        shown
            .lines()
            .any(|l| l.contains('│') && l.contains("Tools") && l.contains("LAUNCH WEIGHT")),
        "panel is not beside the rows in\n{shown}"
    );
    assert!(
        shown.contains("prompt 1,430 · schemas 2,710"),
        "missing panel breakdown in\n{shown}"
    );
    assert!(
        !shown.contains("Per-turn surface"),
        "stacked surface row in\n{shown}"
    );
}

#[test]
fn narrow_screens_keep_the_stacked_bar() {
    let _g = lock();
    let shown = strip(&pane().render(80));
    assert!(
        shown.contains("Per-turn surface: 4,140 tokens"),
        "missing stacked surface row in\n{shown}"
    );
    assert!(
        !shown.lines().any(|l| l.contains('│')),
        "side panel on a narrow screen in\n{shown}"
    );
}

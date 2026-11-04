//! Behavior: `/settings` draws the selected setting's detail (what every
//! turn carries) beside the rows from 100 columns and under them below.

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
fn wide_screens_pair_rows_with_the_detail() {
    let _g = lock();
    let shown = strip(&pane().render(100));
    assert!(
        shown.contains("per turn  4,140 tokens"),
        "missing the detail in\n{shown}"
    );
    assert!(
        shown
            .lines()
            .any(|l| l.contains("Tool output") && l.contains("current")),
        "the detail is not beside the rows in\n{shown}"
    );
    assert!(
        !shown.contains("Per-turn surface"),
        "stacked surface row in\n{shown}"
    );
}

#[test]
fn narrow_screens_stack_the_detail_under_the_rows() {
    let _g = lock();
    let shown = strip(&pane().render(80));
    let lines: Vec<&str> = shown.lines().collect();
    let tools = lines.iter().position(|l| l.contains("Tools")).unwrap();
    let detail = lines
        .iter()
        .position(|l| l.contains("per turn  4,140 tokens"))
        .expect("the detail renders");
    assert!(
        detail > tools,
        "the detail is not under the rows in\n{shown}"
    );
    assert!(
        !lines.iter().any(|l| l.contains('│')),
        "side panel on a narrow screen in\n{shown}"
    );
}

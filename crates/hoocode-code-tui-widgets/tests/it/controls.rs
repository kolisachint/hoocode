//! Component goldens: `/hotkeys` as the Keys pane on its leader layer: the
//! bindings grouped as prompt, transcript and app at 100 columns, stacked at 80.
//! Everything on screen comes from the real keybindings with literal key
//! names (no `option`), so the bytes hold on macOS and Linux. Accept a
//! deliberate change with
//! `UPDATE_GOLDENS=1 cargo nextest run -p hoocode-code-tui-widgets controls`.

use hoocode_code_tui_widgets::controls::ControlsComponent;
use hoocode_tui_render::assert_golden;
use hoocode_tui_render::golden::render_golden;
use hoocode_tui_render::Component;

use crate::support::{has_title, lock, look, Look};

fn check(name: &str, width: u16) {
    check_look(name, width, Look::Dark);
}

fn check_look(name: &str, width: u16, variant: Look) {
    let _g = lock();
    look(variant);
    let mut controls = ControlsComponent::new();
    // The leader layer: the transcript's keys are leader keys now (K-P2, K-P3).
    controls.handle_input("\t");
    let rendered = render_golden(&mut controls, width);
    let text = rendered.split("--- styles ---").next().unwrap_or_default();
    assert!(
        has_title(text, "Keys  ?"),
        "{name}: missing title in\n{text}"
    );
    assert!(
        text.contains("plain")
            && text.contains("ctrl")
            && text.contains("leader")
            && text.contains("alt"),
        "{name}: missing layer tabs in\n{text}"
    );
    assert!(
        text.contains("bindings") && text.contains("shown"),
        "{name}: missing binding count in\n{text}"
    );
    for group in [
        "Prompt",
        "Block focused (pinned, prompt empty)",
        "Picker or pane, empty filter",
        "Blocking dialog, empty reply",
        "Task panel focused",
    ] {
        assert!(
            text.lines()
                .any(|l| l.trim_start_matches(['│', ' ']).starts_with(group)),
            "{name}: missing the {group} group in\n{text}"
        );
    }
    assert!(
        text.contains("tab layer · esc close"),
        "{name}: missing hint row in\n{text}"
    );
    assert_golden!(name, rendered);
}

/// The grouped grid: one group per row of "Where a key acts", in table order.
#[test]
fn controls_100() {
    check("controls_100", 100);
}

/// Below 100 columns the groups take one column.
#[test]
fn controls_80() {
    check("controls_80", 80);
}

#[test]
fn controls_light_100() {
    check_look("controls_light_100", 100, Look::Light);
}

#[test]
fn controls_mono_100() {
    check_look("controls_mono_100", 100, Look::Mono);
}

#[test]
fn keybinding_problems_are_listed_under_the_groups_and_hidden_when_none() {
    let _g = lock();
    let mut controls = ControlsComponent::new();
    controls.set_problems(vec![
        "error: ctrl+c is reserved and cannot be rebound".into()
    ]);
    let rendered = render_golden(&mut controls, 100);
    let text = rendered.split("--- styles ---").next().unwrap_or_default();
    assert!(text.contains("keybindings.json · 1 problem"), "{text}");
    assert!(text.contains("ctrl+c is reserved"), "{text}");
    controls.set_problems(Vec::new());
    let rendered = render_golden(&mut controls, 100);
    assert!(!rendered.contains("keybindings.json ·"), "{rendered}");
}

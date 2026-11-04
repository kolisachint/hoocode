//! Component goldens: `/hotkeys` as the Keys pane: the layer's bindings
//! grouped as prompt, transcript and app at 100 columns, stacked at 80.
//! Everything on screen comes from the real keybindings with literal key
//! names (no `option`), so the bytes hold on macOS and Linux. Accept a
//! deliberate change with
//! `UPDATE_GOLDENS=1 cargo nextest run -p hoocode-code-tui-widgets controls`.

use hoocode_code_tui_widgets::controls::ControlsComponent;
use hoocode_tui_render::assert_golden;
use hoocode_tui_render::golden::render_golden;

use crate::support::lock;

fn check(name: &str, width: u16) {
    let _g = lock();
    let mut controls = ControlsComponent::new();
    let rendered = render_golden(&mut controls, width);
    let text = rendered.split("--- styles ---").next().unwrap_or_default();
    assert!(text.contains("Keys  ?"), "{name}: missing title in\n{text}");
    assert!(
        text.contains("plain") && text.contains("ctrl") && text.contains("alt"),
        "{name}: missing layer tabs in\n{text}"
    );
    assert!(
        text.contains("bindings") && text.contains("shown"),
        "{name}: missing binding count in\n{text}"
    );
    for group in ["prompt", "transcript", "app"] {
        assert!(
            text.lines().any(|l| l.trim_start().starts_with(group)),
            "{name}: missing the {group} group in\n{text}"
        );
    }
    assert!(
        text.contains("tab layer · esc close"),
        "{name}: missing hint row in\n{text}"
    );
    assert_golden!(name, rendered);
}

/// The grouped grid: prompt keys across, transcript and app side by side.
#[test]
fn controls_100() {
    check("controls_100", 100);
}

/// Below the spec width: the groups stack in one column.
#[test]
fn controls_80() {
    check("controls_80", 80);
}

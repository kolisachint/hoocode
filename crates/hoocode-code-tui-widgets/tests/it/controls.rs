//! Component goldens: `/hotkeys` as the CONTROLS console — the drawn
//! keyboard with the ctrl-layer keys lit at 100 columns, the key list at
//! 80. Everything on screen comes from the real keybindings with literal
//! key names (no `option`), so the bytes hold on macOS and Linux. Accept a
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
    assert!(
        text.contains("CONTROLS · /hotkeys"),
        "{name}: missing title in\n{text}"
    );
    assert_golden!(name, rendered);
}

/// The keyboard: esc and the letter rows with the bound caps.
#[test]
fn controls_100() {
    let _g = lock();
    let mut controls = ControlsComponent::new();
    let rendered = render_golden(&mut controls, 100);
    let text = rendered.split("--- styles ---").next().unwrap_or_default();
    assert!(text.contains("CONTROLS · /hotkeys"), "missing title");
    assert!(text.contains("ctrl layer"), "missing layer label");
    assert!(
        text.lines().any(|l| l.contains("esc")),
        "missing keyboard in\n{text}"
    );
    assert!(
        text.contains("bindings"),
        "missing binding count in\n{text}"
    );
    assert_golden!("controls_100", rendered);
}

/// Below the spec width: the key list in the same console.
#[test]
fn controls_80() {
    check("controls_80", 80);
}

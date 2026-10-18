//! Behavior: the CONTROLS console's layer tabs, search filter and close.
//! `tab` cycles plain/ctrl/alt, typing narrows the action list, and `esc`
//! reports `Close` so the app can give the editor back. All three are the
//! foot's promises (`tab layer · type search · esc close`).

use hoocode_code_tui_widgets::controls::{ControlsComponent, ControlsEvent};
use hoocode_tui_render::Component;

use crate::support::lock;

fn text_at(width: u16, controls: &mut ControlsComponent) -> String {
    controls
        .render(width)
        .join("\n")
        .split("--- styles ---")
        .next()
        .unwrap_or_default()
        .to_string()
}

#[test]
fn tab_cycles_the_lit_layer() {
    let _g = lock();
    let mut controls = ControlsComponent::new();
    assert!(text_at(100, &mut controls).contains("ctrl layer"));
    controls.handle_input("\t");
    assert!(text_at(100, &mut controls).contains("alt layer"));
    controls.handle_input("\t");
    assert!(text_at(100, &mut controls).contains("plain layer"));
    controls.handle_input("\t");
    assert!(text_at(100, &mut controls).contains("ctrl layer"));
}

#[test]
fn typing_narrows_the_action_list() {
    let _g = lock();
    let mut controls = ControlsComponent::new();
    let full = text_at(100, &mut controls);
    assert!(full.contains("Clear editor"), "setup in\n{full}");
    for c in "clear".chars() {
        controls.handle_input(&c.to_string());
    }
    let filtered = text_at(100, &mut controls);
    assert!(
        filtered.contains("Clear editor"),
        "missing match in\n{filtered}"
    );
    assert!(
        filtered.lines().count() < full.lines().count(),
        "filter kept every row in\n{filtered}"
    );
}

#[test]
fn a_query_with_no_match_says_so() {
    let _g = lock();
    let mut controls = ControlsComponent::new();
    for c in "zzz-no-match".chars() {
        controls.handle_input(&c.to_string());
    }
    let filtered = text_at(100, &mut controls);
    assert!(
        filtered.contains("No matching bindings"),
        "missing empty note in\n{filtered}"
    );
}

#[test]
fn esc_reports_close() {
    let _g = lock();
    let mut controls = ControlsComponent::new();
    assert!(controls.take_events().is_empty());
    controls.handle_input("\x1b");
    assert_eq!(controls.take_events(), vec![ControlsEvent::Close]);
}

#[test]
fn the_console_takes_focus() {
    let _g = lock();
    let mut controls = ControlsComponent::new();
    assert!(controls.is_focusable());
    controls.set_focused(true);
    let shown = text_at(100, &mut controls);
    assert!(shown.contains("▏"), "missing caret in\n{shown}");
}

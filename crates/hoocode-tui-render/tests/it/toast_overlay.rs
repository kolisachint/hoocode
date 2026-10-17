//! Mission Control transmission toasts (M8 e4) at the frame level: the app
//! draws the toast and the fullscreen frame only floats its pre-rendered
//! lines top-right over the body.

use crate::support::*;
use hoocode_tui_render::Tui;

const WIDTH: u16 = 100;
const HEIGHT: u16 = 12;
const H: usize = HEIGHT as usize;

fn toast() -> Vec<String> {
    vec![
        format!("╭{}╮", "─".repeat(44)),
        format!("│{}│", " toast".to_string() + &" ".repeat(39)),
    ]
}

/// The fullscreen tree: a HUD row, a gap, the body, and the prompt. The
/// toast provider floats two fixed 46-wide lines.
fn setup() -> (Tui, Handle) {
    let (terminal, term) = virtual_terminal(WIDTH, HEIGHT);
    let mut tui = Tui::new(terminal, None);
    tui.set_fullscreen(true);
    tui.add_child(Lines::new(&["HUD"]));
    tui.add_child(Lines::new(&[""]));
    let body = Lines::body(30);
    let handle: hoocode_tui_render::ComponentHandle = body.clone();
    tui.add_child(handle.clone());
    tui.set_fullscreen_body(Some(handle));
    tui.add_child(Lines::new(&["prompt"]));
    tui.set_toast_provider(Box::new(|_| toast()));
    let _events = tui.start();
    tui.request_render(false);
    (tui, term)
}

#[test]
fn toast_floats_top_right_over_the_first_body_rows() {
    let (_tui, term) = setup();
    let rows = term.screen();
    // Screen rows 0-1 are the top chrome; the body starts on row 2.
    assert_eq!(rows[0], "HUD");
    let (head, tail) = (toast()[0].clone(), toast()[1].clone());
    assert!(rows[2].starts_with("body "), "{:?}", rows[2]);
    assert!(rows[2].ends_with(&head), "{:?}", rows[2]);
    assert!(rows[3].ends_with(&tail), "{:?}", rows[3]);
    assert!(
        !rows[4].contains('╭') && !rows[4].contains('│'),
        "{:?}",
        rows[4]
    );
    assert_eq!(rows[H - 1], "prompt");
}

#[test]
fn without_a_provider_the_body_is_untouched() {
    let (terminal, term) = virtual_terminal(WIDTH, HEIGHT);
    let mut tui = Tui::new(terminal, None);
    tui.set_fullscreen(true);
    tui.add_child(Lines::new(&["HUD"]));
    tui.add_child(Lines::new(&[""]));
    let body = Lines::body(30);
    let handle: hoocode_tui_render::ComponentHandle = body.clone();
    tui.add_child(handle.clone());
    tui.set_fullscreen_body(Some(handle));
    tui.add_child(Lines::new(&["prompt"]));
    let _events = tui.start();
    tui.request_render(false);
    let rows = term.screen();
    assert!(rows[2].starts_with("body "), "{:?}", rows[2]);
    assert!(!rows[2].contains('╭'), "{:?}", rows[2]);
}

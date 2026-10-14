//! Theme `fg` and `bg` follow chalk's colour switch (X15): with colour off
//! they return the text unstyled, the same as the chalk modifiers.

use hoocode_code_tui_theme::*;

/// Restores the switch on drop, so a failing assert cannot leave colour off
/// for the other tests in this binary.
struct SwitchGuard;

impl Drop for SwitchGuard {
    fn drop(&mut self) {
        chalk::set_enabled(true);
    }
}

#[test]
fn fg_and_bg_emit_colour_while_the_switch_is_on() {
    let _lock = crate::global_theme_lock();
    init_theme(Some("dark"), false);
    let _restore = SwitchGuard;
    chalk::set_enabled(true);
    let fg = theme().fg("accent", "hi");
    assert!(fg.starts_with("\x1b["), "fg: {fg:?}");
    assert!(fg.ends_with("\x1b[39m"), "fg: {fg:?}");
    let bg = theme().bg("userMessageBg", "hi");
    assert!(bg.starts_with("\x1b["), "bg: {bg:?}");
    assert!(bg.ends_with("\x1b[49m"), "bg: {bg:?}");
}

#[test]
fn fg_and_bg_are_plain_text_while_the_switch_is_off() {
    let _lock = crate::global_theme_lock();
    init_theme(Some("dark"), false);
    let _restore = SwitchGuard;
    chalk::set_enabled(false);
    assert_eq!(theme().fg("accent", "hi"), "hi");
    assert_eq!(theme().bg("userMessageBg", "hi"), "hi");
}

#[test]
fn fill_emits_its_chip_while_the_switch_is_on() {
    let _lock = crate::global_theme_lock();
    init_theme(Some("dark"), false);
    let _restore = SwitchGuard;
    chalk::set_enabled(true);
    let token = session_color_token(1);
    assert!(theme().can_fill(token), "{token} is a fill token");
    let chip = theme().fill(token, "x");
    assert!(chip.starts_with("\x1b["), "fill: {chip:?}");
    assert!(chip.ends_with("\x1b[39m\x1b[49m"), "fill: {chip:?}");
}

#[test]
fn fill_is_plain_text_while_the_switch_is_off() {
    let _lock = crate::global_theme_lock();
    init_theme(Some("dark"), false);
    let _restore = SwitchGuard;
    chalk::set_enabled(false);
    let token = session_color_token(1);
    assert!(theme().can_fill(token), "{token} is a fill token");
    assert_eq!(theme().fill(token, "x"), "x");
}

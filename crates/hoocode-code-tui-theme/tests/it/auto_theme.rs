//! `"theme": "auto"` follows the OS appearance: parsing, resolution and the
//! `set_theme` hook that keeps the current name concrete.

use hoocode_code_tui_theme::*;

use super::global_theme_lock;

#[test]
fn auto_values_are_detected() {
    assert!(is_auto_theme("auto"));
    assert!(is_auto_theme("auto:daylight:deep-space"));
    assert!(!is_auto_theme("dark"));
    assert!(!is_auto_theme("autumn"));
    assert!(!is_auto_theme(""));
}

#[test]
fn non_auto_values_pass_through() {
    assert_eq!(resolve_auto_theme("dark"), "dark");
    assert_eq!(resolve_auto_theme("daylight"), "daylight");
}

#[test]
fn custom_pair_sides_are_honored_when_known() {
    // Both sides the same known theme: the OS reading cannot matter.
    assert_eq!(resolve_auto_theme("auto:dark:dark"), "dark");
    assert_eq!(resolve_auto_theme("auto:light:light"), "light");
}

#[test]
fn unknown_sides_fall_back_to_the_builtin_pair() {
    // Either OS side must land on a bundled theme, never the bogus names.
    let resolved = resolve_auto_theme("auto:bogus-light:bogus-dark");
    assert!(
        resolved == AUTO_LIGHT_THEME || resolved == AUTO_DARK_THEME,
        "unexpected resolution: {resolved}"
    );
}

#[test]
fn auto_resolves_to_a_concrete_bundled_theme() {
    let resolved = resolve_auto_theme("auto");
    assert!(
        resolved == AUTO_LIGHT_THEME || resolved == AUTO_DARK_THEME,
        "unexpected resolution: {resolved}"
    );
}

#[test]
fn set_theme_with_auto_leaves_a_concrete_current_name() {
    let _guard = global_theme_lock();
    set_theme("auto", false).expect("auto resolves to a loadable theme");
    let current = current_theme_name().expect("a theme is current");
    assert!(
        current == AUTO_LIGHT_THEME || current == AUTO_DARK_THEME,
        "current theme is not concrete: {current}"
    );
}

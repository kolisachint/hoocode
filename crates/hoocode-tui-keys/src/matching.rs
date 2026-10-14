//! Matching raw terminal input against a key identifier string (`"ctrl+c"`,
//! `"escape"`, `"shift+ctrl+p"`, ...).
//!
//! Ported from TypeScript `@kolisachint/hoocode-tui` → `keys.ts` (`matchesKey`).

use crate::constants::{
    arrow_codepoints, codepoints, functional_codepoints, is_symbol_key, legacy_key,
    matches_legacy_modifier_sequence, matches_legacy_sequence, modifiers,
};
use crate::kitty::{
    matches_kitty_sequence, matches_modify_other_keys, matches_printable_modify_other_keys,
};
use crate::state::is_kitty_protocol_active;

/// A key identifier such as `"ctrl+c"` or `"shift+ctrl+p"`, parsed once: the
/// key name (the last `+` part) and its modifier bits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeySpec {
    key: String,
    modifier: u32,
}

impl KeySpec {
    /// Parses a key identifier. `None` when it names no key.
    pub fn parse(key_id: &str) -> Option<Self> {
        let lower = key_id.to_lowercase();
        let parts: Vec<&str> = lower.split('+').collect();
        let key = parts.last()?.to_string();
        if key.is_empty() {
            return None;
        }
        let mut modifier = 0u32;
        if parts.contains(&"shift") {
            modifier |= modifiers::SHIFT;
        }
        if parts.contains(&"alt") {
            modifier |= modifiers::ALT;
        }
        if parts.contains(&"ctrl") {
            modifier |= modifiers::CTRL;
        }
        if parts.contains(&"super") {
            modifier |= modifiers::SUPER;
        }
        Some(Self { key, modifier })
    }

    /// Whether raw terminal input `data` is a press of this key.
    pub fn matches(&self, data: &str) -> bool {
        matches_spec(data, &self.key, self.modifier)
    }
}

/// Raw 0x08 (BS) is ambiguous: Windows Terminal uses it for Ctrl+Backspace,
/// other terminals/tmux setups use it for plain Backspace.
fn matches_raw_backspace(data: &str, expected_modifier: u32) -> bool {
    if data == "\x7f" {
        return expected_modifier == 0;
    }
    if data != "\x08" {
        return false;
    }
    if crate::constants::is_windows_terminal_session() {
        expected_modifier == modifiers::CTRL
    } else {
        expected_modifier == 0
    }
}

/// The control character produced by Ctrl+`key` (`code & 0x1f`).
fn raw_ctrl_char(key: &str) -> Option<char> {
    let c = key.chars().next()?.to_ascii_lowercase();
    let code = c as u32;
    if (97..=122).contains(&code) || matches!(c, '[' | '\\' | ']' | '_') {
        return char::from_u32(code & 0x1f);
    }
    if c == '-' {
        return char::from_u32(31); // same physical key as '_' on US keyboards
    }
    None
}

fn is_digit_key(key: &str) -> bool {
    key.len() == 1 && key.chars().next().is_some_and(|c| c.is_ascii_digit())
}

/// Match raw terminal input `data` against a key identifier like `"ctrl+c"`,
/// `"escape"`, or `"shift+ctrl+p"`. Callers that match the same key often
/// should keep the [`KeySpec`].
pub fn matches_key(data: &str, key_id: &str) -> bool {
    KeySpec::parse(key_id).is_some_and(|spec| spec.matches(data))
}

fn matches_spec(data: &str, key: &str, modifier: u32) -> bool {
    let kitty_active = is_kitty_protocol_active();

    match key {
        "escape" | "esc" => {
            if modifier != 0 {
                return false;
            }
            data == "\x1b"
                || matches_kitty_sequence(data, codepoints::ESCAPE, 0)
                || matches_modify_other_keys(data, codepoints::ESCAPE, 0)
        }
        "space" => {
            if !kitty_active {
                if modifier == modifiers::CTRL && data == "\x00" {
                    return true;
                }
                if modifier == modifiers::ALT && data == "\x1b " {
                    return true;
                }
            }
            if modifier == 0 {
                return data == " "
                    || matches_kitty_sequence(data, codepoints::SPACE, 0)
                    || matches_modify_other_keys(data, codepoints::SPACE, 0);
            }
            matches_kitty_sequence(data, codepoints::SPACE, modifier)
                || matches_modify_other_keys(data, codepoints::SPACE, modifier)
        }
        "tab" => {
            if modifier == modifiers::SHIFT {
                return data == "\x1b[Z"
                    || matches_kitty_sequence(data, codepoints::TAB, modifiers::SHIFT)
                    || matches_modify_other_keys(data, codepoints::TAB, modifiers::SHIFT);
            }
            if modifier == 0 {
                return data == "\t" || matches_kitty_sequence(data, codepoints::TAB, 0);
            }
            matches_kitty_sequence(data, codepoints::TAB, modifier)
                || matches_modify_other_keys(data, codepoints::TAB, modifier)
        }
        "enter" | "return" => matches_enter(data, modifier, kitty_active),
        "backspace" => matches_backspace(data, modifier),
        "insert" => matches_legacy_or_kitty_functional(
            data,
            modifier,
            "insert",
            functional_codepoints::INSERT,
        ),
        "delete" => matches_legacy_or_kitty_functional(
            data,
            modifier,
            "delete",
            functional_codepoints::DELETE,
        ),
        "clear" => {
            if modifier == 0 {
                matches_legacy_sequence(data, legacy_key("clear").unwrap().plain)
            } else {
                matches_legacy_modifier_sequence(data, "clear", modifier)
            }
        }
        "home" => {
            matches_legacy_or_kitty_functional(data, modifier, "home", functional_codepoints::HOME)
        }
        "end" => {
            matches_legacy_or_kitty_functional(data, modifier, "end", functional_codepoints::END)
        }
        "pageup" => matches_legacy_or_kitty_functional(
            data,
            modifier,
            "pageUp",
            functional_codepoints::PAGE_UP,
        ),
        "pagedown" => matches_legacy_or_kitty_functional(
            data,
            modifier,
            "pageDown",
            functional_codepoints::PAGE_DOWN,
        ),
        "up" => matches_arrow(data, modifier, "up", arrow_codepoints::UP, "\x1bp"),
        "down" => matches_arrow(data, modifier, "down", arrow_codepoints::DOWN, "\x1bn"),
        "left" => matches_left_right(data, modifier, kitty_active, &LEFT),
        "right" => matches_left_right(data, modifier, kitty_active, &RIGHT),
        "f1" | "f2" | "f3" | "f4" | "f5" | "f6" | "f7" | "f8" | "f9" | "f10" | "f11" | "f12" => {
            if modifier != 0 {
                return false;
            }
            legacy_key(key).is_some_and(|k| matches_legacy_sequence(data, k.plain))
        }
        _ => matches_printable(data, key, modifier, kitty_active),
    }
}

/// What [`matches_left_right`] needs for one horizontal arrow key.
#[derive(Clone, Copy)]
struct LeftRightArgs {
    name: &'static str,
    codepoint: i32,
    alt_csi: &'static str,
    alt_legacy_ss3: &'static str,
    alt_legacy_ansi: &'static str,
    ctrl_csi: &'static str,
}

const LEFT: LeftRightArgs = LeftRightArgs {
    name: "left",
    codepoint: arrow_codepoints::LEFT,
    alt_csi: "\x1b[1;3D",
    alt_legacy_ss3: "\x1bB",
    alt_legacy_ansi: "\x1bb",
    ctrl_csi: "\x1b[1;5D",
};

const RIGHT: LeftRightArgs = LeftRightArgs {
    name: "right",
    codepoint: arrow_codepoints::RIGHT,
    alt_csi: "\x1b[1;3C",
    alt_legacy_ss3: "\x1bF",
    alt_legacy_ansi: "\x1bf",
    ctrl_csi: "\x1b[1;5C",
};

fn matches_left_right(data: &str, modifier: u32, kitty_active: bool, args: &LeftRightArgs) -> bool {
    let LeftRightArgs {
        name,
        codepoint,
        alt_csi,
        alt_legacy_ss3,
        alt_legacy_ansi,
        ctrl_csi,
    } = *args;
    if modifier == modifiers::ALT {
        return data == alt_csi
            || (!kitty_active && data == alt_legacy_ss3)
            || data == alt_legacy_ansi
            || matches_kitty_sequence(data, codepoint, modifiers::ALT);
    }
    if modifier == modifiers::CTRL {
        return data == ctrl_csi
            || matches_legacy_modifier_sequence(data, name, modifiers::CTRL)
            || matches_kitty_sequence(data, codepoint, modifiers::CTRL);
    }
    if modifier == 0 {
        return matches_legacy_sequence(data, legacy_key(name).unwrap().plain)
            || matches_kitty_sequence(data, codepoint, 0);
    }
    if matches_legacy_modifier_sequence(data, name, modifier) {
        return true;
    }
    matches_kitty_sequence(data, codepoint, modifier)
}

fn matches_arrow(data: &str, modifier: u32, name: &str, codepoint: i32, alt_legacy: &str) -> bool {
    if modifier == modifiers::ALT {
        return data == alt_legacy || matches_kitty_sequence(data, codepoint, modifiers::ALT);
    }
    if modifier == 0 {
        return matches_legacy_sequence(data, legacy_key(name).unwrap().plain)
            || matches_kitty_sequence(data, codepoint, 0);
    }
    if matches_legacy_modifier_sequence(data, name, modifier) {
        return true;
    }
    matches_kitty_sequence(data, codepoint, modifier)
}

fn matches_legacy_or_kitty_functional(
    data: &str,
    modifier: u32,
    name: &str,
    codepoint: i32,
) -> bool {
    if modifier == 0 {
        return matches_legacy_sequence(data, legacy_key(name).unwrap().plain)
            || matches_kitty_sequence(data, codepoint, 0);
    }
    if matches_legacy_modifier_sequence(data, name, modifier) {
        return true;
    }
    matches_kitty_sequence(data, codepoint, modifier)
}

fn matches_enter(data: &str, modifier: u32, kitty_active: bool) -> bool {
    if modifier == modifiers::SHIFT {
        if matches_kitty_sequence(data, codepoints::ENTER, modifiers::SHIFT)
            || matches_kitty_sequence(data, codepoints::KP_ENTER, modifiers::SHIFT)
        {
            return true;
        }
        if matches_modify_other_keys(data, codepoints::ENTER, modifiers::SHIFT) {
            return true;
        }
        if kitty_active {
            return data == "\x1b\r" || data == "\n";
        }
        return false;
    }
    if modifier == modifiers::ALT {
        if matches_kitty_sequence(data, codepoints::ENTER, modifiers::ALT)
            || matches_kitty_sequence(data, codepoints::KP_ENTER, modifiers::ALT)
        {
            return true;
        }
        if matches_modify_other_keys(data, codepoints::ENTER, modifiers::ALT) {
            return true;
        }
        if !kitty_active {
            return data == "\x1b\r";
        }
        return false;
    }
    if modifier == 0 {
        return data == "\r"
            || (!kitty_active && data == "\n")
            || data == "\x1bOM"
            || matches_kitty_sequence(data, codepoints::ENTER, 0)
            || matches_kitty_sequence(data, codepoints::KP_ENTER, 0);
    }
    matches_kitty_sequence(data, codepoints::ENTER, modifier)
        || matches_kitty_sequence(data, codepoints::KP_ENTER, modifier)
        || matches_modify_other_keys(data, codepoints::ENTER, modifier)
}

fn matches_backspace(data: &str, modifier: u32) -> bool {
    if modifier == modifiers::ALT {
        if data == "\x1b\x7f" || data == "\x1b\x08" {
            return true;
        }
        return matches_kitty_sequence(data, codepoints::BACKSPACE, modifiers::ALT)
            || matches_modify_other_keys(data, codepoints::BACKSPACE, modifiers::ALT);
    }
    if modifier == modifiers::CTRL {
        if matches_raw_backspace(data, modifiers::CTRL) {
            return true;
        }
        return matches_kitty_sequence(data, codepoints::BACKSPACE, modifiers::CTRL)
            || matches_modify_other_keys(data, codepoints::BACKSPACE, modifiers::CTRL);
    }
    if modifier == 0 {
        return matches_raw_backspace(data, 0)
            || matches_kitty_sequence(data, codepoints::BACKSPACE, 0)
            || matches_modify_other_keys(data, codepoints::BACKSPACE, 0);
    }
    matches_kitty_sequence(data, codepoints::BACKSPACE, modifier)
        || matches_modify_other_keys(data, codepoints::BACKSPACE, modifier)
}

fn matches_printable(data: &str, key: &str, modifier: u32, kitty_active: bool) -> bool {
    let Some(k) = key.chars().next() else {
        return false;
    };
    if key.chars().count() != 1
        || !(k.is_ascii_lowercase() || is_digit_key(key) || is_symbol_key(k))
    {
        return false;
    }

    let codepoint = k as i32;
    let raw_ctrl = raw_ctrl_char(key);
    let is_letter = k.is_ascii_lowercase();
    let is_digit = is_digit_key(key);

    if modifier == modifiers::CTRL + modifiers::ALT && !kitty_active {
        if let Some(rc) = raw_ctrl {
            if data == format!("\x1b{rc}") {
                return true;
            }
        }
    }

    if modifier == modifiers::ALT
        && !kitty_active
        && (is_letter || is_digit)
        && data == format!("\x1b{key}")
    {
        return true;
    }

    if modifier == modifiers::CTRL {
        if let Some(rc) = raw_ctrl {
            if data.chars().eq([rc]) {
                return true;
            }
        }
        return matches_kitty_sequence(data, codepoint, modifiers::CTRL)
            || matches_printable_modify_other_keys(data, codepoint, modifiers::CTRL);
    }

    if modifier == modifiers::SHIFT + modifiers::CTRL {
        return matches_kitty_sequence(data, codepoint, modifiers::SHIFT + modifiers::CTRL)
            || matches_printable_modify_other_keys(
                data,
                codepoint,
                modifiers::SHIFT + modifiers::CTRL,
            );
    }

    if modifier == modifiers::SHIFT {
        if is_letter && data == key.to_uppercase() {
            return true;
        }
        return matches_kitty_sequence(data, codepoint, modifiers::SHIFT)
            || matches_printable_modify_other_keys(data, codepoint, modifiers::SHIFT);
    }

    if modifier != 0 {
        return matches_kitty_sequence(data, codepoint, modifier)
            || matches_printable_modify_other_keys(data, codepoint, modifier);
    }

    data == key || matches_kitty_sequence(data, codepoint, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::set_kitty_protocol_active;

    #[test]
    fn test_matches_key_escape() {
        let _kitty = crate::state::KITTY_TEST_LOCK
            .read()
            .unwrap_or_else(|e| e.into_inner());
        assert!(matches_key("\x1b", "escape"));
        assert!(!matches_key("a", "escape"));
    }

    #[test]
    fn test_matches_key_plain_letter() {
        let _kitty = crate::state::KITTY_TEST_LOCK
            .read()
            .unwrap_or_else(|e| e.into_inner());
        assert!(matches_key("a", "a"));
        assert!(!matches_key("b", "a"));
    }

    #[test]
    fn test_matches_key_ctrl_c() {
        let _kitty = crate::state::KITTY_TEST_LOCK
            .read()
            .unwrap_or_else(|e| e.into_inner());
        assert!(matches_key("\x03", "ctrl+c"));
    }

    #[test]
    fn test_matches_key_ctrl_minus_maps_to_unit_separator() {
        let _kitty = crate::state::KITTY_TEST_LOCK
            .read()
            .unwrap_or_else(|e| e.into_inner());
        assert!(matches_key("\x1f", "ctrl+-"));
    }

    #[test]
    fn test_matches_key_shift_letter_is_uppercase() {
        let _kitty = crate::state::KITTY_TEST_LOCK
            .read()
            .unwrap_or_else(|e| e.into_inner());
        assert!(matches_key("A", "shift+a"));
    }

    #[test]
    fn test_matches_key_tab() {
        let _kitty = crate::state::KITTY_TEST_LOCK
            .read()
            .unwrap_or_else(|e| e.into_inner());
        assert!(matches_key("\t", "tab"));
        assert!(matches_key("\x1b[Z", "shift+tab"));
    }

    #[test]
    fn test_matches_key_enter() {
        let _kitty = crate::state::KITTY_TEST_LOCK
            .read()
            .unwrap_or_else(|e| e.into_inner());
        assert!(matches_key("\r", "enter"));
    }

    #[test]
    fn test_matches_key_backspace() {
        let _kitty = crate::state::KITTY_TEST_LOCK
            .read()
            .unwrap_or_else(|e| e.into_inner());
        assert!(matches_key("\x7f", "backspace"));
    }

    #[test]
    fn test_matches_key_arrow_up() {
        let _kitty = crate::state::KITTY_TEST_LOCK
            .read()
            .unwrap_or_else(|e| e.into_inner());
        assert!(matches_key("\x1b[A", "up"));
        assert!(matches_key("\x1bOA", "up"));
    }

    #[test]
    fn test_matches_key_ctrl_arrow_via_kitty() {
        let _kitty = crate::state::KITTY_TEST_LOCK
            .read()
            .unwrap_or_else(|e| e.into_inner());
        assert!(matches_key("\x1b[1;5A", "ctrl+up"));
    }

    #[test]
    fn test_matches_key_alt_left_legacy() {
        let _kitty = crate::state::KITTY_TEST_LOCK
            .read()
            .unwrap_or_else(|e| e.into_inner());
        assert!(matches_key("\x1bb", "alt+left"));
    }

    #[test]
    fn test_matches_key_function_key() {
        let _kitty = crate::state::KITTY_TEST_LOCK
            .read()
            .unwrap_or_else(|e| e.into_inner());
        assert!(matches_key("\x1bOP", "f1"));
    }

    #[test]
    fn test_matches_key_kitty_ctrl_c_when_protocol_active() {
        let _kitty = crate::state::KITTY_TEST_LOCK
            .write()
            .unwrap_or_else(|e| e.into_inner());
        set_kitty_protocol_active(true);
        assert!(matches_key("\x1b[99;5u", "ctrl+c"));
        set_kitty_protocol_active(false);
    }

    #[test]
    fn test_matches_key_invalid_key_id() {
        let _kitty = crate::state::KITTY_TEST_LOCK
            .read()
            .unwrap_or_else(|e| e.into_inner());
        assert!(!matches_key("a", ""));
    }

    #[test]
    fn test_matches_key_combined_modifiers() {
        let _kitty = crate::state::KITTY_TEST_LOCK
            .read()
            .unwrap_or_else(|e| e.into_inner());
        assert!(matches_key("\x1b[99;7u", "ctrl+alt+c"));
    }

    #[test]
    fn key_spec_parses_once_and_matches_like_matches_key() {
        let _kitty = crate::state::KITTY_TEST_LOCK
            .read()
            .unwrap_or_else(|e| e.into_inner());
        let spec = KeySpec::parse("Shift+Ctrl+P").expect("a key name");
        assert_eq!(spec, KeySpec::parse("shift+ctrl+p").unwrap());
        assert!(spec.matches("\x1b[112;6u"));
        for data in ["\x1b[112;6u", "P", "\x10", "p", "\x1b[1;5A"] {
            assert_eq!(
                spec.matches(data),
                matches_key(data, "shift+ctrl+p"),
                "{data:?}"
            );
        }
        assert!(KeySpec::parse("").is_none());
        assert!(KeySpec::parse("ctrl+").is_none());
    }

    #[test]
    fn test_matches_key_left_and_right_with_modifiers() {
        let _kitty = crate::state::KITTY_TEST_LOCK
            .read()
            .unwrap_or_else(|e| e.into_inner());
        assert!(matches_key("\x1b[1;5D", "ctrl+left"));
        assert!(matches_key("\x1bf", "alt+right"));
        assert!(matches_key("\x1b[1;3C", "alt+right"));
        assert!(!matches_key("\x1b[1;5D", "ctrl+right"));
        assert!(matches_key("\x1b[D", "left"));
    }
}

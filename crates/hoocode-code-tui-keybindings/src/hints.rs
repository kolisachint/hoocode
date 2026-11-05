//! Key text for hint lines (`modes/interactive/components/keybinding-hints.ts`).

use hoocode_code_tui_theme::theme;
pub use hoocode_tui_keys::SPACE_KEY;
use hoocode_tui_keys::{get_keybindings, matches_key};

use crate::keybindings::keybinding;

/// On macOS the alt key is labelled Option.
fn format_key_part(part: &str, capitalize: bool) -> String {
    let display = if cfg!(target_os = "macos") && part.eq_ignore_ascii_case("alt") {
        "option"
    } else {
        part
    };
    if !capitalize {
        return display.to_string();
    }
    let mut chars = display.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// One step of a key (no `/` or space): `+`-joined chords. A bare
/// single-character key prints exactly as typed.
fn format_step(step: &str, capitalize: bool) -> String {
    let parts: Vec<&str> = step.split('+').collect();
    if parts.len() == 1 && parts[0].encode_utf16().count() == 1 {
        return parts[0].to_string();
    }
    parts
        .iter()
        .map(|part| format_key_part(part, capitalize))
        .collect::<Vec<_>>()
        .join("+")
}

/// `formatKeyText`: `a/b` alternatives, `+`-joined chords, and a sequence's
/// steps separated by a space (`ctrl+x m` reads `Ctrl+X m`). A bare
/// single-character key prints exactly as typed (an uppercase letter would
/// name `shift+<letter>`); inside a chord `capitalize` capitalizes each part.
pub fn format_key_text(key: &str, capitalize: bool) -> String {
    key.split('/')
        .map(|alternative| {
            alternative
                .split_whitespace()
                .map(|step| format_step(step, capitalize))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn format_keys(keys: &[String], capitalize: bool) -> String {
    if keys.is_empty() {
        return String::new();
    }
    format_key_text(&keys.join("/"), capitalize)
}

/// The keys bound to `id` in the installed keymap. Before a keymap is installed
/// (tests, early startup) the app's defaults stand in, as [`matches_app_key`]
/// does, so a hint for a known action is never empty.
fn keys_of(id: &str) -> Vec<String> {
    let keybindings = get_keybindings();
    if keybindings.get_definition(id).is_some() {
        return keybindings.get_keys(id);
    }
    app_default_keys(id)
        .into_iter()
        .map(str::to_string)
        .collect()
}

/// `keyText`: every key bound to `id`, as typed.
pub fn key_text(id: &str) -> String {
    format_keys(&keys_of(id), false)
}

/// `keyDisplayText`: every key bound to `id`, capitalized.
pub fn key_display_text(id: &str) -> String {
    format_keys(&keys_of(id), true)
}

/// The first key bound to `id`, as typed, with `escape` shown as `esc`: the
/// key a hint names when one key is enough.
pub fn key_hint_text(id: &str) -> String {
    let first = keys_of(id).first().cloned().unwrap_or_default();
    match format_key_text(&first, false).as_str() {
        "escape" => "esc".to_string(),
        text => text.to_string(),
    }
}

/// The arrow glyph for a bare arrow key, so `↑↓` stays as it reads on screen.
fn arrow_glyph(key: &str) -> Option<&'static str> {
    match key {
        "up" => Some("\u{2191}"),
        "down" => Some("\u{2193}"),
        "left" => Some("\u{2190}"),
        "right" => Some("\u{2192}"),
        _ => None,
    }
}

/// The first key bound to `id`, with a bare arrow shown as its glyph (`←`).
pub fn key_glyph_text(id: &str) -> String {
    let key = key_hint_text(id);
    arrow_glyph(&key).map(str::to_string).unwrap_or(key)
}

/// Two bindings shown as one hint, such as `↑↓` for the up and down keys. Bare
/// arrows print as glyphs; anything else prints as `first/second`.
pub fn key_pair_hint(first: &str, second: &str) -> String {
    let (a, b) = (key_hint_text(first), key_hint_text(second));
    match (arrow_glyph(&a), arrow_glyph(&b)) {
        (Some(x), Some(y)) => format!("{x}{y}"),
        _ => format!("{a}/{b}"),
    }
}

/// `keyDisplayLabel`: the taught (first) key only, capitalized.
pub fn key_display_label(id: &str) -> String {
    keys_of(id)
        .first()
        .map(|k| format_key_text(k, true))
        .unwrap_or_default()
}

/// `keyHint`: `<keys> <description>` in dim/muted.
pub fn key_hint(id: &str, description: &str) -> String {
    let t = theme();
    t.fg("dim", &key_text(id)) + &t.fg("muted", &format!(" {description}"))
}

/// `rawKeyHint`: a hint for a literal key.
pub fn raw_key_hint(key: &str, description: &str) -> String {
    let t = theme();
    t.fg("dim", &format_key_text(key, false)) + &t.fg("muted", &format!(" {description}"))
}

fn app_default_keys(id: &str) -> Vec<&'static str> {
    keybinding(id).map(|e| e.default_keys).unwrap_or_default()
}

/// `matchesAppKey`: match against the installed manager when it knows `id`,
/// else against the app definition's default keys (so a component never goes
/// dead under the bare library defaults).
pub fn matches_app_key(data: &str, id: &str) -> bool {
    let keybindings = get_keybindings();
    if keybindings.get_definition(id).is_some() {
        return keybindings.matches(data, id);
    }
    app_default_keys(id)
        .iter()
        .any(|key| matches_key(data, key))
}

/// `appKeyLabel`: the first configured key for an app binding.
pub fn app_key_label(id: &str) -> String {
    let keybindings = get_keybindings();
    if keybindings.get_definition(id).is_some() {
        if let Some(first) = keybindings.get_keys(id).into_iter().next() {
            return first;
        }
    }
    app_default_keys(id)
        .first()
        .map(|k| k.to_string())
        .unwrap_or_default()
}

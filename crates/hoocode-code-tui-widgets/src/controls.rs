//! The CONTROLS console (`/hotkeys`, M7 spec §4 d9): the keyboard with
//! the ctrl-layer bindings lit, and what each lit key does. Below 100
//! columns the drawing cannot fit, so it falls back to a key list in the
//! same console.
//!
//! Everything on screen comes from the real keybindings: the lit caps from
//! the bindings' effective keys (user customizations included), the actions
//! from their descriptions. Physical cap positions are the board; the text
//! is not hard-coded. Key names print literally (`ctrl`, `alt`, `esc` — no
//! platform `option`), so goldens hold on macOS and Linux.

use std::collections::BTreeMap;

use hoocode_code_tui_keybindings::{keybindings, KeybindingEntry};
use hoocode_tui_keys::get_keybindings;
use hoocode_tui_ratatui::{Line, Palette, Span, Style};
use hoocode_tui_render::Component;
use hoocode_tui_util::truncate_to_width;

use crate::console::{hints, Console};

/// The keyboard is drawn at the spec width; narrower falls back to the list.
const KEYBOARD_MIN_WIDTH: usize = 100;

/// The effective keys of a binding: the installed manager's, or the
/// definition's defaults where the manager knows nothing (so the console
/// never goes dead under the bare library defaults).
fn effective_keys(entry: &KeybindingEntry) -> Vec<String> {
    let kb = get_keybindings();
    if kb.get_definition(entry.id).is_some() {
        kb.get_keys(entry.id)
    } else {
        entry.default_keys.iter().map(|k| k.to_string()).collect()
    }
}

/// A key name with literal parts (`esc` for escape, lower case), so the
/// console reads the same on macOS and Linux.
fn lit_key(key: &str) -> String {
    key.split('+')
        .map(|part| {
            if part.eq_ignore_ascii_case("escape") {
                "esc".to_string()
            } else {
                part.to_ascii_lowercase()
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// The single letter a key names, with its layer: `g` is plain, `ctrl+g` is
/// ctrl, `alt+m` is alt. Chords with anything else name no cap.
fn layer_cap(key: &str) -> Option<(Layer, char)> {
    let lower = key.to_ascii_lowercase();
    let (layer, letter) = match lower.split_once('+') {
        None => {
            let mut chars = lower.chars();
            let c = chars.next().filter(|c| c.is_ascii_alphabetic())?;
            if chars.next().is_some() {
                return None;
            }
            (Layer::Plain, c)
        }
        Some(("ctrl", rest)) => (Layer::Ctrl, single_letter(rest)?),
        Some(("alt" | "option", rest)) => (Layer::Alt, single_letter(rest)?),
        _ => return None,
    };
    Some((layer, letter))
}

fn single_letter(s: &str) -> Option<char> {
    let mut chars = s.chars();
    let c = chars.next().filter(|c| c.is_ascii_alphabetic())?;
    if chars.next().is_some() {
        return None;
    }
    Some(c)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layer {
    Plain,
    Ctrl,
    Alt,
}

/// The caps of one keyboard row that have a binding in `layer`.
fn lit_caps(
    entries: &[(&KeybindingEntry, Vec<String>)],
    layer: Layer,
    letters: &[char],
) -> Vec<char> {
    letters
        .iter()
        .copied()
        .filter(|cap| {
            entries
                .iter()
                .any(|(_, keys)| keys.iter().any(|k| layer_cap(k) == Some((layer, *cap))))
        })
        .collect()
}

/// The ctrl-layer bindings by cap, in board order: the lit keys and what
/// each does. Later entries win a shared cap, so app bindings name a key
/// over the TUI library's editing defaults.
fn ctrl_actions(entries: &[(&KeybindingEntry, Vec<String>)]) -> Vec<(char, String, String)> {
    let mut by_cap: BTreeMap<char, (&KeybindingEntry, String)> = BTreeMap::new();
    for (entry, keys) in entries {
        for key in keys {
            if let Some((Layer::Ctrl, cap)) = layer_cap(key) {
                by_cap.insert(cap, (*entry, lit_key(key)));
            }
        }
    }
    by_cap
        .into_iter()
        .map(|(cap, (entry, key))| (cap, key, entry.description.to_string()))
        .collect()
}

/// One keyboard row: the label cap plus the letter caps, lit caps in
/// `lit`, the rest faint.
fn keyboard_row(label: &str, letters: &[char], lit: &[char], lit_style: Style) -> Line<'static> {
    let pal = Palette::current();
    let mut spans = vec![Span::styled(label.to_string(), pal.dim()), Span::raw("  ")];
    for letter in letters {
        let style = if lit.contains(letter) {
            lit_style
        } else {
            pal.faint()
        };
        spans.push(Span::styled(format!("{letter} "), style));
    }
    Line::from(spans)
}

/// The keyboard board: esc and digits, tab's row, ctrl's row, shift's row.
/// Only caps with a real binding light up; the board itself is positions.
fn keyboard(entries: &[(&KeybindingEntry, Vec<String>)]) -> Vec<Line<'static>> {
    let pal = Palette::current();
    let digits: Vec<char> = "1234567890".chars().collect();
    let top: Vec<char> = "qwertyuiop".chars().collect();
    let home: Vec<char> = "asdfghjkl".chars().collect();
    let bottom: Vec<char> = "zxcvbnm".chars().collect();
    let mut rows = Vec::new();
    let mut esc_row = vec![Span::styled("esc".to_string(), {
        let lit = entries
            .iter()
            .any(|(_, keys)| keys.iter().any(|k| k.eq_ignore_ascii_case("escape")));
        if lit {
            Style::default()
        } else {
            pal.faint()
        }
    })];
    esc_row.push(Span::raw("  "));
    for digit in &digits {
        let lit = entries.iter().any(|(_, keys)| {
            keys.iter().any(|k| {
                let lower = k.to_ascii_lowercase();
                lower == digit.to_string()
            })
        });
        esc_row.push(Span::styled(
            format!("{digit} "),
            if lit { Style::default() } else { pal.faint() },
        ));
    }
    rows.push(Line::from(esc_row));
    rows.push(keyboard_row(
        "tab",
        &top,
        &lit_caps(entries, Layer::Plain, &top),
        Style::default(),
    ));
    rows.push(keyboard_row(
        "ctrl",
        &home,
        &lit_caps(entries, Layer::Ctrl, &home),
        pal.acc(),
    ));
    rows.push(keyboard_row(
        "shift",
        &bottom,
        &lit_caps(entries, Layer::Plain, &bottom),
        Style::default(),
    ));
    rows
}

/// One `key action` row: the key in ink, the action faint.
fn action_row(key: &str, action: &str, inner: usize) -> Line<'static> {
    let pal = Palette::current();
    let text = format!("{key}  {action}");
    let text = truncate_to_width(&text, inner, "…", false);
    let mut spans = vec![Span::styled(key.to_string(), Style::default())];
    let rest = text.strip_prefix(key).unwrap_or(&text);
    spans.push(Span::styled(rest.to_string(), pal.faint()));
    Line::from(spans)
}

/// The full key list: every binding with its first effective key.
fn key_list(entries: &[(&KeybindingEntry, Vec<String>)], inner: usize) -> Vec<Line<'static>> {
    entries
        .iter()
        .filter_map(|(entry, keys)| {
            keys.first()
                .map(|key| action_row(&lit_key(key), entry.description, inner))
        })
        .collect()
}

/// `ControlsComponent`: the CONTROLS console. No state: it reads the live
/// keybindings on every render, so a rebound key repaints the board.
pub struct ControlsComponent;

impl ControlsComponent {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ControlsComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for ControlsComponent {
    /// At the spec width the drawn keyboard with the ctrl-layer actions;
    /// narrower the full key list. Both come from the real keybindings.
    fn render(&mut self, width: u16) -> Vec<String> {
        let pal = Palette::current();
        let width = width as usize;
        let all = keybindings();
        let entries: Vec<(&KeybindingEntry, Vec<String>)> = all
            .iter()
            .map(|entry| (entry, effective_keys(entry)))
            .collect();
        let count = entries.len();
        let mut console = Console::new("CONTROLS · /hotkeys", format!("{} bindings", count));
        // The layer tabs (M8d9): the ctrl layer is the lit one.
        let tabs = Line::from(vec![
            Span::styled("plain ".to_string(), pal.dim()),
            Span::styled("ctrl ".to_string(), pal.acc()),
            Span::styled("alt".to_string(), pal.dim()),
        ]);
        if width >= KEYBOARD_MIN_WIDTH {
            let mut top = vec![tabs];
            top.extend(keyboard(&entries));
            console.top = top;
            console.right = Line::from(vec![Span::styled("ctrl layer", pal.acc())]);
            let inner = width.saturating_sub(4);
            console.rows = ctrl_actions(&entries)
                .into_iter()
                .map(|(_, key, action)| action_row(&key, &action, inner))
                .collect();
        } else {
            let inner = width.saturating_sub(4);
            console.rows = key_list(&entries, inner);
        }
        // The target's `keysRows` foot: the layer switch and the way out.
        console.hint = hints(&[("tab", "layer"), ("esc", "close")]);
        console.render(width)
    }

    fn is_focusable(&self) -> bool {
        false
    }
}

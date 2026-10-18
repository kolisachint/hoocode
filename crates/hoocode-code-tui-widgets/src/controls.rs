//! The CONTROLS console (`/hotkeys`, M7 spec §4 d9): the keyboard with
//! one layer's bindings lit, and what each lit key does. Below 100
//! columns the drawing cannot fit, so it falls back to a key list in
//! the same console.
//!
//! Everything on screen comes from the real keybindings: the lit caps from
//! the bindings' effective keys (user customizations included), the actions
//! from their descriptions. Physical cap positions are the board; the text
//! is not hard-coded. Key names print literally (`ctrl`, `alt`, `esc` — no
//! platform `option`), so goldens hold on macOS and Linux.
//!
//! The console is a picker: `tab` switches the lit layer (plain, ctrl,
//! alt), typing filters the action list, and `esc` closes it. The title
//! tabs name the layers with the lit one accented, and the foot only
//! promises those three keys.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use hoocode_code_tui_keybindings::{keybindings, KeybindingEntry};
use hoocode_code_tui_theme::style_input;
use hoocode_tui_components::Input;
use hoocode_tui_keys::get_keybindings;
use hoocode_tui_ratatui::{Line, Palette, Span, Style};
use hoocode_tui_render::Component;
use hoocode_tui_util::{truncate_to_width, visible_width};

use crate::console::{hints, query_row, Console};

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

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Layer {
    Plain,
    #[default]
    Ctrl,
    Alt,
}

impl Layer {
    /// `tab` order on the title row.
    fn next(self) -> Self {
        match self {
            Layer::Plain => Layer::Ctrl,
            Layer::Ctrl => Layer::Alt,
            Layer::Alt => Layer::Plain,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Layer::Plain => "plain",
            Layer::Ctrl => "ctrl",
            Layer::Alt => "alt",
        }
    }
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

/// The bindings of one layer by cap, in board order: the lit keys and what
/// each does. Later entries win a shared cap, so app bindings name a key
/// over the TUI library's editing defaults. `query` keeps the actions whose
/// key or description contains it; empty keeps all.
fn layer_actions(
    entries: &[(&KeybindingEntry, Vec<String>)],
    layer: Layer,
    query: &str,
) -> Vec<(char, String, String)> {
    let mut by_cap: BTreeMap<char, (&KeybindingEntry, String)> = BTreeMap::new();
    for (entry, keys) in entries {
        for key in keys {
            if let Some((key_layer, cap)) = layer_cap(key) {
                if key_layer == layer {
                    by_cap.insert(cap, (*entry, lit_key(key)));
                }
            }
        }
    }
    let needle = query.to_lowercase();
    by_cap
        .into_iter()
        .filter(|(_, (entry, key))| {
            needle.is_empty()
                || key.to_lowercase().contains(&needle)
                || entry.description.to_lowercase().contains(&needle)
        })
        .map(|(cap, (entry, key))| (cap, key, entry.description.to_string()))
        .collect()
}

/// One keyboard row: the label cap plus the letter caps, lit caps in
/// `lit_style`, the rest faint.
fn keyboard_row(label: &str, letters: &[char], lit: &[char], lit_style: Style) -> Line<'static> {
    let pal = Palette::current();
    let mut spans = vec![
        Span::styled(label.to_string(), pal.dim()),
        Span::raw("  ".to_string()),
    ];
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
/// Only caps with a binding in `layer` light up; the board itself is positions.
fn keyboard(entries: &[(&KeybindingEntry, Vec<String>)], layer: Layer) -> Vec<Line<'static>> {
    let pal = Palette::current();
    let digits: Vec<char> = "1234567890".chars().collect();
    let top: Vec<char> = "qwertyuiop".chars().collect();
    let home: Vec<char> = "asdfghjkl".chars().collect();
    let bottom: Vec<char> = "zxcvbnm".chars().collect();
    let lit_style = if layer == Layer::Plain {
        Style::default()
    } else {
        pal.acc()
    };
    let mut rows = Vec::new();
    let mut esc_row = vec![Span::styled("esc  ".to_string(), {
        let lit = entries
            .iter()
            .any(|(_, keys)| keys.iter().any(|k| k.eq_ignore_ascii_case("escape")));
        if lit && layer == Layer::Plain {
            Style::default()
        } else {
            pal.faint()
        }
    })];
    esc_row.push(Span::raw("  ".to_string()));
    for digit in &digits {
        let lit = entries.iter().any(|(_, keys)| {
            keys.iter().any(|k| {
                let lower = k.to_ascii_lowercase();
                lower == digit.to_string()
            })
        });
        esc_row.push(Span::styled(
            format!("{digit} "),
            if lit && layer == Layer::Plain {
                Style::default()
            } else {
                pal.faint()
            },
        ));
    }
    rows.push(Line::from(esc_row));
    rows.push(keyboard_row(
        "tab  ",
        &top,
        &lit_caps(entries, layer, &top),
        lit_style,
    ));
    rows.push(keyboard_row(
        " ctrl",
        &home,
        &lit_caps(entries, layer, &home),
        lit_style,
    ));
    rows.push(keyboard_row(
        "shift",
        &bottom,
        &lit_caps(entries, layer, &bottom),
        lit_style,
    ));
    rows
}

/// One `key action` row's right side: the key in ink, the action faint.
fn action_spans(key: &str, action: &str) -> Vec<Span<'static>> {
    let pal = Palette::current();
    vec![
        Span::styled(key.to_string(), Style::default()),
        Span::styled(format!("  {action}"), pal.faint()),
    ]
}

/// The board with the layer's action list down the right side (M8d9): the
/// first row names the lit layer, then each action reads beside the board.
/// Extra actions past the board read on action-only rows.
fn board_rows(
    entries: &[(&KeybindingEntry, Vec<String>)],
    layer: Layer,
    actions: &[(char, String, String)],
    query: &str,
    inner: usize,
) -> Vec<Line<'static>> {
    let pal = Palette::current();
    let boards = keyboard(entries, layer);
    let board_width = boards
        .iter()
        .map(|row| row.width())
        .max()
        .unwrap_or(0)
        .min(inner);
    let mut right: Vec<Line<'static>> = Vec::with_capacity(actions.len() + 1);
    right.push(Line::from(vec![Span::styled(
        format!("{} layer", layer.name()),
        pal.acc(),
    )]));
    for (_, key, action) in actions {
        right.push(Line::from(action_spans(key, action)));
    }
    if actions.is_empty() && !query.is_empty() {
        right.push(Line::from(vec![Span::styled(
            "No matching bindings".to_string(),
            pal.faint(),
        )]));
    }
    let total = right.len().max(boards.len() + 3).max(7);
    let mut rows = Vec::with_capacity(total);
    for (i, right) in right.into_iter().enumerate().take(total) {
        let left = if i.is_multiple_of(2) {
            boards.get(i / 2).cloned().unwrap_or_default()
        } else {
            Line::from("")
        };
        let used = left.width().min(inner);
        let mut spans = left.spans.clone();
        spans.push(Span::raw(" ".repeat(board_width.saturating_sub(used) + 2)));
        spans.extend(right.spans.iter().cloned());
        rows.push(Line::from(truncate_spans(spans, inner)));
    }
    rows
}

/// Cut spans to `max` cells without styling the cut.
fn truncate_spans(spans: Vec<Span<'static>>, max: usize) -> Vec<Span<'static>> {
    let mut out = Vec::with_capacity(spans.len());
    let mut used = 0;
    for span in spans {
        let width = visible_width(span.content.as_ref());
        if used + width <= max {
            used += width;
            out.push(span);
            continue;
        }
        let room = max.saturating_sub(used);
        if room == 0 {
            break;
        }
        let mut piece = String::new();
        let mut piece_width = 0;
        for c in span.content.chars() {
            let w = visible_width(&c.to_string());
            if piece_width + w > room {
                break;
            }
            piece_width += w;
            piece.push(c);
        }
        if !piece.is_empty() {
            out.push(Span::styled(piece, span.style));
        }
        break;
    }
    out
}

/// The full key list: every binding with its first effective key, kept by
/// the query. The narrow fallback when the board cannot fit.
fn key_list(
    entries: &[(&KeybindingEntry, Vec<String>)],
    query: &str,
    inner: usize,
) -> Vec<Line<'static>> {
    let pal = Palette::current();
    let needle = query.to_lowercase();
    let mut rows: Vec<Line<'static>> = entries
        .iter()
        .filter(|(entry, keys)| {
            needle.is_empty()
                || keys
                    .first()
                    .is_some_and(|key| lit_key(key).to_lowercase().contains(&needle))
                || entry.description.to_lowercase().contains(&needle)
        })
        .filter_map(|(entry, keys)| {
            keys.first().map(|key| {
                let text = format!("{}  {}", lit_key(key), entry.description);
                Line::from(truncate_to_width(&text, inner, "…", false))
            })
        })
        .collect();
    if rows.is_empty() {
        rows.push(Line::from(vec![Span::styled(
            "No matching bindings".to_string(),
            pal.faint(),
        )]));
    }
    rows
}

/// What the picker owes the app: only closing. Layer and search are the
/// console's own state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlsEvent {
    Close,
}

/// `ControlsComponent`: the CONTROLS console. Reads the live keybindings on
/// every render, so a rebound key repaints the board. `tab` switches the lit
/// layer, typing filters the actions, `esc` closes the picker.
pub struct ControlsComponent {
    layer: Layer,
    search: Rc<RefCell<Input>>,
    events: Vec<ControlsEvent>,
    focused: bool,
}

impl ControlsComponent {
    pub fn new() -> Self {
        let mut search = Input::new();
        style_input(&mut search);
        Self {
            layer: Layer::Ctrl,
            search: Rc::new(RefCell::new(search)),
            events: Vec::new(),
            focused: false,
        }
    }

    /// The events the app has not collected yet.
    pub fn take_events(&mut self) -> Vec<ControlsEvent> {
        std::mem::take(&mut self.events)
    }

    /// The layer tabs for the title edge (M8d9): the lit layer accented.
    fn tabs(&self) -> Line<'static> {
        let pal = Palette::current();
        let mut spans = Vec::new();
        for layer in [Layer::Plain, Layer::Ctrl, Layer::Alt] {
            if !spans.is_empty() {
                spans.push(Span::raw("   ".to_string()));
            }
            let style = if layer == self.layer {
                pal.acc()
            } else {
                pal.dim()
            };
            spans.push(Span::styled(layer.name().to_string(), style));
        }
        Line::from(spans)
    }
}

impl Default for ControlsComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for ControlsComponent {
    /// At the spec width the drawn keyboard with the lit layer's actions;
    /// narrower the full key list. Both come from the real keybindings.
    fn render(&mut self, width: u16) -> Vec<String> {
        let width = width as usize;
        let all = keybindings();
        let entries: Vec<(&KeybindingEntry, Vec<String>)> = all
            .iter()
            .map(|entry| (entry, effective_keys(entry)))
            .collect();
        let count = entries.len();
        let mut console = Console::new("CONTROLS · ?", format!("{count} bindings"));
        console.tabs = self.tabs();
        let query = self.search.borrow().get_value().to_string();
        console.query = Some(query_row(&query, self.focused));
        if width >= KEYBOARD_MIN_WIDTH {
            let inner = width.saturating_sub(4);
            let actions = layer_actions(&entries, self.layer, &query);
            console.rows = board_rows(&entries, self.layer, &actions, &query, inner);
        } else {
            let inner = width.saturating_sub(4);
            console.rows = key_list(&entries, &query, inner);
        }
        // The foot only promises what the console does.
        console.hint = hints(&[("tab", "layer"), ("type", "search"), ("esc", "close")]);
        console.render(width)
    }

    /// The picker focuses the console so `tab` and the search hear keys.
    fn handle_input(&mut self, data: &str) {
        let kb = get_keybindings();
        if kb.matches(data, "tui.input.tab") {
            self.layer = self.layer.next();
            return;
        }
        if kb.matches(data, "tui.select.cancel") {
            self.events.push(ControlsEvent::Close);
            return;
        }
        self.search.borrow_mut().handle_input_with(data, &kb);
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
        self.search.borrow_mut().set_focused(focused);
    }

    fn invalidate(&mut self) {
        self.search.borrow_mut().invalidate();
    }

    fn is_focusable(&self) -> bool {
        true
    }
}

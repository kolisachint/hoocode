//! The Keys pane (`/hotkeys`, which-key style): one layer's bindings in
//! a grid grouped by where the key acts, the five rows of "Where a key
//! acts" (docs/design/tui.md), each `key  what it does`. A key with a
//! different meaning in two places is listed under each. Below 100 columns
//! the groups take one column.
//!
//! Everything on screen comes from the real keybindings: the keys from the
//! bindings' effective keys (user customizations included), the actions
//! from their descriptions. Key names print literally (`ctrl`, `alt`,
//! `esc` — no platform `option`), so goldens hold on macOS and Linux.
//!
//! The pane is a picker: `tab` switches the layer (plain, ctrl, alt),
//! typing filters the actions, and `esc` closes it. The title tabs name the
//! layers with the shown one accented, and the hint row only promises those
//! keys.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use hoocode_code_tui_keybindings::{
    context_of, key_hint_text, keybindings, Context, KeybindingEntry,
};
use hoocode_code_tui_theme::style_input;
use hoocode_tui_components::Input;
use hoocode_tui_keys::get_keybindings;
use hoocode_tui_ratatui::{Line, Palette, Span};
use hoocode_tui_render::Component;
use hoocode_tui_util::visible_width;

use crate::console::{query_row, Console};

/// The groups sit side by side from this width; narrower they stack.
const GRID_MIN_WIDTH: usize = 100;

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
    // A leader sequence keeps its step as typed: `ctrl+x T` is not `ctrl+x t`.
    if let Some((lead, step)) = key.split_once(' ') {
        return format!("{} {step}", lit_key(lead));
    }
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

/// The layer a key is listed on, or `None` for a chord it cannot name.
/// A bare key (no modifier, or `shift` alone) is plain, whatever the key:
/// letters, digits, punctuation and named keys (`enter`, `esc`, arrows,
/// `f1`). `ctrl+` and `alt+` keys are their own layers. `ctrl+x <step>` is
/// the leader layer, the step kept as typed (K-P2), so `m` and `M` stay
/// apart. Any other modifier names no layer.
fn layer_of(key: &str) -> Option<Layer> {
    if let Some((lead, step)) = key.split_once(' ') {
        let ok = !lead.is_empty() && !step.is_empty() && !step.contains([' ', '+']);
        return ok.then_some(Layer::Leader);
    }
    let parts: Vec<&str> = key.split('+').collect();
    let (name, modifiers) = parts.split_last()?;
    if name.is_empty() {
        return None;
    }
    let modifiers: Vec<String> = modifiers.iter().map(|m| m.to_ascii_lowercase()).collect();
    match modifiers.as_slice() {
        [] => Some(Layer::Plain),
        [m] if m == "shift" => Some(Layer::Plain),
        [m] if m == "ctrl" => Some(Layer::Ctrl),
        [m] if m == "alt" || m == "option" => Some(Layer::Alt),
        _ => None,
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Layer {
    Plain,
    #[default]
    Ctrl,
    /// The keys after the leader (`ctrl+x`, then one key).
    Leader,
    Alt,
}

impl Layer {
    /// `tab` order on the title row.
    fn next(self) -> Self {
        match self {
            Layer::Plain => Layer::Ctrl,
            Layer::Ctrl => Layer::Leader,
            Layer::Leader => Layer::Alt,
            Layer::Alt => Layer::Plain,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Layer::Plain => "plain",
            Layer::Ctrl => "ctrl",
            Layer::Leader => "leader",
            Layer::Alt => "alt",
        }
    }
}

/// The five rows of "Where a key acts" (docs/design/tui.md), in table
/// order. The derived order is the table's, so groups sort by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Place {
    Prompt,
    Block,
    Picker,
    Dialog,
    Panel,
}

impl Place {
    const ALL: [Place; 5] = [
        Place::Prompt,
        Place::Block,
        Place::Picker,
        Place::Dialog,
        Place::Panel,
    ];

    /// The row's name, as the table writes it.
    fn name(self) -> &'static str {
        match self {
            Place::Prompt => "Prompt",
            Place::Block => "Block focused (pinned, prompt empty)",
            Place::Picker => "Picker or pane, empty filter",
            Place::Dialog => "Blocking dialog, empty reply",
            Place::Panel => "Task panel focused",
        }
    }
}

/// The row of "Where a key acts" a binding answers in, from its keymap
/// context. The transcript's keys are the Scroll context; a quit choice is
/// asked in a dialog, not at the prompt.
fn place_of(entry: &KeybindingEntry) -> Place {
    if entry.id.starts_with("app.quit.") {
        return Place::Dialog;
    }
    match context_of(entry.id) {
        Context::Global | Context::Prompt | Context::Leader => Place::Prompt,
        Context::Scroll => Place::Block,
        Context::Picker | Context::Subagent | Context::Tree | Context::Models => Place::Picker,
        Context::Panel => Place::Panel,
        Context::Dialog => Place::Dialog,
    }
}

/// One shown binding: its key as the layer prints it, its description, and
/// the row it acts in.
struct Action {
    key: String,
    text: String,
    place: Place,
}

/// The bindings of one layer: one row per key in each place, so every key
/// the layer holds is shown (a binding with two keys on it shows both). A
/// key shared by two bindings in one place shows the later one, as the app's
/// binding names a key over the TUI library's editing default. The same bare
/// key in two places shows once per place. `query` keeps the actions whose
/// key or description contains it; empty keeps all. Sorted by place, then key.
fn layer_actions(
    entries: &[(&KeybindingEntry, Vec<String>)],
    layer: Layer,
    query: &str,
) -> Vec<Action> {
    let mut by_place_key: BTreeMap<(Place, String), &KeybindingEntry> = BTreeMap::new();
    for (entry, keys) in entries {
        for key in keys {
            if layer_of(key) == Some(layer) {
                by_place_key.insert((place_of(entry), lit_key(key)), *entry);
            }
        }
    }
    let needle = query.to_lowercase();
    by_place_key
        .into_iter()
        .filter(|((_, key), entry)| {
            needle.is_empty()
                || key.to_lowercase().contains(&needle)
                || entry.description.to_lowercase().contains(&needle)
        })
        .map(|((place, key), entry)| Action {
            key,
            text: entry.description.to_string(),
            place,
        })
        .collect()
}

/// `items` as `cols` columns filled top to bottom: the key in normal text,
/// the action dim, columns four cells apart.
fn grid(items: &[&Action], cols: usize, inner: usize) -> Vec<Line<'static>> {
    let pal = Palette::current();
    if items.is_empty() {
        return Vec::new();
    }
    let cols = cols.clamp(1, items.len());
    let rows = items.len().div_ceil(cols);
    let key_width = items
        .iter()
        .map(|a| visible_width(&a.key))
        .max()
        .unwrap_or(0);
    let col_widths: Vec<usize> = (0..cols)
        .map(|c| {
            items
                .iter()
                .skip(c * rows)
                .take(rows)
                .map(|a| key_width + 2 + visible_width(&a.text))
                .max()
                .unwrap_or(0)
        })
        .collect();
    (0..rows)
        .map(|r| {
            let mut spans: Vec<Span<'static>> = Vec::new();
            for (c, col_width) in col_widths.iter().enumerate() {
                let Some(action) = items.get(c * rows + r) else {
                    continue;
                };
                let used = key_width + 2 + visible_width(&action.text);
                spans.push(Span::raw(format!("{:<key_width$}", action.key)));
                spans.push(Span::styled(format!("  {}", action.text), pal.dim()));
                if c + 1 < cols {
                    spans.push(Span::raw(" ".repeat(col_width - used + 4)));
                }
            }
            Line::from(truncate_spans(spans, inner))
        })
        .collect()
}

/// A group's header row: the row's name, in ink.
fn group_header(place: Place) -> Line<'static> {
    Line::from(vec![Span::styled(
        place.name().to_string(),
        Palette::current().ink(),
    )])
}

/// The width, in cells, of `items` laid out in `cols` columns by `grid`.
fn grid_width(items: &[&Action], cols: usize) -> usize {
    if items.is_empty() {
        return 0;
    }
    let cols = cols.clamp(1, items.len());
    let rows = items.len().div_ceil(cols);
    let key_width = items
        .iter()
        .map(|a| visible_width(&a.key))
        .max()
        .unwrap_or(0);
    let cells: usize = (0..cols)
        .map(|c| {
            items
                .iter()
                .skip(c * rows)
                .take(rows)
                .map(|a| key_width + 2 + visible_width(&a.text))
                .max()
                .unwrap_or(0)
        })
        .sum();
    cells + 4 * (cols - 1)
}

/// The five rows as groups, in table order: a header and its keys. Each
/// group takes the most columns that fit the width (three from 100
/// columns), so no key is cut off at the edge. Every row keeps its header,
/// so a row with nothing on this layer shows an empty group.
fn group_rows(actions: &[Action], width: usize) -> Vec<Line<'static>> {
    let inner = width.saturating_sub(2);
    let max_cols = if width >= GRID_MIN_WIDTH { 3 } else { 1 };
    let mut rows = Vec::new();
    for place in Place::ALL {
        rows.push(group_header(place));
        let items: Vec<&Action> = actions.iter().filter(|a| a.place == place).collect();
        let cols = (1..=max_cols)
            .rev()
            .find(|&c| grid_width(&items, c) <= inner)
            .unwrap_or(1);
        rows.extend(grid(&items, cols, inner));
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

/// What the picker owes the app: only closing. Layer and search are the
/// console's own state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlsEvent {
    Close,
}

/// `ControlsComponent`: the Keys pane. Reads the live keybindings on every
/// render, so a rebound key repaints the grid. `tab` switches the layer,
/// typing filters the actions, `esc` closes the picker.
pub struct ControlsComponent {
    layer: Layer,
    search: Rc<RefCell<Input>>,
    events: Vec<ControlsEvent>,
    focused: bool,
    /// Problems in `keybindings.json` (errors, then cautions), shown under the groups.
    problems: Vec<String>,
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
            problems: Vec::new(),
        }
    }

    /// Replaces the problems shown under the groups. An empty list hides the section.
    pub fn set_problems(&mut self, problems: Vec<String>) {
        self.problems = problems;
    }

    /// The events the app has not collected yet.
    pub fn take_events(&mut self) -> Vec<ControlsEvent> {
        std::mem::take(&mut self.events)
    }

    /// The layer tabs for the title edge (M8d9): the lit layer accented.
    fn tabs(&self) -> Line<'static> {
        let pal = Palette::current();
        let mut spans = Vec::new();
        for layer in [Layer::Plain, Layer::Ctrl, Layer::Leader, Layer::Alt] {
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
    /// The Keys pane: the layer's bindings grouped by where they act, then
    /// the filter row and the hint row. Both come from the real keybindings.
    fn render(&mut self, width: u16) -> Vec<String> {
        let width = width as usize;
        let all = keybindings();
        let entries: Vec<(&KeybindingEntry, Vec<String>)> = all
            .iter()
            .map(|entry| (entry, effective_keys(entry)))
            .collect();
        let query = self.search.borrow().get_value().to_string();
        let actions = layer_actions(&entries, self.layer, &query);
        let mut console = Console::new(
            "Keys · ?",
            format!("{} bindings · {} shown", entries.len(), actions.len()),
        );
        console.tabs = self.tabs();
        console.query = Some(query_row(&query, self.focused));
        console.top = group_rows(&actions, width);
        if actions.is_empty() {
            console.top = vec![Line::from(vec![Span::styled(
                "No matching bindings".to_string(),
                Palette::current().dim(),
            )])];
        }
        if !self.problems.is_empty() {
            let count = self.problems.len();
            let noun = if count == 1 { "problem" } else { "problems" };
            console.top.push(Line::from(vec![Span::styled(
                format!("keybindings.json · {count} {noun}"),
                Palette::current().warn(),
            )]));
            for problem in &self.problems {
                console.top.push(Line::from(truncate_spans(
                    vec![Span::styled(
                        format!("  {problem}"),
                        Palette::current().dim(),
                    )],
                    width,
                )));
            }
        }
        // The hint row only promises what the pane does, fitted to the
        // width like every other picker's.
        console.set_hints(
            &[
                (key_hint_text("tui.input.tab").as_str(), "layer"),
                (key_hint_text("tui.select.cancel").as_str(), "close"),
            ],
            false,
        );
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

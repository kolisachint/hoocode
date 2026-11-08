//! The Keys pane (`/hotkeys`, which-key style): one layer's bindings in
//! a grid grouped as prompt, transcript and app, each `key  what it does`.
//! Below 100 columns the groups stack in one column.
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
use std::collections::{BTreeMap, HashSet};
use std::rc::Rc;

use hoocode_code_tui_keybindings::{key_hint_text, keybindings, KeybindingEntry};
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

/// The single letter a key names, with its layer: `g` is plain, `ctrl+g` is
/// ctrl, `alt+m` is alt, and `ctrl+x m` (a leader sequence, K-P2) is leader
/// with the step's own character, so `m` and `M` stay apart. Chords with
/// anything else name no cap.
fn layer_cap(key: &str) -> Option<(Layer, char)> {
    if let Some((_, step)) = key.split_once(' ') {
        let mut chars = step.chars();
        return match (chars.next(), chars.next()) {
            (Some(c), None) => Some((Layer::Leader, c)),
            _ => None,
        };
    }
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

/// What part of the app a binding belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Group {
    Prompt,
    Transcript,
    App,
}

impl Group {
    fn name(self) -> &'static str {
        match self {
            Group::Prompt => "prompt",
            Group::Transcript => "transcript",
            Group::App => "app",
        }
    }
}

/// The group of a binding id: the editor and its clipboard keys are the
/// prompt; scrolling, tool output, thinking, subagent and tree keys are the
/// transcript; the rest is the app.
fn group_of(id: &str) -> Group {
    if id.starts_with("tui.") || id == "app.clear" || id.starts_with("app.clipboard.") {
        Group::Prompt
    } else if [
        "app.scroll.",
        "app.tools.",
        "app.thinking.",
        "app.subagent.",
        "app.tree.",
    ]
    .iter()
    .any(|prefix| id.starts_with(prefix))
    {
        Group::Transcript
    } else {
        Group::App
    }
}

/// One shown binding.
struct Action {
    key: String,
    text: String,
    group: Group,
}

/// The bindings of one layer by cap, in cap order. Later entries win a
/// shared cap, so app bindings name a key over the TUI library's editing
/// defaults. One row per binding: an entry with two keys on one layer
/// keeps its smallest cap, so `app.hotkeys.open` (`ctrl+x ?`, `ctrl+x k`)
/// prints `Show keyboard shortcuts` once, not twice. `query` keeps the
/// actions whose key or description contains it; empty keeps all.
fn layer_actions(
    entries: &[(&KeybindingEntry, Vec<String>)],
    layer: Layer,
    query: &str,
) -> Vec<Action> {
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
    let mut shown: HashSet<&str> = HashSet::new();
    by_cap
        .into_values()
        .filter(|(entry, key)| {
            needle.is_empty()
                || key.to_lowercase().contains(&needle)
                || entry.description.to_lowercase().contains(&needle)
        })
        .filter(|(entry, _)| shown.insert(entry.id))
        .map(|(entry, key)| Action {
            key,
            text: entry.description.to_string(),
            group: group_of(entry.id),
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

/// A group's header row.
fn group_header(group: Group) -> Line<'static> {
    Line::from(vec![Span::styled(
        group.name().to_string(),
        Palette::current().dim(),
    )])
}

/// The groups as rows: prompt first (three columns across), then
/// transcript and app side by side when the width allows, else stacked.
fn group_rows(actions: &[Action], width: usize) -> Vec<Line<'static>> {
    let inner = width.saturating_sub(2);
    let of =
        |group: Group| -> Vec<&Action> { actions.iter().filter(|a| a.group == group).collect() };
    let wide = width >= GRID_MIN_WIDTH;
    let mut rows = Vec::new();
    let prompt = of(Group::Prompt);
    if !prompt.is_empty() {
        rows.push(group_header(Group::Prompt));
        rows.extend(grid(&prompt, if wide { 3 } else { 1 }, inner));
    }
    let column = |group: Group| -> Vec<Line<'static>> {
        let items = of(group);
        if items.is_empty() {
            return Vec::new();
        }
        let mut lines = vec![group_header(group)];
        lines.extend(grid(&items, 1, inner));
        lines
    };
    let (left, right) = (column(Group::Transcript), column(Group::App));
    let left_width = left.iter().map(|l| l.width()).max().unwrap_or(0) + 4;
    let right_width = right.iter().map(|l| l.width()).max().unwrap_or(0);
    if wide && !left.is_empty() && !right.is_empty() && left_width + right_width <= inner {
        for i in 0..left.len().max(right.len()) {
            let mut spans = left.get(i).map(|l| l.spans.clone()).unwrap_or_default();
            let used = left.get(i).map(|l| l.width()).unwrap_or(0);
            if let Some(line) = right.get(i) {
                spans.push(Span::raw(" ".repeat(left_width - used)));
                spans.extend(line.spans.iter().cloned());
            }
            rows.push(Line::from(spans));
        }
    } else {
        rows.extend(left);
        rows.extend(right);
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
    /// The Keys pane: the layer's bindings grouped as prompt, transcript
    /// and app, then the filter row and the hint row. Both come from the
    /// real keybindings.
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

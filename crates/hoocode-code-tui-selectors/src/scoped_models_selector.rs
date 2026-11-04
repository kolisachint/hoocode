//! The `/scoped-models` picker, hoocode `components/scoped-models-selector.ts`:
//! which models model cycling steps through, in what order, and with each
//! model's effort and category (scoped models design, decisions 1-3, 9, 17).
//! Changes are session-only until saved.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use hoocode_ai_models::{clamp_thinking_level, get_supported_thinking_levels};
use hoocode_ai_types::{Model, ThinkingLevel};
use hoocode_code_settings::{ModelCategoryName, ScopedModel};
use hoocode_code_tui_keybindings::{format_key_text, key_text};
use hoocode_code_tui_theme::style_input;
use hoocode_code_tui_widgets::console::{help_rows, query_row, Console};
use hoocode_code_tui_widgets::pane::capped_hint_line;
use hoocode_tui_components::{Input, PickerCursor};
use hoocode_tui_fuzzy::fuzzy_filter;
use hoocode_tui_keys::{get_keybindings, matches_key};
use hoocode_tui_ratatui::{Line, Palette, Span};
use hoocode_tui_render::Component;

/// Cycles the selected row's effort through the model's levels, then unset.
/// Handled by the picker itself (no `keybindings.rs` entry).
pub const EFFORT_KEY: &str = "tab";
/// Cycles the selected row's category: none, cheap, fast, standard, capable.
/// `alt+j` is not bound elsewhere; `alt+t` (thinking) and `shift+tab` (mode) are not it.
pub const CATEGORY_KEY: &str = "alt+j";

const MAX_VISIBLE: usize = 8;

/// `None` = all enabled (no filter); `Some` = an explicit ordered list of
/// `provider/id`s.
pub type EnabledIds = Option<Vec<String>>;

fn is_enabled(enabled: &EnabledIds, id: &str) -> bool {
    enabled
        .as_ref()
        .is_none_or(|ids| ids.iter().any(|i| i == id))
}

/// `toggle`: the first toggle from all-enabled keeps only that model.
pub fn toggle(enabled: &EnabledIds, id: &str) -> EnabledIds {
    let Some(ids) = enabled else {
        return Some(vec![id.to_string()]);
    };
    let mut ids = ids.clone();
    match ids.iter().position(|i| i == id) {
        Some(index) => {
            ids.remove(index);
        }
        None => ids.push(id.to_string()),
    }
    Some(ids)
}

/// `enableAll`: `targets` (default every model) on; all on reads as `None`.
pub fn enable_all(enabled: &EnabledIds, all: &[String], targets: Option<&[String]>) -> EnabledIds {
    let ids = enabled.as_ref()?;
    let mut result = ids.clone();
    for id in targets.unwrap_or(all) {
        if !result.contains(id) {
            result.push(id.clone());
        }
    }
    (result.len() != all.len()).then_some(result)
}

/// `clearAll`: `targets` (default every enabled model) off.
pub fn clear_all(enabled: &EnabledIds, all: &[String], targets: Option<&[String]>) -> EnabledIds {
    match enabled {
        None => Some(match targets {
            Some(targets) => all
                .iter()
                .filter(|id| !targets.contains(id))
                .cloned()
                .collect(),
            None => Vec::new(),
        }),
        Some(ids) => {
            let targets: HashSet<&String> = targets.unwrap_or(ids).iter().collect();
            Some(
                ids.iter()
                    .filter(|id| !targets.contains(id))
                    .cloned()
                    .collect(),
            )
        }
    }
}

/// `move`: swap `id` with its neighbour `delta` away, when there is one.
pub fn move_id(enabled: &EnabledIds, id: &str, delta: isize) -> EnabledIds {
    let ids = enabled.as_ref()?;
    let mut list = ids.clone();
    if let Some(index) = list.iter().position(|i| i == id) {
        let target = index as isize + delta;
        if target >= 0 && (target as usize) < list.len() {
            list.swap(index, target as usize);
        }
    }
    Some(list)
}

/// `getSortedIds`: the enabled models in their order, then the rest.
fn sorted_ids(enabled: &EnabledIds, all: &[String]) -> Vec<String> {
    let Some(ids) = enabled else {
        return all.to_vec();
    };
    let set: HashSet<&String> = ids.iter().collect();
    ids.iter()
        .cloned()
        .chain(all.iter().filter(|id| !set.contains(id)).cloned())
        .collect()
}

/// The thinking levels a model supports, lowest first (decision 17).
pub fn effort_levels(model: &Model) -> Vec<&'static str> {
    get_supported_thinking_levels(model)
        .iter()
        .map(ThinkingLevel::as_str)
        .collect()
}

/// Parses a saved effort name and clamps it to what the model supports.
fn clamp_saved_effort(effort: &str, model: &Model) -> Option<String> {
    let level = match effort {
        "off" => ThinkingLevel::Off,
        "minimal" => ThinkingLevel::Minimal,
        "low" => ThinkingLevel::Low,
        "medium" => ThinkingLevel::Medium,
        "high" => ThinkingLevel::High,
        "xhigh" => ThinkingLevel::XHigh,
        _ => return None,
    };
    Some(clamp_thinking_level(model, &level).as_str().to_string())
}

/// The effort after `current` in `levels`; past the last level it is unset
/// (`None`). An unset or unknown `current` starts at the first level.
pub fn next_effort(current: Option<&str>, levels: &[&str]) -> Option<String> {
    match current.and_then(|c| levels.iter().position(|l| *l == c)) {
        Some(index) => levels.get(index + 1).map(|l| l.to_string()),
        None => levels.first().map(|l| l.to_string()),
    }
}

/// The category after `current`: none, cheap, fast, standard, capable, then none.
pub fn next_category(current: Option<ModelCategoryName>) -> Option<ModelCategoryName> {
    match current {
        None => ModelCategoryName::ALL.first().copied(),
        Some(category) => ModelCategoryName::ALL
            .iter()
            .position(|t| *t == category)
            .and_then(|index| ModelCategoryName::ALL.get(index + 1).copied()),
    }
}

/// What a row carries besides its enabled state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Extras {
    effort: Option<String>,
    category: Option<ModelCategoryName>,
    alias: Option<String>,
}

#[derive(Clone)]
struct Item {
    full_id: String,
    model: Model,
    enabled: bool,
    extras: Extras,
}

/// What the picker asks its owner to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopedModelsEvent {
    /// The enabled set or order changed (session-only).
    Change(EnabledIds),
    /// Save these entries to `scopedModels`: order is the priority, and each
    /// entry carries its effort, category and alias.
    Persist(Vec<ScopedModel>),
    Cancel,
}

struct State {
    models: HashMap<String, Model>,
    all: Vec<String>,
    enabled: EnabledIds,
    /// Per model, kept even while the model is disabled, so re-enabling it in
    /// the same session restores its effort and category.
    extras: HashMap<String, Extras>,
    filtered: Vec<Item>,
    cursor: PickerCursor,
    dirty: bool,
}

impl State {
    fn build_items(&self) -> Vec<Item> {
        // IDs whose model is gone (e.g. after logout) drop out.
        sorted_ids(&self.enabled, &self.all)
            .into_iter()
            .filter_map(|id| {
                let model = self.models.get(&id)?.clone();
                Some(Item {
                    enabled: is_enabled(&self.enabled, &id),
                    extras: self.extras.get(&id).cloned().unwrap_or_default(),
                    full_id: id,
                    model,
                })
            })
            .collect()
    }

    /// The title facts: how many models are on, and whether the changes
    /// are saved.
    fn count_text(&self) -> String {
        let on = match &self.enabled {
            None => format!("all {} on", self.all.len()),
            Some(ids) => format!("{} of {} on", ids.len(), self.all.len()),
        };
        if self.dirty {
            format!("{on} · unsaved, this session only")
        } else {
            format!("{on} · this session only")
        }
    }

    /// Every key the picker answers to, in hint order (the first four show;
    /// `f1` lists the rest).
    fn all_hints() -> Vec<(String, String)> {
        let pair = |key: String, action: &str| (key, action.to_string());
        vec![
            pair(key_text("tui.select.confirm"), "on / off"),
            pair(format_key_text(EFFORT_KEY, false), "effort"),
            pair(key_text("app.models.save"), "save to settings"),
            pair("esc".to_string(), "close"),
            pair(format_key_text(CATEGORY_KEY, false), "category"),
            pair(key_text("app.models.enableAll"), "all on"),
            pair(key_text("app.models.clearAll"), "all off"),
            pair(key_text("app.models.toggleProvider"), "provider"),
            pair(
                format!(
                    "{}/{}",
                    key_text("app.models.reorderUp"),
                    key_text("app.models.reorderDown")
                ),
                "reorder",
            ),
        ]
    }

    /// The selected row's facts after its name: the model's own name and
    /// provider, then the effort, category and alias set on it.
    fn row_facts(item: &Item) -> String {
        let mut parts = vec![item.model.name.clone(), item.model.provider.clone()];
        if let Some(effort) = &item.extras.effort {
            parts.push(format!("effort {effort}"));
        }
        if let Some(category) = item.extras.category {
            parts.push(category.as_str().to_string());
        }
        if let Some(alias) = &item.extras.alias {
            parts.push(format!("alias {alias}"));
        }
        parts.join(" · ")
    }

    /// The pane rows: the id padded to the widest, `on` / `off`, and on the
    /// selected row its facts. Returns the rows and the selection in them.
    fn console_rows(&self) -> (Vec<Line<'static>>, Option<usize>) {
        let pal = Palette::current();
        let window = self.cursor.window();
        let selected = self.cursor.selected();
        let id_width = self
            .filtered
            .iter()
            .map(|item| item.model.id.chars().count())
            .max()
            .unwrap_or(0);
        let all_enabled = self.enabled.is_none();
        let rows = window
            .clone()
            .map(|i| {
                let item = &self.filtered[i];
                let on = all_enabled || item.enabled;
                let mut spans = vec![
                    Span::raw(format!("{:<id_width$}", item.model.id)),
                    Span::raw("  "),
                    if on {
                        Span::styled("on ".to_string(), pal.ok())
                    } else {
                        Span::styled("off".to_string(), pal.dim())
                    },
                ];
                if i == selected {
                    spans.push(Span::raw("  "));
                    spans.push(Span::styled(Self::row_facts(item), pal.dim()));
                }
                Line::from(spans)
            })
            .collect();
        (rows, window.clone().position(|i| i == selected))
    }
}

/// `ScopedModelsSelectorComponent`.
pub struct ScopedModelsSelectorComponent {
    search: Rc<RefCell<Input>>,
    state: Rc<RefCell<State>>,
    events: Vec<ScopedModelsEvent>,
    /// The picker has focus: the filter caret draws while set.
    focused: bool,
    /// The full key list shows (`f1`).
    help: bool,
}

impl ScopedModelsSelectorComponent {
    /// `scoped` is the saved list in priority order (`None` or empty: every
    /// model enabled). Entries name concrete `provider/id`s; unknown models are
    /// dropped, and an effort the model does not support is clamped (decision 17).
    pub fn new(all_models: Vec<Model>, scoped: Option<Vec<ScopedModel>>) -> Self {
        let mut models = HashMap::new();
        let mut all = Vec::new();
        for model in all_models {
            let full_id = format!("{}/{}", model.provider, model.id);
            all.push(full_id.clone());
            models.insert(full_id, model);
        }
        let mut extras = HashMap::new();
        let mut enabled_ids: Vec<String> = Vec::new();
        for entry in scoped.unwrap_or_default() {
            if enabled_ids.contains(&entry.model) {
                continue;
            }
            let Some(model) = models.get(&entry.model) else {
                continue;
            };
            let effort = entry
                .effort
                .as_deref()
                .and_then(|effort| clamp_saved_effort(effort, model));
            extras.insert(
                entry.model.clone(),
                Extras {
                    effort,
                    category: entry.category,
                    alias: entry.alias,
                },
            );
            enabled_ids.push(entry.model);
        }
        let mut state = State {
            models,
            all,
            enabled: (!enabled_ids.is_empty()).then_some(enabled_ids),
            extras,
            filtered: Vec::new(),
            cursor: PickerCursor::new(0, MAX_VISIBLE),
            dirty: false,
        };
        state.filtered = state.build_items();
        state.cursor.reset(state.filtered.len());

        let mut input = Input::new();
        style_input(&mut input);
        let search = Rc::new(RefCell::new(input));
        let state = Rc::new(RefCell::new(state));
        Self {
            search,
            state,
            events: Vec::new(),
            focused: false,
            help: false,
        }
    }

    /// Events since the last call.
    pub fn take_events(&mut self) -> Vec<ScopedModelsEvent> {
        std::mem::take(&mut self.events)
    }

    fn query(&self) -> String {
        self.search.borrow().get_value().to_string()
    }

    /// `refresh`: rebuild the rows and the footer.
    fn refresh(&mut self) {
        let query = self.query();
        let mut state = self.state.borrow_mut();
        let items = state.build_items();
        state.filtered = if query.is_empty() {
            items
        } else {
            fuzzy_filter(&items, &query, |i| {
                format!("{} {}", i.model.id, i.model.provider)
            })
        };
        let len = state.filtered.len();
        state.cursor.set_query(len);
    }

    fn notify_change(&mut self) {
        let enabled = self.state.borrow().enabled.clone();
        self.events.push(ScopedModelsEvent::Change(enabled));
    }

    /// Set a new enabled set: marked unsaved, redrawn, reported.
    fn apply(&mut self, enabled: EnabledIds) {
        {
            let mut state = self.state.borrow_mut();
            state.enabled = enabled;
            state.dirty = true;
        }
        self.refresh();
        self.notify_change();
    }

    /// Edits the selected row's effort or category: marked unsaved and redrawn.
    /// Not reported as a `Change`, since the enabled set did not move.
    fn edit_selected(&mut self, edit: impl FnOnce(&mut Extras, &Model)) {
        {
            let mut state = self.state.borrow_mut();
            let Some(item) = state.filtered.get(state.cursor.selected()).cloned() else {
                return;
            };
            let extras = state.extras.entry(item.full_id).or_default();
            edit(extras, &item.model);
            state.dirty = true;
        }
        self.refresh();
    }

    /// The entries to save: the enabled models in priority order, each with its
    /// effort, category and alias. When every model is enabled in list order
    /// with no effort, category or alias, that is no scope, so the list is
    /// empty (the clear).
    fn persisted_entries(&self) -> Vec<ScopedModel> {
        let state = self.state.borrow();
        let ids: Vec<String> = match &state.enabled {
            Some(ids) => ids.clone(),
            None => state.all.clone(),
        };
        let entries: Vec<(String, Extras)> = ids
            .into_iter()
            .filter(|id| state.models.contains_key(id))
            .map(|id| {
                let extras = state.extras.get(&id).cloned().unwrap_or_default();
                (id, extras)
            })
            .collect();
        let no_scope = entries.iter().map(|(id, _)| id).eq(state.all.iter())
            && entries
                .iter()
                .all(|(_, extras)| *extras == Extras::default());
        if no_scope {
            return Vec::new();
        }
        entries
            .into_iter()
            .map(|(model, extras)| ScopedModel {
                model,
                effort: extras.effort,
                category: extras.category,
                alias: extras.alias,
            })
            .collect()
    }

    /// The filtered ids when a search is active (the bulk keys act on those).
    fn search_targets(&self) -> Option<Vec<String>> {
        if self.query().is_empty() {
            return None;
        }
        Some(
            self.state
                .borrow()
                .filtered
                .iter()
                .map(|i| i.full_id.clone())
                .collect(),
        )
    }
}

impl Component for ScopedModelsSelectorComponent {
    /// The Scoped models pane: one row per model with its `on` / `off`,
    /// the selected row's facts after it, the filter row and the hint row.
    /// Keys, filtering and events are unchanged.
    fn render(&mut self, width: u16) -> Vec<String> {
        let pal = Palette::current();
        let state = self.state.borrow();
        let mut console = Console::new("Scoped models · /scoped-models", state.count_text());
        console.query = Some(query_row(&self.query(), self.focused));
        let (rows, selected) = state.console_rows();
        console.rows = rows;
        console.selected = selected;
        if state.filtered.is_empty() {
            console.bottom = vec![Line::from(vec![Span::styled(
                "No matching models".to_string(),
                pal.dim(),
            )])];
        } else if let Some(footer) = state.cursor.footer() {
            console.bottom = vec![Line::from(vec![Span::styled(
                format!("  {footer}"),
                pal.dim(),
            )])];
        }
        let all = State::all_hints();
        let pairs: Vec<(&str, &str)> = all.iter().map(|(k, a)| (k.as_str(), a.as_str())).collect();
        if self.help {
            console.bottom.extend(help_rows(&pairs));
        }
        console.hint = capped_hint_line(&pairs, true);
        console.render(width as usize)
    }

    fn handle_input(&mut self, data: &str) {
        if matches_key(data, "f1") {
            self.help = !self.help;
            return;
        }
        let kb = get_keybindings();
        let navigated = self.state.borrow_mut().cursor.handle_query_key(data, &kb);
        if navigated {
            return;
        }

        let reorder_up = kb.matches(data, "app.models.reorderUp");
        let reorder_down = kb.matches(data, "app.models.reorderDown");
        if reorder_up || reorder_down {
            let (enabled, item, selected) = {
                let state = self.state.borrow();
                (
                    state.enabled.clone(),
                    state
                        .filtered
                        .get(state.cursor.selected())
                        .map(|i| i.full_id.clone()),
                    state.cursor.selected(),
                )
            };
            let (Some(ids), Some(item)) = (&enabled, item) else {
                return;
            };
            let Some(index) = ids.iter().position(|i| *i == item) else {
                return;
            };
            let delta: isize = if reorder_up { -1 } else { 1 };
            let target = index as isize + delta;
            if target >= 0 && (target as usize) < ids.len() {
                let target = (selected as isize + delta).max(0) as usize;
                self.state.borrow_mut().cursor.select(target);
                self.apply(move_id(&enabled, &item, delta));
            }
            return;
        }

        if kb.matches(data, "tui.select.confirm") {
            let (enabled, item) = {
                let state = self.state.borrow();
                (
                    state.enabled.clone(),
                    state
                        .filtered
                        .get(state.cursor.selected())
                        .map(|i| i.full_id.clone()),
                )
            };
            if let Some(item) = item {
                self.apply(toggle(&enabled, &item));
            }
            return;
        }

        let enable = kb.matches(data, "app.models.enableAll");
        if enable || kb.matches(data, "app.models.clearAll") {
            let targets = self.search_targets();
            let next = {
                let state = self.state.borrow();
                if enable {
                    enable_all(&state.enabled, &state.all, targets.as_deref())
                } else {
                    clear_all(&state.enabled, &state.all, targets.as_deref())
                }
            };
            self.apply(next);
            return;
        }

        if kb.matches(data, "app.models.toggleProvider") {
            let next = {
                let state = self.state.borrow();
                state.filtered.get(state.cursor.selected()).map(|item| {
                    let provider = &item.model.provider;
                    let ids: Vec<String> = state
                        .all
                        .iter()
                        .filter(|id| &state.models[*id].provider == provider)
                        .cloned()
                        .collect();
                    if ids.iter().all(|id| is_enabled(&state.enabled, id)) {
                        clear_all(&state.enabled, &state.all, Some(&ids))
                    } else {
                        enable_all(&state.enabled, &state.all, Some(&ids))
                    }
                })
            };
            if let Some(next) = next {
                self.apply(next);
            }
            return;
        }

        if matches_key(data, EFFORT_KEY) {
            self.edit_selected(|extras, model| {
                let levels = effort_levels(model);
                extras.effort = next_effort(extras.effort.as_deref(), &levels);
            });
            return;
        }

        if matches_key(data, CATEGORY_KEY) {
            self.edit_selected(|extras, _| extras.category = next_category(extras.category));
            return;
        }

        if kb.matches(data, "app.models.save") {
            let entries = self.persisted_entries();
            {
                let mut state = self.state.borrow_mut();
                state.dirty = false;
            }
            self.events.push(ScopedModelsEvent::Persist(entries));
            return;
        }

        if matches_key(data, "ctrl+c") {
            if self.query().is_empty() {
                self.events.push(ScopedModelsEvent::Cancel);
            } else {
                self.search.borrow_mut().set_value("");
                self.refresh();
            }
            return;
        }

        if matches_key(data, "escape") {
            self.events.push(ScopedModelsEvent::Cancel);
            return;
        }

        self.search.borrow_mut().handle_input_with(data, &kb);
        self.refresh();
    }

    fn invalidate(&mut self) {
        self.search.borrow_mut().invalidate();
    }

    fn is_focusable(&self) -> bool {
        true
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
        self.search.borrow_mut().set_focused(focused);
    }
}

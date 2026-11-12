//! The `/model` picker, hoocode `components/model-selector.ts`.
//!
//! Adaptations: the original loads the available models itself (refreshing
//! the registry first) and saves the pick as the new default before calling
//! back; here the owner hands in what it loaded, and does the save when the
//! [`ModelSelectorEvent::Select`] arrives.

use hoocode_code_tui_keybindings::{key_hint_text, key_pair_hint};
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use hoocode_ai_models::models_are_equal;
use hoocode_ai_types::Model;
use hoocode_code_tui_theme::style_input;
use hoocode_code_tui_widgets::console::{query_row, Console};
use hoocode_tui_components::{Input, PickerCursor};
use hoocode_tui_fuzzy::fuzzy_filter;
use hoocode_tui_keys::get_keybindings;
use hoocode_tui_ratatui::{bold, Line, Palette, Span};
use hoocode_tui_render::Component;
use hoocode_tui_util::visible_width;

use crate::config_selector::locale_compare;

const MAX_VISIBLE: usize = 10;

/// Which list the picker shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelScope {
    All,
    Scoped,
}

/// What the picker asks its owner to do.
#[derive(Debug, Clone, PartialEq)]
pub enum ModelSelectorEvent {
    Select(Box<Model>),
    Cancel,
}

struct State {
    current: Option<Model>,
    all: Vec<Model>,
    scoped: Vec<Model>,
    scope: ModelScope,
    filtered: Vec<Model>,
    cursor: PickerCursor,
    error: Option<String>,
    /// Providers whose stored OAuth token has expired (see `set_expired_providers`).
    expired: HashSet<String>,
    /// Whether the picker has focus: the query caret draws while set.
    focused: bool,
}

impl State {
    fn active(&self) -> &[Model] {
        match self.scope {
            ModelScope::All => &self.all,
            ModelScope::Scoped => &self.scoped,
        }
    }

    /// `filterModels`.
    fn filter(&mut self, query: &str) {
        self.filtered = if query.is_empty() {
            self.active().to_vec()
        } else {
            fuzzy_filter(self.active(), query, |m| {
                let (id, provider) = (&m.id, &m.provider);
                format!("{id} {provider} {provider}/{id} {provider} {id}")
            })
        };
        self.cursor.set_query(self.filtered.len());
    }

    /// The title facts: the scope and the filtered models of its models
    /// (`showing all · 2 of 3 models`).
    fn count_text(&self, query: &str) -> String {
        let scope = match self.scope {
            ModelScope::All => "all",
            ModelScope::Scoped => "scoped",
        };
        let total = self.active().len();
        if query.is_empty() {
            format!("showing {scope} · {}", models_text(total))
        } else {
            format!(
                "showing {scope} · {} of {total}",
                models_text(self.filtered.len())
            )
        }
    }

    /// The count's short form for a narrow title (`2 models`): the scope
    /// prefix drops at 40 columns (settings `count_for_width` pattern).
    fn short_text(&self, query: &str) -> String {
        let total = self.active().len();
        if query.is_empty() {
            models_text(total)
        } else {
            format!("{} of {total}", models_text(self.filtered.len()))
        }
    }

    /// The full count while it fits the title, else the short form.
    fn count_for_width(&self, query: &str, width: usize) -> String {
        let full = self.count_text(query);
        // `  Models  /model  ` before the count.
        if 18 + visible_width(&full) <= width {
            full
        } else {
            self.short_text(query)
        }
    }

    /// Rows below the list: the scroll position, errors, and a note when
    /// nothing matches.
    fn bottom_rows(&self, pal: &Palette) -> Vec<Line<'static>> {
        let mut rows = Vec::new();
        if let Some(footer) = self.cursor.footer() {
            rows.push(Line::from(vec![Span::styled(
                format!("  {footer}"),
                pal.dim(),
            )]));
        }
        if let Some(error) = &self.error {
            for line in error.split('\n') {
                rows.push(Line::from(vec![Span::styled(
                    (*line).to_string(),
                    pal.err(),
                )]));
            }
        } else if self.filtered.is_empty() {
            rows.push(Line::from(vec![Span::styled(
                "No matching models",
                pal.dim(),
            )]));
        }
        rows
    }

    /// The detail column for the selected model: its id and state, then
    /// provider, context, price and thinking support.
    fn detail_rows(&self, pal: &Palette) -> Vec<Line<'static>> {
        if self.error.is_some() {
            return Vec::new();
        }
        let Some(model) = self.filtered.get(self.cursor.selected()) else {
            return Vec::new();
        };
        let mut head = vec![Span::styled(model.id.clone(), pal.ink())];
        if self.expired.contains(&model.provider) {
            head.push(Span::styled("  expired".to_string(), bold(pal.ink())));
        } else if models_are_equal(self.current.as_ref(), Some(model)) {
            head.push(Span::styled("  in use".to_string(), pal.dim()));
        }
        let fact = |label: &str, value: String| {
            Line::from(vec![
                Span::styled(format!("{label:<10}"), pal.dim()),
                Span::styled(value, pal.ink()),
            ])
        };
        let mut rows = vec![Line::from(head)];
        if model.name != model.id {
            rows.push(fact("name", model.name.clone()));
        }
        rows.push(fact("provider", model.provider.clone()));
        rows.push(fact("context", format_context(model.context_window)));
        rows.push(fact("price", format_price(&model.cost)));
        rows.push(fact(
            "thinking",
            if model.reasoning {
                "supported".to_string()
            } else {
                "fixed".to_string()
            },
        ));
        rows
    }
}

/// `1 model`, `N models`.
fn models_text(count: usize) -> String {
    if count == 1 {
        "1 model".to_string()
    } else {
        format!("{count} models")
    }
}

/// One model row (content only; the console adds the ` › ` marker or
/// gutter and the selection band): `● id  price · ctx · thinking`. The
/// mark carries the state (`●` the model in use, `○` the rest); the price
/// is primary and never drops, context then thinking drop right-to-left.
/// A model whose login expired says so as fact text (`expired`, ink bold),
/// never colour alone.
fn model_row(model: &Model, expired: bool, current: Option<&Model>, width: usize) -> Line<'static> {
    let pal = Palette::current();
    let mut primary = vec![(format_price(&model.cost), pal.dim())];
    if expired {
        primary.push(("expired".to_string(), bold(pal.ink())));
    }
    let thinking = if model.reasoning { "thinking" } else { "fixed" };
    let secondary = vec![
        (format_context(model.context_window), pal.dim()),
        (thinking.to_string(), pal.dim()),
    ];
    crate::framed_list::picker_row(
        models_are_equal(current, Some(model)) && !expired,
        &model.id,
        "",
        primary,
        secondary,
        width,
        &pal,
    )
}

/// `200k` for a 200,000-token window, the raw count below a thousand.
fn format_context(window: u64) -> String {
    if window >= 1000 {
        format!("{}k", window / 1000)
    } else {
        format!("{window}")
    }
}

/// `$in / $out` per 1M tokens, or `free` where the catalog prices nothing.
fn format_price(cost: &hoocode_ai_types::ModelCost) -> String {
    if cost.input == 0.0 && cost.output == 0.0 {
        return "free".to_string();
    }
    format!("${:.2} / ${:.2}", cost.input, cost.output)
}

/// `ModelSelectorComponent`: a searchable model list, with an all/scoped
/// switch when the session has a model scope.
pub struct ModelSelectorComponent {
    search: Rc<RefCell<Input>>,
    state: Rc<RefCell<State>>,
    events: Vec<ModelSelectorEvent>,
    submitted: Rc<RefCell<bool>>,
}

impl ModelSelectorComponent {
    /// `available` is the registry's models with configured auth, or why they
    /// could not be listed; `load_error` is a models.json problem (built-ins
    /// still list). `scoped` is the session's model scope, in its order.
    pub fn new(
        current: Option<Model>,
        available: Result<Vec<Model>, String>,
        load_error: Option<String>,
        scoped: Vec<Model>,
        initial_search: Option<&str>,
    ) -> Self {
        let scope = if scoped.is_empty() {
            ModelScope::All
        } else {
            ModelScope::Scoped
        };
        let (all, scoped, error) = match available {
            Ok(models) => {
                let mut sorted = models;
                // Current model first, then by provider (a stable sort).
                sorted.sort_by(|a, b| {
                    let a_current = models_are_equal(current.as_ref(), Some(a));
                    let b_current = models_are_equal(current.as_ref(), Some(b));
                    b_current
                        .cmp(&a_current)
                        .then_with(|| locale_compare(&a.provider, &b.provider))
                });
                (sorted, scoped, load_error)
            }
            Err(error) => (Vec::new(), Vec::new(), Some(error)),
        };
        let mut state = State {
            current,
            all,
            scoped,
            scope,
            filtered: Vec::new(),
            cursor: PickerCursor::new(0, MAX_VISIBLE),
            error,
            expired: HashSet::new(),
            focused: false,
        };
        state.filtered = state.active().to_vec();
        state.cursor.reset(state.filtered.len());
        let current_row = state
            .filtered
            .iter()
            .position(|m| models_are_equal(state.current.as_ref(), Some(m)))
            .unwrap_or(0);
        state.cursor.select(current_row);

        let mut input = Input::new();
        style_input(&mut input);
        if let Some(search) = initial_search.filter(|s| !s.is_empty()) {
            input.set_value(search);
            state.filter(search);
        }
        let submitted: Rc<RefCell<bool>> = Rc::default();
        let flag = submitted.clone();
        input.on_submit = Some(Box::new(move |_: &str| *flag.borrow_mut() = true));
        let search = Rc::new(RefCell::new(input));
        let state = Rc::new(RefCell::new(state));
        Self {
            search,
            state,
            events: Vec::new(),
            submitted,
        }
    }

    /// Marks the models of these providers `expired`: their stored OAuth token
    /// needs a refresh. The marker only; the picker never refreshes.
    pub fn set_expired_providers(&mut self, providers: impl IntoIterator<Item = String>) {
        self.state.borrow_mut().expired = providers.into_iter().collect();
    }

    /// Events since the last call.
    pub fn take_events(&mut self) -> Vec<ModelSelectorEvent> {
        std::mem::take(&mut self.events)
    }

    /// The query line.
    pub fn search_value(&self) -> String {
        self.search.borrow().get_value().to_string()
    }

    fn select_current(&mut self) {
        let state = self.state.borrow();
        if let Some(model) = state.filtered.get(state.cursor.selected()) {
            self.events
                .push(ModelSelectorEvent::Select(Box::new(model.clone())));
        }
    }

    /// `setScope`.
    fn set_scope(&mut self, scope: ModelScope) {
        let query = self.search_value();
        let mut state = self.state.borrow_mut();
        if state.scope == scope {
            return;
        }
        state.scope = scope;
        let current_row = state
            .active()
            .iter()
            .position(|m| models_are_equal(state.current.as_ref(), Some(m)))
            .unwrap_or(0);
        state.filter(&query);
        state.cursor.select(current_row);
    }
}

impl Component for ModelSelectorComponent {
    /// The Models pane: the model rows with ` › ` on the selection band, the
    /// selected model's detail beside them (below at narrow widths), the
    /// filter row and the hint row. Keys and filtering are unchanged.
    fn render(&mut self, width: u16) -> Vec<String> {
        let pal = Palette::current();
        let state = self.state.borrow();
        let search = self.search.borrow();
        let query = search.get_value().to_string();
        let mut console = Console::new(
            "Models · /model",
            state.count_for_width(&query, width as usize),
        );
        console.query = Some(query_row(&query, state.focused));
        let expired = state.expired.clone();
        let current = state.current.clone();
        console.detail = state.detail_rows(&pal);
        // The detail is set before the rows: a split narrows the list
        // column, and rows are built to that width.
        let row_width = console.row_width(width as usize);
        let selected = state.cursor.selected();
        console.rows = state
            .filtered
            .iter()
            .map(|model| {
                model_row(
                    model,
                    expired.contains(&model.provider),
                    current.as_ref(),
                    row_width,
                )
            })
            .collect();
        console.selected = (!state.filtered.is_empty()).then_some(selected);
        console.max_visible = Some(state.cursor.max_visible());
        console.bottom = state.bottom_rows(&pal);
        // The picker keys (the scope switch only where it flips something).
        if state.scoped.is_empty() {
            console.set_hints(
                &[
                    (
                        key_pair_hint("tui.select.up", "tui.select.down").as_str(),
                        "move",
                    ),
                    (key_hint_text("tui.select.confirm").as_str(), "pick"),
                    (key_hint_text("tui.select.cancel").as_str(), "close"),
                ],
                false,
            );
        } else {
            console.set_hints(
                &[
                    (
                        key_pair_hint("tui.select.up", "tui.select.down").as_str(),
                        "move",
                    ),
                    (key_hint_text("tui.select.confirm").as_str(), "pick"),
                    (key_hint_text("tui.input.tab").as_str(), "all / scoped"),
                    (key_hint_text("tui.select.cancel").as_str(), "close"),
                ],
                false,
            );
        }
        console.render(width as usize)
    }

    fn handle_input(&mut self, data: &str) {
        let kb = get_keybindings();
        if kb.matches(data, "tui.input.tab") {
            let next = {
                let state = self.state.borrow();
                (!state.scoped.is_empty()).then_some(match state.scope {
                    ModelScope::All => ModelScope::Scoped,
                    ModelScope::Scoped => ModelScope::All,
                })
            };
            if let Some(next) = next {
                self.set_scope(next);
            }
            return;
        }
        let navigated = self.state.borrow_mut().cursor.handle_query_key(data, &kb);
        if navigated {
            return;
        }
        if kb.matches(data, "tui.select.confirm") {
            self.select_current();
        } else if kb.matches(data, "tui.select.cancel") {
            self.events.push(ModelSelectorEvent::Cancel);
        } else {
            self.search.borrow_mut().handle_input_with(data, &kb);
            if std::mem::take(&mut *self.submitted.borrow_mut()) {
                self.select_current();
            }
            let query = self.search_value();
            self.state.borrow_mut().filter(&query);
        }
    }

    fn invalidate(&mut self) {
        self.search.borrow_mut().invalidate();
    }

    fn is_focusable(&self) -> bool {
        true
    }

    fn set_focused(&mut self, focused: bool) {
        self.state.borrow_mut().focused = focused;
        self.search.borrow_mut().set_focused(focused);
    }
}

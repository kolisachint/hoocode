//! The `/model` picker, hoocode `components/model-selector.ts`.
//!
//! Adaptations: the original loads the available models itself (refreshing
//! the registry first) and saves the pick as the new default before calling
//! back; here the owner hands in what it loaded, and does the save when the
//! [`ModelSelectorEvent::Select`] arrives.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use hoocode_ai_models::models_are_equal;
use hoocode_ai_types::Model;
use hoocode_code_agent_session::format::format_tokens;
use hoocode_code_tui_keybindings::key_text;
use hoocode_code_tui_theme::style_input;
use hoocode_code_tui_widgets::console::{hints, query_row, Console};
use hoocode_tui_components::{Input, PickerCursor};
use hoocode_tui_fuzzy::fuzzy_filter;
use hoocode_tui_keys::get_keybindings;
use hoocode_tui_ratatui::{Line, Palette, Span, Style};
use hoocode_tui_render::Component;

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

    fn scope_text(&self, pal: &Palette) -> Line<'static> {
        let pick = |scope, label: &str| {
            Span::styled(
                label.to_string(),
                if self.scope == scope {
                    pal.acc()
                } else {
                    pal.faint()
                },
            )
        };
        Line::from(vec![
            Span::styled("Scope: ", pal.faint()),
            pick(ModelScope::All, "all"),
            Span::styled(" | ", pal.faint()),
            pick(ModelScope::Scoped, "scoped"),
        ])
    }

    /// The HANGAR title count: the filtered ships of the scope's ships.
    fn count_text(&self, query: &str) -> String {
        let total = self.active().len();
        if query.is_empty() {
            format!("{} signed in", ships(total))
        } else {
            format!("{} of {total} signed in", ships(self.filtered.len()))
        }
    }

    /// Scope lines above the query: the switch and its hint, or the
    /// providers warning when the scope is all there is.
    fn top_rows(&self, pal: &Palette) -> Vec<Line<'static>> {
        if self.scope == ModelScope::All {
            vec![Line::from(vec![Span::styled(
                "Only showing models from configured providers. Use /login to add providers.",
                pal.warn(),
            )])]
        } else {
            vec![
                self.scope_text(pal),
                Line::from(vec![
                    Span::styled(key_text("tui.input.tab"), pal.dim()),
                    Span::styled(" scope", pal.faint()),
                    Span::styled(" (all/scoped)", pal.faint()),
                ]),
            ]
        }
    }

    /// Rows below the list: the scroll position, errors, the selected
    /// ship's display name, and its context window, price and thinking
    /// support (M8d3).
    fn bottom_rows(&self, pal: &Palette) -> Vec<Line<'static>> {
        let mut rows = Vec::new();
        if let Some(footer) = self.cursor.footer() {
            rows.push(Line::from(vec![Span::styled(
                format!("  {footer}"),
                pal.faint(),
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
                "  No matching models",
                pal.faint(),
            )]));
        } else {
            rows.push(Line::raw(""));
            let model = &self.filtered[self.cursor.selected()];
            rows.push(Line::from(vec![Span::styled(
                format!("  Model Name: {}", model.name),
                pal.faint(),
            )]));
            rows.push(Line::from(vec![Span::styled(
                format!(
                    "  {} context · {} per 1M · thinking {}",
                    format_context(model.context_window),
                    format_price(&model.cost),
                    if model.reasoning {
                        "supported"
                    } else {
                        "fixed"
                    },
                ),
                pal.faint(),
            )]));
        }
        rows
    }

    /// The hint foot: the console keys (the scope switch only where it
    /// flips something).
    fn hint_row(&self) -> Line<'static> {
        if self.scoped.is_empty() {
            hints(&[("↑↓", "ship"), ("enter", "fly it"), ("esc", "close")])
        } else {
            hints(&[
                ("↑↓", "ship"),
                ("enter", "fly it"),
                ("tab", "scope"),
                ("esc", "close"),
            ])
        }
    }
}

/// One ship row (content only; the console adds the `›` marker or gutter
/// and the selection band): the id, the provider, the context window and
/// the in/out price per 1M tokens (the target's `hangarRows` columns), plus
/// the `expired` or current `✓` marker.
fn ship_row(
    model: &Model,
    current: Option<&Model>,
    expired: &HashSet<String>,
    selected: bool,
) -> Line<'static> {
    let pal = Palette::current();
    let mut spans = vec![
        Span::styled(
            format!("{:<12}", model.id),
            if selected {
                pal.acc()
            } else {
                Style::default()
            },
        ),
        Span::raw("  "),
        Span::styled(format!("{:<10}", model.provider), pal.faint()),
        Span::raw("  "),
        Span::styled(
            format!("{:<8}", format_tokens(model.context_window)),
            pal.ink(),
        ),
        Span::raw("  "),
        Span::styled(ship_price(model), pal.dim()),
    ];
    if expired.contains(&model.provider) {
        spans.push(Span::styled(" expired".to_string(), pal.warn()));
    } else if models_are_equal(current, Some(model)) {
        spans.push(Span::styled(" ✓", pal.ok()));
    }
    Line::from(spans)
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

/// The in/out price per 1M tokens (`$3.00 / $15.00`), or `free` when the
/// ship costs nothing (the target's `hangarRows` price column).
fn ship_price(model: &Model) -> String {
    if model.cost.input == 0.0 && model.cost.output == 0.0 {
        return "free".to_string();
    }
    format!("${:.2} / ${:.2}", model.cost.input, model.cost.output)
}
}

/// `1 ship`, `N ships`.
fn ships(count: usize) -> String {
    if count == 1 {
        "1 ship".to_string()
    } else {
        format!("{count} ships")
    }
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
    /// The HANGAR console: scope lines, the `▸` query row, the ships with
    /// `›` on the selection band, the scroll position and detail rows, and
    /// the hint foot. Keys and filtering are unchanged.
    fn render(&mut self, width: u16) -> Vec<String> {
        let pal = Palette::current();
        let state = self.state.borrow();
        let search = self.search.borrow();
        let query = search.get_value().to_string();
        let mut console = Console::new("HANGAR · /model", state.count_text(&query));
        console.top = state.top_rows(&pal);
        if !state.filtered.is_empty() {
            // The column header over the ships (the target's `hangarRows`
            // header): ship, provider, context, and price per 1M tokens.
            console.top.push(Line::from(vec![Span::styled(
                "  ship         provider    context   price in / out per 1M".to_string(),
                pal.faint(),
            )]));
        }
        console.query = Some(query_row(&query, state.focused));
        let current = state.current.clone();
        let expired = state.expired.clone();
        let selected = state.cursor.selected();
        console.rows = state
            .filtered
            .iter()
            .enumerate()
            .map(|(i, model)| ship_row(model, current.as_ref(), &expired, i == selected))
            .collect();
        console.selected = (!state.filtered.is_empty()).then_some(selected);
        console.max_visible = Some(state.cursor.max_visible());
        console.bottom = state.bottom_rows(&pal);
        console.hint = state.hint_row();
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

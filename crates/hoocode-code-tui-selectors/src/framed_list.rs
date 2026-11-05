//! A `SelectList` in a pane: the shape the small pickers share
//! (`Thinking`, `Theme`, `Images`, `Session colour`).
//!
//! Behaviour (keys, filtering, the `on_select` / `on_cancel` /
//! `on_selection_change` callbacks the owners wire, e.g. the theme live
//! preview) stays on the shared [`SelectList`]; only the chrome changed:
//! the title names the pane, the items draw as pane rows, the selected
//! item's detail sits beside them, and the hint closes the pane. No
//! `InputFrame`, no SGR theme painting.

use hoocode_code_tui_keybindings::{key_hint_text, key_pair_hint};
use std::cell::RefCell;
use std::rc::Rc;

use hoocode_code_tui_theme::get_select_list_theme;
use hoocode_code_tui_widgets::console::{hints, Console};
use hoocode_tui_components::{SelectItem, SelectList, SelectListLayoutOptions};
use hoocode_tui_ratatui::{bold, Line, Palette, Span};
use hoocode_tui_render::Component;

/// The pickers' primary column: at least 12 cells, at most 32. Still the
/// [`SelectList`] column layout for the submenus that render the list
/// itself (`settings_selector`); the console rows below do not align
/// columns.
pub(crate) fn layout(min: usize, max: usize) -> SelectListLayoutOptions {
    SelectListLayoutOptions {
        min_primary_column_width: Some(min),
        max_primary_column_width: Some(max),
    }
}

/// What a pane shows beside the list for the selected item.
pub type DetailFn = fn(&SelectItem) -> Vec<Line<'static>>;

/// One picker row (content only; the pane adds the ` › ` marker or gutter
/// and the selection band): the label with its dim description.
fn select_row(item: &SelectItem, pal: &Palette) -> Line<'static> {
    let label = if item.label.is_empty() {
        item.value.clone()
    } else {
        item.label.clone()
    };
    let mut spans = vec![Span::raw(label)];
    if let Some(description) = item.description.as_deref() {
        let single = description
            .split(['\r', '\n'])
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let single = single.trim();
        if !single.is_empty() {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(single.to_string(), pal.dim()));
        }
    }
    Line::from(spans)
}

/// The theme picker's detail: the theme's name and description, then a
/// few transcript lines drawn in the live theme (the picker previews as
/// the cursor moves, so the sample shows the highlighted theme).
pub(crate) fn theme_detail(item: &SelectItem) -> Vec<Line<'static>> {
    let pal = Palette::current();
    let description = hoocode_code_tui_theme::get_theme_description(&item.value)
        .or_else(|| (item.value == "auto").then(|| "Follows the terminal background".to_string()))
        .unwrap_or_default();
    let mut rows = vec![Line::from(vec![
        Span::styled(item.value.clone(), pal.ink()),
        Span::raw("  "),
        Span::styled(description, pal.dim()),
    ])];
    // Where the theme lives: its file for a user theme, else built in.
    if item.value != "auto" {
        let source = match hoocode_code_tui_theme::user_theme_path(&item.value) {
            Some(path) => path.display().to_string(),
            None => "built-in".to_string(),
        };
        rows.push(Line::from(Span::styled(source, pal.dim())));
    }
    rows.push(Line::raw(""));
    rows.push(Line::from(vec![
        Span::styled("› ".to_string(), pal.dim()),
        Span::styled("add a width check to wrap_text".to_string(), pal.ink()),
    ]));
    rows.push(Line::from(vec![
        Span::styled("● ".to_string(), pal.ok()),
        Span::styled("Edit".to_string(), bold(pal.dim())),
        Span::styled(" src/wrap.rs  +4 −1".to_string(), pal.dim()),
    ]));
    rows.push(Line::from(vec![
        Span::styled("⊗ ".to_string(), pal.err()),
        Span::styled("Shell".to_string(), bold(pal.dim())),
        Span::styled(" cargo test -p wrap  exit 1 · 3.1s".to_string(), pal.dim()),
    ]));
    rows.push(Line::from(vec![Span::styled(
        "Width is checked now; the wrap tests pass.".to_string(),
        pal.ink(),
    )]));
    rows
}

/// A titled select list: the title names the console, the list owns the
/// behaviour.
pub struct FramedSelectList {
    name: String,
    count: String,
    list: Rc<RefCell<SelectList>>,
    /// The detail beside the list for the selected item; none draws a
    /// list alone.
    detail: Option<DetailFn>,
}

impl FramedSelectList {
    pub(crate) fn new(
        title: &str,
        items: Vec<SelectItem>,
        max_visible: usize,
        selected: Option<usize>,
    ) -> Self {
        let mut list = SelectList::new(
            items,
            max_visible,
            get_select_list_theme(),
            SelectListLayoutOptions::default(),
        );
        if let Some(index) = selected {
            list.set_selected_index(index);
        }
        Self {
            name: title.to_string(),
            count: String::new(),
            list: Rc::new(RefCell::new(list)),
            detail: None,
        }
    }

    /// The console count beside the title (`8 themes`), blank by default.
    pub fn with_count(mut self, count: impl Into<String>) -> Self {
        self.count = count.into();
        self
    }

    /// The detail drawn beside the list for the selected item.
    pub fn with_detail(mut self, detail: DetailFn) -> Self {
        self.detail = Some(detail);
        self
    }

    /// `getSelectList()`.
    pub fn select_list(&self) -> Rc<RefCell<SelectList>> {
        self.list.clone()
    }
}

impl Component for FramedSelectList {
    /// The pane: the named title, one row per item with ` › ` on the
    /// selection band, the selected item's detail, the scroll position
    /// below, and the hint row.
    fn render(&mut self, width: u16) -> Vec<String> {
        let pal = Palette::current();
        let list = self.list.borrow();
        let mut console = Console::new(self.name.clone(), self.count.clone());
        console.rows = list
            .shown_items()
            .iter()
            .map(|item| select_row(item, &pal))
            .collect();
        console.selected =
            (!console.rows.is_empty()).then(|| list.selected_index().min(console.rows.len() - 1));
        console.max_visible = Some(list.max_visible());
        if let Some(footer) = list.footer() {
            console.bottom = vec![Line::from(vec![Span::styled(
                format!("  {footer}"),
                pal.dim(),
            )])];
        }
        if let (Some(detail), Some(item)) = (self.detail, list.get_selected_item()) {
            console.detail = detail(item);
        }
        console.hint = hints(&[
            (
                key_pair_hint("tui.select.up", "tui.select.down").as_str(),
                "move",
            ),
            (key_hint_text("tui.select.confirm").as_str(), "pick"),
            (key_hint_text("tui.select.cancel").as_str(), "back"),
        ]);
        console.render(width as usize)
    }

    fn handle_input(&mut self, data: &str) {
        self.list.borrow_mut().handle_input(data);
    }
}

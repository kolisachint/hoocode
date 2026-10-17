//! A `SelectList` in a Mission Control console: the shape the small pickers
//! share (`thinking`, `theme`, `images`, `session colour`).
//!
//! Behaviour (keys, filtering, the `on_select` / `on_cancel` /
//! `on_selection_change` callbacks the owners wire, e.g. the theme live
//! preview) stays on the shared [`SelectList`]; only the chrome changed:
//! the title names the [`Console`], the items draw as console rows, and the
//! hint sits in the foot edge. No `InputFrame`, no SGR theme painting.

use std::cell::RefCell;
use std::rc::Rc;

use hoocode_code_tui_theme::get_select_list_theme;
use hoocode_code_tui_widgets::console::{hints, Console};
use hoocode_tui_components::{SelectItem, SelectList, SelectListLayoutOptions};
use hoocode_tui_ratatui::{Line, Palette, Span};
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

/// One picker row (content only; the console adds the `›` marker or gutter
/// and the selection band): the primary label with its dim description.
fn select_row(item: &SelectItem) -> Line<'static> {
    let pal = Palette::current();
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

/// A titled select list: the title names the console, the list owns the
/// behaviour.
pub struct FramedSelectList {
    name: String,
    list: Rc<RefCell<SelectList>>,
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
            list: Rc::new(RefCell::new(list)),
        }
    }

    /// `getSelectList()`.
    pub fn select_list(&self) -> Rc<RefCell<SelectList>> {
        self.list.clone()
    }
}

impl Component for FramedSelectList {
    /// The console: the named frame, one row per item with `›` on the
    /// selection band, the scroll position below, and the hint foot.
    fn render(&mut self, width: u16) -> Vec<String> {
        let pal = Palette::current();
        let list = self.list.borrow();
        let mut console = Console::new(self.name.clone(), "");
        console.rows = list.shown_items().iter().map(select_row).collect();
        console.selected =
            (!console.rows.is_empty()).then(|| list.selected_index().min(console.rows.len() - 1));
        console.max_visible = Some(list.max_visible());
        if let Some(footer) = list.footer() {
            console.bottom = vec![Line::from(vec![Span::styled(
                format!("  {footer}"),
                pal.faint(),
            )])];
        }
        console.hint = hints(&[("↑↓", "navigate"), ("enter", "select"), ("esc", "cancel")]);
        console.render(width as usize)
    }

    fn handle_input(&mut self, data: &str) {
        self.list.borrow_mut().handle_input(data);
    }
}

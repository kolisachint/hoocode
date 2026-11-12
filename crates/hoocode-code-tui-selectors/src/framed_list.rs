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
use hoocode_tui_glyphs::status;
use hoocode_tui_ratatui::{bold, truncate_line, Line, Palette, Span, Style};
use hoocode_tui_render::Component;
use hoocode_tui_util::{truncate_at_word, visible_width};

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

/// The fewest cells a picker row's name keeps when its primary facts push
/// it to cut (`picker_row`). A shorter name is never cut.
const NAME_FLOOR: usize = 12;

/// One depth-2 picker row on the U21/U22 row grammar (`docs/design/tui.md`
/// "Row format"): `mark + bold name + ink target + dim facts joined with ·`.
///
/// The mark carries the state so a row never reads as colour alone: `●`
/// (ink) for the row holding the state, `○` (dim) for the rest. Secondary
/// facts drop whole, right-to-left; the target shortens at a word (a first
/// word longer than the budget is cut hard, so a single long token still
/// fits); then, when primary facts are set, the name cuts at a word, but
/// only once every secondary fact is gone and never below
/// [`NAME_FLOOR`] cells (or its full length, when shorter), so the id keeps
/// its place before the primary facts (`● claude-sonnet-5-5  anthropic · on`
/// becomes `● claude-son…  anthropic · on`); primary facts never drop. A
/// final `truncate_line` guard keeps the row within `width` no matter what.
/// The cut order mirrors `tool_row::header_spans`, measured with visible
/// widths. It lives here rather than reusing that helper because `Mark`
/// has no `○` shape and picker facts keep their own colours (the green
/// `configured`, the ink `on`, the warn `expired`).
pub(crate) fn picker_row(
    active: bool,
    name: &str,
    target: &str,
    primary: Vec<(String, Style)>,
    secondary: Vec<(String, Style)>,
    width: usize,
    pal: &Palette,
) -> Line<'static> {
    // Green is the done mark only: a picker's `●` (on, current) is ink.
    let (mark, mark_style) = if active {
        (status::DONE_STR, pal.ink())
    } else {
        (status::TODO_STR, pal.dim())
    };
    let render_text = |name: &str, target: &str, secs: &[(String, Style)]| -> String {
        let mut out = format!("{mark} {name}");
        if !target.is_empty() {
            out.push(' ');
            out.push_str(target);
        }
        let facts = fact_texts(&primary, secs);
        if !facts.is_empty() {
            out.push_str("  ");
            out.push_str(&facts.join(" · "));
        }
        out
    };
    // Step 1: secondary facts drop whole, right to left.
    let mut kept = secondary;
    while visible_width(&render_text(name, target, &kept)) > width {
        if kept.pop().is_none() {
            break;
        }
    }
    // Step 2: the target shortens at a word against the kept facts.
    let mut short = target.to_string();
    if visible_width(&render_text(name, &short, &kept)) > width {
        let facts = fact_texts(&primary, &kept);
        let facts_len = if facts.is_empty() {
            0
        } else {
            2 + visible_width(&facts.join(" · "))
        };
        let head_len = visible_width(&format!("{mark} {name}"))
            .saturating_add(if short.is_empty() { 0 } else { 1 });
        short = truncate_at_word(target, width.saturating_sub(head_len + facts_len));
    }
    // Step 3: the name cuts at a word against the primary facts, so they keep
    // their place. It runs only when no secondary fact is left (step 1 drops
    // them all before it can overflow) and it keeps at least `NAME_FLOOR`
    // cells. Only with primary facts: a row without them keeps its name.
    let mut shown_name = name.to_string();
    if !primary.is_empty() && visible_width(&render_text(name, &short, &kept)) > width {
        let fixed = visible_width(&render_text(name, &short, &kept)) - visible_width(name);
        let floor = NAME_FLOOR.min(visible_width(name));
        shown_name = truncate_at_word(name, width.saturating_sub(fixed).max(floor));
    }
    let name = shown_name.as_str();
    let mut spans = vec![
        Span::styled(mark.to_string(), mark_style),
        Span::raw(" ".to_string()),
        Span::styled(name.to_string(), bold(pal.ink())),
    ];
    if !short.is_empty() {
        spans.push(Span::raw(" ".to_string()));
        spans.push(Span::styled(short, pal.ink()));
    }
    let mut first = true;
    for (text, style) in primary.iter().chain(kept.iter()) {
        if text.is_empty() {
            continue;
        }
        if first {
            spans.push(Span::raw("  ".to_string()));
            first = false;
        } else {
            spans.push(Span::styled(" · ".to_string(), pal.dim()));
        }
        spans.push(Span::styled(text.clone(), *style));
    }
    truncate_line(&Line::from(spans), width)
}

/// The non-empty fact texts, primary then secondary, for measuring and
/// joining (` · `).
fn fact_texts<'a>(
    primary: &'a [(String, Style)],
    secondary: &'a [(String, Style)],
) -> Vec<&'a str> {
    primary
        .iter()
        .map(|(text, _)| text.as_str())
        .chain(secondary.iter().map(|(text, _)| text.as_str()))
        .filter(|text| !text.is_empty())
        .collect()
}

/// One picker row (content only; the pane adds the ` › ` marker or gutter
/// and the selection band): `●`/`○` mark, the bold name, the ink target,
/// and the dim facts. A thinking description's token parenthetical is its
/// own fact (`Light reasoning (~2k tokens)` gives target `Light reasoning`
/// and fact `~2k tokens`); `in use` is a fact, never colour alone. The
/// selected row carries `●`: the cursor is the working state (the theme
/// live-previews as it moves).
fn select_row(item: &SelectItem, selected: bool, width: usize, pal: &Palette) -> Line<'static> {
    let name = if item.label.is_empty() {
        item.value.clone()
    } else {
        item.label.clone()
    };
    let (target, facts) = split_description(item.description.as_deref());
    let facts = facts.into_iter().map(|fact| (fact, pal.dim())).collect();
    picker_row(selected, &name, &target, Vec::new(), facts, width, pal)
}

/// A description as an ink target plus dim facts: `in use` is a fact; a
/// trailing `(…)` parenthetical splits off as its own fact; anything else
/// stays the target.
fn split_description(description: Option<&str>) -> (String, Vec<String>) {
    let single = description
        .map(|text| {
            text.split(['\r', '\n'])
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty());
    let Some(single) = single else {
        return (String::new(), Vec::new());
    };
    if single == "in use" {
        return (String::new(), vec!["in use".to_string()]);
    }
    if let Some((target, token)) = split_token_fact(&single) {
        return (target, vec![token]);
    }
    (single, Vec::new())
}

/// `Light reasoning (~2k tokens)` → (`Light reasoning`, `~2k tokens`): a
/// trailing parenthetical is its own fact. `None` without one.
fn split_token_fact(text: &str) -> Option<(String, String)> {
    let inner = text.strip_suffix(')')?;
    let (target, token) = inner.rsplit_once('(')?;
    let (target, token) = (target.trim(), token.trim());
    (!target.is_empty() && !token.is_empty()).then(|| (target.to_string(), token.to_string()))
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
        Span::styled("Edit".to_string(), bold(pal.ink())),
        Span::styled(" src/wrap.rs  +4 −1".to_string(), pal.dim()),
    ]));
    rows.push(Line::from(vec![
        Span::styled("⊗ ".to_string(), pal.err()),
        Span::styled("Shell".to_string(), bold(pal.ink())),
        Span::styled(" cargo test -p wrap".to_string(), pal.ink()),
        Span::styled("  ".to_string(), pal.dim()),
        Span::styled("exit 1".to_string(), pal.err()),
        Span::styled(" · 3.1s".to_string(), pal.dim()),
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
        if let (Some(detail), Some(item)) = (self.detail, list.get_selected_item()) {
            console.detail = detail(item);
        }
        // The detail is set before the rows: a split narrows the list
        // column, and rows are built to that width.
        let row_width = console.row_width(width as usize);
        let shown = list.shown_items();
        let selected = (!shown.is_empty()).then(|| list.selected_index().min(shown.len() - 1));
        console.rows = shown
            .iter()
            .enumerate()
            .map(|(i, item)| select_row(item, Some(i) == selected, row_width, &pal))
            .collect();
        console.selected = selected;
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

#[cfg(test)]
mod picker_row_tests {
    use super::*;

    fn row_text(line: &Line<'static>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn a_single_token_longer_than_the_budget_is_cut_to_width() {
        let pal = Palette::current();
        // A 40-cell token with no spaces: the old word-prefix fallback
        // returned the whole token plus `…`, overrunning the row.
        let target = "a".repeat(40);
        let line = picker_row(false, "Name", &target, Vec::new(), Vec::new(), 20, &pal);
        let text = row_text(&line);
        assert!(visible_width(&text) <= 20, "{text}");
        assert!(text.contains('…'), "{text}");
    }
}

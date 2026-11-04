//! The picker console: the pane every picker draws (declutter spec v2.4,
//! drawn by [`Pane`]). The console name `HANGAR · /model` splits into the
//! title chip and the dim command; the tabs, count and right label follow as
//! facts. Rows mark the selection with ` › ` on the selection background;
//! the query is the pane's input row at the bottom; the hint row is last.
//!
//! Rows, query and hint arrive as styled [`Line`]s; this module only maps
//! them onto the pane. The picker keeps all behaviour (keys, filtering,
//! events).

use hoocode_tui_ratatui::{Line, Span};

use crate::pane::{hint_line, input_line, Chip, Pane, PaneStyle, PANE_CURSOR};

/// The ` › ` marker every console flags its selected row with (accent).
/// Unselected rows carry a blank gutter of the same width.
pub const CONSOLE_CURSOR: &str = PANE_CURSOR;

/// The blank gutter of an unselected console row.
pub fn console_gutter() -> String {
    crate::pane::gutter()
}

/// A console: the title (name + count), selectable rows with a scrolling
/// window, the optional query (input) row and the hint row. `top` rows sit
/// above the list (scope lines, headers); `bottom` rows below it (footers,
/// errors, detail lines).
#[derive(Debug, Clone)]
pub struct Console {
    /// `HANGAR · /model`: the part before ` · ` is the title chip, the rest
    /// the dim command.
    pub name: String,
    /// `3 ships` (empty when the console counts nothing).
    pub count: String,
    /// Rows above the list.
    pub top: Vec<Line<'static>>,
    /// The query row's content (see [`query_row`]); none without a query.
    pub query: Option<Line<'static>>,
    /// The selectable rows (content only; gutter and marker added here).
    pub rows: Vec<Line<'static>>,
    /// The selected row's index into [`Console::rows`]; none when empty.
    pub selected: Option<usize>,
    /// The list window: at most this many rows draw, centred on the
    /// selection like [`PickerCursor::window`](hoocode_tui_components::PickerCursor).
    pub max_visible: Option<usize>,
    /// Rows below the list.
    pub bottom: Vec<Line<'static>>,
    /// The hint row, drawn last.
    pub hint: Line<'static>,
    /// A label after the count (`/tasks · ctrl+g`); empty draws nothing.
    pub right: Line<'static>,
    /// Layer tabs between the name and the count (CONTROLS' `plain ctrl
    /// alt`); empty draws nothing.
    pub tabs: Line<'static>,
    /// How the tabs join the name.
    pub tabs_join: TabsJoin,
    /// The title chip's look.
    pub chip: Chip,
}

/// What sits between a console's name and its tabs on the title row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabsJoin {
    /// Three spaces (CONTROLS' `plain ctrl alt`).
    Gap,
    /// Two spaces (the task panel's tab strip).
    Edge,
    /// Two spaces: kept for the task panel's narrowing steps, which now
    /// draw like [`TabsJoin::Edge`].
    Space,
}

impl Console {
    /// A console named `name` counting `count`.
    pub fn new(name: impl Into<String>, count: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            count: count.into(),
            top: Vec::new(),
            query: None,
            rows: Vec::new(),
            selected: None,
            max_visible: None,
            bottom: Vec::new(),
            hint: Line::raw(""),
            right: Line::raw(""),
            tabs: Line::raw(""),
            tabs_join: TabsJoin::Gap,
            chip: Chip::Accent,
        }
    }

    /// The cells [`Console::tabs`] may take on the title row at `width`:
    /// the title budget less the name, the join, the count and the right
    /// label. Wider tabs are cut with `…`.
    pub fn tabs_budget(&self, width: usize) -> usize {
        let pane = self.title_pane(Line::raw("x"));
        let fixed = pane.title_width(width).saturating_sub(1);
        pane.title_budget(width).saturating_sub(fixed)
    }

    /// The pane carrying only the title: chip, command, then the facts
    /// (tabs, count, right label).
    fn title_pane(&self, tabs: Line<'static>) -> Pane {
        let st = PaneStyle::current();
        let (title, command) = match self.name.split_once(" · ") {
            Some((title, command)) => (title.to_string(), command.to_string()),
            None => (self.name.clone(), String::new()),
        };
        let mut facts: Vec<Span<'static>> = Vec::new();
        if tabs.width() > 0 {
            // The pane puts two spaces before the facts; the gap join adds one.
            if self.tabs_join == TabsJoin::Gap && !title.is_empty() {
                facts.push(Span::raw(" "));
            }
            facts.extend(tabs.spans);
        }
        if !self.count.is_empty() {
            if !facts.is_empty() {
                facts.push(Span::raw("  "));
            }
            facts.push(Span::styled(self.count.clone(), st.dim_style()));
        }
        if self.right.width() > 0 {
            if !facts.is_empty() {
                facts.push(Span::raw("  "));
            }
            facts.extend(self.right.spans.iter().cloned());
        }
        let mut pane = Pane::new(title);
        pane.command = command;
        pane.chip = self.chip;
        pane.facts = Line::from(facts);
        pane
    }

    /// Draws the console at `width` columns.
    pub fn render(&self, width: usize) -> Vec<String> {
        let mut pane = self.title_pane(self.tabs.clone());
        pane.top = self.top.clone();
        pane.rows = self.rows.clone();
        pane.selected = self.selected;
        pane.max_visible = self.max_visible;
        pane.bottom = self.bottom.clone();
        pane.input = self.query.clone();
        pane.hint = self.hint.clone();
        pane.render(width)
    }
}

/// The query row's content: the typed text, or the dim `filter`
/// placeholder; a caret while the picker has focus.
pub fn query_row(text: &str, caret: bool) -> Line<'static> {
    input_line(text, "filter", caret)
}

/// One hints row: keys in normal text, dim actions, joined by ` · `.
pub fn hints(hints: &[(&str, &str)]) -> Line<'static> {
    hint_line(hints)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoocode_tui_util::{strip_vt_control_characters, visible_width};

    fn console() -> Console {
        let mut console = Console::new("HANGAR · /model", "3 ships");
        console.rows = vec![
            Line::raw("mock-large  mock  200k"),
            Line::raw("mock-fast  mock  128k"),
            Line::raw("mock-local  ollama  32k"),
        ];
        console.selected = Some(1);
        console.query = Some(query_row("", false));
        console.hint = hints(&[("↑↓", "ship"), ("enter", "fly it"), ("esc", "close")]);
        console
    }

    fn plain(rows: &[String]) -> Vec<String> {
        rows.iter()
            .map(|row| strip_vt_control_characters(row))
            .collect()
    }

    /// Every emitted line fits its width, even where the pane has no room.
    #[test]
    fn console_rows_fit() {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        for width in [3, 9, 10, 19, 40, 80, 100] {
            let rows = console().render(width);
            assert!(!rows.is_empty(), "width {width}");
            for row in &rows {
                assert!(
                    visible_width(row) <= width,
                    "width {width}: {row:?} is too wide"
                );
            }
        }
    }

    #[test]
    fn selected_row_carries_the_marker() {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        let rows = plain(&console().render(40));
        let selected = rows.iter().find(|r| r.contains("mock-fast")).unwrap();
        assert!(selected.starts_with(" › mock-fast"), "{selected:?}");
        let unselected = rows.iter().find(|r| r.contains("mock-large")).unwrap();
        assert!(unselected.starts_with("   mock-large"), "{unselected:?}");
    }

    #[test]
    fn title_names_the_console_and_its_count() {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        let rows = plain(&console().render(100));
        assert!(
            rows[0].starts_with("  HANGAR  /model  3 ships"),
            "{:?}",
            rows[0]
        );
        // The query is the input row, above the hint row.
        assert!(rows[rows.len() - 2].starts_with(" › filter"), "{rows:?}");
        assert!(
            rows[rows.len() - 1].contains("esc close"),
            "{:?}",
            rows[rows.len() - 1]
        );
    }

    #[test]
    fn tabs_budget_leaves_room_for_the_title() {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        let mut console = Console::new("FLIGHT PLAN", "");
        console.tabs_join = TabsJoin::Edge;
        let budget = console.tabs_budget(40);
        console.tabs = Line::raw("x".repeat(budget));
        let rows = plain(&console.render(40));
        assert!(!rows[0].contains('…'), "{:?}", rows[0]);
        console.tabs = Line::raw("x".repeat(budget + 1));
        let rows = plain(&console.render(40));
        assert!(rows[0].contains('…'), "{:?}", rows[0]);
    }
}

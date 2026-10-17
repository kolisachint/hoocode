//! The Mission Control console (M7, spec §4 d1–d10): the thick block that
//! replaces the comms frame for every picker. The title names the console
//! and its count (`HANGAR · /model  3 ships`); an optional query row carries
//! `▸ text`; each row marks the selection with accent `›` on the selection
//! band; the hint row sits in the foot edge.
//!
//! The chrome is the shared card chrome ([`render_chrome`]: thick [`Block`]);
//! the frame defaults to the `you` token (spec §2.4: console frames that
//! take keys). Rows, query and hint arrive as styled [`Line`]s from the
//! [`Palette`]; this module adds only the marker, the gutter, the band and
//! the scrolling window.

use hoocode_tui_ratatui::{bold, render_chrome, truncate_line, Chrome, Line, Palette, Span, Style};
use hoocode_tui_util::visible_width;

/// The `›` marker every console flags its selected row with (accent, spec
/// §2.4). Unselected rows carry a blank gutter of the same width.
pub const CONSOLE_CURSOR: &str = "› ";

/// The blank gutter of an unselected console row.
pub fn console_gutter() -> String {
    " ".repeat(visible_width(CONSOLE_CURSOR))
}

/// A console: the title (name + count), optional query, selectable rows
/// with a scrolling window, and the hint foot. `top` rows sit above the
/// query (scope lines, headers); `bottom` rows below the list (footers,
/// errors, detail lines). The picker keeps all behaviour (keys, filtering,
/// events); this only draws.
#[derive(Debug, Clone)]
pub struct Console {
    /// `HANGAR · /model`.
    pub name: String,
    /// `3 ships` (empty when the console counts nothing).
    pub count: String,
    /// Rows above the query.
    pub top: Vec<Line<'static>>,
    /// The `▸ text` query row (see [`query_row`]); none without a query.
    pub query: Option<Line<'static>>,
    /// The selectable rows (content only; marker and band added here).
    pub rows: Vec<Line<'static>>,
    /// The selected row's index into [`Console::rows`]; none when empty.
    pub selected: Option<usize>,
    /// The list window: at most this many rows draw, centred on the
    /// selection like [`PickerCursor::window`](hoocode_tui_components::PickerCursor).
    pub max_visible: Option<usize>,
    /// Rows below the list.
    pub bottom: Vec<Line<'static>>,
    /// The hint row, drawn in the foot edge.
    pub hint: Line<'static>,
    /// The right label on the top edge (`/tasks · ctrl+g`); empty draws nothing.
    pub right: Line<'static>,
    /// The frame colour; the `you` token when `None`.
    pub frame: Option<Style>,
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
            frame: None,
        }
    }

    /// Draws the console at `width` columns. Narrow frames draw bare, like
    /// the card chrome.
    pub fn render(&self, width: usize) -> Vec<String> {
        let pal = Palette::current();
        let frame = self.frame.unwrap_or_else(|| pal.you());
        let title = if self.count.is_empty() {
            self.name.clone()
        } else {
            format!("{}  {}", self.name, self.count)
        };
        let (rows, selected) = window_rows(&self.rows, self.selected, self.max_visible);
        let mut body = self.top.clone();
        if let Some(query) = &self.query {
            body.push(query.clone());
        }
        body.extend(
            rows.iter()
                .zip(rows_visible(rows.len(), selected))
                .map(|(line, is_selected)| console_row(line, is_selected, &pal, width)),
        );
        body.extend(self.bottom.iter().cloned());
        render_chrome(
            &Chrome {
                frame,
                title: Line::from(vec![Span::styled(title, bold(frame))]),
                right: self.right.clone(),
                foot: self.hint.clone(),
            },
            &body,
            width,
        )
    }
}

/// `true` per visible row: only the selected row.
fn rows_visible(len: usize, selected: Option<usize>) -> Vec<bool> {
    (0..len).map(|i| Some(i) == selected).collect()
}

/// The visible window of `rows`: at most `max` rows centred on `selected`
/// (the head or tail when the selection sits at an end). No cap without
/// `max`. The selection is remapped into the window (clamped away when it
/// points past the rows).
fn window_rows(
    rows: &[Line<'static>],
    selected: Option<usize>,
    max: Option<usize>,
) -> (Vec<Line<'static>>, Option<usize>) {
    let len = rows.len();
    let Some(max) = max.filter(|&max| max > 0 && len > max) else {
        let sel = selected.filter(|&s| s < len);
        return (rows.to_vec(), sel);
    };
    let sel = selected.map(|s| s.min(len.saturating_sub(1)));
    let start = sel
        .map(|s| s.saturating_sub(max / 2).min(len - max))
        .unwrap_or(0);
    let window = rows[start..start + max].to_vec();
    (window, sel.map(|s| s.saturating_sub(start)))
}

/// One list row: the accent `›` marker (or the blank gutter) plus the
/// picker's content. The selected row carries the selection band, padded to
/// the inner width.
fn console_row(line: &Line<'static>, selected: bool, pal: &Palette, width: usize) -> Line<'static> {
    let mut spans = if selected {
        vec![Span::styled(CONSOLE_CURSOR, pal.acc())]
    } else {
        vec![Span::raw(console_gutter())]
    };
    spans.extend(line.spans.iter().cloned());
    let mut row = Line::from(spans);
    if selected {
        let band = pal.band();
        for span in &mut row.spans {
            span.style = span.style.patch(band);
        }
        let inner = width.saturating_sub(2);
        let used: usize = row.spans.iter().map(|s| s.width()).sum();
        if used < inner {
            row.spans.push(Span::styled(" ".repeat(inner - used), band));
        }
    }
    truncate_line(&row, width.saturating_sub(2).max(1))
}

/// The `▸ text` query row, with a warn caret while the picker has focus.
pub fn query_row(text: &str, caret: bool) -> Line<'static> {
    let pal = Palette::current();
    let mut spans = vec![
        Span::styled("▸", pal.acc()),
        Span::raw(" "),
        Span::raw(text.to_string()),
    ];
    if caret {
        spans.push(Span::styled("▏", pal.warn()));
    }
    Line::from(spans)
}

/// One hints row: dim keys plus faint descriptions, joined by `·`.
pub fn hints(hints: &[(&str, &str)]) -> Line<'static> {
    let pal = Palette::current();
    let mut spans = Vec::new();
    for (i, (key, description)) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" · ", pal.faint()));
        }
        spans.push(Span::styled((*key).to_string(), pal.dim()));
        spans.push(Span::styled(format!(" {description}"), pal.faint()));
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoocode_tui_ratatui::flatten;

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

    /// Every emitted line fits its width, even where the frame has no room.
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
    fn selected_row_carries_the_marker_and_band() {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        let rows = console().render(40);
        let selected = rows.iter().find(|r| r.contains("mock-fast")).unwrap();
        assert!(selected.contains('›'), "{selected:?} marks the selection");
        let plain: String = hoocode_tui_util::strip_vt_control_characters(selected)
            .chars()
            .collect();
        assert!(plain.starts_with("┃› mock-fast"), "{plain:?}");
        let unselected = rows.iter().find(|r| r.contains("mock-large")).unwrap();
        assert!(!unselected.contains('›'), "{unselected:?} has no marker");
    }

    #[test]
    fn window_centres_on_the_selection() {
        let rows: Vec<Line<'static>> = (0..10).map(|i| Line::raw(format!("row {i}"))).collect();
        let (window, selected) = window_rows(&rows, Some(7), Some(4));
        assert_eq!(window.len(), 4);
        assert_eq!(selected, Some(2));
        assert_eq!(flatten(&window[2]), "row 7");
        let (head, selected) = window_rows(&rows, Some(0), Some(4));
        assert_eq!(selected, Some(0));
        assert_eq!(flatten(&head[0]), "row 0");
        let (tail, selected) = window_rows(&rows, Some(9), Some(4));
        assert_eq!(selected, Some(3));
        assert_eq!(flatten(&tail[3]), "row 9");
        // No cap without max; a past-the-end selection clamps away.
        let (all, selected) = window_rows(&rows, Some(99), None);
        assert_eq!(all.len(), 10);
        assert_eq!(selected, None);
    }

    #[test]
    fn title_names_the_console_and_its_count() {
        use hoocode_tui_util::strip_vt_control_characters;
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        // ANSI splits styled runs: strip it before searching the text.
        let rows: Vec<String> = console()
            .render(100)
            .iter()
            .map(|row| strip_vt_control_characters(row))
            .collect();
        assert!(
            rows[0].contains("HANGAR · /model  3 ships"),
            "{:?}",
            rows[0]
        );
        assert!(
            rows[rows.len() - 1].contains("esc close"),
            "{:?}",
            rows[rows.len() - 1]
        );
    }
}

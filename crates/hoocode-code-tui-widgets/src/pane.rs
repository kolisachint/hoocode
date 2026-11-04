//! Panes (declutter spec v2.4, v2.9): the nvim-style surface every dialog,
//! picker and card draws on.
//!
//! In colour a pane is borderless: its own background is the edge. Top to
//! bottom it has a title row (a ` Title ` chip, then the dim `/command` and
//! facts), the rows (a 3-column gutter, ` › ` marks the selected row on the
//! selection background), an optional input row (input background, ` › `,
//! a dim placeholder) and the hint row last. With a detail column and at
//! least [`SPLIT_MIN_WIDTH`] columns the body splits into list | detail: the
//! detail column one stronger shade, after a blank 1-column gutter. Narrower,
//! the detail stacks under the list (or hides, see [`NarrowDetail`]).
//!
//! In monochrome (colour off: NO_COLOR, `TERM=dumb`, no tty) the pane
//! gets light `┌─┐│└┘` borders with the title in the top border, the split
//! gutter becomes a `│`, and the selected row keeps ` › ` plus bold. There is
//! no rule above the input row.
//!
//! Every painted background also sets its foreground ([`PaneStyle`]). The
//! pane only draws: callers keep keys, filtering and events.

use hoocode_tui_ratatui::core::style::Color;
use hoocode_tui_ratatui::{
    buffer_to_lines, flatten, truncate_line, Buffer, Line, Modifier, Palette, Rect, Span, Style,
};
use hoocode_tui_util::{truncate_to_width, visible_width};

/// The selected row's marker, in the 3-column gutter.
pub const PANE_CURSOR: &str = " › ";

/// The width of the row gutter (` › ` or three blanks).
pub const GUTTER: usize = 3;

/// The narrowest pane that splits into list | detail.
pub const SPLIT_MIN_WIDTH: usize = 100;

/// Below this width a monochrome pane has no room for borders.
const MONO_BORDER_MIN_WIDTH: usize = 10;

/// The hint row shows at most this many items before `f1 more`.
pub const MAX_HINTS: usize = 4;

/// The title chip's look.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Chip {
    /// Accent chip with dark chip text (every ordinary pane).
    #[default]
    Accent,
    /// Yellow chip with dark chip text: the pane blocks the agent
    /// (permission, question).
    Warn,
    /// Bold title text, no chip.
    Plain,
}

/// What a split pane does with its detail column below [`SPLIT_MIN_WIDTH`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NarrowDetail {
    /// The detail rows go under the list, full width (spec v2.4).
    #[default]
    Stacked,
    /// The list alone.
    Hidden,
}

/// The pane colours. Colour panes paint every cell with a background and a
/// foreground from here; a monochrome pane paints no colour at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneStyle {
    /// Borders and bold/dim only (NO_COLOR, `TERM=dumb`, < 256 colours).
    pub mono: bool,
    /// The prompt / user-message surface.
    pub surface: Color,
    /// The pane background.
    pub pane: Color,
    /// The selected row's background.
    pub pane_sel: Color,
    /// The input row's background.
    pub pane_input: Color,
    /// The detail column's background (split panes).
    pub pane_detail: Color,
    /// The accent chip's background.
    pub chip: Color,
    /// The blocking (warn) chip's background.
    pub chip_warn: Color,
    /// The accent chip's text.
    pub chip_fg: Color,
    /// The warn chip's text (dark on both themes).
    pub chip_warn_fg: Color,
    /// Normal text.
    pub text: Color,
    /// Dim text (the one secondary tone).
    pub dim: Color,
    /// The accent (the ` › ` marker).
    pub accent: Color,
}

const fn rgb(hex: u32) -> Color {
    Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

impl PaneStyle {
    /// The dark pane palette (spec v2.4, v2.9).
    pub const fn dark() -> Self {
        Self {
            mono: false,
            surface: rgb(0x2a2a36),
            pane: rgb(0x1e1e2a),
            pane_sel: rgb(0x233239),
            pane_input: rgb(0x2a2a36),
            pane_detail: rgb(0x292b40),
            chip: rgb(0x5cc8bb),
            chip_warn: rgb(0xf0c674),
            chip_fg: rgb(0x14141c),
            chip_warn_fg: rgb(0x14141c),
            text: rgb(0xe5e5e7),
            dim: rgb(0x989898),
            accent: rgb(0x5cc8bb),
        }
    }

    /// The light pane palette (spec v2.7, v2.9): panes darker than the page.
    pub const fn light() -> Self {
        Self {
            mono: false,
            surface: rgb(0xdadbe6),
            pane: rgb(0xe8e8f0),
            pane_sel: rgb(0xc3dcdc),
            pane_input: rgb(0xdadbe6),
            pane_detail: rgb(0xd3d7e9),
            chip: rgb(0x00625c),
            chip_warn: rgb(0xf0c674),
            chip_fg: rgb(0xf8f8fb),
            chip_warn_fg: rgb(0x2b2f36),
            text: rgb(0x2b2f36),
            dim: rgb(0x555864),
            accent: rgb(0x00625c),
        }
    }

    /// The monochrome pane: borders, bold and dim, no colour.
    pub const fn mono() -> Self {
        let mut style = Self::dark();
        style.mono = true;
        style
    }

    /// The pane style for the current theme and terminal: monochrome when
    /// colour is off (NO_COLOR, `TERM=dumb`; the app has no below-256-colour
    /// tier yet, see `detect_color_mode`); otherwise the theme's pane
    /// tokens and text colours, over the dark or light palette (picked by
    /// the theme's surfaces) where a colour does not resolve.
    pub fn current() -> Self {
        if !hoocode_code_tui_theme::chalk::enabled() {
            return Self::mono();
        }
        let pal = Palette::current();
        let mut style = if theme_is_light(&pal) {
            Self::light()
        } else {
            Self::dark()
        };
        // The theme's pane surfaces (each falls back to `selectedBg` in a
        // theme that predates them) and its chip text.
        for (token, slot) in [
            ("surface", &mut style.surface),
            ("pane", &mut style.pane),
            ("paneSel", &mut style.pane_sel),
            ("paneInput", &mut style.pane_input),
            ("paneDetail", &mut style.pane_detail),
            ("chip", &mut style.chip),
            ("chipWarn", &mut style.chip_warn),
        ] {
            if let Some(color) = pal.bg_color(token) {
                *slot = color;
            }
        }
        // Text colours: the theme's where they resolve (`ink` is empty on
        // dark, meaning the default text, so the dark text stands in). The
        // warn chip's dark text has no token: it stays the palette's.
        for (fg, slot) in [
            (pal.style("chipFg").fg, &mut style.chip_fg),
            (pal.ink().fg, &mut style.text),
            (pal.dim().fg, &mut style.dim),
            (pal.acc().fg, &mut style.accent),
        ] {
            if let Some(color @ (Color::Rgb(..) | Color::Indexed(_))) = fg {
                *slot = color;
            }
        }
        style
    }

    /// Normal text: the text colour (no colour in monochrome).
    pub fn text_style(&self) -> Style {
        if self.mono {
            Style::default()
        } else {
            Style::new().fg(self.text)
        }
    }

    /// Dim text: the dim colour, or the dim attribute in monochrome.
    pub fn dim_style(&self) -> Style {
        if self.mono {
            Style::new().add_modifier(Modifier::DIM)
        } else {
            Style::new().fg(self.dim)
        }
    }

    /// The accent (the ` › ` marker); bold in monochrome.
    pub fn accent_style(&self) -> Style {
        if self.mono {
            Style::new().add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(self.accent)
        }
    }

    /// A fill: `bg` with the text colour as foreground (every painted
    /// background also sets its foreground). Nothing in monochrome.
    fn fill(&self, bg: Color) -> Style {
        if self.mono {
            Style::default()
        } else {
            Style::new().bg(bg).fg(self.text)
        }
    }
}

/// Whether the theme paints on paper: its user-message (then selection)
/// background is light.
fn theme_is_light(pal: &Palette) -> bool {
    let bg = pal
        .bg_color("userMessageBg")
        .or_else(|| pal.band().bg)
        .filter(|c| matches!(c, Color::Rgb(..)));
    match bg {
        Some(Color::Rgb(r, g, b)) => {
            let lin = |c: u8| {
                let c = f64::from(c) / 255.0;
                if c <= 0.03928 {
                    c / 12.92
                } else {
                    ((c + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b) > 0.5
        }
        _ => false,
    }
}

/// A pane: title, rows, optional detail column, optional input row and the
/// hint row. All content arrives as styled [`Line`]s; the pane adds the
/// gutter, the marker, the backgrounds and (in monochrome) the borders.
#[derive(Debug, Clone, Default)]
pub struct Pane {
    /// The chip text (`Models`); empty draws no chip.
    pub title: String,
    /// The chip's look.
    pub chip: Chip,
    /// The dim `/command` after the chip; empty draws nothing.
    pub command: String,
    /// Facts after the title, two spaces on (styled by the caller).
    pub facts: Line<'static>,
    /// Rows above the list (1-column pad, not selectable).
    pub top: Vec<Line<'static>>,
    /// The list rows (3-column gutter).
    pub rows: Vec<Line<'static>>,
    /// The selected row's index into [`Pane::rows`].
    pub selected: Option<usize>,
    /// At most this many list rows draw, centred on the selection.
    pub max_visible: Option<usize>,
    /// Rows below the list (1-column pad, not selectable).
    pub bottom: Vec<Line<'static>>,
    /// The detail column (split panes); empty draws no split.
    pub detail: Vec<Line<'static>>,
    /// The list column's width in a split; about a third of the pane when
    /// `None`.
    pub list_width: Option<usize>,
    /// What the detail does below [`SPLIT_MIN_WIDTH`].
    pub narrow_detail: NarrowDetail,
    /// The input row's content (see [`input_line`]); none draws no input row.
    pub input: Option<Line<'static>>,
    /// The hint row (see [`hint_line`]); empty draws no hint row.
    pub hint: Line<'static>,
    /// The colours; [`PaneStyle::current`] when `None`.
    pub style: Option<PaneStyle>,
}

/// What one pane row is.
enum Row {
    Title,
    /// A padded row in the list column (top / bottom rows).
    Padded(Line<'static>),
    /// A list row with the gutter; `true` when selected.
    Item(Line<'static>, bool),
    /// Blank list cells beside a taller detail.
    Blank,
    /// A stacked detail row, full width.
    Stacked(Line<'static>),
    Input(Line<'static>),
    Hint,
}

impl Pane {
    /// A pane titled `title` with an accent chip.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            ..Self::default()
        }
    }

    fn resolved_style(&self) -> PaneStyle {
        self.style.unwrap_or_else(PaneStyle::current)
    }

    /// Whether the pane has a title row (or, in monochrome, a titled border).
    fn has_title(&self) -> bool {
        !self.title.is_empty() || !self.command.is_empty() || self.facts.width() > 0
    }

    /// The title content: chip (or bold title), `/command` and facts.
    fn title_line(&self, st: &PaneStyle, bordered: bool) -> Line<'static> {
        let mut spans = Vec::new();
        if !self.title.is_empty() {
            if st.mono || self.chip == Chip::Plain {
                spans.push(Span::styled(
                    self.title.clone(),
                    st.text_style().add_modifier(Modifier::BOLD),
                ));
            } else {
                let (bg, fg) = match self.chip {
                    Chip::Warn => (st.chip_warn, st.chip_warn_fg),
                    Chip::Accent | Chip::Plain => (st.chip, st.chip_fg),
                };
                spans.push(Span::styled(
                    format!(" {} ", self.title),
                    Style::new().bg(bg).fg(fg).add_modifier(Modifier::BOLD),
                ));
            }
        }
        let after_title = if bordered { " ─ " } else { " " };
        if !self.command.is_empty() {
            if !spans.is_empty() {
                let join = if bordered {
                    Span::styled(after_title, st.dim_style())
                } else {
                    Span::raw(after_title)
                };
                spans.push(join);
            }
            spans.push(Span::styled(self.command.clone(), st.dim_style()));
        }
        if self.facts.width() > 0 {
            if !spans.is_empty() {
                if bordered && self.command.is_empty() {
                    spans.push(Span::styled(after_title, st.dim_style()));
                } else {
                    spans.push(Span::raw("  "));
                }
            }
            spans.extend(self.facts.spans.iter().cloned());
        }
        Line::from(spans)
    }

    /// The cells the title content (chip, command and facts) may take at
    /// `width`; wider titles are cut with `…`.
    pub fn title_budget(&self, width: usize) -> usize {
        let st = self.resolved_style();
        if st.mono && width >= MONO_BORDER_MIN_WIDTH {
            // `┌─ ` + content + ` ─┐`.
            width.saturating_sub(6)
        } else {
            width.saturating_sub(2)
        }
    }

    /// The cells the title content takes at full length.
    pub fn title_width(&self, width: usize) -> usize {
        let st = self.resolved_style();
        let bordered = st.mono && width >= MONO_BORDER_MIN_WIDTH;
        self.title_line(&st, bordered).width()
    }

    /// The cells a list row's content may use at `width` columns: the list
    /// column (the whole pane unless it splits) less the gutter and the
    /// right pad. Rows built to this width never get cut.
    pub fn row_width(&self, width: usize) -> usize {
        let st = self.resolved_style();
        let bordered = st.mono && width >= MONO_BORDER_MIN_WIDTH;
        let inner = if bordered { width - 2 } else { width };
        let split = !self.detail.is_empty() && width >= SPLIT_MIN_WIDTH;
        let column = if split {
            self.split_list_width(inner)
        } else {
            inner
        };
        column.saturating_sub(GUTTER + 1)
    }

    /// The list column width of a split at inner width `inner`.
    fn split_list_width(&self, inner: usize) -> usize {
        let want = self.list_width.unwrap_or(inner * 36 / 100);
        want.clamp(
            GUTTER + 8,
            inner.saturating_sub(GUTTER + 20).max(GUTTER + 8),
        )
    }

    /// Draws the pane at `width` columns. Every row is at most `width`
    /// cells.
    pub fn render(&self, width: usize) -> Vec<String> {
        if width == 0 {
            return Vec::new();
        }
        let st = self.resolved_style();
        let bordered = st.mono && width >= MONO_BORDER_MIN_WIDTH;
        let (x0, inner) = if bordered { (1, width - 2) } else { (0, width) };
        let split = !self.detail.is_empty() && width >= SPLIT_MIN_WIDTH;

        // The rows, top to bottom (the bordered title lives in the border).
        let mut rows: Vec<Row> = Vec::new();
        if self.has_title() && !bordered {
            rows.push(Row::Title);
        }
        let (items, selected) = window_rows(&self.rows, self.selected, self.max_visible);
        let mut list: Vec<Row> = self.top.iter().cloned().map(Row::Padded).collect();
        list.extend(
            items
                .into_iter()
                .enumerate()
                .map(|(i, line)| Row::Item(line, Some(i) == selected)),
        );
        list.extend(self.bottom.iter().cloned().map(Row::Padded));
        let body_start = rows.len();
        let mut body_len = list.len();
        if split {
            body_len = body_len.max(self.detail.len());
            list.resize_with(body_len, || Row::Blank);
        }
        rows.extend(list);
        if !split && !self.detail.is_empty() && self.narrow_detail == NarrowDetail::Stacked {
            if st.mono {
                rows.push(Row::Blank);
            }
            rows.extend(self.detail.iter().cloned().map(Row::Stacked));
        }
        if let Some(input) = &self.input {
            rows.push(Row::Input(input.clone()));
        }
        if self.hint.width() > 0 {
            rows.push(Row::Hint);
        }

        if st.mono && !bordered {
            return rows
                .iter()
                .map(|row| {
                    let text = match row {
                        Row::Title => flatten(&self.title_line(&st, false)),
                        Row::Padded(line) | Row::Stacked(line) => format!(" {}", flatten(line)),
                        Row::Item(line, true) => format!("{PANE_CURSOR}{}", flatten(line)),
                        Row::Item(line, false) => format!("   {}", flatten(line)),
                        Row::Input(line) => format!("{PANE_CURSOR}{}", flatten(line)),
                        Row::Blank => String::new(),
                        Row::Hint => format!(" {}", flatten(&self.hint)),
                    };
                    truncate_to_width(&text, width, "…", false)
                })
                .collect();
        }

        let top_border = usize::from(bordered);
        let height = rows.len() + 2 * top_border;
        let area = Rect::new(0, 0, width as u16, height as u16);
        let mut buf = Buffer::empty(area);
        buf.set_style(area, st.fill(st.pane));

        let list_w = if split {
            self.split_list_width(inner)
        } else {
            inner
        };
        for (i, row) in rows.iter().enumerate() {
            let y = (i + top_border) as u16;
            let in_body = i >= body_start && i < body_start + body_len;
            let col_w = if in_body { list_w } else { inner };
            match row {
                Row::Title => {
                    let line = self.title_line(&st, false);
                    put(&mut buf, x0 + 1, y, inner.saturating_sub(2), &line);
                }
                Row::Padded(line) => {
                    put(&mut buf, x0 + 1, y, col_w.saturating_sub(2), line);
                }
                Row::Item(line, is_selected) => {
                    if *is_selected {
                        fill(&mut buf, x0, y, col_w, st.fill(st.pane_sel));
                        put(
                            &mut buf,
                            x0,
                            y,
                            col_w.min(GUTTER),
                            &Line::styled(PANE_CURSOR, st.accent_style()),
                        );
                        let line = if st.mono {
                            emboldened(line)
                        } else {
                            line.clone()
                        };
                        put(
                            &mut buf,
                            x0 + GUTTER,
                            y,
                            col_w.saturating_sub(GUTTER + 1),
                            &line,
                        );
                    } else {
                        put(
                            &mut buf,
                            x0 + GUTTER,
                            y,
                            col_w.saturating_sub(GUTTER + 1),
                            line,
                        );
                    }
                }
                Row::Blank => {}
                Row::Stacked(line) => {
                    fill(&mut buf, x0, y, inner, st.fill(st.pane_detail));
                    put(&mut buf, x0 + 1, y, inner.saturating_sub(2), line);
                }
                Row::Input(line) => {
                    fill(&mut buf, x0, y, inner, st.fill(st.pane_input));
                    put(
                        &mut buf,
                        x0,
                        y,
                        inner.min(GUTTER),
                        &Line::styled(PANE_CURSOR, st.accent_style()),
                    );
                    put(
                        &mut buf,
                        x0 + GUTTER,
                        y,
                        inner.saturating_sub(GUTTER + 1),
                        line,
                    );
                }
                Row::Hint => {
                    put(&mut buf, x0 + 1, y, inner.saturating_sub(2), &self.hint);
                }
            }
            if in_body && split {
                // The gutter (blank in colour, `│` in monochrome), then the
                // detail column.
                let gx = x0 + list_w;
                let detail_x = gx + 1;
                let detail_w = inner.saturating_sub(list_w + 1);
                if st.mono {
                    put(&mut buf, gx, y, 1, &Line::styled("│", st.dim_style()));
                } else {
                    fill(&mut buf, gx, y, 1, st.fill(st.pane));
                    fill(&mut buf, detail_x, y, detail_w, st.fill(st.pane_detail));
                }
                if let Some(line) = self.detail.get(i - body_start) {
                    put(&mut buf, detail_x + 1, y, detail_w.saturating_sub(2), line);
                }
            }
        }

        if bordered {
            self.draw_border(&mut buf, &st, width, height, split.then_some(x0 + list_w));
        }
        buffer_to_lines(&buf)
    }

    /// The monochrome border: `┌─ title ─ /command  facts ──┐`, `│` sides,
    /// `└──┘`, and a `┬` over the split's `│` when the title leaves room.
    fn draw_border(
        &self,
        buf: &mut Buffer,
        st: &PaneStyle,
        width: usize,
        height: usize,
        split_x: Option<usize>,
    ) {
        let edge = st.dim_style();
        let last = (height - 1) as u16;
        let right = (width - 1) as u16;
        let rule = "─".repeat(width - 2);
        buf.set_string(0, 0, format!("┌{rule}┐"), edge);
        buf.set_string(0, last, format!("└{rule}┘"), edge);
        for y in 1..last {
            buf.set_string(0, y, "│", edge);
            buf.set_string(right, y, "│", edge);
        }
        let mut title_end = 1;
        if self.has_title() {
            let content = truncate_line(&self.title_line(st, true), self.title_budget(width));
            let mut spans = vec![Span::styled("─ ", edge)];
            spans.extend(content.spans);
            spans.push(Span::raw(" "));
            let line = Line::from(spans);
            title_end = 1 + line.width();
            buf.set_line(1, 0, &line, (width - 2) as u16);
        }
        if let Some(x) = split_x {
            if x > title_end && x < width - 1 {
                buf.set_string(x as u16, 0, "┬", edge);
            }
        }
    }
}

/// `line` with bold added to every span (the monochrome selected row).
fn emboldened(line: &Line<'static>) -> Line<'static> {
    Line::from(
        line.spans
            .iter()
            .map(|span| {
                Span::styled(
                    span.content.clone(),
                    span.style.add_modifier(Modifier::BOLD),
                )
            })
            .collect::<Vec<_>>(),
    )
}

/// Paints `w` cells of row `y` from `x` with `style`.
fn fill(buf: &mut Buffer, x: usize, y: u16, w: usize, style: Style) {
    if w == 0 {
        return;
    }
    buf.set_style(Rect::new(x as u16, y, w as u16, 1), style);
}

/// Writes `line` at (`x`, `y`), cut with `…` to `w` cells. The spans patch
/// the cells' fill, so unstyled text keeps the pane's colours.
fn put(buf: &mut Buffer, x: usize, y: u16, w: usize, line: &Line<'static>) {
    if w == 0 {
        return;
    }
    let line = truncate_line(line, w);
    buf.set_line(x as u16, y, &line, w as u16);
}

/// The visible window of `rows`: at most `max` rows centred on `selected`
/// (the head or tail when the selection sits at an end). No cap without
/// `max`. The selection is remapped into the window (clamped away when it
/// points past the rows).
pub fn window_rows(
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

/// One hint row from every `(key, action)`: keys in normal text, actions
/// and the ` · ` separators dim.
pub fn hint_line(hints: &[(&str, &str)]) -> Line<'static> {
    hint_spans(hints.iter().copied(), &PaneStyle::current())
}

/// The hint row the spec asks for: at most [`MAX_HINTS`] items, then
/// `f1 more` when there are more (or `more` says the pane has more keys).
pub fn capped_hint_line(hints: &[(&str, &str)], more: bool) -> Line<'static> {
    let more = more || hints.len() > MAX_HINTS;
    let shown = hints.iter().copied().take(MAX_HINTS);
    let items: Vec<(&str, &str)> = if more {
        shown.chain(std::iter::once(("f1", "more"))).collect()
    } else {
        shown.collect()
    };
    hint_spans(items.into_iter(), &PaneStyle::current())
}

fn hint_spans<'a>(
    hints: impl Iterator<Item = (&'a str, &'a str)>,
    st: &PaneStyle,
) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, (key, action)) in hints.enumerate() {
        if i > 0 {
            spans.push(Span::styled(" · ", st.dim_style()));
        }
        spans.push(Span::styled(key.to_string(), st.text_style()));
        if !action.is_empty() {
            spans.push(Span::styled(format!(" {action}"), st.dim_style()));
        }
    }
    Line::from(spans)
}

/// The input row's content: the typed `text` (normal), or the dim
/// `placeholder` while empty; a `▏` caret while the pane has focus.
pub fn input_line(text: &str, placeholder: &str, caret: bool) -> Line<'static> {
    let st = PaneStyle::current();
    let mut spans = Vec::new();
    if text.is_empty() {
        if caret {
            spans.push(Span::styled("▏", st.accent_style()));
        }
        if !placeholder.is_empty() {
            spans.push(Span::styled(placeholder.to_string(), st.dim_style()));
        }
    } else {
        spans.push(Span::styled(text.to_string(), st.text_style()));
        if caret {
            spans.push(Span::styled("▏", st.accent_style()));
        }
    }
    Line::from(spans)
}

/// The blank gutter of an unselected row.
pub fn gutter() -> String {
    " ".repeat(visible_width(PANE_CURSOR))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoocode_tui_util::strip_vt_control_characters;

    fn pane(style: PaneStyle) -> Pane {
        let mut pane = Pane::new("Models");
        pane.command = "/model".to_string();
        pane.facts = Line::raw("showing all");
        pane.rows = vec![Line::raw("mock-large  free"), Line::raw("mock-fast  free")];
        pane.selected = Some(0);
        pane.style = Some(style);
        pane
    }

    #[test]
    fn rows_fit_every_width() {
        for style in [PaneStyle::dark(), PaneStyle::light(), PaneStyle::mono()] {
            let mut pane = pane(style);
            pane.detail = vec![Line::raw("provider  mock"), Line::raw("context  128k")];
            pane.input = Some(Line::raw("filter"));
            pane.hint = Line::raw("esc close");
            for width in [1, 3, 9, 10, 19, 40, 80, 99, 100, 140] {
                let rows = pane.render(width);
                assert!(!rows.is_empty(), "width {width}");
                for row in &rows {
                    assert!(visible_width(row) <= width, "width {width}: {row:?}");
                }
            }
        }
    }

    #[test]
    fn selected_row_carries_the_marker() {
        let rows = pane(PaneStyle::dark()).render(40);
        let plain: Vec<String> = rows
            .iter()
            .map(|r| strip_vt_control_characters(r))
            .collect();
        assert!(
            plain[0].starts_with("  Models  /model  showing all"),
            "{plain:?}"
        );
        assert!(plain[1].starts_with(" › mock-large"), "{plain:?}");
        assert!(plain[2].starts_with("   mock-fast"), "{plain:?}");
    }

    #[test]
    fn mono_title_sits_in_the_border() {
        let rows = pane(PaneStyle::mono()).render(40);
        let plain: Vec<String> = rows
            .iter()
            .map(|r| strip_vt_control_characters(r))
            .collect();
        assert!(
            plain[0].starts_with("┌─ Models ─ /model  showing all ─"),
            "{plain:?}"
        );
        assert!(plain[0].ends_with('┐'));
        assert!(plain[1].starts_with("│ › mock-large"), "{plain:?}");
        assert!(plain.last().unwrap().starts_with('└'));
    }

    #[test]
    fn hints_cap_at_four_plus_more() {
        let line = capped_hint_line(
            &[("a", "1"), ("b", "2"), ("c", "3"), ("d", "4"), ("e", "5")],
            false,
        );
        assert_eq!(flatten(&line), "a 1 · b 2 · c 3 · d 4 · f1 more");
        let line = capped_hint_line(&[("a", "1")], true);
        assert_eq!(flatten(&line), "a 1 · f1 more");
        assert_eq!(flatten(&capped_hint_line(&[("a", "1")], false)), "a 1");
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
        let (all, selected) = window_rows(&rows, Some(99), None);
        assert_eq!(all.len(), 10);
        assert_eq!(selected, None);
    }
}

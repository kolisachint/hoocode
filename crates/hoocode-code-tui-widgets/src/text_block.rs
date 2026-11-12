//! The shared text-block helper for the U22 pane grammar (P6(d), roadmap
//! item 46): the S22 MCP status pane and the S23 text output blocks
//! (`/session`, `/perf`, `/changelog` and the rest) render through one
//! component. The core U22 builder owns `pane.rs` and `console.rs`; this is
//! only the inner content both panes share.
//!
//! S22 and S23 as resolved from the mock (`tui-single-view.html`, dark and
//! light, `--cols:100` with `data-w="40"` narrow variants):
//!
//! - S22 MCP status: ` MCP servers` in bold with an optional dim detail
//!   (the config paths, `~/.agents/mcp.json · ./.agents/mcp.json`), then one
//!   glyph row per server at indent 3: `●` in success, `◉` in accent, `⊗` in
//!   error; the name in ink padded to the longest name plus two; the source
//!   in dim padded to the longest source plus two; then the state. The state
//!   is ink for values (`4 tools`), ink bold when it waits on the user
//!   (`auth needed`), dim for reasons (`timeout after 30s`). An overflow
//!   hint (`/mcp login github`) stays inline behind two spaces while it fits
//!   and wraps to an ink continuation line at indent 5 below it. The source
//!   column (and the heading detail) hides below 60 columns, matching the
//!   pane grammar's "under 60" detail rule, and drops whenever the full
//!   row overflows at any width.
//! - S23 text output blocks: ` Session info` in bold, then one row per
//!   field: the label in dim padded to the longest label plus two, the value
//!   in ink, no blank rows inside.
//!
//! One deliberate deviation: the wide S23 frame pads labels to the longest
//! label plus four while the narrow frame uses plus two. The helper pins
//! plus two at every width, matching the narrow frame.
//!
//! The helper never exceeds its width (overlong lines truncate with `…`)
//! and never emits a blank row. Which rows and facts a narrow pane shows
//! (the S23 narrow frame drops the `File` row and trailing facts) stays with
//! the call sites; a future adoption wires `mode/mcp.rs` and
//! `resource_display.rs` onto this helper.

use hoocode_code_tui_theme::theme;
use hoocode_tui_render::Component;
use hoocode_tui_util::{truncate_to_width, visible_width, wrap_text_with_ansi};

use crate::fold::ELLIPSIS;

/// Below this width the detail hides: the heading detail and the status
/// rows' source column (the S22 narrow frame shows neither, and the pane
/// grammar hides detail under 60). At any width a row that still overflows
/// drops the source column first, then truncates with `…`.
const DETAIL_MIN_WIDTH: usize = 60;

/// The state of an [`StatusRow`], as its glyph colours it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusTone {
    /// Done: `●` in success.
    Ready,
    /// Waiting on the user: `◉` in accent.
    Working,
    /// Failed: `⊗` in error.
    Failed,
}

impl StatusTone {
    fn glyph(self) -> &'static str {
        match self {
            StatusTone::Ready => "●",
            StatusTone::Working => "◉",
            StatusTone::Failed => "⊗",
        }
    }

    fn token(self) -> &'static str {
        match self {
            StatusTone::Ready => "success",
            StatusTone::Working => "accent",
            StatusTone::Failed => "error",
        }
    }
}

/// How an [`StatusRow`]'s state words draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailTone {
    /// A value: ink.
    Ink,
    /// A reason: dim.
    Dim,
    /// A status word that needs the user's eye: ink bold. Yellow is for
    /// blocking dialogs only (`color-semantics.json`).
    Bold,
}

impl DetailTone {
    fn token(self) -> &'static str {
        match self {
            DetailTone::Ink | DetailTone::Bold => "ink",
            DetailTone::Dim => "dim",
        }
    }
}

/// One S22 glyph row: the name, its dim source column, the state, and an
/// optional ink hint (a follow-up command, e.g. `/mcp login github`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusRow {
    pub tone: StatusTone,
    pub name: String,
    pub source: String,
    pub state: String,
    pub state_tone: DetailTone,
    pub hint: Option<String>,
}

impl StatusRow {
    /// The mock's default tones: a ready row states ink values, a working
    /// row states its wait in ink bold, a failed row dims its reason. Override with
    /// [`StatusRow::with_state_tone`] when a row differs.
    pub fn new(tone: StatusTone, name: &str, source: &str, state: &str) -> Self {
        let state_tone = match tone {
            StatusTone::Ready => DetailTone::Ink,
            StatusTone::Working => DetailTone::Bold,
            StatusTone::Failed => DetailTone::Dim,
        };
        Self {
            tone,
            name: name.to_string(),
            source: source.to_string(),
            state: state.to_string(),
            state_tone,
            hint: None,
        }
    }

    pub fn with_state_tone(mut self, tone: DetailTone) -> Self {
        self.state_tone = tone;
        self
    }

    pub fn with_hint(mut self, hint: &str) -> Self {
        self.hint = Some(hint.to_string());
        self
    }
}

/// One S22/S23 text block: a bold heading with an optional dim detail, dim
/// labels with ink values, and glyph status rows. No blank rows inside.
#[derive(Debug, Clone, Default)]
pub struct TextBlockComponent {
    heading: String,
    heading_detail: String,
    fields: Vec<(String, String)>,
    rows: Vec<StatusRow>,
}

impl TextBlockComponent {
    pub fn new(heading: &str) -> Self {
        Self {
            heading: heading.to_string(),
            heading_detail: String::new(),
            fields: Vec::new(),
            rows: Vec::new(),
        }
    }

    /// The dim detail behind the heading (e.g. the S22 config paths).
    /// Hidden when it does not fit.
    pub fn set_heading_detail(&mut self, detail: &str) {
        self.heading_detail = detail.to_string();
    }

    /// One S23 row: a dim `label`, an ink `value`.
    pub fn add_field(&mut self, label: &str, value: &str) {
        self.fields.push((label.to_string(), value.to_string()));
    }

    /// One S22 glyph row.
    pub fn add_status(&mut self, row: StatusRow) {
        self.rows.push(row);
    }

    fn fit(&self, line: &str, width: usize) -> String {
        if visible_width(line) <= width {
            line.to_string()
        } else {
            truncate_to_width(line, width, ELLIPSIS, false)
        }
    }

    fn heading_line(&self, width: usize) -> Option<String> {
        if self.heading.is_empty() {
            return None;
        }
        let t = theme();
        let head = format!(" {}", t.fg("ink", &t.bold(&self.heading)));
        if self.heading_detail.is_empty() {
            return Some(self.fit(&head, width));
        }
        if width >= DETAIL_MIN_WIDTH {
            let full = format!("{head}  {}", t.fg("dim", &self.heading_detail));
            if visible_width(&full) <= width {
                return Some(full);
            }
        }
        // The detail drops instead of truncating (the S22 narrow frame
        // shows the heading alone).
        Some(self.fit(&head, width))
    }

    fn field_lines(&self, width: usize) -> Vec<String> {
        let t = theme();
        let label_width = self
            .fields
            .iter()
            .map(|(label, _)| visible_width(label))
            .max()
            .unwrap_or(0);
        self.fields
            .iter()
            .filter(|(label, value)| !(label.is_empty() && value.is_empty()))
            .map(|(label, value)| {
                let cell = format!(
                    "{label}{}",
                    " ".repeat(label_width + 2 - visible_width(label))
                );
                let line = if value.is_empty() {
                    format!(" {}", t.fg("dim", &cell))
                } else {
                    format!(" {}  {}", t.fg("dim", &cell), t.fg("ink", value))
                };
                self.fit(&line, width)
            })
            .collect()
    }

    fn status_lines(&self, width: usize) -> Vec<String> {
        let t = theme();
        let name_width = self
            .rows
            .iter()
            .map(|row| visible_width(&row.name))
            .max()
            .unwrap_or(0);
        let source_width = self
            .rows
            .iter()
            .map(|row| visible_width(&row.source))
            .max()
            .unwrap_or(0);
        let mut out = Vec::new();
        for row in &self.rows {
            let glyph = t.fg(row.tone.token(), row.tone.glyph());
            let name = format!(
                "{}{}",
                row.name,
                " ".repeat(name_width + 2 - visible_width(&row.name))
            );
            let state = match row.state_tone {
                DetailTone::Bold => t.fg("ink", &t.bold(&row.state)),
                tone => t.fg(tone.token(), &row.state),
            };
            let head = format!("   {glyph} {}", t.fg("ink", &name));
            let full = if row.source.is_empty() || width < DETAIL_MIN_WIDTH {
                format!("{head}{state}")
            } else {
                let source = format!(
                    "{}{}",
                    row.source,
                    " ".repeat(source_width + 2 - visible_width(&row.source))
                );
                format!("{head} {} {state}", t.fg("dim", &source))
            };
            // The hint stays inline behind two spaces while it fits,
            // else it wraps to ink continuation lines at indent 5.
            let inline_hint = |head: &str| {
                let hint = row.hint.as_deref().filter(|h| !h.is_empty())?;
                Some(format!("{head}  {}", theme().fg("ink", hint)))
            };
            if visible_width(&full) <= width {
                match inline_hint(&full) {
                    Some(inline) if visible_width(&inline) <= width => out.push(inline),
                    _ => {
                        out.push(full);
                        out.extend(self.hint_lines(row, width));
                    }
                }
                continue;
            }
            // The source column drops with its separator (the S22 narrow
            // frame keeps `◉ github  auth needed`): the padded name
            // already leaves at least two columns before the state.
            let compact = format!("{head}{state}");
            match inline_hint(&compact) {
                Some(inline) if visible_width(&inline) <= width => out.push(inline),
                _ => {
                    out.push(self.fit(&compact, width));
                    out.extend(self.hint_lines(row, width));
                }
            }
        }
        out
    }

    fn hint_lines(&self, row: &StatusRow, width: usize) -> Vec<String> {
        let Some(hint) = row.hint.as_deref().filter(|h| !h.is_empty()) else {
            return Vec::new();
        };
        let t = theme();
        let inner = width.saturating_sub(5).max(1);
        wrap_text_with_ansi(hint, inner)
            .into_iter()
            .map(|line| self.fit(&format!("     {}", t.fg("ink", &line)), width))
            .collect()
    }
}

impl Component for TextBlockComponent {
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn render(&mut self, width: u16) -> Vec<String> {
        let width = usize::from(width).max(1);
        let mut lines = Vec::new();
        if let Some(head) = self.heading_line(width) {
            lines.push(head);
        }
        lines.extend(self.field_lines(width));
        lines.extend(self.status_lines(width));
        lines.into_iter().filter(|l| !l.trim().is_empty()).collect()
    }

    fn invalidate(&mut self) {}
}

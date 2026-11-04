//! The tool row of the declutter transcript (spec "Transcript"):
//! ` ● Name subject  facts`, then an indented dim body.
//!
//! One glyph, the tool name in bold, the subject in plain text, then two
//! spaces and the facts in dim, directly after the subject (never
//! right-aligned). No step number, no rail. Rows are drawn from spans through
//! a ratatui buffer, like every other row of the widgets.

use hoocode_tui_glyphs::status;
use hoocode_tui_ratatui::core::text::Span;
use hoocode_tui_ratatui::{ansi_spans, bold, row, spans_width, Palette};
use hoocode_tui_util::{strip_vt_control_characters, wrap_text_with_ansi};

/// Columns a body row sits in: past the glyph, under the tool name's column.
pub const BODY_INDENT: usize = 4;

/// Columns the calls of a subagent sit in under its row.
pub const SUB_INDENT: usize = 3;

/// What a tool row's glyph says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// Done: green `●`.
    Done,
    /// Still running: dim `●`.
    Running,
    /// Failed: red `⊗`.
    Failed,
    /// Stopped or denied: dim `⊗`.
    Stopped,
}

impl Mark {
    /// The glyph, styled for the palette.
    pub fn span(self, palette: &Palette) -> Span<'static> {
        match self {
            Mark::Done => Span::styled(status::DONE_STR.to_string(), palette.ok()),
            Mark::Running => Span::styled(status::DONE_STR.to_string(), palette.dim()),
            Mark::Failed => Span::styled(status::FAILED_STR.to_string(), palette.err()),
            Mark::Stopped => Span::styled(status::FAILED_STR.to_string(), palette.dim()),
        }
    }
}

/// A fact in dim: `30 lines`.
pub fn dim_fact(text: &str, palette: &Palette) -> Span<'static> {
    Span::styled(text.to_string(), palette.dim())
}

/// Facts joined by a dim ` · `. Empty facts are dropped, so a zero value
/// never shows.
pub fn join_facts(facts: Vec<Vec<Span<'static>>>, palette: &Palette) -> Vec<Span<'static>> {
    let mut out: Vec<Span<'static>> = Vec::new();
    for fact in facts.into_iter().filter(|f| !f.is_empty()) {
        if !out.is_empty() {
            out.push(Span::styled(" · ".to_string(), palette.dim()));
        }
        out.extend(fact);
    }
    out
}

/// `+2 −2`: added in green, removed in red; a zero count is left out.
pub fn diff_facts(added: usize, removed: usize, palette: &Palette) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    if added > 0 {
        out.push(Span::styled(format!("+{added}"), palette.ok()));
    }
    if removed > 0 {
        if !out.is_empty() {
            out.push(Span::raw(" ".to_string()));
        }
        out.push(Span::styled(format!("−{removed}"), palette.err()));
    }
    out
}

/// `N lines` / `1 line`; empty for zero.
pub fn lines_fact(total: usize) -> String {
    match total {
        0 => String::new(),
        1 => "1 line".to_string(),
        n => format!("{n} lines"),
    }
}

/// A short duration: `4s`, `1m12s`.
pub fn short_dur(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else {
        format!("{}m{}s", secs / 60, secs % 60)
    }
}

/// A tool row as rows: ` ● label  facts`. The label (bold name, subject)
/// arrives styled and wraps under itself rather than being cut; the facts
/// follow the last label row after two spaces, or take a row of their own
/// when they do not fit beside it. `indent` is the nesting (0 for a call,
/// [`SUB_INDENT`] for a call inside a subagent).
pub fn tool_rows(
    indent: usize,
    mark: Mark,
    label: &str,
    facts: Vec<Span<'static>>,
    width: usize,
    palette: &Palette,
) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let lead = if indent == 0 { 1 } else { indent };
    let avail = width.saturating_sub(lead + 2).max(1);
    let mut pieces = wrap_text_with_ansi(label, avail);
    if pieces.is_empty() {
        pieces.push(String::new());
    }
    let facts_width = spans_width(&facts);
    let count = pieces.len();
    let hang = " ".repeat(lead + 2);
    let mut rows = Vec::with_capacity(count + 1);
    for (index, piece) in pieces.into_iter().enumerate() {
        let mut spans = if index == 0 {
            vec![
                Span::raw(" ".repeat(lead)),
                mark.span(palette),
                Span::raw(" ".to_string()),
            ]
        } else {
            vec![Span::raw(hang.clone())]
        };
        spans.extend(ansi_spans(&piece));
        let last = index + 1 == count;
        if last && facts_width > 0 && spans_width(&spans) + 2 + facts_width <= width {
            spans.push(Span::raw("  ".to_string()));
            spans.extend(facts.clone());
            rows.push(row(spans, width));
        } else {
            rows.push(row(spans, width));
            if last && facts_width > 0 {
                let mut own = vec![Span::raw(hang.clone())];
                own.extend(facts.clone());
                rows.push(row(own, width));
            }
        }
    }
    rows
}

/// The label of a call line: the tool name in bold, one space, the subject
/// in plain text. The `name` is the first word of `plain`'s first line.
pub fn label_spans(plain: &str, palette: &Palette) -> Vec<Span<'static>> {
    let plain = plain.trim();
    let (name, rest) = match plain.split_once(char::is_whitespace) {
        Some((name, rest)) => (name, rest.trim_start()),
        None => (plain, ""),
    };
    let mut spans = vec![Span::styled(name.to_string(), bold(palette.ink()))];
    if !rest.is_empty() {
        spans.push(Span::styled(format!(" {rest}"), palette.ink()));
    }
    spans
}

/// A dim body row from a plain line, at the body indent.
pub fn dim_body_row(text: &str, width: usize, palette: &Palette) -> String {
    row(
        vec![
            Span::raw(" ".repeat(BODY_INDENT)),
            Span::styled(text.to_string(), palette.dim()),
        ],
        width,
    )
}

/// The hint row of a fold: `    … 27 more lines`, dim.
pub fn fold_row(text: &str, width: usize, palette: &Palette) -> String {
    dim_body_row(text, width, palette)
}

/// Whether a rendered row is blank once its styling is gone.
pub fn is_blank_row(line: &str) -> bool {
    strip_vt_control_characters(line).trim().is_empty()
}

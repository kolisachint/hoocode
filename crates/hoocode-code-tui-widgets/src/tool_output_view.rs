//! `core/tool-output-view.ts`: the radar / peek / full dial.
//!
//! Radar folds each run of tool calls to one line. Peek shows each call's
//! line and the first [`PEEK_LINES`] lines of what it returned. Full draws
//! what peek draws with nothing cut: no line budgets, no `… N more lines`
//! folds, no width ellipsis on content (long rows wrap), whole thinking,
//! whole tool output. Ctrl+O steps radar → peek → full → radar.

pub use hoocode_code_settings::ToolOutputView;

use crate::fold::more_lines_hint;

/// The dial's order, least to most. Cycling wraps at both ends.
pub const TOOL_OUTPUT_VIEWS: [ToolOutputView; 3] = [
    ToolOutputView::Radar,
    ToolOutputView::Peek,
    ToolOutputView::Full,
];

/// How many lines of a result the `peek` stop shows.
pub const PEEK_LINES: usize = 5;

/// Where a fresh install starts.
pub const DEFAULT_TOOL_OUTPUT_VIEW: ToolOutputView = ToolOutputView::Peek;

/// Whether a stop draws everything uncut: the `expanded` flag renderers get.
pub fn is_full(view: ToolOutputView) -> bool {
    view == ToolOutputView::Full
}

/// The lines of a result a renderer shows: [`PEEK_LINES`], or all of them
/// when `expanded` (the full stop).
pub fn line_budget(expanded: bool) -> usize {
    if expanded {
        usize::MAX
    } else {
        PEEK_LINES
    }
}

/// One-line description per view, shared by the settings pane and `/hotkeys`.
pub fn tool_output_view_description(view: ToolOutputView) -> &'static str {
    match view {
        ToolOutputView::Radar => {
            "one line per run of tool calls; failures still show why they failed"
        }
        ToolOutputView::Peek => "the call line and the first few lines of the result",
        ToolOutputView::Full => "peek with nothing cut: whole prompts, thinking and tool output",
    }
}

/// The body a tool result shows at the peek stop, shared by every renderer
/// that trims its own output: the first [`line_budget`] lines, styled by
/// `style`, then the dim `… N more lines` hint when lines were left out.
/// Callers put any leading newline themselves, and skip the call for an
/// empty output.
pub fn peek_block(
    expanded: bool,
    lines: &[String],
    style: impl FnOnce(&[String]) -> Vec<String>,
) -> String {
    let shown = lines.len().min(line_budget(expanded));
    peek_window(lines.len(), &lines[..shown], style)
}

/// [`peek_block`] for a caller that has built only the lines it shows: `shown`
/// is the first lines (at most the budget) of `total` lines, styled by
/// `style`.
pub fn peek_window(
    total: usize,
    shown: &[String],
    style: impl FnOnce(&[String]) -> Vec<String>,
) -> String {
    let body = style(shown).join("\n");
    if total <= shown.len() {
        return body;
    }
    format!("{body}\n{}", more_lines_hint(total - shown.len()))
}

/// `LEGACY_TOOL_OUTPUT_VIEWS`: values written by older versions. `standard`
/// loaded when it was a stop of its own; it loads as peek.
pub const LEGACY_TOOL_OUTPUT_VIEWS: [(&str, ToolOutputView); 3] = [
    ("collapsed", ToolOutputView::Radar),
    ("glance", ToolOutputView::Peek),
    ("standard", ToolOutputView::Peek),
];

/// `isToolOutputView`.
pub fn is_tool_output_view(value: Option<&str>) -> bool {
    value.is_some_and(|v| ToolOutputView::parse(v).is_some())
}

/// `cycleToolOutputView`: the next stop, wrapping at both ends. Ctrl+O steps
/// forward: radar → peek → full → radar.
pub fn cycle_tool_output_view(current: ToolOutputView, forward: bool) -> ToolOutputView {
    let index = TOOL_OUTPUT_VIEWS
        .iter()
        .position(|v| *v == current)
        .unwrap_or(0);
    let step = if forward {
        1
    } else {
        TOOL_OUTPUT_VIEWS.len() - 1
    };
    TOOL_OUTPUT_VIEWS[(index + step) % TOOL_OUTPUT_VIEWS.len()]
}

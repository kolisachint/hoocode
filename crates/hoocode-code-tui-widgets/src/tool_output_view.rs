//! `core/tool-output-view.ts`: the radar / peek dial.
//!
//! Radar folds each run of tool calls to one line. Peek shows each call's
//! line and the first [`PEEK_LINES`] lines of what it returned. Ctrl+O
//! toggles between the two. The old third stop (full) is gone: whatever it
//! showed, peek shows a trimmed form of.

use hoocode_code_tui_theme::theme;

pub use hoocode_code_settings::ToolOutputView;

/// The dial's order, least to most. Cycling wraps at both ends.
pub const TOOL_OUTPUT_VIEWS: [ToolOutputView; 2] = [ToolOutputView::Radar, ToolOutputView::Peek];

/// How many lines of a result the `peek` stop shows.
pub const PEEK_LINES: usize = 5;

/// Where a fresh install starts.
pub const DEFAULT_TOOL_OUTPUT_VIEW: ToolOutputView = ToolOutputView::Peek;

/// One-line description per view, shared by the settings pane and `/hotkeys`.
pub fn tool_output_view_description(view: ToolOutputView) -> &'static str {
    match view {
        ToolOutputView::Radar => {
            "one line per run of tool calls; failures still show why they failed"
        }
        ToolOutputView::Peek => "the call line and the first few lines of the result",
    }
}

/// The body a tool result shows at the peek stop, shared by every renderer
/// that trims its own output: the first [`PEEK_LINES`] lines, styled by
/// `style`, then the muted `... (N more lines)` hint when lines were left
/// out. Callers put any leading newline themselves, and skip the call for an
/// empty output.
pub fn peek_block(lines: &[String], style: impl FnOnce(&[String]) -> Vec<String>) -> String {
    let shown = lines.len().min(PEEK_LINES);
    let body = style(&lines[..shown]).join("\n");
    if lines.len() <= shown {
        return body;
    }
    let remaining = lines.len() - shown;
    format!(
        "{body}{}",
        theme().fg("muted", &format!("\n... ({remaining} more lines)"))
    )
}

/// `LEGACY_TOOL_OUTPUT_VIEWS`: values written by older versions. `standard`
/// and `full` loaded before the Full stop went; they load as peek.
pub const LEGACY_TOOL_OUTPUT_VIEWS: [(&str, ToolOutputView); 4] = [
    ("collapsed", ToolOutputView::Radar),
    ("glance", ToolOutputView::Peek),
    ("standard", ToolOutputView::Peek),
    ("full", ToolOutputView::Peek),
];

/// `isToolOutputView`.
pub fn is_tool_output_view(value: Option<&str>) -> bool {
    value.is_some_and(|v| ToolOutputView::parse(v).is_some())
}

/// `cycleToolOutputView`: the next stop, wrapping at both ends. With two
/// stops a step either way is the Ctrl+O toggle.
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

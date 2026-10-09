//! `core/tool-output-view.ts`: the radar / peek / full dial.

use hoocode_code_tui_keybindings::key_hint;
use hoocode_code_tui_theme::theme;

pub use hoocode_code_settings::ToolOutputView;

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

/// The stop `app.tools.expand` jumps to, and returns from.
pub const MAX_TOOL_OUTPUT_VIEW: ToolOutputView = ToolOutputView::Full;

/// One-line description per view, shared by the settings pane and `/hotkeys`.
pub fn tool_output_view_description(view: ToolOutputView) -> &'static str {
    match view {
        ToolOutputView::Radar => {
            "one line per run of tool calls; failures still show why they failed"
        }
        ToolOutputView::Peek => "the call line and the first few lines of the result",
        ToolOutputView::Full => "the call line and the whole result",
    }
}

/// The body a tool result shows at the peek stop, shared by every renderer
/// that trims its own output: the first `PEEK_LINES` lines (all of them when
/// `expanded`), styled by `style`, then the muted `... (N more lines, to
/// expand)` hint when lines were left out. Callers put any leading newline
/// themselves, and skip the call for an empty output.
pub fn peek_block(
    lines: &[String],
    expanded: bool,
    style: impl FnOnce(&[String]) -> Vec<String>,
) -> String {
    let max = if expanded { lines.len() } else { PEEK_LINES };
    let shown = lines.len().min(max);
    let body = style(&lines[..shown]).join("\n");
    if lines.len() <= shown {
        return body;
    }
    let remaining = lines.len() - shown;
    let t = theme();
    format!(
        "{body}{} {})",
        t.fg("muted", &format!("\n... ({remaining} more lines,")),
        key_hint("app.tools.expand", "to expand")
    )
}

/// `isToolOutputView`.
pub fn is_tool_output_view(value: Option<&str>) -> bool {
    value.is_some_and(|v| ToolOutputView::parse(v).is_some())
}

/// `cycleToolOutputView`: the next stop, wrapping at both ends.
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

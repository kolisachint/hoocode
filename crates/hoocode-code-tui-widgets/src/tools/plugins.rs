//! `core/tools/plugins.ts` renderers: the plugin tools' results (count line,
//! then rows, peek-trimmed).

use std::rc::Rc;

use hoocode_code_tui_theme::theme;
use hoocode_tui_components::markdown::js_trim;

use super::text;
use crate::render_utils::get_text_output;
use crate::tool_execution::{ToolRenderDefinition, ToolRenderResultOptions, ToolResultView};
use crate::tool_output_view::peek_block;

/// The plugin tools that render their own result.
pub const RENDERED_PLUGIN_TOOLS: [&str; 4] = [
    "SearchPlugins",
    "ListPlugins",
    "SuggestPluginInstall",
    "InstallPlugin",
];

pub fn format_plugin_tool_result(
    result: &ToolResultView<'_>,
    options: ToolRenderResultOptions,
) -> String {
    let t = theme();
    let output = get_text_output(Some(result.content), false);
    let output = js_trim(&output);
    if output.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = output.split('\n').map(str::to_string).collect();
    // The first row (the count line) is muted, the rows under it are body text.
    peek_block(&lines, options.expanded, |shown| {
        shown
            .iter()
            .enumerate()
            .map(|(i, line)| {
                if i == 0 {
                    t.fg("muted", line)
                } else {
                    t.fg("toolOutput", line)
                }
            })
            .collect()
    })
}

pub fn definition() -> ToolRenderDefinition {
    ToolRenderDefinition {
        render_call: None,
        render_result: Some(Rc::new(|result, options, _| {
            Ok(text(format_plugin_tool_result(result, options)))
        })),
        render_shell: None,
    }
}

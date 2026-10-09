//! The built-in tools' renderers (`renderCall` / `renderResult` /
//! `renderShell` of `core/tools/*.ts`), keyed by tool name.

use std::cell::RefCell;
use std::rc::Rc;

use hoocode_tui_components::Text;
use hoocode_tui_render::ComponentHandle;

use crate::tool_execution::ToolRenderDefinition;

pub mod bash;
pub mod edit;
pub mod plugins;
pub mod read;
pub mod search;
pub mod subagent;
pub mod web;
pub mod write;

/// `createAllToolDefinitions(cwd)[name]`, rendering half. Tools whose
/// renderers are not ported yet still count as built-in (the block uses its
/// fallbacks), which keeps the shell choice and slot inheritance right.
pub fn builtin_tool_definition(name: &str) -> Option<ToolRenderDefinition> {
    match name {
        "Read" => Some(read::definition()),
        "Write" => Some(write::definition()),
        "CodeSearch" => Some(search::definition()),
        "WebFetch" => Some(web::webfetch_definition()),
        "WebSearch" => Some(web::websearch_definition()),
        "Shell" => Some(bash::definition()),
        "Edit" => Some(edit::definition()),
        _ => None,
    }
}

/// The rendering half of a registered (non built-in) tool's definition: the
/// subagent and plugin tools bring renderers; any other registered tool has
/// none, which still makes the block draw its dot and fallbacks.
pub fn registered_tool_definition(name: &str) -> ToolRenderDefinition {
    match name {
        "Agent" => subagent::task_definition(),
        "AgentOutput" => subagent::task_output_definition(),
        n if plugins::RENDERED_PLUGIN_TOOLS.contains(&n) => plugins::definition(),
        _ => ToolRenderDefinition::default(),
    }
}

/// A `Text(text, 0, 0)` component.
pub(crate) fn text(content: String) -> ComponentHandle {
    Rc::new(RefCell::new(Text::new(content, 0, 0)))
}

/// `trimTrailingEmptyLines`.
pub(crate) fn trim_trailing_empty_lines(mut lines: Vec<String>) -> Vec<String> {
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines
}

//! The built-in tools' renderers (`renderCall` / `renderResult` /
//! `renderShell` of `core/tools/*.ts`), keyed by tool name.

use std::cell::RefCell;
use std::rc::Rc;

use hoocode_agent_types::ToolOrigin;
use hoocode_tui_components::Text;
use hoocode_tui_render::ComponentHandle;

use crate::tool_execution::ToolRenderDefinition;

pub mod bash;
pub mod edit;
pub mod plugins;
pub mod read;
pub mod search;
pub mod subagent;
pub mod todo;
pub mod web;
pub mod write;

/// The renderer id of the web fetch row (`plugins.md`, Naming and identity).
pub const RENDER_WEB_FETCH: &str = "web-fetch";
/// The renderer id of the web search row.
pub const RENDER_WEB_SEARCH: &str = "web-search";

/// `createAllToolDefinitions(cwd)[name]`, rendering half, for the built-in tools. Tools whose
/// renderers are not ported yet still count as built-in (the block uses its
/// fallbacks), which keeps the shell choice and slot inheritance right. Plugin and MCP tools
/// never reach this table: they draw by their render hint ([`renderer_for`]).
pub fn builtin_tool_definition(name: &str) -> Option<ToolRenderDefinition> {
    match name {
        "Read" => Some(read::definition()),
        "Write" => Some(write::definition()),
        "CodeSearch" => Some(search::definition()),
        "Shell" => Some(bash::definition()),
        "Edit" => Some(edit::definition()),
        _ => None,
    }
}

/// The renderer a plugin's render hint names. An absent or unknown id is `None`: the generic row.
pub fn hinted_definition(render: &str) -> Option<ToolRenderDefinition> {
    match render {
        RENDER_WEB_FETCH => Some(web::webfetch_definition()),
        RENDER_WEB_SEARCH => Some(web::websearch_definition()),
        _ => None,
    }
}

/// The renderer of a call. A plugin or MCP tool draws by its render hint and never by its
/// name. A built-in (or a call whose origin is not known) draws by its built-in name.
pub fn renderer_for(
    name: &str,
    origin: Option<&ToolOrigin>,
    render: Option<&str>,
) -> Option<ToolRenderDefinition> {
    match origin {
        Some(ToolOrigin::Plugin { .. } | ToolOrigin::Mcp { .. }) => {
            render.and_then(hinted_definition)
        }
        _ => builtin_tool_definition(name),
    }
}

/// The rendering half of a registered (non built-in) tool's definition: the
/// subagent and plugin tools bring renderers; any other registered tool has
/// none, which still makes the block draw its dot and fallbacks.
pub fn registered_tool_definition(name: &str) -> ToolRenderDefinition {
    match name {
        "Agent" => subagent::task_definition(),
        "AgentOutput" => subagent::task_output_definition(),
        "TodoWrite" => todo::definition(),
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

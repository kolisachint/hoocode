//! What a session needs from the installed plugins to keep their tools live (`PL-4`,
//! `docs/design/plugins.md`).
//!
//! The session does not load plugins itself. The CLI implements [`PluginSource`]: it knows the
//! plugin set, the settings and the permission gate. The session asks the source at the points
//! it syncs its MCP tools (before a prompt, between the turns of a run, and on `/reload`). A
//! source whose [`generation`](PluginSource::generation) moved gets its tools and MCP hub built
//! again, so a plugin installed, removed or trusted by any path shows up on the next turn.

use std::sync::Arc;

use hoocode_code_tool_api::ToolDefinition;

use crate::mcp::McpHub;

pub trait PluginSource: Send + Sync {
    /// Changes whenever the plugins may have changed. Read before [`tools`](Self::tools) and
    /// [`mcp_hub`](Self::mcp_hub), so a change made during a build is built again next time.
    fn generation(&self) -> u64;

    /// Whether a new plugin tool starts active. False in a session whose mode restricts its
    /// active tools.
    fn auto_activate(&self) -> bool {
        true
    }

    /// The plugin tools of the current plugins. `taken` are the names of the built-in and the
    /// session's own tools: a plugin tool never takes one of them. The second value is the
    /// problems found, one line each.
    fn tools(&self, taken: &[String]) -> (Vec<ToolDefinition>, Vec<String>);

    /// A new, started MCP hub for the current plugins' servers. `None` when this session does
    /// not run MCP. The session shuts the old hub down and uses the new one.
    fn mcp_hub(&self) -> Option<Arc<McpHub>>;
}

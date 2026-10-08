//! Server configuration: how to reach one MCP server.

use std::collections::BTreeMap;
use std::path::PathBuf;

/// How to start or reach one MCP server. Legacy HTTP+SSE is not supported
/// (`docs/design/mcp.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpServerConfig {
    /// A child process that speaks MCP on its stdin and stdout. Its stderr is
    /// discarded so it cannot write into the terminal.
    Stdio {
        command: String,
        args: Vec<String>,
        env: BTreeMap<String, String>,
        cwd: Option<PathBuf>,
    },
    /// A Streamable HTTP endpoint. `headers` are sent with every request, for
    /// example `Authorization`.
    Http {
        url: String,
        headers: BTreeMap<String, String>,
    },
}

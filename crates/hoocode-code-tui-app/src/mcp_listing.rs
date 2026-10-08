//! The `/mcp` listing: each MCP server with its source, state and tool count.

use hoocode_code_agent_session::mcp::{McpServerInfo, McpStatus};

/// The text `/mcp` shows. An empty list says where servers are configured.
pub fn format_listing(servers: &[McpServerInfo]) -> String {
    if servers.is_empty() {
        return "No MCP servers. Add them to ~/.agents/mcp.json or .agents/mcp.json in the project."
            .to_owned();
    }
    let name_width = servers
        .iter()
        .map(|s| s.name.chars().count())
        .max()
        .unwrap_or(0);
    let source_width = servers
        .iter()
        .map(|s| s.source.chars().count())
        .max()
        .unwrap_or(0);
    let mut lines = vec!["MCP servers".to_owned()];
    for server in servers {
        lines.push(format!(
            "  {:<name_width$}  {:<source_width$}  {}",
            server.name,
            server.source,
            describe(server),
        ));
    }
    lines.join("\n")
}

/// The state, and for a connected server its tool count.
fn describe(server: &McpServerInfo) -> String {
    match &server.status {
        McpStatus::Connected => {
            let noun = if server.tool_count == 1 {
                "tool"
            } else {
                "tools"
            };
            format!("connected, {} {noun}", server.tool_count)
        }
        McpStatus::Connecting => "connecting".to_owned(),
        McpStatus::NotTrusted => "not trusted".to_owned(),
        McpStatus::AuthNeeded => format!("auth needed: run /mcp login {}", server.name),
        McpStatus::Failed(reason) => format!("failed: {reason}"),
        McpStatus::Disabled => "disabled".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(name: &str, source: &str, status: McpStatus, tool_count: usize) -> McpServerInfo {
        McpServerInfo {
            name: name.to_owned(),
            source: source.to_owned(),
            status,
            tool_count,
        }
    }

    #[test]
    fn empty_list_says_where_to_configure_servers() {
        assert!(format_listing(&[]).starts_with("No MCP servers."));
    }

    #[test]
    fn lists_each_server_with_source_state_and_tools() {
        let text = format_listing(&[
            server("mine", "user", McpStatus::Connected, 2),
            server("repo-tool", "project", McpStatus::NotTrusted, 0),
            server("remote", "project", McpStatus::AuthNeeded, 0),
            server("broken", "plugin p", McpStatus::Failed("exit 1".into()), 0),
            server("off", "project", McpStatus::Disabled, 0),
            server("one", "user", McpStatus::Connected, 1),
        ]);
        assert_eq!(
            text,
            [
                "MCP servers",
                "  mine       user      connected, 2 tools",
                "  repo-tool  project   not trusted",
                "  remote     project   auth needed: run /mcp login remote",
                "  broken     plugin p  failed: exit 1",
                "  off        project   disabled",
                "  one        user      connected, 1 tool",
            ]
            .join("\n")
        );
    }
}

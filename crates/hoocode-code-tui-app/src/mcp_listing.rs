//! The `/mcp` listing: each MCP server with its source, state and tool
//! count, as a transcript text block (spec §4 S22): glyph rows where only
//! the glyph carries colour, `auth needed` in warning yellow because it
//! waits on you, and the login command on its own row.

use hoocode_code_agent_session::mcp::{McpServerInfo, McpStatus};
use hoocode_code_tui_widgets::text_block::{cell, StatusGlyph, TextBlock};

/// The config files file-based servers come from, in heading order.
const USER_CONFIG: &str = "~/.agents/mcp.json";
const PROJECT_CONFIG: &str = "./.agents/mcp.json";

/// The text `/mcp` shows. An empty list says where servers are configured.
pub fn format_listing(servers: &[McpServerInfo]) -> String {
    if servers.is_empty() {
        return "No MCP servers. Add them to ~/.agents/mcp.json or .agents/mcp.json in the project."
            .to_owned();
    }
    let mut facts = Vec::new();
    if servers.iter().any(|s| s.source == "user") {
        facts.push(USER_CONFIG);
    }
    if servers.iter().any(|s| s.source == "project") {
        facts.push(PROJECT_CONFIG);
    }
    let mut block = TextBlock::new("MCP servers", &facts.join(" · "));
    for server in servers {
        match &server.status {
            McpStatus::Connected => {
                let noun = if server.tool_count == 1 {
                    "tool"
                } else {
                    "tools"
                };
                block.status(
                    StatusGlyph::Done,
                    &server.name,
                    vec![
                        cell(&server.source, "dim"),
                        cell(&format!("{} {noun}", server.tool_count), "dim"),
                    ],
                );
            }
            McpStatus::Connecting => {
                block.status(
                    StatusGlyph::Working,
                    &server.name,
                    vec![cell(&server.source, "dim"), cell("connecting", "dim")],
                );
            }
            McpStatus::NotTrusted => {
                block.status(
                    StatusGlyph::Failed,
                    &server.name,
                    vec![cell(&server.source, "dim"), cell("not trusted", "dim")],
                );
            }
            McpStatus::AuthNeeded => {
                block.status(
                    StatusGlyph::Working,
                    &server.name,
                    vec![cell(&server.source, "dim"), cell("auth needed", "warning")],
                );
                block.raw(format!("   /mcp login {}", server.name));
            }
            McpStatus::Failed(reason) => {
                block.status(
                    StatusGlyph::Failed,
                    &server.name,
                    vec![cell(&server.source, "dim"), cell(reason, "dim")],
                );
            }
            McpStatus::Disabled => {
                block.status(
                    StatusGlyph::Done,
                    &server.name,
                    vec![cell(&server.source, "dim"), cell("disabled", "dim")],
                );
            }
        }
    }
    block.render()
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

    fn plain(text: &str) -> String {
        hoocode_tui_util::strip_vt_control_characters(text)
    }

    #[test]
    fn empty_list_says_where_to_configure_servers() {
        assert!(format_listing(&[]).starts_with("No MCP servers."));
    }

    #[test]
    fn lists_each_server_with_source_state_and_tools() {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        hoocode_code_tui_theme::chalk::set_enabled(true);
        let text = plain(&format_listing(&[
            server("mine", "user", McpStatus::Connected, 2),
            server("repo-tool", "project", McpStatus::NotTrusted, 0),
            server("remote", "project", McpStatus::AuthNeeded, 0),
            server("broken", "plugin p", McpStatus::Failed("exit 1".into()), 0),
            server("off", "project", McpStatus::Disabled, 0),
            server("one", "user", McpStatus::Connected, 1),
        ]));
        assert_eq!(
            text,
            [
                "MCP servers  ~/.agents/mcp.json · ./.agents/mcp.json",
                " ● mine       user  2 tools",
                " ⊗ repo-tool  project  not trusted",
                " ◉ remote     project  auth needed",
                "   /mcp login remote",
                " ⊗ broken     plugin p  exit 1",
                " ● off        project  disabled",
                " ● one        user  1 tool",
            ]
            .join("\n")
        );
    }

    #[test]
    fn auth_needed_reads_warning_yellow() {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        hoocode_code_tui_theme::chalk::set_enabled(true);
        let text = format_listing(&[server("remote", "project", McpStatus::AuthNeeded, 0)]);
        let t = hoocode_code_tui_theme::theme();
        assert!(text.contains(&t.fg("warning", "auth needed")));
        // …but the working glyph itself stays accent, never yellow.
        assert!(text.contains(&t.fg("accent", "◉")));
    }
}

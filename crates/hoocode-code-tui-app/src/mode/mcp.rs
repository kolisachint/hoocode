//! MCP commands: `/mcp`, the login flow, and the trust prompt at startup.

use hoocode_tui_components::{Spacer, Text};

use super::*;

/// The answers to the MCP trust prompt.
const MCP_TRUST_YES: &str = "Trust and start";

const MCP_TRUST_NO: &str = "Not now";

impl Mode {
    /// `/perf`: the Phase 0 counters (threads, RSS, frame and keystroke timing,
    /// stalls), as a notice in the transcript. See `perf.rs`.
    /// `/mcp`: each MCP server with its source, state and tool count. `/mcp login <server>`
    /// starts the OAuth login of a server that needs one (see `start_mcp_login`).
    pub(super) fn handle_mcp_command(&mut self, command: &str) {
        let args = command
            .trim()
            .trim_start_matches('/')
            .strip_prefix("mcp")
            .unwrap_or("")
            .trim();
        if let Some(server) = args.strip_prefix("login") {
            self.start_mcp_login(server.trim());
            return;
        }
        if !args.is_empty() {
            self.show_warning("Usage: /mcp, or /mcp login <server>");
            return;
        }
        let servers = self
            .session
            .mcp()
            .map(|hub| hub.servers())
            .unwrap_or_default();
        let text = crate::mcp_listing::format_listing(&servers);
        self.add_to_chat(as_component(&handle(Spacer::new(1))));
        self.add_to_chat(as_component(&handle(Text::new(text, 1, 0))));
    }

    /// `/mcp login <server>`: the hub runs the login off the UI thread. The login link and the
    /// outcome come back as chat records, so the link can be followed when the browser does not
    /// open.
    fn start_mcp_login(&mut self, server: &str) {
        use hoocode_code_permissions::PermissionUi as _;
        if server.is_empty() {
            self.show_warning("Usage: /mcp login <server>");
            return;
        }
        let Some(hub) = self.session.mcp() else {
            self.show_warning("No MCP servers are configured.");
            return;
        };
        let started = hub.login(server, |message| {
            crate::dialog_bridge::TuiPermissionUi.notify(&message);
        });
        match started {
            Ok(()) => self.show_status(&format!("Logging in to MCP server {server}.")),
            Err(error) => self.show_warning(&error),
        }
    }

    /// The one-time trust prompt for project and plugin MCP servers. Asked on a
    /// thread (the answer waits for the user); the grant saves the trust store and
    /// starts the servers, off the UI thread.
    pub(super) fn ask_mcp_trust(&self) {
        let Some(hub) = self.session.mcp() else {
            return;
        };
        let prompts = hub.pending_prompts();
        if prompts.is_empty() {
            return;
        }
        let servers = hub.servers();
        let spawned = hoocode_runtime::spawn_named_thread("hoocode-mcp-trust", move || {
            use hoocode_code_permissions::PermissionUi as _;
            let ui = crate::dialog_bridge::TuiPermissionUi;
            for prompt in prompts {
                let question = hoocode_code_agent_session::mcp::trust_question(&prompt, &servers);
                let answer = ui.select(&question, &[MCP_TRUST_YES, MCP_TRUST_NO]);
                if answer.as_deref() != Some(MCP_TRUST_YES) {
                    continue;
                }
                let message = match hub.grant(std::slice::from_ref(&prompt.key)) {
                    Ok(_) => format!("MCP servers from {} are trusted.", prompt.source.label()),
                    Err(error) => format!("Could not save the MCP trust grant: {error}"),
                };
                ui.notify(&message);
            }
        });
        // A thread that cannot start leaves the servers untrusted: the safe default.
        let _ = spawned;
    }
}

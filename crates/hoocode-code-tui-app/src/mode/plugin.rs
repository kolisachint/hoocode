//! `/plugin`: the listing, marketplaces, installs, removes, updates and trust (part 2 of
//! `docs/design/plugins.md`). The command itself is `hoocode_code_plugins::command`; this file
//! shows its reply and makes a change live: the MCP servers of the new plugin set start, and the resources reload
//! (the `/reload` path). A change during a turn waits for the turn to end.

use std::sync::Arc;

use hoocode_code_agent_session::mcp::McpHub;
use hoocode_code_plugins::command;
use hoocode_code_plugins::install::Paths;
use hoocode_tui_components::{Spacer, Text};

use super::*;

/// How the command is spelled here, for its usage line and hints.
const PREFIX: &str = "/plugin";

impl Mode {
    /// `/plugin <command>`: run it, then show what it said. A change is made live (see
    /// [`Mode::apply_plugin_change`]).
    pub(super) fn handle_plugin_command(&mut self, command: &str) {
        let args = command
            .trim()
            .trim_start_matches('/')
            .strip_prefix("plugin")
            .unwrap_or("")
            .trim();
        let words: Vec<&str> = args.split_whitespace().collect();
        let paths = Paths::for_cwd(self.session.cwd());
        match command::run(&paths, &words, PREFIX, hoocode_code_plugins::current) {
            Ok(reply) if reply.changed => {
                // The reload replays the transcript, so the text is shown after it.
                self.plugin_notices.push(reply.text);
                self.apply_plugin_change();
            }
            Ok(reply) => self.show_plugin_text(&reply.text),
            Err(error) => self.show_warning(&error),
        }
    }

    /// Make a change live now when idle, else when the turn ends.
    fn apply_plugin_change(&mut self) {
        if self.session.is_streaming() || self.session.is_compacting() {
            self.plugin_reload_pending = true;
            self.show_status("Plugin changes take effect when this turn ends.");
            return;
        }
        self.reload_plugins();
    }

    /// Reload the plugins: the current set, then the MCP servers of its plugins (only when this
    /// session runs MCP), then the resources. Skills and subagents are live from the next turn.
    pub(super) fn reload_plugins(&mut self) {
        let cwd = self.session.cwd().to_path_buf();
        let set = Paths::for_cwd(&cwd).load();
        hoocode_code_plugins::set_current(set.clone());
        if let Some(old) = self.session.mcp() {
            let hub = Arc::new(McpHub::for_folder_with_plugins(&cwd, &set.mcp_servers()));
            hub.set_elicitation(Arc::new(crate::mcp_elicitation::TuiElicitation));
            self.session.attach_mcp(hub.clone(), true);
            hub.start();
            old.shutdown();
            self.ask_mcp_trust();
        }
        self.handle_reload_command();
    }

    /// The notices of `/plugin` changes, shown after a reload.
    pub(super) fn show_plugin_notices(&mut self) {
        for notice in std::mem::take(&mut self.plugin_notices) {
            self.show_record(&notice);
        }
    }

    fn show_plugin_text(&mut self, text: &str) {
        self.add_to_chat(as_component(&handle(Spacer::new(1))));
        self.add_to_chat(as_component(&handle(Text::new(text.to_owned(), 1, 0))));
    }
}

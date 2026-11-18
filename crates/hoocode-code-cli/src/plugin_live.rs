//! The CLI's side of live plugins (PL-4, `docs/design/plugins.md`): the plugin tools and the
//! plugin MCP servers of a session, built again whenever the plugins change.
//!
//! The session asks [`PluginToolSource`] for a rebuild at its sync points (see
//! `hoocode_code_agent_session::PluginSource`). The source reads the current plugin set, so
//! `/plugin`, the model's plugin tools and `/reload` all reach a session the same way.

use std::sync::{Arc, Mutex};

use hoocode_agent_types::PermissionGate;
use hoocode_code_agent_session::mcp::McpHub;
use hoocode_code_agent_session::PluginSource;
use hoocode_code_settings::SettingsManager;
use hoocode_code_tool_api::ToolDefinition;

use crate::ext_tools;
use crate::runtime::{new_mcp_hub, Diagnostics};

/// The plugin tools and MCP servers of one session.
pub(crate) struct PluginToolSource {
    pub(crate) cwd: std::path::PathBuf,
    /// Interactive mode answers the MCP servers' questions in the TUI.
    pub(crate) interactive: bool,
    /// The session's tool list can use `ext_` tools (see `ext_tools_wanted`).
    pub(crate) ext: bool,
    /// The session runs MCP (see `mcp_wanted`).
    pub(crate) mcp: bool,
    /// False when the session's mode restricts its active tools: a new tool stays off.
    pub(crate) auto_activate: bool,
    pub(crate) settings: Arc<Mutex<SettingsManager>>,
    pub(crate) gate: Arc<dyn PermissionGate>,
}

impl PluginSource for PluginToolSource {
    fn generation(&self) -> u64 {
        hoocode_code_plugins::generation()
    }

    fn auto_activate(&self) -> bool {
        self.auto_activate
    }

    fn tools(&self, taken: &[String]) -> (Vec<ToolDefinition>, Vec<String>) {
        if !self.ext {
            return (Vec::new(), Vec::new());
        }
        let set = hoocode_code_plugins::current();
        let mut diagnostics = Diagnostics::new();
        let settings = self.settings.lock().unwrap_or_else(|e| e.into_inner());
        let tools = ext_tools::build(&set, &settings, self.gate.clone(), taken, &mut diagnostics);
        let notices = diagnostics
            .into_iter()
            .map(|(_, message)| message)
            .collect();
        (tools, notices)
    }

    fn mcp_hub(&self) -> Option<Arc<McpHub>> {
        if !self.mcp {
            return None;
        }
        let set = hoocode_code_plugins::current();
        let hub = new_mcp_hub(&self.cwd, self.interactive, &set);
        hub.start();
        Some(hub)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use hoocode_code_mcp::TrustStore;
    use hoocode_code_plugins::{load_plugins, LoadOptions};
    use serde_json::json;

    use super::*;

    const FIXTURE: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../hoocode-code-wasm/tests/fixtures/echo-ext.wasm"
    );

    struct AllowAll;

    impl PermissionGate for AllowAll {
        fn request(
            &self,
            _tool_call: &hoocode_agent_types::AgentToolCall,
        ) -> hoocode_agent_types::PermissionDecision {
            hoocode_agent_types::PermissionDecision::Grant
        }
    }

    fn source(ext: bool) -> PluginToolSource {
        PluginToolSource {
            cwd: std::env::temp_dir(),
            interactive: false,
            ext,
            mcp: false,
            auto_activate: true,
            settings: Arc::new(Mutex::new(SettingsManager::in_memory(
                json!({}).as_object().unwrap().clone(),
            ))),
            gate: Arc::new(AllowAll),
        }
    }

    /// Loads a set with one plugin `demo` (the echo fixture) from a temp folder.
    fn demo_set(dir: &std::path::Path) -> hoocode_code_plugins::PluginSet {
        let root = dir.join("plugins").join("demo");
        std::fs::create_dir_all(root.join("dev.hoocode")).unwrap();
        std::fs::write(
            root.join("plugin.json"),
            r#"{"$schema": "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json", "name": "demo", "version": "1.0.0", "description": "Test plugin"}"#,
        )
        .unwrap();
        std::fs::copy(FIXTURE, root.join("dev.hoocode/extension.wasm")).unwrap();
        let trust = TrustStore::empty(dir.join("trust.json"));
        load_plugins(&LoadOptions {
            project_folder: dir.join("repo"),
            user_plugins_dir: dir.join("plugins"),
            data_root: dir.join("plugin-data"),
            trust: &trust,
            untrusted_installs: BTreeSet::new(),
            installs_unreadable: None,
        })
    }

    #[test]
    fn plugin_tools_follow_the_current_plugin_set() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("repo")).unwrap();
        let installed = demo_set(dir.path());
        let names = |tools: &[ToolDefinition]| -> Vec<String> {
            tools.iter().map(|t| t.name.clone()).collect()
        };

        // No plugins: no plugin tools.
        hoocode_code_plugins::set_current(hoocode_code_plugins::PluginSet::default());
        let (tools, notices) = source(true).tools(&[]);
        assert!(tools.is_empty() && notices.is_empty(), "{notices:?}");

        // Installed: the tool is there, named as the model sees it.
        hoocode_code_plugins::set_current(installed);
        let (tools, notices) = source(true).tools(&[]);
        assert!(notices.is_empty(), "{notices:?}");
        assert!(names(&tools).contains(&"ext_demo_echo".to_owned()));

        // A name the session already has is not taken by a plugin.
        let (tools, notices) = source(true).tools(&["ext_demo_echo".to_owned()]);
        assert!(!names(&tools).contains(&"ext_demo_echo".to_owned()));
        assert_eq!(notices.len(), 1, "{notices:?}");

        // A session without ext tools builds none.
        let (tools, _) = source(false).tools(&[]);
        assert!(tools.is_empty());

        // Every change to the set moves the generation, so a session builds again.
        let before = hoocode_code_plugins::generation();
        hoocode_code_plugins::set_current(hoocode_code_plugins::PluginSet::default());
        assert!(hoocode_code_plugins::generation() > before);
        assert!(source(false).mcp_hub().is_none());
    }
}

//! The CLI's side of live plugins (PL-4, `docs/design/plugins.md`): the plugin tools and the
//! plugin MCP servers of a session, built again whenever the plugins change.
//!
//! The session asks [`PluginToolSource`] for a rebuild at its sync points (see
//! `hoocode_code_agent_session::PluginSource`). The source reads the current plugin set, so
//! `/plugin`, the model's plugin tools and `/reload` all reach a session the same way.

use std::sync::{Arc, Mutex};

use hoocode_agent_types::{PermissionGate, ToolOrigin};
use hoocode_code_agent_session::mcp::McpHub;
use hoocode_code_agent_session::PluginSource;
use hoocode_code_mcp::PluginServers;
use hoocode_code_settings::SettingsManager;
use hoocode_code_tool_api::ToolDefinition;

use crate::ext_tools;
use crate::native_child::ChildNativeState;
use crate::native_tools::{self, NativeHolder};
use crate::runtime::{new_mcp_hub, Diagnostics};

/// The plugin tools and MCP servers of one session.
pub(crate) struct PluginToolSource {
    pub(crate) cwd: std::path::PathBuf,
    /// Interactive mode answers the MCP servers' questions in the TUI.
    pub(crate) interactive: bool,
    /// The session's tool list can use plugin WASM tools (see `ext_tools_wanted`).
    pub(crate) ext: bool,
    /// The session runs MCP (see `mcp_wanted`).
    pub(crate) mcp: bool,
    /// False when the session's mode restricts its active tools: a new tool stays off.
    pub(crate) auto_activate: bool,
    pub(crate) settings: Arc<Mutex<SettingsManager>>,
    pub(crate) gate: Arc<dyn PermissionGate>,
    /// A subagent runs no plugin MCP server (`docs/design/subagents.md`).
    pub(crate) subagent: bool,
    /// Tier C runs only in single-session processes. The app-server shares one
    /// process across sessions with no per-session tier C state, so its
    /// sessions pass false: no sync, no build, no hint publishing, no host
    /// install (see `runtime::assemble_session`).
    pub(crate) tier_c: bool,
    /// The session's tier C supervisor state (PL-18 slice 4a). One per root session, shared
    /// with the Shell env; subagents never use it (their tools come from `child_native`).
    pub(crate) native: Arc<NativeHolder>,
    /// The session's child-side tier C state, shared by the subagent registration, the
    /// Shell hook and the pool host (see `native_child`).
    pub(crate) child_native: Arc<ChildNativeState>,
}

impl PluginSource for PluginToolSource {
    fn generation(&self) -> u64 {
        hoocode_code_plugins::generation()
    }

    fn auto_activate(&self) -> bool {
        self.auto_activate
    }

    fn tools(&self, taken: &[(String, ToolOrigin)]) -> (Vec<ToolDefinition>, Vec<String>) {
        if !self.ext {
            return (Vec::new(), Vec::new());
        }
        let set = hoocode_code_plugins::current();
        let mut diagnostics = Diagnostics::new();
        // Copy the config values out, then release the settings lock before any component is
        // compiled or loaded.
        let configs = {
            let settings = self.settings.lock().unwrap_or_else(|e| e.into_inner());
            ext_tools::plugin_configs(&set, &settings)
        };
        let tools = Vec::new();
        // Tier C native helpers (PL-18): bare names with the same collision rules, trusted-only.
        // First in load order wins across tiers: each plugin registers its WASM tools then its
        // native tools before the next plugin is considered, so an earlier plugin's native tool
        // beats a later plugin's WASM tool. In a subagent only the `shared: true` helpers
        // register, running through the child's cached connection (spec D6), and only when
        // this process has a parent channel to ask EnsureHelper through: without a relay
        // every call would fail, so no shared tool is offered. Plain plugin MCP tools and
        // non-shared helpers stay absent, like the plugin MCP servers.
        let mut tools = tools;
        let holder = self.native.clone();
        // Without tier C (the app-server) the holder is never synced: no
        // supervisor, and no hint publishing below.
        if self.tier_c && !self.subagent {
            let settings = self.settings.lock().unwrap_or_else(|e| e.into_inner());
            for warning in holder.sync(&set, &settings) {
                diagnostics.push((crate::args::DiagnosticKind::Warning, warning));
            }
        }
        // A relay that cannot answer EnsureHelper (a headless fail-closed child
        // of an rpc root, warm workers) is no parent channel either.
        let relay_up = !self.subagent
            || hoocode_code_subagents::instance::clearance_relay()
                .is_some_and(|relay| relay.can_ensure());
        // Tier C tools are built only where tier C runs (the app-server
        // registers none).
        let tier_c_tools = self.tier_c && (!self.subagent || relay_up);
        if tier_c_tools {
            // The D2 hints are process-wide and `set_native_hints` replaces the
            // whole map, so publish once for every trusted helper here (root:
            // all; subagent: shared only), before the per-plugin loop below.
            // A per-plugin publish would leave only the last plugin's hints,
            // and a shared `dangerous` tool in an earlier plugin would read as
            // an ungated WASM-like tool.
            let trusted: std::collections::HashSet<&str> = set
                .plugins
                .iter()
                .filter(|p| p.marketplace_trusted)
                .map(|p| p.name.as_str())
                .collect();
            let hint_entries: Vec<hoocode_code_plugins::NativeHelperEntry> = set
                .native_helpers()
                .into_iter()
                .filter(|entry| {
                    trusted.contains(entry.plugin.as_str())
                        && (!self.subagent || entry.helper.spec.shared)
                })
                .collect();
            crate::native_tools::publish_hints(&hint_entries);
        }
        let mut taken_acc: Vec<(String, ToolOrigin)> = taken.to_vec();
        for plugin in &set.plugins {
            let single = hoocode_code_plugins::PluginSet {
                plugins: vec![plugin.clone()],
                ..hoocode_code_plugins::PluginSet::default()
            };
            let wasm = ext_tools::build(
                &single,
                &configs,
                self.gate.clone(),
                &taken_acc,
                &mut diagnostics,
            );
            taken_acc.extend(wasm.iter().map(|t| (t.name.clone(), t.origin.clone())));
            let native_defs = if !tier_c_tools {
                Vec::new()
            } else if self.subagent {
                crate::native_child::build_shared(
                    &single,
                    &self.child_native,
                    &taken_acc,
                    &mut diagnostics,
                )
            } else {
                native_tools::build(&single, &holder, &taken_acc, &mut diagnostics)
            };
            taken_acc.extend(
                native_defs
                    .iter()
                    .map(|t| (t.name.clone(), t.origin.clone())),
            );
            tools.extend(wasm);
            tools.extend(native_defs);
        }
        let notices = diagnostics
            .into_iter()
            .map(|(_, message)| message)
            .collect();
        (tools, notices)
    }

    fn mcp_servers(&self) -> Option<Vec<PluginServers>> {
        Some(self.plugin_servers())
    }

    fn mcp_hub(&self) -> Option<Arc<McpHub>> {
        if !self.mcp {
            return None;
        }
        let hub = new_mcp_hub(&self.cwd, self.interactive, &self.plugin_servers());
        hub.start();
        Some(hub)
    }
}

impl PluginToolSource {
    /// The plugins' MCP servers this session runs: none for a subagent.
    fn plugin_servers(&self) -> Vec<PluginServers> {
        if self.subagent {
            Vec::new()
        } else {
            hoocode_code_plugins::current().mcp_servers()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::PathBuf;

    use hoocode_code_mcp::TrustStore;
    use hoocode_code_plugins::native::{
        NativeHelper, NativeLaunch, NativePermission, NativeSpec, NativeTool,
    };
    use hoocode_code_plugins::{
        load_plugins, LoadOptions, LoadedPlugin, PluginFormat, PluginScope,
    };
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
            subagent: false,
            tier_c: true,
            native: crate::native_tools::NativeHolder::new(),
            child_native: Arc::new(super::super::native_child::ChildNativeState::new()),
        }
    }

    /// Loads a set with one plugin `demo` (the echo fixture) from a temp folder.
    fn demo_set(dir: &std::path::Path) -> hoocode_code_plugins::PluginSet {
        demo_set_with(dir, false)
    }

    /// [`demo_set`], with the plugin's own MCP server when `with_mcp`.
    fn demo_set_with(dir: &std::path::Path, with_mcp: bool) -> hoocode_code_plugins::PluginSet {
        let root = dir.join("plugins").join("demo");
        std::fs::create_dir_all(root.join("dev.hoocode")).unwrap();
        std::fs::write(
            root.join("plugin.json"),
            r#"{"$schema": "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json", "name": "demo", "version": "1.0.0", "description": "Test plugin"}"#,
        )
        .unwrap();
        std::fs::copy(FIXTURE, root.join("dev.hoocode/extension.wasm")).unwrap();
        if with_mcp {
            std::fs::write(
                root.join("mcp.json"),
                r#"{"$schema": "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json",
                  "mcpServers": {"echo": {"type": "stdio", "command": "./bin/echo"}}}"#,
            )
            .unwrap();
        }
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
    fn a_subagent_runs_no_plugin_mcp_server() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("repo")).unwrap();
        hoocode_code_plugins::set_current(demo_set_with(dir.path(), true));
        let main = PluginToolSource {
            mcp: true,
            ..source(true)
        };
        let child = PluginToolSource {
            mcp: true,
            subagent: true,
            ..source(true)
        };
        // The plugin declares a server: the main session gets it, the subagent none.
        let count = |source: &PluginToolSource| source.mcp_servers().map(|s| s.len());
        assert_eq!(count(&main), Some(1));
        assert_eq!(count(&child), Some(0));
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
        assert!(names(&tools).contains(&"echo".to_owned()));

        // A name the session already has is not taken by a plugin.
        let (tools, notices) = source(true).tools(&[("echo".to_owned(), ToolOrigin::BuiltIn)]);
        assert!(!names(&tools).contains(&"echo".to_owned()));
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

    /// A plugin fixture with one tier C helper: `tool_name` with `permission`,
    /// `shared` as given, served by the `echo` stdio server.
    fn write_native_plugin(dir: &std::path::Path, name: &str, shared: bool, tool_name: &str) {
        let root = dir.join(name);
        std::fs::create_dir_all(&root).unwrap();
        let manifest = serde_json::json!({
            "$schema": "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json",
            "name": name,
            "version": "1.0.0",
            "description": "Tier C fixture",
            "extensions": {"dev.hoocode": {"native": {
                "server": "echo",
                "shared": shared,
                "tools": [
                    {"name": tool_name, "description": "Tier C fixture tool.",
                     "inputSchema": {"type": "object"}, "permission": "read"},
                ],
            }}},
        });
        std::fs::write(root.join("plugin.json"), manifest.to_string()).unwrap();
        std::fs::write(
            root.join("mcp.json"),
            r#"{"$schema": "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json",
                "mcpServers": {"echo": {"type": "stdio", "command": "never-started"}}}"#,
        )
        .unwrap();
    }

    /// Two tier C plugins side by side: `sh` shared, `rt` root-only.
    fn native_set(dir: &std::path::Path) -> hoocode_code_plugins::PluginSet {
        let plugins = dir.join("plugins");
        write_native_plugin(&plugins, "sh", true, "shared_echo");
        write_native_plugin(&plugins, "rt", false, "root_echo");
        let trust = TrustStore::empty(dir.join("trust.json"));
        load_plugins(&LoadOptions {
            project_folder: dir.join("repo"),
            user_plugins_dir: plugins,
            data_root: dir.join("plugin-data"),
            trust: &trust,
            untrusted_installs: BTreeSet::new(),
            installs_unreadable: None,
        })
    }

    /// Subagent tier C rule (spec D6): only `shared: true` helpers register, with the
    /// `Plugin` origin; non-shared helpers and plain plugin MCP servers stay absent.
    #[test]
    fn a_subagent_registers_only_shared_tier_c_tools() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("repo")).unwrap();
        let set = native_set(dir.path());
        assert!(
            set.native_helpers()
                .iter()
                .all(|entry| { entry.helper.spec.server == "echo" }),
            "both fixtures parse their native block"
        );
        let mut diagnostics = Vec::new();
        let state = Arc::new(super::super::native_child::ChildNativeState::new());
        let tools = super::super::native_child::build_shared(&set, &state, &[], &mut diagnostics);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(tools.len(), 1, "{tools:?}");
        assert_eq!(tools[0].name, "shared_echo");
        assert_eq!(
            tools[0].origin,
            ToolOrigin::Plugin {
                plugin: "sh".to_owned(),
                tool: "shared_echo".to_owned(),
            }
        );
    }

    /// One hand-built loaded plugin: WASM (`extension_wasm`), a native
    /// helper, or both. Rooted at `dir` (the WASM host reads under it);
    /// tool building never starts a process.
    fn loaded_plugin(
        dir: &std::path::Path,
        name: &str,
        wasm: bool,
        native_tool: Option<&str>,
    ) -> LoadedPlugin {
        let root = dir.join(name);
        std::fs::create_dir_all(&root).unwrap();
        LoadedPlugin {
            name: name.to_owned(),
            description: String::new(),
            version: None,
            format: PluginFormat::AgentPlugins,
            scope: PluginScope::User,
            root: root.clone(),
            marketplace_trusted: true,
            data_dir: root,
            skill_dirs: Vec::new(),
            extension_wasm: wasm.then(|| PathBuf::from(FIXTURE)),
            tool_render: BTreeMap::new(),
            native: native_tool.map(|tool| NativeHelper {
                spec: NativeSpec {
                    server: "echo".to_owned(),
                    shared: true,
                    binaries: BTreeMap::new(),
                    tools: vec![NativeTool {
                        name: tool.to_owned(),
                        description: "Tier C fixture tool.".to_owned(),
                        input_schema: json!({"type": "object"}),
                        permission: NativePermission::Read,
                    }],
                },
                launch: NativeLaunch {
                    command: "never-started".to_owned(),
                    args: Vec::new(),
                    env: BTreeMap::new(),
                    cwd: None,
                },
            }),
            mcp_path: None,
            mcp_servers: Vec::new(),
        }
    }

    fn plugin_set(plugins: Vec<LoadedPlugin>) -> hoocode_code_plugins::PluginSet {
        hoocode_code_plugins::PluginSet {
            plugins,
            ..hoocode_code_plugins::PluginSet::default()
        }
    }

    /// First in load order wins across tiers: each plugin registers its WASM
    /// tools then its native tools before the next plugin is considered, so
    /// an earlier native tool beats a later WASM tool and vice versa. (The
    /// echo WASM fixture provides a tool named `echo`.)
    #[test]
    fn first_plugin_in_load_order_wins_across_tiers() {
        let dir = tempfile::tempdir().unwrap();
        for (first_native, winner) in [(true, "aaa"), (false, "zzz")] {
            let (first, second) = if first_native {
                (
                    loaded_plugin(dir.path(), "aaa", false, Some("echo")),
                    loaded_plugin(dir.path(), "zzz", true, None),
                )
            } else {
                (
                    loaded_plugin(dir.path(), "zzz", true, None),
                    loaded_plugin(dir.path(), "aaa", false, Some("echo")),
                )
            };
            hoocode_code_plugins::set_current(plugin_set(vec![first, second]));
            let (tools, notices) = source(true).tools(&[]);
            // The loser reports a taken-name diagnostic naming both origins.
            assert!(
                notices
                    .iter()
                    .any(|notice| notice.contains("already taken")),
                "{notices:?}"
            );
            // The fixture WASM component provides several tools; only `echo`
            // is contested. Exactly one `echo` survives, from the winner.
            let echoes: Vec<_> = tools.iter().filter(|t| t.name == "echo").collect();
            assert_eq!(echoes.len(), 1, "{tools:?}");
            assert_eq!(
                echoes[0].origin,
                ToolOrigin::Plugin {
                    plugin: winner.to_owned(),
                    tool: "echo".to_owned(),
                },
                "first loaded wins across tiers"
            );
        }
        hoocode_code_plugins::set_current(hoocode_code_plugins::PluginSet::default());
    }

    /// A subagent process with no parent channel is offered no shared tools:
    /// without an EnsureHelper relay every call would fail.
    #[test]
    fn a_subagent_without_a_relay_gets_no_shared_tools() {
        assert!(hoocode_code_subagents::instance::clearance_relay().is_none());
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("repo")).unwrap();
        hoocode_code_plugins::set_current(native_set(dir.path()));
        let child = PluginToolSource {
            subagent: true,
            ..source(true)
        };
        let (tools, _) = child.tools(&[]);
        assert!(
            tools.is_empty(),
            "no shared tool without a relay: {tools:?}"
        );
        hoocode_code_plugins::set_current(hoocode_code_plugins::PluginSet::default());
    }
}

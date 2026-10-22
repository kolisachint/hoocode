//! Plugins for the hoocode coding agent: loading (part 1 of `docs/design/plugins.md`).
//!
//! A plugin is a folder with a manifest. Two layouts are read:
//!
//! - **Agent Plugins 1.0**: `plugin.json` at the root (`$schema`, `name`, ...), `skills/`, and
//!   `mcp.json`. Its rules are in [`manifest`] and [`agent_mcp`].
//! - **Claude**: `.claude-plugin/plugin.json`, `skills/`, `agents/`, and `.mcp.json` (or an
//!   inline `mcpServers` in the manifest).
//!
//! [`load_plugins`] reads `~/.agents/plugins/` (user) and `<cwd>/.agents/plugins/` (project;
//! needs folder trust). A user plugin wins a name over a project plugin, so a repository cannot
//! shadow the user's plugins; within one folder the first name in sorted order wins. The result
//! is a [`PluginSet`]: [`PluginSet::skill_entries`] for the skill loader (skills are named
//! `<plugin>:<skill>`), [`PluginSet::agent_files`] for subagent definitions, and
//! [`PluginSet::mcp_servers`] for the MCP config. Hooks, commands, themes and providers are
//! skipped with a warning.
//!
//! [`command`] is the plugin command shared by `/plugin` and the CLI subcommands.
//! [`install`] copies plugins from [`marketplace`]s (Claude's `marketplace.json`, read only) into the
//! user or project plugin folder and records where each came from. [`model`] and [`propose`] back
//! the model's plugin tools: a trusted-only install and search, and writing a plugin the model
//! authored (an Agent Plugins package, untrusted until the user grants its MCP servers).

pub mod agent_mcp;
pub mod command;
pub mod install;
mod load;
pub mod manifest;
pub mod marketplace;
pub mod model;
pub mod paths;
pub mod propose;
pub mod skills;
mod trust;

use std::path::PathBuf;
use std::sync::RwLock;

use hoocode_code_mcp::{PluginServers, ServerDef};
use hoocode_code_resources::package_resolve::PathMetadata;
use hoocode_code_resources::source_info::{SourceOrigin, SourceScope};
use hoocode_code_resources::PathEntry;

pub use load::{load_plugins, LoadOptions};
pub use trust::{approve_skill_tools, load_default_trust, trust_project_plugins};

/// The plugins the running session uses. Set at session start and after an install, remove,
/// update or trust (`/plugin`), so the resource loader and the skill gate read the same set.
static CURRENT: RwLock<Option<PluginSet>> = RwLock::new(None);

/// Make `set` the current plugins (see [`current`]).
pub fn set_current(set: PluginSet) {
    *CURRENT.write().unwrap_or_else(|e| e.into_inner()) = Some(set);
}

/// The current plugins: what was last set, or none.
pub fn current() -> PluginSet {
    CURRENT
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_default()
}

/// Which layout a plugin uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginFormat {
    AgentPlugins,
    Claude,
}

/// Where a plugin was found. Project plugins load only after folder trust.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginScope {
    Project,
    User,
}

/// How serious a [`PluginDiagnostic`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    /// Skipped or ignored, and the plugin still loads.
    Warning,
    /// Something was refused: a whole plugin, or one part of one.
    Error,
}

/// A problem found while loading. Loading never fails as a whole.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginDiagnostic {
    /// The plugin, when its name is known.
    pub plugin: Option<String>,
    /// The file or folder the problem is about.
    pub path: PathBuf,
    /// The MCP server entry the problem is about, if any.
    pub server: Option<String>,
    pub severity: Severity,
    pub message: String,
}

/// A plugin that loaded, with what it contributes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedPlugin {
    pub name: String,
    /// The manifest's `description`, or empty.
    pub description: String,
    pub format: PluginFormat,
    pub scope: PluginScope,
    /// The canonical plugin folder.
    pub root: PathBuf,
    /// False only for an install from an untrusted marketplace (see `install`). Its MCP servers
    /// then need a grant like any other untrusted server. Hand-placed plugins are trusted.
    pub marketplace_trusted: bool,
    /// `PLUGIN_DATA` for this plugin (`~/.agents/plugin-data/<name>`). Created when the plugin
    /// has an Agent Plugins `mcp.json`.
    pub data_dir: PathBuf,
    /// Skill folders (each holds a `SKILL.md`), in name order.
    pub skill_dirs: Vec<PathBuf>,
    /// Subagent definition files (Claude `agents/*.md`), in name order.
    pub agent_files: Vec<PathBuf>,
    /// The file the MCP servers came from (`mcp.json`, `.mcp.json`, or the manifest).
    pub mcp_path: Option<PathBuf>,
    /// The MCP servers, with placeholders already expanded, for `hoocode-code-mcp`.
    pub mcp_servers: Vec<ServerDef>,
}

/// Project plugins that were found but are not loaded because the folder is not trusted yet.
/// Granting trust is [`trust_project_plugins`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectTrustPrompt {
    pub folder: PathBuf,
    /// The hash of the project plugins as they are now. A change to them asks again.
    pub fingerprint: String,
    /// The folder names of the project plugins.
    pub plugins: Vec<String>,
}

/// The result of [`load_plugins`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PluginSet {
    pub plugins: Vec<LoadedPlugin>,
    pub diagnostics: Vec<PluginDiagnostic>,
    pub pending_project_trust: Option<ProjectTrustPrompt>,
}

impl PluginSet {
    /// Skill folders for the skill loader. Each carries its plugin name as the namespace, so
    /// its skills are named `<plugin>:<skill>`.
    pub fn skill_entries(&self) -> Vec<PathEntry> {
        self.plugins
            .iter()
            .flat_map(|plugin| {
                let scope = match plugin.scope {
                    PluginScope::Project => SourceScope::Project,
                    PluginScope::User => SourceScope::User,
                };
                plugin.skill_dirs.iter().map(move |dir| PathEntry {
                    path: dir.to_string_lossy().into_owned(),
                    metadata: PathMetadata {
                        source: "plugin".into(),
                        scope,
                        origin: SourceOrigin::TopLevel,
                        base_dir: Some(plugin.root.to_string_lossy().into_owned()),
                        namespace: Some(plugin.name.clone()),
                    },
                })
            })
            .collect()
    }

    /// Subagent definition files for the agent registry.
    pub fn agent_files(&self) -> Vec<String> {
        self.plugins
            .iter()
            .flat_map(|plugin| plugin.agent_files.iter())
            .map(|path| path.to_string_lossy().into_owned())
            .collect()
    }

    /// The MCP servers of each plugin that has any, for `hoocode-code-mcp`.
    pub fn mcp_servers(&self) -> Vec<PluginServers> {
        self.plugins
            .iter()
            .filter(|plugin| !plugin.mcp_servers.is_empty())
            .filter_map(|plugin| {
                Some(PluginServers {
                    id: plugin.name.clone(),
                    path: plugin.mcp_path.clone()?,
                    // A project plugin has passed folder trust by the time it loads. Whether
                    // the plugin's MCP servers need their own grant depends on its marketplace.
                    marketplace_trusted: plugin.marketplace_trusted,
                    servers: plugin.mcp_servers.clone(),
                })
            })
            .collect()
    }
}

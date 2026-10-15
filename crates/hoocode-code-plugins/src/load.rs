//! The loader: which plugin folders are found, which one wins a name, and what each contributes.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use hoocode_code_mcp::{
    parse_document, ServerDef, Severity as McpSeverity, Source, Transport, TrustStore,
};

use crate::agent_mcp::{parse_mcp, resolve_command, McpContext};
use crate::manifest::{parse_agent_plugin_manifest, parse_claude_manifest};
use crate::paths::{plugin_relative, within};
use crate::trust::{fingerprint, plugin_roots};
use crate::{
    LoadedPlugin, PluginDiagnostic, PluginFormat, PluginScope, PluginSet, ProjectTrustPrompt,
    Severity,
};

/// The folder under `.agents` (project and user) that holds plugins.
const PLUGINS_DIR: &str = "plugins";
/// Component folders hoocode does not run. A plugin that has one still loads, with a warning.
const UNSUPPORTED_DIRS: [&str; 4] = ["hooks", "commands", "themes", "providers"];

/// Where to look, and the trust that decides whether project plugins load.
pub struct LoadOptions<'a> {
    /// The project folder (normally the working directory). Its `.agents/plugins` is read.
    pub project_folder: PathBuf,
    /// The user plugin folder (default `~/.agents/plugins`).
    pub user_plugins_dir: PathBuf,
    /// Where each plugin's `PLUGIN_DATA` goes (default `~/.agents/plugin-data`).
    pub data_root: PathBuf,
    pub trust: &'a TrustStore,
    /// Installed plugins (by canonical folder) whose marketplace is not trusted. Their MCP
    /// servers need a grant like any other untrusted server. Empty when nothing is installed.
    pub untrusted_installs: BTreeSet<PathBuf>,
    /// Set when the install records cannot be read. Then no plugin's marketplace is trusted
    /// (fail closed), and the loader says why.
    pub installs_unreadable: Option<String>,
}

impl<'a> LoadOptions<'a> {
    /// The defaults for `project_folder` and the given trust store: the user and project
    /// folders, and the install records of the hoocode data directory.
    pub fn new(project_folder: PathBuf, trust: &'a TrustStore) -> Self {
        crate::install::Paths::for_cwd(&project_folder).load_options(trust)
    }
}

/// Load the plugins for one session. Project plugins load only when the folder's trust matches
/// them. User plugins are read first, so a user plugin wins a name over a project plugin; within
/// one folder the first name in sorted order wins. Nothing here starts a process.
pub fn load_plugins(options: &LoadOptions<'_>) -> PluginSet {
    let mut set = PluginSet::default();
    if let Some(error) = &options.installs_unreadable {
        set.diagnostics.push(warn(
            None,
            &options.data_root,
            None,
            format!("plugin install records cannot be read, so no plugin is trusted as a marketplace install: {error}"),
        ));
    }
    let project_dir = options
        .project_folder
        .join(hoocode_code_mcp::AGENTS_DIR_NAME)
        .join(PLUGINS_DIR);
    // Under the home folder the project and user folders are one; read it once.
    let same_folder = hoocode_code_paths::canonicalize_path(&project_dir)
        == hoocode_code_paths::canonicalize_path(&options.user_plugins_dir);
    let project_roots = if same_folder {
        Vec::new()
    } else {
        plugin_roots(&project_dir)
    };
    let user_roots = plugin_roots(&options.user_plugins_dir);

    let project_fingerprint = fingerprint(&project_roots);
    let folder_key = options.project_folder.display().to_string();
    let trusted = !project_roots.is_empty()
        && options
            .trust
            .project_plugins_trusted(&folder_key, &project_fingerprint);
    if !project_roots.is_empty() && !trusted {
        let names: Vec<String> = project_roots.iter().map(|root| file_name(root)).collect();
        set.diagnostics.push(warn(
            None,
            &project_dir,
            None,
            format!(
                "project plugins are not loaded until this folder is trusted: {}",
                names.join(", ")
            ),
        ));
        set.pending_project_trust = Some(ProjectTrustPrompt {
            folder: options.project_folder.clone(),
            fingerprint: project_fingerprint,
            plugins: names,
        });
    }

    // The user's plugins come first, so a repository cannot replace one by name (mcp.md). A
    // project plugin that is not trusted is not a candidate at all.
    let project_candidates = if trusted { project_roots } else { Vec::new() };
    let candidates = user_roots
        .into_iter()
        .map(|root| (root, PluginScope::User))
        .chain(
            project_candidates
                .into_iter()
                .map(|root| (root, PluginScope::Project)),
        );

    let mut claimed: BTreeMap<String, PathBuf> = BTreeMap::new();
    for (dir, scope) in candidates {
        let Some(plugin) = load_one(&dir, scope, options, &mut set.diagnostics) else {
            continue;
        };
        if let Some(winner) = claimed.get(&plugin.name) {
            set.diagnostics.push(warn(
                Some(&plugin.name),
                &plugin.root,
                None,
                format!(
                    "a plugin named \"{}\" is already loaded from {}; this copy is skipped",
                    plugin.name,
                    winner.display()
                ),
            ));
            continue;
        }
        claimed.insert(plugin.name.clone(), plugin.root.clone());
        set.plugins.push(plugin);
    }
    set
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn warn(
    plugin: Option<&str>,
    path: &Path,
    server: Option<&str>,
    message: String,
) -> PluginDiagnostic {
    PluginDiagnostic {
        plugin: plugin.map(str::to_owned),
        path: path.to_path_buf(),
        server: server.map(str::to_owned),
        severity: Severity::Warning,
        message,
    }
}

fn reject(path: &Path, reason: String) -> PluginDiagnostic {
    PluginDiagnostic {
        plugin: None,
        path: path.to_path_buf(),
        server: None,
        severity: Severity::Error,
        message: format!("plugin rejected: {reason}"),
    }
}

/// Load one plugin folder. `None` when it is not a plugin or is rejected (then a diagnostic says
/// why, unless it simply has no manifest).
fn load_one(
    dir: &Path,
    scope: PluginScope,
    options: &LoadOptions<'_>,
    diags: &mut Vec<PluginDiagnostic>,
) -> Option<LoadedPlugin> {
    let root = hoocode_code_paths::canonicalize_path(dir);
    let agent_manifest = root.join("plugin.json");
    let claude_manifest = root.join(".claude-plugin").join("plugin.json");
    let (format, manifest_path) = if agent_manifest.is_file() {
        if claude_manifest.is_file() {
            diags.push(warn(
                None,
                &root,
                None,
                "has both plugin.json and .claude-plugin/plugin.json; the Claude manifest is ignored"
                    .to_owned(),
            ));
        }
        (PluginFormat::AgentPlugins, agent_manifest)
    } else if claude_manifest.is_file() {
        (PluginFormat::Claude, claude_manifest)
    } else {
        return None;
    };

    if !within(&root, &manifest_path) {
        diags.push(reject(
            &manifest_path,
            "the manifest resolves outside the plugin folder".to_owned(),
        ));
        return None;
    }
    let text = match fs::read_to_string(&manifest_path) {
        Ok(text) => text,
        Err(e) => {
            diags.push(reject(
                &manifest_path,
                format!("cannot read the manifest: {e}"),
            ));
            return None;
        }
    };

    // The name and the MCP schema version come from the manifest; the rest of the checks follow.
    let (name, description, schema, warnings) = match format {
        PluginFormat::AgentPlugins => match parse_agent_plugin_manifest(&text) {
            Ok(m) => (m.name, m.description, Some(m.schema), m.warnings),
            Err(reason) => {
                diags.push(reject(&manifest_path, reason));
                return None;
            }
        },
        PluginFormat::Claude => match parse_claude_manifest(&text) {
            Ok(m) => (m.name, m.description, None, Vec::new()),
            Err(reason) => {
                diags.push(reject(&manifest_path, reason));
                return None;
            }
        },
    };
    for message in warnings {
        diags.push(warn(Some(&name), &manifest_path, None, message));
    }

    // A hand-placed plugin (no install record) is trusted, as before. An install from an
    // untrusted marketplace is not, and nothing is trusted when the records cannot be read.
    let marketplace_trusted =
        options.installs_unreadable.is_none() && !options.untrusted_installs.contains(&root);
    let mut plugin = LoadedPlugin {
        data_dir: options.data_root.join(&name),
        name,
        description,
        format,
        scope,
        root: root.clone(),
        marketplace_trusted,
        skill_dirs: Vec::new(),
        agent_files: Vec::new(),
        mcp_path: None,
        mcp_servers: Vec::new(),
    };
    plugin.skill_dirs = skill_dirs(&plugin, diags);
    match schema {
        Some(schema) => load_agent_plugins_mcp(&mut plugin, &schema, diags),
        None => {
            plugin.agent_files = agent_files(&plugin, diags);
            load_claude_mcp(&mut plugin, &manifest_path, &text, diags);
        }
    }
    for dir in UNSUPPORTED_DIRS {
        if root.join(dir).is_dir() {
            diags.push(warn(
                Some(&plugin.name),
                &root,
                None,
                format!("{dir}/ is not supported by hoocode; skipped"),
            ));
        }
    }
    Some(plugin)
}

/// `skills/`: each child folder with a `SKILL.md` is one skill (§6.1, §7.1). Other children are
/// not skills. A `skills` that is not a folder, or that leaves the plugin, is an invalid
/// component (§6.2) and is skipped.
fn skill_dirs(plugin: &LoadedPlugin, diags: &mut Vec<PluginDiagnostic>) -> Vec<PathBuf> {
    let dir = plugin.root.join("skills");
    if !dir.exists() {
        return Vec::new();
    }
    if !dir.is_dir() || !within(&plugin.root, &dir) {
        diags.push(warn(
            Some(&plugin.name),
            &dir,
            None,
            "skills is not a folder inside the plugin; skipped".to_owned(),
        ));
        return Vec::new();
    }
    let mut out = Vec::new();
    for child in sorted_children(&dir) {
        if !child.is_dir() || !child.join("SKILL.md").is_file() {
            continue;
        }
        if !within(&plugin.root, &child) || !within(&plugin.root, &child.join("SKILL.md")) {
            diags.push(warn(
                Some(&plugin.name),
                &child,
                None,
                "skill resolves outside the plugin; skipped".to_owned(),
            ));
            continue;
        }
        out.push(child);
    }
    out
}

/// Claude `agents/*.md`: subagent definitions, one file each, not recursive.
fn agent_files(plugin: &LoadedPlugin, diags: &mut Vec<PluginDiagnostic>) -> Vec<PathBuf> {
    let dir = plugin.root.join("agents");
    if !dir.exists() {
        return Vec::new();
    }
    if !dir.is_dir() || !within(&plugin.root, &dir) {
        diags.push(warn(
            Some(&plugin.name),
            &dir,
            None,
            "agents is not a folder inside the plugin; skipped".to_owned(),
        ));
        return Vec::new();
    }
    sorted_children(&dir)
        .into_iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .filter(|path| path.is_file() && within(&plugin.root, path))
        .collect()
}

fn sorted_children(dir: &Path) -> Vec<PathBuf> {
    let Ok(read) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = read
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .collect();
    paths.sort();
    paths
}

/// Agent Plugins `mcp.json`. A missing file is no MCP (§6.2). A file that is not a regular file
/// inside the plugin is an invalid component: skipped.
fn load_agent_plugins_mcp(
    plugin: &mut LoadedPlugin,
    schema: &str,
    diags: &mut Vec<PluginDiagnostic>,
) {
    let path = plugin.root.join("mcp.json");
    if !path.exists() {
        return;
    }
    if !path.is_file() || !within(&plugin.root, &path) {
        diags.push(warn(
            Some(&plugin.name),
            &path,
            None,
            "mcp.json is not a regular file inside the plugin; MCP servers skipped".to_owned(),
        ));
        return;
    }
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) => {
            diags.push(warn(
                Some(&plugin.name),
                &path,
                None,
                format!("cannot read mcp.json: {e}"),
            ));
            return;
        }
    };
    // Created before the paths are checked: PLUGIN_DATA must exist to be resolved (§9.1).
    if let Err(e) = fs::create_dir_all(&plugin.data_dir) {
        diags.push(warn(
            Some(&plugin.name),
            &plugin.data_dir,
            None,
            format!("cannot create the plugin data folder: {e}"),
        ));
    }
    plugin.data_dir = hoocode_code_paths::canonicalize_path(&plugin.data_dir);
    let ctx = McpContext {
        plugin: &plugin.name,
        root: &plugin.root,
        data_dir: &plugin.data_dir,
        mcp_path: &path,
        marketplace_trusted: plugin.marketplace_trusted,
    };
    let parsed = parse_mcp(&text, schema, &ctx);
    plugin.mcp_servers = parsed.servers;
    plugin.mcp_path = Some(path);
    diags.extend(parsed.diagnostics);
}

/// The stdio paths of a Claude server, held to the plugin root: `command` is a bare name or a
/// `./` path inside the plugin (returned as that absolute path), and `cwd` is a `./` path inside
/// it. Other transports pass unchanged.
fn contain_claude_stdio(root: &Path, transport: &Transport) -> Result<Transport, String> {
    let Transport::Stdio {
        command,
        args,
        env,
        cwd,
    } = transport
    else {
        return Ok(transport.clone());
    };
    let command = resolve_command(command, root)?;
    let cwd = match cwd {
        None => None,
        Some(text) => Some(
            plugin_relative(root, text)
                .ok_or_else(|| format!("\"cwd\" \"{text}\" must be a ./ path inside the plugin"))?
                .to_string_lossy()
                .into_owned(),
        ),
    };
    Ok(Transport::Stdio {
        command,
        args: args.clone(),
        env: env.clone(),
        cwd,
    })
}

/// Claude MCP: `.mcp.json` when present, else the manifest's inline `mcpServers`. The entries
/// are read by `hoocode-code-mcp`'s own parser; no variables are expanded, and the stdio paths
/// are held to the plugin root (see [`contain_claude_stdio`]).
fn load_claude_mcp(
    plugin: &mut LoadedPlugin,
    manifest_path: &Path,
    manifest_text: &str,
    diags: &mut Vec<PluginDiagnostic>,
) {
    let file = plugin.root.join(".mcp.json");
    let (source_path, text) = if file.exists() {
        if !file.is_file() || !within(&plugin.root, &file) {
            diags.push(warn(
                Some(&plugin.name),
                &file,
                None,
                ".mcp.json is not a regular file inside the plugin; MCP servers skipped".to_owned(),
            ));
            return;
        }
        match fs::read_to_string(&file) {
            Ok(text) => (file, text),
            Err(e) => {
                diags.push(warn(
                    Some(&plugin.name),
                    &file,
                    None,
                    format!("cannot read .mcp.json: {e}"),
                ));
                return;
            }
        }
    } else {
        (manifest_path.to_path_buf(), manifest_text.to_owned())
    };
    let source = Source::Plugin {
        id: plugin.name.clone(),
        path: source_path.clone(),
        marketplace_trusted: plugin.marketplace_trusted,
    };
    let mut raw = Vec::new();
    let parsed = parse_document(&text, &source, &mut raw);
    for d in raw {
        diags.push(PluginDiagnostic {
            plugin: Some(plugin.name.clone()),
            path: source_path.clone(),
            server: d.server,
            severity: match d.severity {
                McpSeverity::Warning => Severity::Warning,
                McpSeverity::Error => Severity::Error,
            },
            message: d.message,
        });
    }
    // The same containment as Agent Plugins for a stdio command and cwd: a server whose paths
    // leave the plugin is skipped. Without this, a `./` command would run from the session's
    // folder, not the plugin's.
    let mut servers = Vec::new();
    for def in parsed {
        match contain_claude_stdio(&plugin.root, &def.transport) {
            Ok(transport) => servers.push(ServerDef { transport, ..def }),
            Err(reason) => diags.push(PluginDiagnostic {
                plugin: Some(plugin.name.clone()),
                path: source_path.clone(),
                server: Some(def.name.clone()),
                severity: Severity::Error,
                message: format!("invalid server, skipped: {reason}"),
            }),
        }
    }
    if !servers.is_empty() {
        plugin.mcp_path = Some(source_path);
        plugin.mcp_servers = servers;
    }
}

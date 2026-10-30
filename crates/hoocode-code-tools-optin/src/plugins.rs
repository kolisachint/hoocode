//! The plugin tools (`docs/design/plugins.md`, part 3): SearchPlugins, InstallPlugin,
//! UninstallPlugin, ListPlugins and ProposePlugin. Registered only with `enablePluginTools` (or
//! `--enable-plugintools`), and never in a subagent. Adding a marketplace is not a tool: the user
//! does it with `/plugin marketplace add`.
//!
//! Permissions live in `hoocode-code-tools` (`permissions.rs`): the two listing tools are read
//! only, InstallPlugin is granted (it refuses an untrusted marketplace itself), and
//! UninstallPlugin and ProposePlugin ask.

use std::path::PathBuf;
use std::sync::Arc;

use hoocode_agent_types::AgentToolResult;
use hoocode_ai_types::{Content, TextContent};
use hoocode_code_plugins::install::{self, Paths, Scope};
use hoocode_code_plugins::model;
use hoocode_code_plugins::propose::{self, Proposal, ProposedSkill};
use hoocode_code_tool_api::{ToolDefinition, ToolError};
use serde_json::{json, Map, Value};

/// `SearchPlugins`.
pub const SEARCH_PLUGINS_TOOL_NAME: &str = "SearchPlugins";
/// `InstallPlugin`.
pub const INSTALL_PLUGIN_TOOL_NAME: &str = "InstallPlugin";
/// `UninstallPlugin`.
pub const UNINSTALL_PLUGIN_TOOL_NAME: &str = "UninstallPlugin";
/// `ListPlugins`.
pub const LIST_PLUGINS_TOOL_NAME: &str = "ListPlugins";
/// `ProposePlugin`.
pub const PROPOSE_PLUGIN_TOOL_NAME: &str = "ProposePlugin";

/// Every plugin tool's name. A subagent is never given any of them.
pub const PLUGIN_TOOL_NAMES: [&str; 5] = [
    SEARCH_PLUGINS_TOOL_NAME,
    INSTALL_PLUGIN_TOOL_NAME,
    UNINSTALL_PLUGIN_TOOL_NAME,
    LIST_PLUGINS_TOOL_NAME,
    PROPOSE_PLUGIN_TOOL_NAME,
];

/// What the plugin tools need from the session: the folders, the trust file, and where a plugin
/// goes when a call does not say (`pluginInstallScope`).
#[derive(Clone, Debug)]
pub struct PluginToolHost {
    pub paths: Paths,
    pub trust_path: PathBuf,
    pub default_scope: Scope,
}

/// The five plugin tool definitions, in the order the model sees them.
pub fn create_plugin_tool_definitions(host: Arc<PluginToolHost>) -> Vec<ToolDefinition> {
    vec![
        search_definition(host.clone()),
        install_definition(host.clone()),
        uninstall_definition(host.clone()),
        list_definition(host.clone()),
        propose_definition(host),
    ]
}

fn definition(
    name: &str,
    label: &str,
    description: &str,
    prompt_snippet: &str,
    parameters: Value,
    execute: hoocode_code_tool_api::DefinitionExecuteFn,
) -> ToolDefinition {
    ToolDefinition {
        ordered_start: false,
        background_when: None,
        background_placeholder: None,
        name: name.into(),
        label: label.into(),
        description: description.into(),
        prompt_snippet: Some(prompt_snippet.into()),
        prompt_guidelines: Vec::new(),
        parameters,
        prepare_arguments: None,
        execution_mode: None,
        background: false,
        execute,
    }
}

fn text_result(text: String, details: Value) -> AgentToolResult {
    AgentToolResult {
        content: vec![Content::Text(TextContent {
            text_signature: None,
            text,
        })],
        details,
        terminate: false,
    }
}

fn string_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, ToolError> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("{key} is required").into())
}

/// The optional `scope` argument, or the host's default.
fn scope_arg(args: &Value, host: &PluginToolHost) -> Result<Scope, ToolError> {
    match args.get("scope").and_then(Value::as_str) {
        None => Ok(host.default_scope),
        Some(text) => Scope::parse(text)
            .ok_or_else(|| format!("scope must be \"user\" or \"project\", not \"{text}\"").into()),
    }
}

fn scope_schema() -> Value {
    json!({
        "type": "string",
        "enum": ["user", "project"],
        "description": "Where the plugin goes: user (every project) or project (this folder only; it loads after /plugin trust). Defaults to the pluginInstallScope setting."
    })
}

fn notes_text(notes: &[String]) -> String {
    notes
        .iter()
        .map(|n| format!("\nNote: {n}"))
        .collect::<String>()
}

fn search_definition(host: Arc<PluginToolHost>) -> ToolDefinition {
    definition(
        SEARCH_PLUGINS_TOOL_NAME,
        "Search plugins",
        "Search the plugin marketplaces the user has listed, by name or description. Returns each match with its marketplace, version, and whether it is installed. Use it before writing a plugin yourself, to see whether one already exists.",
        "Search the listed plugin marketplaces for a plugin",
        json!({
            "type": "object",
            "required": ["query"],
            "properties": {
                "query": {"type": "string", "description": "Words to match against plugin names and descriptions."}
            }
        }),
        Arc::new(move |_id, args, _signal, _on_update, _ctx| {
            let query = string_arg(&args, "query")?;
            let (matches, notes) = model::search(&host.paths, query)?;
            let installed = install::records(&host.paths)?;
            let lines: Vec<String> = matches
                .iter()
                .map(|f| {
                    let mark = if installed.iter().any(|r| r.name == f.entry.name) {
                        ", installed"
                    } else {
                        ""
                    };
                    let version = if f.entry.version.is_empty() {
                        String::new()
                    } else {
                        format!(" {}", f.entry.version)
                    };
                    format!(
                        "- {}{version} ({}{mark}): {}",
                        f.entry.name, f.marketplace, f.entry.description
                    )
                })
                .collect();
            let text = if lines.is_empty() {
                format!("No plugin matches \"{query}\".{}", notes_text(&notes))
            } else {
                format!("{}{}", lines.join("\n"), notes_text(&notes))
            };
            Ok(text_result(
                text,
                json!({ "matches": matches.len(), "query": query }),
            ))
        }),
    )
}

fn install_definition(host: Arc<PluginToolHost>) -> ToolDefinition {
    definition(
        INSTALL_PLUGIN_TOOL_NAME,
        "Install plugin",
        "Install a plugin from a marketplace the user trusts. Give the plugin name, or name@marketplace when more than one marketplace has it. Installs from a marketplace the user has not trusted are refused: ask the user to install it with /plugin install. The plugin's skills load on /reload.",
        "Install a plugin from a trusted marketplace",
        json!({
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": {"type": "string", "description": "The plugin name, or name@marketplace."},
                "scope": scope_schema()
            }
        }),
        Arc::new(move |_id, args, _signal, _on_update, _ctx| {
            let spec = string_arg(&args, "name")?;
            let scope = scope_arg(&args, &host)?;
            let installed = model::install_trusted(&host.paths, spec, scope)?;
            model::reload(&host.paths, &host.trust_path);
            let text = format!(
                "Installed {} {} from {} ({} scope) at {}. Run /reload to load its skills now.{}",
                installed.name,
                installed.version,
                installed.marketplace,
                installed.scope.label(),
                installed.root.display(),
                notes_text(&installed.notes),
            );
            Ok(text_result(
                text,
                json!({ "name": installed.name, "marketplace": installed.marketplace, "scope": installed.scope.label() }),
            ))
        }),
    )
}

fn uninstall_definition(host: Arc<PluginToolHost>) -> ToolDefinition {
    definition(
        UNINSTALL_PLUGIN_TOOL_NAME,
        "Uninstall plugin",
        "Remove an installed plugin: its folder goes, and its data folder is kept (the reply says where). Give the scope when the plugin is installed in both.",
        "Remove an installed plugin",
        json!({
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": {"type": "string", "description": "The installed plugin's name."},
                "scope": scope_schema()
            }
        }),
        Arc::new(move |_id, args, _signal, _on_update, _ctx| {
            let name = string_arg(&args, "name")?;
            let scope = match args.get("scope").and_then(Value::as_str) {
                None => None,
                Some(_) => Some(scope_arg(&args, &host)?),
            };
            let removed = install::remove(&host.paths, name, scope)?;
            model::reload(&host.paths, &host.trust_path);
            let text = format!(
                "Removed {} ({} scope). Its data folder is kept at {}.",
                removed.name,
                removed.scope.label(),
                removed.data_dir.display()
            );
            Ok(text_result(text, json!({ "name": removed.name })))
        }),
    )
}

fn list_definition(host: Arc<PluginToolHost>) -> ToolDefinition {
    definition(
        LIST_PLUGINS_TOOL_NAME,
        "List plugins",
        "List the marketplaces, the loaded plugins with where each came from, the installs that did not load, and anything skipped with its reason.",
        "List the loaded and installed plugins",
        json!({"type": "object", "properties": {}}),
        Arc::new(move |_id, _args, _signal, _on_update, _ctx| {
            let set = model::load(&host.paths, &host.trust_path);
            let records = install::records(&host.paths)?;
            let text = install::listing(&host.paths, &set, &records);
            Ok(text_result(
                text,
                json!({ "loaded": set.plugins.len(), "installed": records.len() }),
            ))
        }),
    )
}

fn propose_definition(host: Arc<PluginToolHost>) -> ToolDefinition {
    definition(
        PROPOSE_PLUGIN_TOOL_NAME,
        "Propose plugin",
        "Write a plugin of your own: skills, and optionally MCP servers, as an Agent Plugins package. Use it only for a capability worth keeping that SearchPlugins did not find. Skills load at once. MCP servers do not start until the user approves them in /mcp. The user is asked before this runs.",
        "Write a reusable plugin of skills and MCP servers",
        json!({
            "type": "object",
            "required": ["name", "description", "skills"],
            "properties": {
                "name": {"type": "string", "description": "Plugin name: a-z, 0-9, '-' and '.', 1 to 64 characters, starting and ending with a letter or digit. Name the capability, not the occasion."},
                "description": {"type": "string", "description": "What the plugin is for, and when to reach for it. Up to 1024 characters."},
                "skills": {
                    "type": "array",
                    "description": "The skills, one folder each.",
                    "items": {
                        "type": "object",
                        "required": ["name", "description", "body"],
                        "properties": {
                            "name": {"type": "string", "description": "Skill name: a-z, 0-9 and '-', 1 to 64 characters."},
                            "description": {"type": "string", "description": "When to use the skill, in the words someone would use. Up to 1024 characters."},
                            "body": {"type": "string", "description": "The skill's instructions, in Markdown."}
                        }
                    }
                },
                "mcpServers": {
                    "type": "object",
                    "description": "Optional MCP servers, keyed by name, in the Agent Plugins mcp.json form: {\"type\": \"stdio\", \"command\": ..., \"args\": [...]} or {\"type\": \"streamable-http\", \"url\": ...}."
                },
                "scope": scope_schema()
            }
        }),
        Arc::new(move |_id, args, _signal, _on_update, _ctx| {
            let proposal = proposal_arg(&args)?;
            let scope = scope_arg(&args, &host)?;
            let proposed = propose::propose(&host.paths, &proposal, scope)?;
            model::reload(&host.paths, &host.trust_path);
            let servers = if proposed.mcp_servers.is_empty() {
                String::new()
            } else {
                format!(
                    " Its MCP servers ({}) stay off until the user approves them in /mcp.",
                    proposed.mcp_servers.join(", ")
                )
            };
            let text = format!(
                "Wrote {} to {} ({} scope). Skills: {}.{servers}",
                proposed.name,
                proposed.root.display(),
                proposed.scope.label(),
                proposed.skills.join(", "),
            );
            Ok(text_result(
                text,
                json!({ "name": proposed.name, "skills": proposed.skills, "mcpServers": proposed.mcp_servers }),
            ))
        }),
    )
}

/// The `ProposePlugin` arguments, read into a [`Proposal`]. Shape errors name the field.
fn proposal_arg(args: &Value) -> Result<Proposal, ToolError> {
    let name = string_arg(args, "name")?.to_owned();
    let description = string_arg(args, "description")?.to_owned();
    let skills = args
        .get("skills")
        .and_then(Value::as_array)
        .ok_or("skills must be an array")?
        .iter()
        .map(|item| -> Result<ProposedSkill, ToolError> {
            let text = |key: &str| -> Result<String, ToolError> {
                item.get(key)
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .ok_or_else(|| format!("each skill needs a string {key}").into())
            };
            Ok(ProposedSkill {
                name: text("name")?,
                description: text("description")?,
                body: text("body")?,
            })
        })
        .collect::<Result<Vec<_>, ToolError>>()?;
    let mcp_servers: Option<Map<String, Value>> = match args.get("mcpServers") {
        None | Some(Value::Null) => None,
        Some(Value::Object(map)) => Some(map.clone()),
        Some(_) => return Err("mcpServers must be an object".into()),
    };
    Ok(Proposal {
        name,
        description,
        skills,
        mcp_servers,
    })
}

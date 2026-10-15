//! `ProposePlugin`: write a plugin the model authored (`docs/design/plugins.md`, part 3).
//!
//! The package is an Agent Plugins 1.0 package and nothing else: a root `plugin.json` with the
//! 1.0 `$schema`, `skills/<name>/SKILL.md`, and `mcp.json` when it has MCP servers. Never a Claude
//! or Copilot layout. hoocode's own fields go under `extensions.hoocode`, the namespace the spec
//! reserves for extensions. The package is checked with the loader's own parsers in a staging
//! folder, and only then moves into the plugin folder.
//!
//! Its record is not from a marketplace, so it is untrusted: its MCP servers need a grant in
//! `/mcp` before they start. Its skills are passive and load at once.

use std::fs;
use std::path::PathBuf;

use serde_json::{json, Map, Value};

use crate::agent_mcp::{parse_mcp, McpContext};
use crate::install::{self, Paths, Record, Scope};
use crate::manifest::{parse_agent_plugin_manifest, validate_name, AGENT_PLUGIN_SCHEMA};
use crate::Severity;

/// The `marketplace` a proposed plugin's record names. Not a valid marketplace name (it has a
/// capital letter), so no marketplace can take it.
pub const PROPOSED_MARKETPLACE: &str = "ProposePlugin";
/// The version written into a proposed plugin's manifest.
pub const PROPOSED_VERSION: &str = "0.1.0";
/// The `$schema` of an Agent Plugins 1.0.0 `mcp.json`.
const MCP_SCHEMA: &str = "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json";
/// The longest description a skill or plugin may have (the Agent Skills limit).
const MAX_DESCRIPTION: usize = 1024;

/// One skill: a `SKILL.md` with this name, description and body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProposedSkill {
    pub name: String,
    pub description: String,
    pub body: String,
}

/// What the model asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proposal {
    pub name: String,
    pub description: String,
    pub skills: Vec<ProposedSkill>,
    /// The `mcpServers` object of `mcp.json`, as the Agent Plugins spec writes it.
    pub mcp_servers: Option<Map<String, Value>>,
}

/// What a proposal wrote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proposed {
    pub name: String,
    pub root: PathBuf,
    pub scope: Scope,
    /// The skills written, by name.
    pub skills: Vec<String>,
    /// The MCP servers written, by name. They need a grant in `/mcp` before they start.
    pub mcp_servers: Vec<String>,
}

/// Skill names follow the Agent Skills rules: 1 to 64 characters of `a-z`, `0-9` and `-`, not
/// starting or ending with `-`, and no `--`.
fn validate_skill_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 64 {
        return Err(format!("skill name \"{name}\" must be 1 to 64 characters"));
    }
    let ok_char = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-';
    if !name.bytes().all(ok_char) || name.starts_with('-') || name.ends_with('-') {
        return Err(format!(
            "skill name \"{name}\" may use only a-z, 0-9 and '-', and must not start or end with '-'"
        ));
    }
    if name.contains("--") {
        return Err(format!("skill name \"{name}\" must not contain \"--\""));
    }
    Ok(())
}

fn check_description(what: &str, text: &str) -> Result<(), String> {
    let text = text.trim();
    if text.is_empty() {
        return Err(format!("{what} needs a description"));
    }
    if text.len() > MAX_DESCRIPTION {
        return Err(format!(
            "{what}'s description is longer than {MAX_DESCRIPTION} characters"
        ));
    }
    Ok(())
}

/// Write `proposal` as a plugin in `scope`. Refuses a name that is already installed there, and
/// a package the loader's parsers would reject.
pub fn propose(paths: &Paths, proposal: &Proposal, scope: Scope) -> Result<Proposed, String> {
    let name = proposal.name.as_str();
    validate_name(name).map_err(|e| format!("plugin name \"{name}\": {e}"))?;
    check_description("the plugin", &proposal.description)?;

    let mut skill_names: Vec<&str> = Vec::new();
    for skill in &proposal.skills {
        validate_skill_name(&skill.name)?;
        check_description(&format!("skill \"{}\"", skill.name), &skill.description)?;
        if skill.body.trim().is_empty() {
            return Err(format!("skill \"{}\" has no body", skill.name));
        }
        if skill_names.contains(&skill.name.as_str()) {
            return Err(format!("skill \"{}\" is listed twice", skill.name));
        }
        skill_names.push(&skill.name);
    }
    let servers = proposal
        .mcp_servers
        .as_ref()
        .filter(|servers| !servers.is_empty());
    if skill_names.is_empty() && servers.is_none() {
        return Err("nothing to propose: give at least one skill or MCP server".to_owned());
    }

    let scope_dir = paths.scope_dir(scope);
    let dest = scope_dir.join(name);
    if dest.exists() {
        return Err(format!(
            "{name} is already installed at {}; remove it first",
            dest.display()
        ));
    }

    let stage = paths.staging_dir().join(format!("propose-{name}"));
    let _ = fs::remove_dir_all(&stage);
    let written = write_package(&stage, proposal, servers);
    let checked = written.and_then(|()| check_package(paths, &stage, name, servers));
    if let Err(error) = checked {
        let _ = fs::remove_dir_all(&stage);
        return Err(error);
    }

    fs::create_dir_all(&scope_dir).map_err(|e| e.to_string())?;
    let root = hoocode_code_paths::canonicalize_path(&scope_dir).join(name);
    install::add_record(
        paths,
        Record {
            name: name.to_owned(),
            root: root.clone(),
            scope,
            marketplace: PROPOSED_MARKETPLACE.to_owned(),
            source: "model proposal".to_owned(),
            version: PROPOSED_VERSION.to_owned(),
            trusted: false,
        },
    )?;
    // The record goes first, so a folder that appears always has its record. If the move fails,
    // the record is left behind: it names a folder that does not exist, which loads nothing.
    if let Err(error) = fs::rename(&stage, &dest) {
        let _ = fs::remove_dir_all(&stage);
        return Err(format!("cannot move the plugin into place: {error}"));
    }

    Ok(Proposed {
        name: name.to_owned(),
        root,
        scope,
        skills: skill_names.iter().map(|s| (*s).to_owned()).collect(),
        mcp_servers: servers
            .map(|s| s.keys().cloned().collect())
            .unwrap_or_default(),
    })
}

/// The package files, in `stage`.
fn write_package(
    stage: &std::path::Path,
    proposal: &Proposal,
    servers: Option<&Map<String, Value>>,
) -> Result<(), String> {
    let manifest = json!({
        "$schema": AGENT_PLUGIN_SCHEMA,
        "name": proposal.name,
        "version": PROPOSED_VERSION,
        "description": proposal.description.trim(),
        "extensions": { "hoocode": { "origin": PROPOSED_MARKETPLACE } },
    });
    write_file(&stage.join("plugin.json"), &pretty(&manifest)?)?;
    for skill in &proposal.skills {
        // The description is a JSON string, which is also a valid YAML double-quoted scalar, so
        // a colon or a quote in it cannot break the frontmatter.
        let description =
            serde_json::to_string(skill.description.trim()).map_err(|e| e.to_string())?;
        let text = format!(
            "---\nname: {}\ndescription: {description}\n---\n\n{}\n",
            skill.name,
            skill.body.trim()
        );
        write_file(
            &stage.join("skills").join(&skill.name).join("SKILL.md"),
            &text,
        )?;
    }
    if let Some(servers) = servers {
        let mcp = json!({ "$schema": MCP_SCHEMA, "mcpServers": servers });
        write_file(&stage.join("mcp.json"), &pretty(&mcp)?)?;
    }
    Ok(())
}

/// The written package must pass the loader's parsers: the manifest, and each MCP server. Any
/// error is a refusal; a warning is not.
fn check_package(
    paths: &Paths,
    stage: &std::path::Path,
    name: &str,
    servers: Option<&Map<String, Value>>,
) -> Result<(), String> {
    let manifest = fs::read_to_string(stage.join("plugin.json")).map_err(|e| e.to_string())?;
    parse_agent_plugin_manifest(&manifest).map_err(|e| format!("plugin.json: {e}"))?;
    if servers.is_some() {
        let mcp_path = stage.join("mcp.json");
        let text = fs::read_to_string(&mcp_path).map_err(|e| e.to_string())?;
        let data_dir = paths.data_root.join(name);
        let ctx = McpContext {
            plugin: name,
            root: stage,
            data_dir: &data_dir,
            mcp_path: &mcp_path,
            marketplace_trusted: false,
        };
        let parsed = parse_mcp(&text, AGENT_PLUGIN_SCHEMA, &ctx);
        if let Some(error) = parsed
            .diagnostics
            .iter()
            .find(|d| d.severity == Severity::Error)
        {
            let server = error
                .server
                .as_deref()
                .map(|s| format!("server \"{s}\": "))
                .unwrap_or_default();
            return Err(format!("mcp.json: {server}{}", error.message));
        }
    }
    Ok(())
}

fn pretty(value: &Value) -> Result<String, String> {
    serde_json::to_string_pretty(value).map_err(|e| e.to_string())
}

fn write_file(path: &std::path::Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

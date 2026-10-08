//! `mcp.json` sources, parsing and name precedence.
//!
//! Three kinds of source, in precedence order when names clash:
//!
//! 1. **user**: `~/.agents/mcp.json` (`HOOCODE_USER_AGENTS_DIR` moves the folder). Always trusted.
//! 2. **project**: `<cwd>/.agents/mcp.json`. Needs folder trust.
//! 3. **plugin**: `mcp.json` files of installed plugins, in the order the caller gives them
//!    (first wins, as plugins.md does for plugin names). Need plugin trust unless the plugin
//!    came from a trusted marketplace.
//!
//! A later source's entry with the same name as an earlier one is dropped and reported as a
//! warning. Every valid entry a source declares is kept in [`McpConfig::sources`], shadowed or
//! not, because trust binds to what the file declares.

use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

/// The file name read in the user and project folders and in plugins.
pub const MCP_FILE_NAME: &str = "mcp.json";
/// The folder that holds `mcp.json` in the user and project scopes.
pub const AGENTS_DIR_NAME: &str = ".agents";

/// Where a server definition came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// `~/.agents/mcp.json`.
    User { path: PathBuf },
    /// `<folder>/.agents/mcp.json`; `folder` is the trust key.
    Project { folder: PathBuf, path: PathBuf },
    /// A plugin's `mcp.json`. `id` is the trust key. `marketplace_trusted` is set when the
    /// plugin was installed from a trusted marketplace, which counts as trust.
    Plugin {
        id: String,
        path: PathBuf,
        marketplace_trusted: bool,
    },
}

impl Source {
    /// The file this source was read from.
    pub fn path(&self) -> &Path {
        match self {
            Source::User { path } | Source::Project { path, .. } | Source::Plugin { path, .. } => {
                path
            }
        }
    }

    /// The trust store key: the folder for a project, `plugin:<id>` for a plugin, and `None`
    /// for the user source, which needs no grant.
    pub fn trust_key(&self) -> Option<String> {
        match self {
            Source::User { .. } => None,
            Source::Project { folder, .. } => Some(folder.display().to_string()),
            Source::Plugin { id, .. } => Some(format!("plugin:{id}")),
        }
    }

    /// Short human label for messages: `user`, `project`, or `plugin <id>`.
    pub fn label(&self) -> String {
        match self {
            Source::User { .. } => "user".to_owned(),
            Source::Project { .. } => "project".to_owned(),
            Source::Plugin { id, .. } => format!("plugin {id}"),
        }
    }
}

/// How a server is reached.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Transport {
    /// A local process started with `command`, `args`, `env` and an optional working
    /// directory (kept as written).
    Stdio {
        command: String,
        args: Vec<String>,
        env: BTreeMap<String, String>,
        cwd: Option<String>,
    },
    /// Streamable HTTP at `url`, with optional request headers.
    Http {
        url: String,
        headers: BTreeMap<String, String>,
    },
}

/// One server entry, as declared in a source file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerDef {
    pub name: String,
    pub source: Source,
    pub transport: Transport,
    /// `"disabled": true` in the entry.
    pub disabled: bool,
}

/// How serious a diagnostic is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    /// Skipped on purpose (for example a legacy `sse` entry), or shadowed.
    Warning,
    /// Invalid entry or unreadable file. The rest of the file still loads.
    Error,
}

/// A problem found while loading. Loading never fails as a whole.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub source: Source,
    /// The server entry the problem is about, if any.
    pub server: Option<String>,
    pub severity: Severity,
    pub message: String,
}

/// The valid entries one source file declares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedSource {
    pub source: Source,
    pub declared: Vec<ServerDef>,
}

/// A plugin's `mcp.json` handed in by the caller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginMcpFile {
    pub id: String,
    pub path: PathBuf,
    pub marketplace_trusted: bool,
}

/// Where to look. [`ConfigSources::new`] fills in the default user and project paths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigSources {
    /// The user `mcp.json` (default `~/.agents/mcp.json`).
    pub user: PathBuf,
    /// The project folder (normally the working directory).
    pub project_folder: PathBuf,
    /// Plugin files in precedence order.
    pub plugins: Vec<PluginMcpFile>,
}

impl ConfigSources {
    /// Default user path and the given project folder and plugins.
    pub fn new(project_folder: PathBuf, plugins: Vec<PluginMcpFile>) -> Self {
        Self {
            user: hoocode_code_paths::user_agents_dir().join(MCP_FILE_NAME),
            project_folder,
            plugins,
        }
    }
}

/// The result of loading all sources.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct McpConfig {
    /// Every source file that exists, in precedence order.
    pub sources: Vec<LoadedSource>,
    /// Effective servers: one per name, after precedence.
    pub servers: Vec<ServerDef>,
    /// Problems found while loading, shadowing included.
    pub diagnostics: Vec<Diagnostic>,
}

impl McpConfig {
    /// The valid entries `source` declares (empty when the file is missing).
    pub fn declared(&self, source: &Source) -> &[ServerDef] {
        self.sources
            .iter()
            .find(|loaded| &loaded.source == source)
            .map_or(&[], |loaded| loaded.declared.as_slice())
    }
}

/// Load every source in `sources` and apply name precedence.
pub fn load(sources: &ConfigSources) -> McpConfig {
    let mut diagnostics = Vec::new();
    let mut loaded = Vec::new();

    let user = Source::User {
        path: sources.user.clone(),
    };
    loaded.extend(load_file(user, &mut diagnostics));

    let project_path = sources
        .project_folder
        .join(AGENTS_DIR_NAME)
        .join(MCP_FILE_NAME);
    // When the project folder is the home folder, the project file is the user file.
    let same_as_user = hoocode_code_paths::canonicalize_path(&project_path)
        == hoocode_code_paths::canonicalize_path(&sources.user);
    if !same_as_user {
        let project = Source::Project {
            folder: sources.project_folder.clone(),
            path: project_path,
        };
        loaded.extend(load_file(project, &mut diagnostics));
    }

    for plugin in &sources.plugins {
        let source = Source::Plugin {
            id: plugin.id.clone(),
            path: plugin.path.clone(),
            marketplace_trusted: plugin.marketplace_trusted,
        };
        loaded.extend(load_file(source, &mut diagnostics));
    }

    let mut servers = Vec::new();
    let mut seen: BTreeMap<String, Source> = BTreeMap::new();
    for source in &loaded {
        for def in &source.declared {
            match seen.get(&def.name) {
                Some(winner) => diagnostics.push(Diagnostic {
                    source: source.source.clone(),
                    server: Some(def.name.clone()),
                    severity: Severity::Warning,
                    message: format!("shadowed by the {} entry of the same name", winner.label()),
                }),
                None => {
                    seen.insert(def.name.clone(), source.source.clone());
                    servers.push(def.clone());
                }
            }
        }
    }

    McpConfig {
        sources: loaded,
        servers,
        diagnostics,
    }
}

/// Read one source. A missing file is not a problem; an unreadable one is an error.
fn load_file(source: Source, diagnostics: &mut Vec<Diagnostic>) -> Option<LoadedSource> {
    let text = match std::fs::read_to_string(source.path()) {
        Ok(text) => text,
        Err(e) if e.kind() == ErrorKind::NotFound => return None,
        Err(e) => {
            diagnostics.push(Diagnostic {
                source,
                server: None,
                severity: Severity::Error,
                message: format!("cannot read mcp.json: {e}"),
            });
            return None;
        }
    };
    let declared = parse_document(&text, &source, diagnostics);
    Some(LoadedSource { source, declared })
}

/// Parse the text of an `mcp.json` file. Invalid entries become diagnostics and are left out.
pub fn parse_document(
    text: &str,
    source: &Source,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<ServerDef> {
    let mut problem = |server: Option<&str>, severity: Severity, message: String| {
        diagnostics.push(Diagnostic {
            source: source.clone(),
            server: server.map(str::to_owned),
            severity,
            message,
        });
    };

    let value: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(e) => {
            problem(None, Severity::Error, format!("invalid JSON: {e}"));
            return Vec::new();
        }
    };
    let Some(root) = value.as_object() else {
        problem(
            None,
            Severity::Error,
            "the top level must be an object".to_owned(),
        );
        return Vec::new();
    };
    let Some(servers) = root.get("mcpServers") else {
        return Vec::new();
    };
    let Some(servers) = servers.as_object() else {
        problem(
            None,
            Severity::Error,
            "\"mcpServers\" must be an object".to_owned(),
        );
        return Vec::new();
    };

    let mut out = Vec::new();
    for (name, entry) in servers {
        if name.is_empty() || name.trim() != name {
            problem(
                Some(name),
                Severity::Error,
                "server name must not be empty or start or end with a space".to_owned(),
            );
            continue;
        }
        match parse_entry(name, entry, source) {
            Ok(def) => out.push(def),
            Err(Failure::Skip(message)) => problem(Some(name), Severity::Warning, message),
            Err(Failure::Invalid(message)) => problem(Some(name), Severity::Error, message),
        }
    }
    out
}

/// Why an entry was not loaded.
enum Failure {
    /// Skipped on purpose, with a warning.
    Skip(String),
    /// Malformed, with an error.
    Invalid(String),
}

fn parse_entry(name: &str, entry: &Value, source: &Source) -> Result<ServerDef, Failure> {
    let obj = entry
        .as_object()
        .ok_or_else(|| Failure::Invalid("entry must be an object".to_owned()))?;

    let disabled = match obj.get("disabled") {
        None => false,
        Some(Value::Bool(flag)) => *flag,
        Some(_) => {
            return Err(Failure::Invalid(
                "\"disabled\" must be true or false".to_owned(),
            ))
        }
    };
    let kind = match obj.get("type") {
        None => None,
        Some(Value::String(kind)) => Some(kind.as_str()),
        Some(_) => return Err(Failure::Invalid("\"type\" must be a string".to_owned())),
    };
    let has_command = obj.contains_key("command");
    let has_url = obj.contains_key("url");

    let transport = match kind {
        Some("sse") => {
            return Err(Failure::Skip(
                "legacy HTTP+SSE transport (\"type\":\"sse\") is not supported; skipped".to_owned(),
            ))
        }
        Some("stdio") => stdio(obj)?,
        Some("http" | "streamable-http" | "streamableHttp") => http(obj)?,
        Some(other) => {
            return Err(Failure::Invalid(format!(
                "unknown transport type \"{other}\""
            )))
        }
        None => match (has_command, has_url) {
            (true, false) => stdio(obj)?,
            (false, true) => http(obj)?,
            (true, true) => {
                return Err(Failure::Invalid(
                    "set \"command\" or \"url\", not both".to_owned(),
                ))
            }
            (false, false) => {
                return Err(Failure::Invalid(
                    "needs \"command\" (local process) or \"url\" (HTTP)".to_owned(),
                ))
            }
        },
    };

    Ok(ServerDef {
        name: name.to_owned(),
        source: source.clone(),
        transport,
        disabled,
    })
}

fn stdio(obj: &Map<String, Value>) -> Result<Transport, Failure> {
    if obj.contains_key("url") {
        return Err(Failure::Invalid(
            "\"url\" belongs to HTTP servers, not stdio".to_owned(),
        ));
    }
    let command = required_string(obj, "command")?;
    if command.trim().is_empty() {
        return Err(Failure::Invalid("\"command\" must not be empty".to_owned()));
    }
    Ok(Transport::Stdio {
        command,
        args: string_list(obj, "args")?,
        env: string_map(obj, "env")?,
        cwd: optional_string(obj, "cwd")?,
    })
}

fn http(obj: &Map<String, Value>) -> Result<Transport, Failure> {
    if obj.contains_key("command") {
        return Err(Failure::Invalid(
            "\"command\" belongs to stdio servers, not HTTP".to_owned(),
        ));
    }
    let url = required_string(obj, "url")?;
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return Err(Failure::Invalid(
            "\"url\" must start with http:// or https://".to_owned(),
        ));
    }
    Ok(Transport::Http {
        url,
        headers: string_map(obj, "headers")?,
    })
}

fn required_string(obj: &Map<String, Value>, key: &str) -> Result<String, Failure> {
    match obj.get(key) {
        Some(Value::String(value)) => Ok(value.clone()),
        _ => Err(Failure::Invalid(format!("\"{key}\" must be a string"))),
    }
}

fn optional_string(obj: &Map<String, Value>, key: &str) -> Result<Option<String>, Failure> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => required_string(obj, key).map(Some),
    }
}

fn string_list(obj: &Map<String, Value>, key: &str) -> Result<Vec<String>, Failure> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| match item {
                Value::String(s) => Ok(s.clone()),
                _ => Err(Failure::Invalid(format!(
                    "\"{key}\" must hold only strings"
                ))),
            })
            .collect(),
        Some(_) => Err(Failure::Invalid(format!(
            "\"{key}\" must be an array of strings"
        ))),
    }
}

fn string_map(obj: &Map<String, Value>, key: &str) -> Result<BTreeMap<String, String>, Failure> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(BTreeMap::new()),
        Some(Value::Object(map)) => map
            .iter()
            .map(|(k, v)| match v {
                Value::String(s) => Ok((k.clone(), s.clone())),
                _ => Err(Failure::Invalid(format!(
                    "\"{key}\" values must be strings"
                ))),
            })
            .collect(),
        Some(_) => Err(Failure::Invalid(format!(
            "\"{key}\" must be an object of strings"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(folder: &Path) -> Source {
        Source::Project {
            folder: folder.to_path_buf(),
            path: folder.join(AGENTS_DIR_NAME).join(MCP_FILE_NAME),
        }
    }

    fn parse(text: &str) -> (Vec<ServerDef>, Vec<Diagnostic>) {
        let source = Source::User {
            path: PathBuf::from("/nowhere/mcp.json"),
        };
        let mut diagnostics = Vec::new();
        let defs = parse_document(text, &source, &mut diagnostics);
        (defs, diagnostics)
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn parses_stdio_and_http_entries() {
        let (defs, diags) = parse(
            r#"{"mcpServers": {
                "fs": {"command": "npx", "args": ["-y", "fs"], "env": {"A": "1"}, "cwd": "/x"},
                "remote": {"url": "https://example.com/mcp", "headers": {"Authorization": "t"}},
                "off": {"command": "x", "disabled": true}
            }}"#,
        );
        assert!(diags.is_empty(), "{diags:?}");
        assert_eq!(defs.len(), 3);
        let fs = defs.iter().find(|d| d.name == "fs").unwrap();
        assert_eq!(
            fs.transport,
            Transport::Stdio {
                command: "npx".into(),
                args: vec!["-y".into(), "fs".into()],
                env: BTreeMap::from([("A".to_owned(), "1".to_owned())]),
                cwd: Some("/x".into()),
            }
        );
        let remote = defs.iter().find(|d| d.name == "remote").unwrap();
        assert!(matches!(remote.transport, Transport::Http { .. }));
        assert!(defs.iter().find(|d| d.name == "off").unwrap().disabled);
    }

    #[test]
    fn explicit_types_are_accepted() {
        let (defs, diags) = parse(
            r#"{"mcpServers": {
                "a": {"type": "stdio", "command": "c"},
                "b": {"type": "http", "url": "http://h/mcp"},
                "c": {"type": "streamable-http", "url": "https://h/mcp"}
            }}"#,
        );
        assert!(diags.is_empty(), "{diags:?}");
        assert_eq!(defs.len(), 3);
    }

    #[test]
    fn legacy_sse_is_skipped_with_a_warning() {
        let (defs, diags) = parse(
            r#"{"mcpServers": {"old": {"type": "sse", "url": "https://h/sse"},
                               "ok": {"command": "c"}}}"#,
        );
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "ok");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].severity, Severity::Warning);
        assert_eq!(diags[0].server.as_deref(), Some("old"));
    }

    #[test]
    fn bad_entries_are_reported_and_the_rest_loads() {
        let (defs, diags) = parse(
            r#"{"mcpServers": {
                "both": {"command": "c", "url": "https://h"},
                "neither": {"args": []},
                "badurl": {"url": "ftp://h"},
                "badargs": {"command": "c", "args": [1]},
                "notobj": 5,
                "good": {"command": "c"}
            }}"#,
        );
        assert_eq!(
            defs.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(),
            ["good"]
        );
        assert_eq!(diags.len(), 5);
        assert!(diags.iter().all(|d| d.severity == Severity::Error));
    }

    #[test]
    fn broken_documents_yield_one_error() {
        let (defs, diags) = parse("{not json");
        assert!(defs.is_empty());
        assert_eq!(diags.len(), 1);
        let (defs, diags) = parse(r#"{"mcpServers": []}"#);
        assert!(defs.is_empty());
        assert_eq!(diags.len(), 1);
        let (defs, diags) = parse("{}");
        assert!(defs.is_empty() && diags.is_empty());
    }

    #[test]
    fn precedence_is_user_then_project_then_plugin() {
        let dir = tempfile::tempdir().unwrap();
        let user_path = dir.path().join("home/.agents/mcp.json");
        let folder = dir.path().join("repo");
        let plugin_a = dir.path().join("plugins/a/mcp.json");
        let plugin_b = dir.path().join("plugins/b/mcp.json");
        write(
            &user_path,
            r#"{"mcpServers": {"shared": {"command": "user"}, "u": {"command": "u"}}}"#,
        );
        write(
            &folder.join(".agents/mcp.json"),
            r#"{"mcpServers": {"shared": {"command": "project"}, "p": {"command": "p"}}}"#,
        );
        write(
            &plugin_a,
            r#"{"mcpServers": {"shared": {"command": "plugin-a"}, "pa": {"command": "pa"}}}"#,
        );
        write(
            &plugin_b,
            r#"{"mcpServers": {"pa": {"command": "pb-clash"}, "pb": {"command": "pb"}}}"#,
        );
        let sources = ConfigSources {
            user: user_path,
            project_folder: folder.clone(),
            plugins: vec![
                PluginMcpFile {
                    id: "a".into(),
                    path: plugin_a,
                    marketplace_trusted: true,
                },
                PluginMcpFile {
                    id: "b".into(),
                    path: plugin_b,
                    marketplace_trusted: false,
                },
            ],
        };
        let config = load(&sources);

        let winner = |name: &str| {
            config
                .servers
                .iter()
                .find(|d| d.name == name)
                .map(|d| d.source.label())
        };
        assert_eq!(winner("shared").as_deref(), Some("user"));
        assert_eq!(winner("u").as_deref(), Some("user"));
        assert_eq!(winner("p").as_deref(), Some("project"));
        assert_eq!(winner("pa").as_deref(), Some("plugin a"));
        assert_eq!(winner("pb").as_deref(), Some("plugin b"));
        assert_eq!(config.servers.len(), 5);

        // The shadowed entries stay declared by their own source, and are reported.
        let plugin_a_source = Source::Plugin {
            id: "a".into(),
            path: dir.path().join("plugins/a/mcp.json"),
            marketplace_trusted: true,
        };
        assert_eq!(config.declared(&plugin_a_source).len(), 2);
        let shadowed: Vec<_> = config
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .map(|d| (d.source.label(), d.server.clone().unwrap()))
            .collect();
        assert!(shadowed.contains(&("plugin a".to_owned(), "shared".to_owned())));
        assert!(shadowed.contains(&("plugin b".to_owned(), "pa".to_owned())));
        assert!(shadowed.contains(&("project".to_owned(), "shared".to_owned())));
    }

    #[test]
    fn missing_files_are_not_problems_and_bad_files_do_not_stop_others() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("repo");
        write(&folder.join(".agents/mcp.json"), "{oops");
        let sources = ConfigSources {
            user: dir.path().join("none/mcp.json"),
            project_folder: folder.clone(),
            plugins: vec![PluginMcpFile {
                id: "gone".into(),
                path: dir.path().join("gone/mcp.json"),
                marketplace_trusted: false,
            }],
        };
        let config = load(&sources);
        assert!(config.servers.is_empty());
        assert_eq!(config.sources.len(), 1);
        assert_eq!(config.sources[0].source, project(&folder));
        assert_eq!(config.diagnostics.len(), 1);
        assert_eq!(config.diagnostics[0].severity, Severity::Error);
    }

    #[test]
    fn project_file_equal_to_user_file_is_read_once() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        write(
            &home.join(".agents/mcp.json"),
            r#"{"mcpServers": {"x": {"command": "c"}}}"#,
        );
        let sources = ConfigSources {
            user: home.join(".agents/mcp.json"),
            project_folder: home,
            plugins: vec![],
        };
        let config = load(&sources);
        assert_eq!(config.sources.len(), 1);
        assert!(config.diagnostics.is_empty());
        assert_eq!(config.servers.len(), 1);
    }
}

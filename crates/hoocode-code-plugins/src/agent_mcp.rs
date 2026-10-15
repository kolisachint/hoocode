//! Agent Plugins `mcp.json` (spec §7.2): the closed server variants, placeholder expansion and
//! the failure boundaries. A bad file turns MCP off for the plugin; a bad server skips only
//! itself. The servers come out as `hoocode-code-mcp` definitions.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use hoocode_code_mcp::{ServerDef, Source, Transport};
use serde_json::{Map, Value};

use crate::paths::{expand, plugin_relative, resolve_cwd, PLUGIN_DATA_VAR, PLUGIN_ROOT_VAR};
use crate::{PluginDiagnostic, Severity};

/// Where an `mcp.json` sits and whom it belongs to.
pub struct McpContext<'a> {
    /// The plugin's name.
    pub plugin: &'a str,
    /// The canonical plugin root.
    pub root: &'a Path,
    /// The plugin's persistent data directory (`PLUGIN_DATA`).
    pub data_dir: &'a Path,
    /// The `mcp.json` file, as the trust store and `/mcp` name it.
    pub mcp_path: &'a Path,
    /// Whether the plugin's marketplace is trusted (see `LoadedPlugin::marketplace_trusted`).
    pub marketplace_trusted: bool,
}

/// What reading one `mcp.json` produced.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct McpParse {
    pub servers: Vec<ServerDef>,
    pub diagnostics: Vec<PluginDiagnostic>,
}

/// The `mcp.json` `$schema` that goes with a manifest `$schema` of the same version.
fn mcp_schema_for(manifest_schema: &str) -> Option<String> {
    manifest_schema
        .strip_suffix("plugin.schema.json")
        .map(|prefix| format!("{prefix}mcp.schema.json"))
}

/// Read an Agent Plugins `mcp.json` whose plugin declares `manifest_schema`.
pub fn parse_mcp(text: &str, manifest_schema: &str, ctx: &McpContext<'_>) -> McpParse {
    let mut out = McpParse::default();
    let disabled = |reason: String| McpParse {
        servers: Vec::new(),
        diagnostics: vec![diag(
            ctx,
            None,
            Severity::Error,
            format!("MCP is off for this plugin: {reason}"),
        )],
    };

    let value: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(e) => return disabled(format!("mcp.json is not valid JSON: {e}")),
    };
    let Some(root) = value.as_object() else {
        return disabled("mcp.json must be a JSON object".to_owned());
    };
    let expected = mcp_schema_for(manifest_schema);
    match root.get("$schema") {
        Some(Value::String(schema)) if Some(schema.as_str()) == expected.as_deref() => {}
        Some(Value::String(schema)) => {
            return disabled(format!(
                "mcp.json targets \"{schema}\", but the plugin targets \"{manifest_schema}\""
            ))
        }
        _ => return disabled("mcp.json needs a $schema".to_owned()),
    }
    if let Some(extra) = root
        .keys()
        .find(|key| !matches!(key.as_str(), "$schema" | "mcpServers"))
    {
        return disabled(format!(
            "mcp.json has an unknown top-level field \"{extra}\""
        ));
    }
    let Some(servers) = root.get("mcpServers").and_then(Value::as_object) else {
        return disabled("mcpServers must be an object".to_owned());
    };

    for (name, entry) in servers {
        if name.is_empty() {
            out.diagnostics.push(diag(
                ctx,
                None,
                Severity::Error,
                "a server has an empty name; skipped".to_owned(),
            ));
            continue;
        }
        match parse_server(entry, ctx) {
            Ok(Outcome::Server(transport)) => out.servers.push(ServerDef {
                name: name.clone(),
                source: Source::Plugin {
                    id: ctx.plugin.to_owned(),
                    path: ctx.mcp_path.to_path_buf(),
                    marketplace_trusted: ctx.marketplace_trusted,
                },
                transport,
                disabled: false,
            }),
            Ok(Outcome::Unsupported(kind)) => out.diagnostics.push(diag(
                ctx,
                Some(name),
                Severity::Warning,
                format!("transport \"{kind}\" is not supported by hoocode; server skipped"),
            )),
            Err(reason) => out.diagnostics.push(diag(
                ctx,
                Some(name),
                Severity::Error,
                format!("invalid server, skipped: {reason}"),
            )),
        }
    }
    out
}

fn diag(
    ctx: &McpContext<'_>,
    server: Option<&str>,
    severity: Severity,
    message: String,
) -> PluginDiagnostic {
    PluginDiagnostic {
        plugin: Some(ctx.plugin.to_owned()),
        path: ctx.mcp_path.to_path_buf(),
        server: server.map(str::to_owned),
        severity,
        message,
    }
}

enum Outcome {
    Server(Transport),
    /// A valid entry with a transport this client does not run (`sse`).
    Unsupported(String),
}

fn parse_server(entry: &Value, ctx: &McpContext<'_>) -> Result<Outcome, String> {
    let obj = entry
        .as_object()
        .ok_or_else(|| "a server entry must be an object".to_owned())?;
    let kind = obj
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "a server needs a string \"type\"".to_owned())?;
    match kind {
        "stdio" => {
            check_fields(obj, kind, &["type", "command", "args", "env", "cwd"])?;
            stdio(obj, ctx).map(Outcome::Server)
        }
        "streamable-http" => {
            check_fields(obj, kind, &["type", "url", "headers"])?;
            http(obj).map(Outcome::Server)
        }
        "sse" => Ok(Outcome::Unsupported(kind.to_owned())),
        other => Err(format!("unknown server type \"{other}\"")),
    }
}

fn check_fields(obj: &Map<String, Value>, kind: &str, allowed: &[&str]) -> Result<(), String> {
    match obj.keys().find(|key| !allowed.contains(&key.as_str())) {
        Some(key) => Err(format!("\"{key}\" is not a field of a \"{kind}\" server")),
        None => Ok(()),
    }
}

fn string_field<'a>(obj: &'a Map<String, Value>, key: &str) -> Result<Option<&'a str>, String> {
    match obj.get(key) {
        None => Ok(None),
        Some(Value::String(text)) => Ok(Some(text)),
        Some(_) => Err(format!("\"{key}\" must be a string")),
    }
}

fn string_list(obj: &Map<String, Value>, key: &str) -> Result<Vec<String>, String> {
    match obj.get(key) {
        None => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| match item {
                Value::String(text) => Ok(text.clone()),
                _ => Err(format!("\"{key}\" must hold only strings")),
            })
            .collect(),
        Some(_) => Err(format!("\"{key}\" must be an array of strings")),
    }
}

fn string_map(obj: &Map<String, Value>, key: &str) -> Result<BTreeMap<String, String>, String> {
    match obj.get(key) {
        None => Ok(BTreeMap::new()),
        Some(Value::Object(map)) => map
            .iter()
            .map(|(name, value)| match value {
                Value::String(text) => Ok((name.clone(), text.clone())),
                _ => Err(format!("\"{key}\" values must be strings")),
            })
            .collect(),
        Some(_) => Err(format!("\"{key}\" must be an object of strings")),
    }
}

fn stdio(obj: &Map<String, Value>, ctx: &McpContext<'_>) -> Result<Transport, String> {
    let root = ctx.root.to_string_lossy().into_owned();
    let data = ctx.data_dir.to_string_lossy().into_owned();

    let command = resolve_command(
        string_field(obj, "command")?.ok_or("a stdio server needs \"command\"")?,
        ctx.root,
    )?;

    let args = string_list(obj, "args")?
        .iter()
        .map(|arg| expand(arg, &root, &data))
        .collect();

    let mut env = BTreeMap::new();
    for (name, value) in string_map(obj, "env")? {
        // Case-insensitive: on Windows `plugin_root` would name the same variable.
        if name.eq_ignore_ascii_case(PLUGIN_ROOT_VAR) || name.eq_ignore_ascii_case(PLUGIN_DATA_VAR)
        {
            return Err(format!("\"env\" must not set {name}; the client sets it"));
        }
        env.insert(name, expand(&value, &root, &data));
    }
    // The client always sets these two (§9.1), after the configured entries.
    env.insert(PLUGIN_ROOT_VAR.to_owned(), root.clone());
    env.insert(PLUGIN_DATA_VAR.to_owned(), data.clone());

    // An omitted cwd is the plugin root (§7.2.1).
    let cwd = match string_field(obj, "cwd")? {
        None => root.clone(),
        Some(text) => resolve_cwd(text, ctx.root, ctx.data_dir)
            .ok_or_else(|| {
                format!("\"cwd\" \"{text}\" is not an allowed form or leaves its folder")
            })?
            .to_string_lossy()
            .into_owned(),
    };

    Ok(Transport::Stdio {
        command,
        args,
        env,
        cwd: Some(cwd),
    })
}

/// `command` is one token: a bare executable name, or a `./` path inside the plugin (§7.2.1).
/// Shared with the Claude layout, whose `command` gets the same containment.
pub(crate) fn resolve_command(command: &str, root: &Path) -> Result<String, String> {
    if command.is_empty() || command.chars().any(char::is_whitespace) {
        return Err("\"command\" must be one executable token".to_owned());
    }
    if command.starts_with("./") {
        return plugin_relative(root, command)
            .map(|path| path.to_string_lossy().into_owned())
            .ok_or_else(|| "\"command\" leaves the plugin folder".to_owned());
    }
    let is_bare = !command.chars().any(|c| c == '/' || c == '\\') && !command.contains("${");
    if is_bare {
        Ok(command.to_owned())
    } else {
        Err(
            "\"command\" must be a bare name or a ./ path; placeholders are not expanded in it"
                .to_owned(),
        )
    }
}

fn http(obj: &Map<String, Value>) -> Result<Transport, String> {
    let url = string_field(obj, "url")?.ok_or("a streamable-http server needs \"url\"")?;
    check_url(url)?;
    let headers = string_map(obj, "headers")?;
    let mut seen = BTreeSet::new();
    for (name, value) in &headers {
        if name.is_empty() || !name.bytes().all(is_token_byte) {
            return Err(format!(
                "header name \"{name}\" is not a valid HTTP field name"
            ));
        }
        if !seen.insert(name.to_ascii_lowercase()) {
            return Err(format!("header \"{name}\" is set more than once"));
        }
        if value.chars().any(|c| c.is_control() && c != '\t') {
            return Err(format!("header \"{name}\" has an invalid value"));
        }
    }
    Ok(Transport::Http {
        url: url.to_owned(),
        headers,
    })
}

/// RFC 9110 `tchar`: the bytes a header field name may use.
fn is_token_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}

/// An absolute http or https URL with no user information or fragment. Plain http only for a
/// loopback host (§7.2.1).
fn check_url(url: &str) -> Result<(), String> {
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("\"url\" must not contain spaces".to_owned());
    }
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| "\"url\" must be an absolute http or https URL".to_owned())?;
    let https = match scheme.to_ascii_lowercase().as_str() {
        "https" => true,
        "http" => false,
        _ => return Err("\"url\" must be an http or https URL".to_owned()),
    };
    if rest.contains('#') {
        return Err("\"url\" must not contain a fragment".to_owned());
    }
    let authority = rest.split(['/', '?']).next().unwrap_or_default();
    if authority.contains('@') {
        return Err("\"url\" must not contain user information".to_owned());
    }
    let host = host_of(authority);
    if host.is_empty() {
        return Err("\"url\" needs a host".to_owned());
    }
    if https || is_loopback(host) {
        Ok(())
    } else {
        Err("a non-loopback \"url\" must use https".to_owned())
    }
}

/// The host of an authority (`host`, `host:port`, `[v6]` or `[v6]:port`), without brackets.
fn host_of(authority: &str) -> &str {
    if let Some(rest) = authority.strip_prefix('[') {
        return rest.split(']').next().unwrap_or_default();
    }
    authority
        .rsplit_once(':')
        .map_or(authority, |(host, _port)| host)
}

fn is_loopback(host: &str) -> bool {
    host == "localhost"
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::AGENT_PLUGIN_SCHEMA;
    use std::path::PathBuf;

    struct Fixture {
        _dir: tempfile::TempDir,
        root: PathBuf,
        data: PathBuf,
        mcp: PathBuf,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("plug");
        let data = dir.path().join("data");
        std::fs::create_dir_all(root.join("bin")).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        Fixture {
            root: root.canonicalize().unwrap(),
            data: data.canonicalize().unwrap(),
            mcp: dir.path().join("plug/mcp.json"),
            _dir: dir,
        }
    }

    fn parse(f: &Fixture, text: &str) -> McpParse {
        let ctx = McpContext {
            plugin: "plug",
            root: &f.root,
            data_dir: &f.data,
            mcp_path: &f.mcp,
            marketplace_trusted: true,
        };
        parse_mcp(text, AGENT_PLUGIN_SCHEMA, &ctx)
    }

    fn doc(servers: &str) -> String {
        format!(
            r#"{{"$schema": "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json", "mcpServers": {{{servers}}}}}"#
        )
    }

    fn errors(out: &McpParse) -> Vec<String> {
        out.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .map(|d| d.message.clone())
            .collect()
    }

    #[test]
    fn stdio_expands_placeholders_and_sets_the_reserved_env() {
        let f = fixture();
        let out = parse(
            &f,
            &doc(
                r#""local": {"type": "stdio", "command": "./bin/tool", "args": ["--data", "${PLUGIN_DATA}/db", "${HOME}"],
                "env": {"CONFIG": "${PLUGIN_ROOT}/config.json"}}"#,
            ),
        );
        assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
        let def = &out.servers[0];
        assert_eq!(def.name, "local");
        let Transport::Stdio {
            command,
            args,
            env,
            cwd,
        } = &def.transport
        else {
            panic!("not stdio");
        };
        assert_eq!(command, &f.root.join("bin/tool").to_string_lossy());
        assert_eq!(
            args,
            &[
                "--data".to_owned(),
                format!("{}/db", f.data.display()),
                "${HOME}".to_owned()
            ]
        );
        assert_eq!(env["CONFIG"], format!("{}/config.json", f.root.display()));
        assert_eq!(env["PLUGIN_ROOT"], f.root.to_string_lossy());
        assert_eq!(env["PLUGIN_DATA"], f.data.to_string_lossy());
        // Omitted cwd is the plugin root.
        assert_eq!(cwd.as_deref(), Some(f.root.to_string_lossy().as_ref()));
        assert!(matches!(
            def.source,
            Source::Plugin { ref id, marketplace_trusted: true, .. } if id == "plug"
        ));
    }

    #[test]
    fn bare_commands_are_kept_and_placeholders_in_command_are_refused() {
        let f = fixture();
        let out = parse(&f, &doc(r#""npx": {"type": "stdio", "command": "npx"}"#));
        assert_eq!(out.servers.len(), 1);
        let Transport::Stdio { command, .. } = &out.servers[0].transport else {
            panic!()
        };
        assert_eq!(command, "npx");

        for bad in [
            "${PLUGIN_ROOT}/bin/tool",
            "../bin/tool",
            "bin/tool",
            "npx -y",
            "/usr/bin/x",
        ] {
            let out = parse(
                &f,
                &doc(&format!(r#""s": {{"type": "stdio", "command": "{bad}"}}"#)),
            );
            assert!(out.servers.is_empty(), "{bad} was accepted");
            assert_eq!(errors(&out).len(), 1, "{bad}");
        }
    }

    #[test]
    fn command_that_leaves_the_plugin_is_refused() {
        let f = fixture();
        let out = parse(&f, &doc(r#""s": {"type": "stdio", "command": "./../x"}"#));
        assert!(out.servers.is_empty());
    }

    #[test]
    fn cwd_forms_are_checked_after_expansion() {
        let f = fixture();
        let good = parse(
            &f,
            &doc(r#""s": {"type": "stdio", "command": "x", "cwd": "${PLUGIN_DATA}/work"}"#),
        );
        let Transport::Stdio { cwd, .. } = &good.servers[0].transport else {
            panic!()
        };
        assert_eq!(
            cwd.as_deref(),
            Some(f.data.join("work").to_string_lossy().as_ref())
        );

        for bad in ["/etc", "data", "../x", "${PLUGIN_ROOT}/../x", "${HOME}"] {
            let out = parse(
                &f,
                &doc(&format!(
                    r#""s": {{"type": "stdio", "command": "x", "cwd": "{bad}"}}"#
                )),
            );
            assert!(out.servers.is_empty(), "{bad} was accepted");
        }
    }

    #[test]
    fn env_must_not_set_the_reserved_names() {
        let f = fixture();
        let out = parse(
            &f,
            &doc(r#""s": {"type": "stdio", "command": "x", "env": {"PLUGIN_DATA": "/tmp"}}"#),
        );
        assert!(out.servers.is_empty());
        assert_eq!(errors(&out).len(), 1);
        // Any case of the names is refused, so no second variable differs only by case.
        for name in ["plugin_root", "Plugin_Data"] {
            let out = parse(
                &f,
                &doc(&format!(
                    r#""s": {{"type": "stdio", "command": "x", "env": {{"{name}": "/tmp"}}}}"#
                )),
            );
            assert!(out.servers.is_empty(), "{name} was accepted");
        }
    }

    #[test]
    fn streamable_http_needs_https_unless_loopback() {
        let f = fixture();
        let ok = [
            "https://deploy.example.com/mcp",
            "http://localhost:8080/mcp",
            "http://127.0.0.1/mcp",
            "http://[::1]:9000/mcp",
        ];
        for url in ok {
            let out = parse(
                &f,
                &doc(&format!(
                    r#""h": {{"type": "streamable-http", "url": "{url}"}}"#
                )),
            );
            assert_eq!(out.servers.len(), 1, "{url}: {:?}", out.diagnostics);
        }
        let bad = [
            "http://example.com/mcp",
            "ftp://example.com/mcp",
            "https://user@example.com/mcp",
            "https://example.com/mcp#frag",
            "/relative",
            "https://",
            "http://${HOME}/mcp",
        ];
        for url in bad {
            let out = parse(
                &f,
                &doc(&format!(
                    r#""h": {{"type": "streamable-http", "url": "{url}"}}"#
                )),
            );
            assert!(out.servers.is_empty(), "{url} was accepted");
        }
    }

    #[test]
    fn headers_are_fixed_literals_and_unique() {
        let f = fixture();
        let ok = parse(
            &f,
            &doc(
                r#""h": {"type": "streamable-http", "url": "https://x.example/mcp", "headers": {"X-Tenant": "${HOME}"}}"#,
            ),
        );
        assert_eq!(ok.servers.len(), 1);
        let Transport::Http { headers, .. } = &ok.servers[0].transport else {
            panic!()
        };
        assert_eq!(headers["X-Tenant"], "${HOME}", "headers are not expanded");

        let dup = parse(
            &f,
            &doc(
                r#""h": {"type": "streamable-http", "url": "https://x.example/mcp", "headers": {"X-A": "1", "x-a": "2"}}"#,
            ),
        );
        assert!(dup.servers.is_empty());
        let bad_name = parse(
            &f,
            &doc(
                r#""h": {"type": "streamable-http", "url": "https://x.example/mcp", "headers": {"Bad Name": "1"}}"#,
            ),
        );
        assert!(bad_name.servers.is_empty());
    }

    #[test]
    fn variant_fields_do_not_mix_and_unknown_types_are_refused() {
        let f = fixture();
        let mixed = parse(
            &f,
            &doc(r#""m": {"type": "stdio", "command": "x", "url": "https://a.example"}"#),
        );
        assert!(mixed.servers.is_empty());
        let http_with_command = parse(
            &f,
            &doc(r#""m": {"type": "streamable-http", "url": "https://a.example", "command": "x"}"#),
        );
        assert!(http_with_command.servers.is_empty());
        let unknown = parse(
            &f,
            &doc(r#""m": {"type": "websocket", "url": "wss://a.example"}"#),
        );
        assert!(unknown.servers.is_empty());
        let no_type = parse(&f, &doc(r#""m": {"command": "x"}"#));
        assert!(no_type.servers.is_empty());
        let disabled = parse(
            &f,
            &doc(r#""m": {"type": "stdio", "command": "x", "disabled": true}"#),
        );
        assert!(disabled.servers.is_empty());
    }

    #[test]
    fn sse_is_skipped_with_a_warning_and_the_rest_still_loads() {
        let f = fixture();
        let out = parse(
            &f,
            &doc(
                r#""old": {"type": "sse", "url": "https://legacy.example/sse"}, "ok": {"type": "stdio", "command": "x"}"#,
            ),
        );
        assert_eq!(out.servers.len(), 1);
        assert_eq!(out.servers[0].name, "ok");
        assert_eq!(out.diagnostics.len(), 1);
        assert_eq!(out.diagnostics[0].severity, Severity::Warning);
        assert_eq!(out.diagnostics[0].server.as_deref(), Some("old"));
    }

    #[test]
    fn a_bad_server_is_skipped_and_the_others_load() {
        let f = fixture();
        let out = parse(
            &f,
            &doc(r#""bad": {"type": "stdio"}, "good": {"type": "stdio", "command": "x"}"#),
        );
        assert_eq!(
            out.servers
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>(),
            ["good"]
        );
        assert_eq!(errors(&out).len(), 1);
    }

    #[test]
    fn file_level_problems_turn_mcp_off_for_the_plugin() {
        let f = fixture();
        let server = r#""s": {"type": "stdio", "command": "x"}"#;
        let cases = [
            ("not json", "{".to_owned()),
            ("not an object", "[]".to_owned()),
            ("no $schema", format!(r#"{{"mcpServers": {{{server}}}}}"#)),
            (
                "other version",
                format!(r#"{{"$schema": "https://agent-plugins.org/schemas/2.0.0/mcp.schema.json", "mcpServers": {{{server}}}}}"#),
            ),
            (
                "wrong schema kind",
                format!(r#"{{"$schema": "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json", "mcpServers": {{{server}}}}}"#),
            ),
            (
                "unknown top-level field",
                format!(r#"{{"$schema": "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json", "mcpServers": {{{server}}}, "extra": 1}}"#),
            ),
            (
                "mcpServers not an object",
                r#"{"$schema": "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json", "mcpServers": []}"#
                    .to_owned(),
            ),
        ];
        for (what, text) in cases {
            let out = parse(&f, &text);
            assert!(out.servers.is_empty(), "{what}");
            assert_eq!(errors(&out).len(), 1, "{what}");
            assert!(errors(&out)[0].starts_with("MCP is off"), "{what}");
        }
    }

    #[test]
    fn an_empty_server_map_is_valid() {
        let f = fixture();
        let out = parse(&f, &doc(""));
        assert!(out.servers.is_empty());
        assert!(out.diagnostics.is_empty());
    }
}

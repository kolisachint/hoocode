//! Tier C native helpers: the `native` manifest block (PL-18 slice 2, parsing only).
//!
//! An Agent Plugins manifest may declare one native helper in
//! `extensions."dev.hoocode".native` (spec D2, `docs/design/plugins.md`, Native tier contract):
//! which of the plugin's `mcp.json` stdio servers to run as the helper, whether subagents get
//! its tools, per-platform download binaries, and the declared tools with their permission hints.
//!
//! Parsing and data model only: nothing here starts a process, downloads a binary, or wires a
//! session. A bad `native` block turns the native part off for that plugin with an [`Error`]
//! diagnostic ([`Severity::Error`]); the rest of the plugin still loads (skills, WASM, other MCP
//! servers). Unknown fields inside `native` are [`Warning`] diagnostics; the block still loads.
//!
//! [`Error`]: Severity::Error
//! [`Warning`]: Severity::Warning

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use hoocode_agent_types::valid_plugin_tool_name;
use hoocode_code_mcp::{ServerDef, Transport};
use serde_json::{Map, Value};

use crate::{PluginDiagnostic, Severity};

/// The per-platform keys `native.binaries` accepts (spec D2).
pub const NATIVE_PLATFORMS: [&str; 5] = [
    "linux-x86_64",
    "linux-aarch64",
    "darwin-aarch64",
    "darwin-x86_64",
    "windows-x86_64",
];

/// This platform's key in `native.binaries`, or `None` where tier C ships no binary.
/// Via `cfg!`, so an unsupported target compiles and simply matches nothing.
pub fn current_platform_key() -> Option<&'static str> {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("linux-x86_64")
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        Some("linux-aarch64")
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("darwin-aarch64")
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some("darwin-x86_64")
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some("windows-x86_64")
    } else {
        None
    }
}

/// The permission hint of one declared native tool (spec D2: `read`, `act`, `dangerous`).
/// Missing means [`Act`]. The gate maps it later; parsing only checks the spelling.
///
/// [`Act`]: NativePermission::Act
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativePermission {
    Read,
    Act,
    Dangerous,
}

impl NativePermission {
    /// The manifest spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Act => "act",
            Self::Dangerous => "dangerous",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "read" => Some(Self::Read),
            "act" => Some(Self::Act),
            "dangerous" => Some(Self::Dangerous),
            _ => None,
        }
    }
}

/// One per-platform download in `native.binaries`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeBinary {
    /// Must be an `https` URL (plain binaries only; archives are out of scope).
    pub url: String,
    /// 64 lowercase or uppercase hex characters.
    pub sha256: String,
}

/// One declared tool in `native.tools`.
#[derive(Clone, Debug, PartialEq)]
pub struct NativeTool {
    /// Bare name: the tier B charset, never the `mcp__` prefix (any case).
    pub name: String,
    pub description: String,
    /// Must be a JSON object with `"type": "object"`.
    pub input_schema: Value,
    /// Missing means [`Act`].
    ///
    /// [`Act`]: NativePermission::Act
    pub permission: NativePermission,
}

/// The validated `native` block of one plugin.
#[derive(Clone, Debug, PartialEq)]
pub struct NativeSpec {
    /// Names a server in the plugin's `mcp.json` whose type is stdio.
    pub server: String,
    /// `true`: subagents get the helper's tools. Default false.
    pub shared: bool,
    pub binaries: BTreeMap<String, NativeBinary>,
    /// Required, non-empty, names unique case-insensitively.
    pub tools: Vec<NativeTool>,
}

/// How to launch the helper from the named `mcp.json` stdio entry: its `command`, `args`,
/// `env` and `cwd` as already expanded by `agent_mcp.rs`. The executable override order
/// (user `binaryPath`, then the downloaded `binaries` entry, then this `command`) is applied
/// by the supervisor, not here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeLaunch {
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub cwd: Option<String>,
}

/// A loaded native helper: the validated spec plus how to launch it.
#[derive(Clone, Debug, PartialEq)]
pub struct NativeHelper {
    pub spec: NativeSpec,
    pub launch: NativeLaunch,
}

/// What the plugin's `mcp.json` contributes, for resolving `native.server`.
pub struct NativeContext<'a> {
    /// The plugin's name, for diagnostics.
    pub plugin: &'a str,
    /// The manifest path, which is where the `native` block lives.
    pub path: &'a Path,
    /// The plugin's already-parsed MCP servers, placeholders expanded.
    pub servers: &'a [ServerDef],
}

/// What reading one `native` block produced. `helper` is `None` when the plugin declares no
/// block, or when a violation turns the native part off (then an [`Error`] says why).
///
/// [`Error`]: Severity::Error
#[derive(Debug, Default)]
pub struct NativeParse {
    pub helper: Option<NativeHelper>,
    pub diagnostics: Vec<PluginDiagnostic>,
}

/// Read the `native` block of `dev_hoocode` (`extensions."dev.hoocode"`, `None` when absent).
pub fn parse_native(
    dev_hoocode: Option<&Map<String, Value>>,
    ctx: &NativeContext<'_>,
) -> NativeParse {
    let mut out = NativeParse::default();
    let Some(native) = dev_hoocode.and_then(|dev| dev.get("native")) else {
        return out;
    };
    let Some(block) = native.as_object() else {
        out.diagnostics
            .push(off(ctx, None, "\"native\" must be an object"));
        return out;
    };
    for key in block.keys() {
        if !matches!(key.as_str(), "server" | "shared" | "binaries" | "tools") {
            out.diagnostics.push(warn(
                ctx,
                None,
                format!("extensions.dev.hoocode.native.{key} is unknown; ignored"),
            ));
        }
    }

    let server = match block.get("server").and_then(Value::as_str) {
        None if !block.contains_key("server") => {
            out.diagnostics
                .push(off(ctx, None, "\"server\" is required"));
            None
        }
        None => {
            out.diagnostics
                .push(off(ctx, None, "\"server\" must be a string"));
            None
        }
        Some(name) => Some(name.to_owned()),
    };
    let shared = match block.get("shared") {
        None => false,
        Some(Value::Bool(shared)) => *shared,
        Some(_) => {
            out.diagnostics
                .push(off(ctx, None, "\"shared\" must be a boolean"));
            false
        }
    };
    let binaries = parse_binaries(block.get("binaries"), ctx, &mut out.diagnostics);
    let tools = parse_tools(block.get("tools"), ctx, &mut out.diagnostics);

    if out
        .diagnostics
        .iter()
        .any(|d| d.severity == Severity::Error)
    {
        return out;
    }
    // Checked above: no error was recorded, so every part parsed.
    let (Some(server), Some(binaries), Some(tools)) = (server, binaries, tools) else {
        return out;
    };
    let Some(def) = ctx.servers.iter().find(|s| s.name == server) else {
        out.diagnostics.push(off(
            ctx,
            None,
            format!("server \"{server}\" is not in this plugin's mcp.json"),
        ));
        return out;
    };
    let Transport::Stdio {
        command,
        args,
        env,
        cwd,
    } = &def.transport
    else {
        out.diagnostics.push(off(
            ctx,
            Some(&server),
            format!(
                "server \"{server}\" is not a stdio server; a native helper needs \"type\": \"stdio\""
            ),
        ));
        return out;
    };
    out.helper = Some(NativeHelper {
        spec: NativeSpec {
            server,
            shared,
            binaries,
            tools,
        },
        launch: NativeLaunch {
            command: command.clone(),
            args: args.clone(),
            env: env.clone(),
            cwd: cwd.clone(),
        },
    });
    out
}

/// `None` when a violation was recorded (the caller turns the helper off on any [`Error`]).
fn parse_binaries(
    value: Option<&Value>,
    ctx: &NativeContext<'_>,
    diags: &mut Vec<PluginDiagnostic>,
) -> Option<BTreeMap<String, NativeBinary>> {
    let Some(value) = value else {
        return Some(BTreeMap::new());
    };
    let Some(entries) = value.as_object() else {
        diags.push(off(ctx, None, "\"binaries\" must be an object"));
        return None;
    };
    let mut binaries = BTreeMap::new();
    for (platform, entry) in entries {
        if !NATIVE_PLATFORMS.contains(&platform.as_str()) {
            diags.push(off(
                ctx,
                None,
                format!(
                    "binaries \"{platform}\" is not a known platform (one of {})",
                    NATIVE_PLATFORMS.join(", ")
                ),
            ));
            continue;
        }
        let Some(entry) = entry.as_object() else {
            diags.push(off(
                ctx,
                None,
                format!("binaries.{platform} must be an object"),
            ));
            continue;
        };
        for key in entry.keys() {
            if !matches!(key.as_str(), "url" | "sha256") {
                diags.push(warn(
                    ctx,
                    None,
                    format!("extensions.dev.hoocode.native.binaries.{platform}.{key} is unknown; ignored"),
                ));
            }
        }
        let url = entry.get("url").and_then(Value::as_str).unwrap_or_default();
        if !is_https_url(url) {
            diags.push(off(
                ctx,
                None,
                format!("binaries.{platform}.url must be an https URL"),
            ));
            continue;
        }
        let sha256 = entry
            .get("sha256")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !is_sha256(sha256) {
            diags.push(off(
                ctx,
                None,
                format!("binaries.{platform}.sha256 must be 64 hex characters"),
            ));
            continue;
        }
        binaries.insert(
            platform.clone(),
            NativeBinary {
                url: url.to_owned(),
                sha256: sha256.to_owned(),
            },
        );
    }
    Some(binaries)
}

/// `None` when a violation was recorded (the caller turns the helper off on any [`Error`]).
fn parse_tools(
    value: Option<&Value>,
    ctx: &NativeContext<'_>,
    diags: &mut Vec<PluginDiagnostic>,
) -> Option<Vec<NativeTool>> {
    let Some(items) = value.and_then(Value::as_array) else {
        diags.push(off(
            ctx,
            None,
            "\"tools\" is required and must be a non-empty array",
        ));
        return None;
    };
    if items.is_empty() {
        diags.push(off(
            ctx,
            None,
            "\"tools\" is required and must be a non-empty array",
        ));
        return None;
    }
    let mut tools = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for (index, item) in items.iter().enumerate() {
        let Some(tool) = item.as_object() else {
            diags.push(off(ctx, None, format!("tools[{index}] must be an object")));
            continue;
        };
        let name = tool.get("name").and_then(Value::as_str).unwrap_or_default();
        for key in tool.keys() {
            if !matches!(
                key.as_str(),
                "name" | "description" | "inputSchema" | "permission"
            ) {
                diags.push(warn(
                    ctx,
                    None,
                    format!(
                        "extensions.dev.hoocode.native.tools[{index}].{key} is unknown; ignored"
                    ),
                ));
            }
        }
        if !valid_plugin_tool_name(name) {
            diags.push(off(
                ctx,
                None,
                format!(
                    "tools[{index}].name {name:?} is invalid: bare names matching ^[A-Za-z][A-Za-z0-9_-]{{0,63}}$ that do not start with mcp__ (any case)"
                ),
            ));
            continue;
        }
        if !seen.insert(name.to_ascii_lowercase()) {
            diags.push(off(
                ctx,
                None,
                format!("tool \"{name}\" duplicates another tool (names match case-insensitively)"),
            ));
            continue;
        }
        let description = tool.get("description").and_then(Value::as_str);
        let Some(description) = description else {
            diags.push(off(
                ctx,
                None,
                format!("tool \"{name}\" needs a string \"description\""),
            ));
            continue;
        };
        let Some(input_schema) = tool.get("inputSchema").cloned() else {
            diags.push(off(
                ctx,
                None,
                format!(
                    "tool \"{name}\" needs an \"inputSchema\" object with \"type\": \"object\""
                ),
            ));
            continue;
        };
        let valid_schema = input_schema
            .as_object()
            .is_some_and(|schema| schema.get("type").and_then(Value::as_str) == Some("object"));
        if !valid_schema {
            diags.push(off(
                ctx,
                None,
                format!(
                    "tool \"{name}\" needs an \"inputSchema\" object with \"type\": \"object\""
                ),
            ));
            continue;
        }
        let permission = match tool.get("permission") {
            None => NativePermission::Act,
            Some(Value::String(text)) => match NativePermission::parse(text) {
                Some(permission) => permission,
                None => {
                    diags.push(off(
                        ctx,
                        None,
                        format!(
                            "tool \"{name}\" permission must be one of \"read\", \"act\", \"dangerous\""
                        ),
                    ));
                    continue;
                }
            },
            Some(_) => {
                diags.push(off(
                    ctx,
                    None,
                    format!(
                        "tool \"{name}\" permission must be one of \"read\", \"act\", \"dangerous\""
                    ),
                ));
                continue;
            }
        };
        tools.push(NativeTool {
            name: name.to_owned(),
            description: description.to_owned(),
            input_schema,
            permission,
        });
    }
    Some(tools)
}

fn is_https_url(url: &str) -> bool {
    url.starts_with("https://")
        && url.len() > "https://".len()
        && !url.chars().any(|c| c.is_whitespace() || c.is_control())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A violation: the native part is off for this plugin, with an [`Error`] naming the plugin
/// (via the diagnostic) and the problem.
///
/// [`Error`]: Severity::Error
fn off(
    ctx: &NativeContext<'_>,
    server: Option<&str>,
    problem: impl Into<String>,
) -> PluginDiagnostic {
    PluginDiagnostic {
        plugin: Some(ctx.plugin.to_owned()),
        path: ctx.path.to_path_buf(),
        server: server.map(str::to_owned),
        severity: Severity::Error,
        message: format!("native helpers are off for this plugin: {}", problem.into()),
    }
}

fn warn(ctx: &NativeContext<'_>, server: Option<&str>, message: String) -> PluginDiagnostic {
    PluginDiagnostic {
        plugin: Some(ctx.plugin.to_owned()),
        path: ctx.path.to_path_buf(),
        server: server.map(str::to_owned),
        severity: Severity::Warning,
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_platform_has_a_known_key() {
        let key = current_platform_key().expect("tier C ships every tested platform");
        assert!(NATIVE_PLATFORMS.contains(&key));
    }

    #[test]
    fn permissions_parse_exactly() {
        assert_eq!(
            NativePermission::parse("read"),
            Some(NativePermission::Read)
        );
        assert_eq!(NativePermission::parse("act"), Some(NativePermission::Act));
        assert_eq!(
            NativePermission::parse("dangerous"),
            Some(NativePermission::Dangerous)
        );
        assert_eq!(NativePermission::parse("Read"), None);
        assert_eq!(NativePermission::parse(""), None);
    }

    #[test]
    fn https_and_sha256_shapes() {
        assert!(is_https_url("https://example.com/bin"));
        assert!(!is_https_url("http://example.com/bin"));
        assert!(!is_https_url("https://"));
        assert!(!is_https_url("https://exa mple.com"));
        assert!(is_sha256(&"ab".repeat(32)));
        assert!(is_sha256(&"AB".repeat(32)));
        assert!(!is_sha256(&"ab".repeat(31)));
        assert!(!is_sha256(&"xb".repeat(32)));
    }
}

//! Where a tool comes from (`docs/design/plugins.md`, Naming and identity).
//!
//! Every tool has a [`ToolOrigin`], kept by the host as data. The model sees only the
//! model-facing name, so nothing reads the origin back out of a name. Selectors
//! ([`ToolSelector`]) match a tool by origin: permission rules, agent tool lists and the
//! `--tools` / `--disallowed-tools` flags all use them.

use std::fmt;

/// Where a tool comes from.
/// Serialized as `{"kind": "builtIn" | "plugin" | "mcp", ...}`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ToolOrigin {
    /// A tool hoocode ships, or one the session itself registers.
    BuiltIn,
    /// A WASM tool of a plugin. `tool` is the name the plugin declares.
    Plugin { plugin: String, tool: String },
    /// An MCP tool. `server` is the key in `mcp.json`, `tool` is the server's own name for
    /// it, and `plugin` is the owning plugin, or `None` for user and project config.
    Mcp {
        server: String,
        tool: String,
        plugin: Option<String>,
    },
}

impl ToolOrigin {
    /// The owning plugin: the plugin of a WASM tool, or of an MCP tool a plugin declares.
    pub fn plugin(&self) -> Option<&str> {
        match self {
            Self::BuiltIn => None,
            Self::Plugin { plugin, .. } => Some(plugin),
            Self::Mcp { plugin, .. } => plugin.as_deref(),
        }
    }

    /// The short source label a tool header shows after the name (`docs/design/tui.md`):
    /// the plugin name for a WASM tool, `mcp:<server>` for an MCP tool. `None` for built-ins.
    pub fn label(&self) -> Option<String> {
        match self {
            Self::BuiltIn => None,
            Self::Plugin { plugin, .. } => Some(plugin.clone()),
            Self::Mcp { server, .. } => Some(format!("mcp:{server}")),
        }
    }

    /// The tool's own name: the name a plugin declares, or the server's name for an MCP
    /// tool. A built-in has none of its own, so `model_name` (its model-facing name) stands.
    pub fn own_name<'a>(&'a self, model_name: &'a str) -> &'a str {
        match self {
            Self::BuiltIn => model_name,
            Self::Plugin { tool, .. } | Self::Mcp { tool, .. } => tool,
        }
    }

    /// The origin as a diagnostic names it: `built-in tool`, `plugin webtools`,
    /// `mcp github`, or `mcp github (plugin docs)`.
    pub fn describe(&self) -> String {
        match self {
            Self::BuiltIn => "built-in tool".to_owned(),
            Self::Plugin { plugin, .. } => format!("plugin {plugin}"),
            Self::Mcp {
                server,
                plugin: None,
                ..
            } => format!("mcp {server}"),
            Self::Mcp {
                server,
                plugin: Some(plugin),
                ..
            } => format!("mcp {server} (plugin {plugin})"),
        }
    }
}

/// Whether `name` may be a plugin WASM tool's model-facing name: `^[A-Za-z][A-Za-z0-9_-]{0,63}$`
/// and not `mcp__` at the start, in any case (`docs/design/plugins.md`, Naming and identity). An
/// MCP tool's name is `mcp__<server>__<tool>`, so a plugin tool with that prefix would pass as an
/// MCP tool in a selector and in the transcript. The loader of a component and the agent's tool
/// setup both check a declared name with this one function.
pub fn valid_plugin_tool_name(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= 64
        && !name.to_ascii_lowercase().starts_with("mcp__")
        && chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// A tool that is not loaded because its model-facing name is taken: `name` is that name,
/// `origin` the tool that is not loaded and `existing` the one that keeps the name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collision {
    pub name: String,
    pub origin: ToolOrigin,
    pub existing: ToolOrigin,
}

/// The diagnostics for the tools not loaded because their names are taken, one line per
/// requesting plugin or server and holder, in the order first seen:
/// `plugin echo-dup: 3 tools not loaded (echo, foo, bar): names already taken by plugin echo`.
/// Origins are named as [`ToolOrigin::describe`] names them.
pub fn collision_notices(collisions: &[Collision]) -> Vec<String> {
    // (requester, holder, names), so each group is one line.
    let mut groups: Vec<(String, String, Vec<&str>)> = Vec::new();
    for collision in collisions {
        let requester = collision.origin.describe();
        let holder = collision.existing.describe();
        match groups
            .iter_mut()
            .find(|(r, h, _)| *r == requester && *h == holder)
        {
            Some((_, _, names)) => names.push(&collision.name),
            None => groups.push((requester, holder, vec![&collision.name])),
        }
    }
    groups
        .into_iter()
        .map(|(requester, holder, names)| {
            let count = names.len();
            let (tools, noun) = if count == 1 {
                ("tool", "name is")
            } else {
                ("tools", "names are")
            };
            format!(
                "{requester}: {count} {tools} not loaded ({}): {noun} already taken by {holder}",
                names.join(", ")
            )
        })
        .collect()
}

impl fmt::Display for ToolOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.describe())
    }
}

/// One entry of a selector list: `Name`, `plugin:<p>`, `plugin:<p>/<Tool>`, `plugin:*`,
/// `mcp:<s>`, `mcp:<s>/<tool>` or `mcp:*`. Anything else is malformed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolSelector {
    /// The tool whose model-facing name is this. Case-insensitive.
    Name(String),
    /// Every tool of plugin `p`, WASM and MCP.
    Plugin(String),
    /// One tool of plugin `p`, by the tool's own name.
    PluginTool { plugin: String, tool: String },
    /// Every plugin tool, WASM and MCP.
    AnyPlugin,
    /// Every MCP tool of server `s`, from any source.
    Mcp(String),
    /// One MCP tool, by the server's own name for it.
    McpTool { server: String, tool: String },
    /// Every MCP tool.
    AnyMcp,
}

/// Why a selector entry is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolSelectorError {
    /// The entry is empty or only spaces.
    Empty,
    /// The entry is not a name or one of the selector forms. Holds the entry as written.
    Malformed(String),
    /// The entry is a tool name from before the rename (`ext_*`, `mcp_*`). It would match no
    /// tool, so it is refused rather than allowing or denying nothing silently. Holds the entry.
    OldName(String),
}

impl fmt::Display for ToolSelectorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("empty tool selector"),
            Self::Malformed(entry) => write!(f, "malformed tool selector \"{entry}\""),
            Self::OldName(name) => write!(
                f,
                "old tool name `{name}`: use plugin:/mcp: selectors or the new tool name"
            ),
        }
    }
}

impl std::error::Error for ToolSelectorError {}

/// A plugin name, server key or plugin tool name: letters, digits, `.`, `_` and `-`.
fn plain(part: &str) -> bool {
    !part.is_empty()
        && part
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// A bare model-facing name: letters, digits, `_` and `-`.
fn bare_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// A bare name from before the rename: `ext_<tool>` (plugin tools) or `mcp_<x>` (MCP tools,
/// now `mcp__<server>__<tool>`). No current tool has such a name, so a deny or allow entry
/// with one would silently match nothing (case-insensitive, as names match).
fn old_tool_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with("ext_") || (lower.starts_with("mcp_") && !lower.starts_with("mcp__"))
}

impl ToolSelector {
    /// Parses one entry. Surrounding spaces are ignored.
    pub fn parse(entry: &str) -> Result<Self, ToolSelectorError> {
        let text = entry.trim();
        if text.is_empty() {
            return Err(ToolSelectorError::Empty);
        }
        let malformed = || ToolSelectorError::Malformed(text.to_owned());
        if let Some(rest) = text.strip_prefix("plugin:") {
            if rest == "*" {
                return Ok(Self::AnyPlugin);
            }
            return match rest.split_once('/') {
                None if plain(rest) => Ok(Self::Plugin(rest.to_owned())),
                Some((plugin, tool)) if plain(plugin) && plain(tool) => Ok(Self::PluginTool {
                    plugin: plugin.to_owned(),
                    tool: tool.to_owned(),
                }),
                _ => Err(malformed()),
            };
        }
        if let Some(rest) = text.strip_prefix("mcp:") {
            if rest == "*" {
                return Ok(Self::AnyMcp);
            }
            return match rest.split_once('/') {
                None if plain(rest) => Ok(Self::Mcp(rest.to_owned())),
                Some((server, tool)) if plain(server) && plain(tool) => Ok(Self::McpTool {
                    server: server.to_owned(),
                    tool: tool.to_owned(),
                }),
                _ => Err(malformed()),
            };
        }
        if bare_name(text) {
            if old_tool_name(text) {
                Err(ToolSelectorError::OldName(text.to_owned()))
            } else {
                Ok(Self::Name(text.to_owned()))
            }
        } else {
            Err(malformed())
        }
    }

    /// Whether this selector takes the tool with model-facing name `name` and origin `origin`.
    pub fn matches(&self, name: &str, origin: &ToolOrigin) -> bool {
        match (self, origin) {
            (Self::Name(wanted), _) => wanted.eq_ignore_ascii_case(name),
            (Self::AnyPlugin, _) => origin.plugin().is_some(),
            (Self::Plugin(plugin), _) => origin.plugin() == Some(plugin.as_str()),
            (Self::PluginTool { plugin, tool }, ToolOrigin::Plugin { tool: own, .. }) => {
                origin.plugin() == Some(plugin.as_str()) && own == tool
            }
            (Self::PluginTool { plugin, tool }, ToolOrigin::Mcp { tool: own, .. }) => {
                origin.plugin() == Some(plugin.as_str()) && own == tool
            }
            (Self::PluginTool { .. }, ToolOrigin::BuiltIn) => false,
            (Self::AnyMcp, ToolOrigin::Mcp { .. }) => true,
            (Self::Mcp(server), ToolOrigin::Mcp { server: own, .. }) => own == server,
            (
                Self::McpTool { server, tool },
                ToolOrigin::Mcp {
                    server: s, tool: t, ..
                },
            ) => s == server && t == tool,
            _ => false,
        }
    }

    /// Whether a plugin WASM component could supply a tool this selector takes. A bare name
    /// may be one, so any bare name that is not MCP-shaped counts.
    pub fn may_match_plugin_wasm(&self) -> bool {
        match self {
            Self::Name(name) => !name.starts_with("mcp__"),
            Self::AnyPlugin | Self::Plugin(_) | Self::PluginTool { .. } => true,
            Self::Mcp(_) | Self::McpTool { .. } | Self::AnyMcp => false,
        }
    }

    /// Whether an MCP server could supply a tool this selector takes. An MCP tool's model
    /// name is `mcp__<server>__<tool>`, so a bare name counts only when it has that shape.
    /// A plugin selector takes no MCP tool: a plugin's MCP servers are not started for it
    /// (subagents never run them, `docs/design/subagents.md`).
    pub fn may_match_mcp(&self) -> bool {
        match self {
            Self::Name(name) => name.starts_with("mcp__"),
            Self::AnyPlugin | Self::Plugin(_) | Self::PluginTool { .. } => false,
            Self::Mcp(_) | Self::McpTool { .. } | Self::AnyMcp => true,
        }
    }

    /// Whether this entry names an origin rather than a model-facing name.
    pub fn is_qualified(&self) -> bool {
        !matches!(self, Self::Name(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wasm(plugin: &str, tool: &str) -> ToolOrigin {
        ToolOrigin::Plugin {
            plugin: plugin.into(),
            tool: tool.into(),
        }
    }

    fn mcp(server: &str, tool: &str, plugin: Option<&str>) -> ToolOrigin {
        ToolOrigin::Mcp {
            server: server.into(),
            tool: tool.into(),
            plugin: plugin.map(str::to_owned),
        }
    }

    #[test]
    fn labels_and_descriptions_name_the_origin() {
        assert_eq!(ToolOrigin::BuiltIn.label(), None);
        assert_eq!(wasm("webtools", "WebFetch").label().unwrap(), "webtools");
        assert_eq!(
            mcp("github", "create_issue", None).label().unwrap(),
            "mcp:github"
        );
        assert_eq!(ToolOrigin::BuiltIn.describe(), "built-in tool");
        assert_eq!(wasm("webtools", "WebFetch").describe(), "plugin webtools");
        assert_eq!(mcp("github", "x", None).describe(), "mcp github");
        assert_eq!(
            mcp("github", "x", Some("docs")).describe(),
            "mcp github (plugin docs)"
        );
    }

    #[test]
    fn collisions_are_one_line_per_requester_and_holder() {
        let echo_dup = wasm("echo-dup", "echo");
        let collision = |name: &str, origin: &ToolOrigin, existing: &ToolOrigin| Collision {
            name: name.into(),
            origin: origin.clone(),
            existing: existing.clone(),
        };
        let lines = collision_notices(&[
            collision("echo", &echo_dup, &ToolOrigin::BuiltIn),
            collision("foo", &wasm("echo-dup", "foo"), &ToolOrigin::BuiltIn),
            collision("bar", &wasm("echo-dup", "bar"), &wasm("echo", "bar")),
        ]);
        assert_eq!(
            lines,
            [
                "plugin echo-dup: 2 tools not loaded (echo, foo): names are already taken by built-in tool",
                "plugin echo-dup: 1 tool not loaded (bar): name is already taken by plugin echo",
            ]
        );
    }

    #[test]
    fn a_collision_names_mcp_requesters_and_holders_as_they_are() {
        let lines = collision_notices(&[Collision {
            name: "mcp__github__create_issue".into(),
            origin: mcp("github", "create_issue", Some("docs")),
            existing: mcp("gitlab", "create_issue", None),
        }]);
        assert_eq!(
            lines,
            ["mcp github (plugin docs): 1 tool not loaded (mcp__github__create_issue): name is already taken by mcp gitlab"]
        );
        assert!(collision_notices(&[]).is_empty());
    }

    #[test]
    fn parses_every_selector_form() {
        assert_eq!(
            ToolSelector::parse(" Read ").unwrap(),
            ToolSelector::Name("Read".into())
        );
        assert_eq!(
            ToolSelector::parse("plugin:web").unwrap(),
            ToolSelector::Plugin("web".into())
        );
        assert_eq!(
            ToolSelector::parse("plugin:web/WebFetch").unwrap(),
            ToolSelector::PluginTool {
                plugin: "web".into(),
                tool: "WebFetch".into()
            }
        );
        assert_eq!(
            ToolSelector::parse("plugin:*").unwrap(),
            ToolSelector::AnyPlugin
        );
        assert_eq!(
            ToolSelector::parse("mcp:github").unwrap(),
            ToolSelector::Mcp("github".into())
        );
        assert_eq!(
            ToolSelector::parse("mcp:github/create_issue").unwrap(),
            ToolSelector::McpTool {
                server: "github".into(),
                tool: "create_issue".into()
            }
        );
        assert_eq!(ToolSelector::parse("mcp:*").unwrap(), ToolSelector::AnyMcp);
    }

    #[test]
    fn refuses_globs_and_other_malformed_entries() {
        for bad in [
            "",
            "   ",
            "ext_*",
            "*",
            "plugin:",
            "plugin:a/",
            "plugin:*/x",
            "mcp:/t",
            "mcp:a/b/c",
            "a b",
        ] {
            assert!(
                ToolSelector::parse(bad).is_err(),
                "{bad:?} should be refused"
            );
        }
    }

    #[test]
    fn a_plugin_tool_name_never_has_the_mcp_prefix() {
        assert!(valid_plugin_tool_name("WebFetch"));
        assert!(!valid_plugin_tool_name("mcp__search__lookup"));
        assert!(!valid_plugin_tool_name("Mcp__search__lookup"));
        assert!(!valid_plugin_tool_name("mcp__"));
    }

    #[test]
    fn refuses_old_tool_names_so_they_do_not_match_nothing() {
        for old in [
            "ext_read",
            "EXT_Read",
            "ext_",
            "mcp_github",
            "Mcp_x",
            "mcp_",
        ] {
            assert!(
                matches!(
                    ToolSelector::parse(old),
                    Err(ToolSelectorError::OldName(ref name)) if name == old
                ),
                "{old:?} should be refused as an old name"
            );
        }
        let message = ToolSelector::parse("ext_read").unwrap_err().to_string();
        assert_eq!(
            message,
            "old tool name `ext_read`: use plugin:/mcp: selectors or the new tool name"
        );
        // The new names are still names.
        assert_eq!(
            ToolSelector::parse("mcp__github__x").unwrap(),
            ToolSelector::Name("mcp__github__x".into())
        );
        assert_eq!(
            ToolSelector::parse("extra").unwrap(),
            ToolSelector::Name("extra".into())
        );
    }

    #[test]
    fn name_matches_case_insensitively_on_any_origin() {
        let sel = ToolSelector::parse("webfetch").unwrap();
        assert!(sel.matches("WebFetch", &wasm("webtools", "WebFetch")));
        assert!(sel.matches("WebFetch", &ToolOrigin::BuiltIn));
        assert!(!sel.matches("WebSearch", &ToolOrigin::BuiltIn));
    }

    #[test]
    fn plugin_selectors_match_wasm_and_mcp_tools_of_that_plugin() {
        let web_fetch = wasm("webtools", "WebFetch");
        let web_mcp = mcp("search", "lookup", Some("webtools"));
        let other = wasm("docs", "Lookup");
        let p = ToolSelector::parse("plugin:webtools").unwrap();
        assert!(p.matches("WebFetch", &web_fetch));
        assert!(p.matches("lookup", &web_mcp));
        assert!(!p.matches("Lookup", &other));
        assert!(!p.matches("Read", &ToolOrigin::BuiltIn));

        let one = ToolSelector::parse("plugin:webtools/WebFetch").unwrap();
        assert!(one.matches("WebFetch", &web_fetch));
        assert!(!one.matches("WebSearch", &wasm("webtools", "WebSearch")));

        let any = ToolSelector::parse("plugin:*").unwrap();
        assert!(any.matches("WebFetch", &web_fetch) && any.matches("lookup", &web_mcp));
        assert!(!any.matches("mcp__github__x", &mcp("github", "x", None)));
    }

    #[test]
    fn mcp_selectors_match_by_server_and_server_tool_name() {
        let issue = mcp("github", "create_issue", None);
        assert!(ToolSelector::parse("mcp:github")
            .unwrap()
            .matches("mcp__github__create_issue", &issue));
        assert!(ToolSelector::parse("mcp:github/create_issue")
            .unwrap()
            .matches("mcp__github__create_issue", &issue));
        assert!(!ToolSelector::parse("mcp:github/list")
            .unwrap()
            .matches("mcp__github__create_issue", &issue));
        assert!(!ToolSelector::parse("mcp:gitlab")
            .unwrap()
            .matches("mcp__github__create_issue", &issue));
        assert!(ToolSelector::parse("mcp:*")
            .unwrap()
            .matches("mcp__github__create_issue", &issue));
        assert!(!ToolSelector::parse("mcp:*")
            .unwrap()
            .matches("WebFetch", &wasm("webtools", "WebFetch")));
    }

    #[test]
    fn plugin_wanted_and_mcp_wanted_follow_the_selector_shape() {
        let read = ToolSelector::parse("Read").unwrap();
        assert!(
            read.may_match_plugin_wasm(),
            "a bare name may be a plugin tool"
        );
        assert!(!read.may_match_mcp());
        let mcp_name = ToolSelector::parse("mcp__x__echo").unwrap();
        assert!(mcp_name.may_match_mcp());
        assert!(!mcp_name.may_match_plugin_wasm());
        assert!(ToolSelector::parse("mcp:x").unwrap().may_match_mcp());
        assert!(!ToolSelector::parse("mcp:x")
            .unwrap()
            .may_match_plugin_wasm());
        assert!(ToolSelector::parse("plugin:*")
            .unwrap()
            .may_match_plugin_wasm());
        // A plugin selector never takes an MCP tool, so it starts no MCP server.
        assert!(!ToolSelector::parse("plugin:*").unwrap().may_match_mcp());
        assert!(!ToolSelector::parse("plugin:web").unwrap().may_match_mcp());
        assert!(!ToolSelector::parse("plugin:web/WebFetch")
            .unwrap()
            .may_match_mcp());
    }
}

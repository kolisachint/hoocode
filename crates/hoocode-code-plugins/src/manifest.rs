//! Plugin manifests: Agent Plugins `plugin.json` and Claude's `.claude-plugin/plugin.json`.
//!
//! The Agent Plugins rules are hand-coded from the spec (§5). Its schemas are never fetched and
//! not parsed at run time. An unknown top-level field, or an `extensions` that is not an object,
//! is a warning. Any other violation rejects the plugin (`docs/design/plugins.md`, Details).

use serde_json::{Map, Value};

/// The `$schema` of an Agent Plugins 1.0.0 manifest. The only version this crate supports.
pub const AGENT_PLUGIN_SCHEMA: &str = "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json";

/// The top-level fields the closed Agent Plugins manifest allows (§5.2).
const MANIFEST_FIELDS: [&str; 10] = [
    "$schema",
    "name",
    "version",
    "description",
    "author",
    "homepage",
    "repository",
    "license",
    "keywords",
    "extensions",
];

/// The fields of an `author` object (§5.4).
const AUTHOR_FIELDS: [&str; 3] = ["name", "email", "url"];

/// A valid Agent Plugins manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentPluginManifest {
    pub name: String,
    /// The `description` field, or empty.
    pub description: String,
    /// The `$schema` value. The MCP file must target the same version.
    pub schema: String,
    /// Unknown top-level fields and a non-object `extensions`. Reported, then ignored.
    pub warnings: Vec<String>,
}

/// A valid Claude manifest. Only the name is read; the other fields belong to Claude.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaudeManifest {
    pub name: String,
    /// The `description` field, or empty.
    pub description: String,
}

/// The plugin name rules (§5.5): 1 to 64 characters of `a-z`, `0-9`, `-` and `.`; alphanumeric
/// at both ends; no `--` and no `..`.
pub fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 64 {
        return Err("must be 1 to 64 characters".to_owned());
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
    {
        return Err("may use only a-z, 0-9, '-' and '.'".to_owned());
    }
    let is_alnum = |b: Option<&u8>| b.is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
    if !is_alnum(name.as_bytes().first()) || !is_alnum(name.as_bytes().last()) {
        return Err("must start and end with a letter or digit".to_owned());
    }
    if name.contains("--") || name.contains("..") {
        return Err("must not contain \"--\" or \"..\"".to_owned());
    }
    Ok(())
}

fn parse_json(text: &str) -> Result<Value, String> {
    serde_json::from_str(text).map_err(|e| format!("not valid JSON: {e}"))
}

/// Parse and check an Agent Plugins `plugin.json`. The error is the reason the plugin is rejected.
pub fn parse_agent_plugin_manifest(text: &str) -> Result<AgentPluginManifest, String> {
    let value = parse_json(text)?;
    let root = value
        .as_object()
        .ok_or_else(|| "the manifest must be a JSON object".to_owned())?;

    let schema = match root.get("$schema") {
        Some(Value::String(schema)) if schema == AGENT_PLUGIN_SCHEMA => schema.clone(),
        Some(Value::String(schema)) => {
            return Err(format!(
                "unsupported $schema \"{schema}\"; this client supports Agent Plugins 1.0.0"
            ))
        }
        Some(_) => return Err("$schema must be a string".to_owned()),
        None => return Err("$schema is required".to_owned()),
    };

    let name = match root.get("name") {
        Some(Value::String(name)) => name.clone(),
        Some(_) => return Err("name must be a string".to_owned()),
        None => return Err("name is required".to_owned()),
    };
    validate_name(&name).map_err(|e| format!("name \"{name}\" {e}"))?;

    let mut warnings: Vec<String> = root
        .keys()
        .filter(|key| !MANIFEST_FIELDS.contains(&key.as_str()))
        .map(|key| format!("unknown field \"{key}\" ignored"))
        .collect();

    for field in [
        "version",
        "description",
        "homepage",
        "repository",
        "license",
    ] {
        if root.get(field).is_some_and(|v| !v.is_string()) {
            return Err(format!("{field} must be a string"));
        }
    }
    if let Some(author) = root.get("author") {
        check_author(author)?;
    }
    if let Some(keywords) = root.get("keywords") {
        let strings = keywords
            .as_array()
            .is_some_and(|items| items.iter().all(Value::is_string));
        if !strings {
            return Err("keywords must be an array of strings".to_owned());
        }
    }
    if root.get("extensions").is_some_and(|v| !v.is_object()) {
        warnings.push("extensions is not an object; ignored".to_owned());
    }

    Ok(AgentPluginManifest {
        name,
        description: description_of(root),
        schema,
        warnings,
    })
}

/// The `description` text of a manifest object, or empty (the field is checked as a string).
fn description_of(root: &Map<String, Value>) -> String {
    root.get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn check_author(author: &Value) -> Result<(), String> {
    let Some(author) = author.as_object() else {
        return Err("author must be an object".to_owned());
    };
    check_string_fields(author, &AUTHOR_FIELDS, "author")
}

fn check_string_fields(
    obj: &Map<String, Value>,
    allowed: &[&str],
    what: &str,
) -> Result<(), String> {
    for (key, value) in obj {
        if !allowed.contains(&key.as_str()) {
            return Err(format!("{what} may not contain \"{key}\""));
        }
        if !value.is_string() {
            return Err(format!("{what}.{key} must be a string"));
        }
    }
    Ok(())
}

/// Parse a Claude `.claude-plugin/plugin.json`. Its name follows the same rules as an Agent
/// Plugins name, so one name means one plugin in both layouts.
pub fn parse_claude_manifest(text: &str) -> Result<ClaudeManifest, String> {
    let value = parse_json(text)?;
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| "name is required and must be a string".to_owned())?;
    validate_name(name).map_err(|e| format!("name \"{name}\" {e}"))?;
    let description = value
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    Ok(ClaudeManifest {
        name: name.to_owned(),
        description,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_rules_follow_the_spec() {
        for ok in ["my-plugin", "acme.tools", "lint3r", "a", "x1.y-2"] {
            assert!(validate_name(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "My-Plugin",
            "-start",
            "end-",
            "has--double",
            "too.many..dots",
            ".dot",
            "under_score",
            "sp ace",
        ] {
            assert!(validate_name(bad).is_err(), "{bad}");
        }
        assert!(validate_name(&"a".repeat(64)).is_ok());
        assert!(validate_name(&"a".repeat(65)).is_err());
    }

    fn manifest(extra: &str) -> String {
        format!(r#"{{"$schema": "{AGENT_PLUGIN_SCHEMA}", "name": "demo"{extra}}}"#)
    }

    #[test]
    fn minimal_and_full_manifests_are_valid() {
        let m = parse_agent_plugin_manifest(&manifest("")).unwrap();
        assert_eq!(m.name, "demo");
        assert!(m.warnings.is_empty());

        let full = r#"{
            "$schema": "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json",
            "name": "plugin-name", "version": "1.2.0", "description": "d",
            "author": {"name": "A", "email": "a@example.com", "url": "https://example.com"},
            "homepage": "https://docs.example.com", "repository": "https://github.com/x/y",
            "license": "MIT", "keywords": ["k1", "k2"],
            "extensions": {"com.example.client": {"setting": true}}
        }"#;
        let m = parse_agent_plugin_manifest(full).unwrap();
        assert_eq!(m.name, "plugin-name");
        assert!(m.warnings.is_empty());
    }

    #[test]
    fn unknown_top_level_field_is_a_warning_not_a_rejection() {
        let m = parse_agent_plugin_manifest(&manifest(r#", "hooks": {}"#)).unwrap();
        assert_eq!(
            m.warnings,
            vec!["unknown field \"hooks\" ignored".to_owned()]
        );
    }

    #[test]
    fn non_object_extensions_is_a_warning() {
        let m = parse_agent_plugin_manifest(&manifest(r#", "extensions": []"#)).unwrap();
        assert_eq!(m.warnings.len(), 1);
    }

    #[test]
    fn extension_contents_are_not_validated() {
        let m = parse_agent_plugin_manifest(&manifest(r#", "extensions": {"a.b": 5}"#)).unwrap();
        assert!(m.warnings.is_empty());
    }

    #[test]
    fn invalid_manifests_are_rejected() {
        let cases = [
            ("not json", "{".to_owned()),
            ("not an object", "[]".to_owned()),
            (
                "no $schema",
                r#"{"name": "demo"}"#.to_owned(),
            ),
            (
                "wrong $schema version",
                r#"{"$schema": "https://agent-plugins.org/schemas/2.0.0/plugin.schema.json", "name": "demo"}"#
                    .to_owned(),
            ),
            (
                "$schema not a string",
                r#"{"$schema": 1, "name": "demo"}"#.to_owned(),
            ),
            ("no name", format!(r#"{{"$schema": "{AGENT_PLUGIN_SCHEMA}"}}"#)),
            ("name not a string", format!(r#"{{"$schema": "{AGENT_PLUGIN_SCHEMA}", "name": 3}}"#)),
            ("bad name", format!(r#"{{"$schema": "{AGENT_PLUGIN_SCHEMA}", "name": "Bad"}}"#)),
            ("empty name", format!(r#"{{"$schema": "{AGENT_PLUGIN_SCHEMA}", "name": ""}}"#)),
            ("version not a string", manifest(r#", "version": 1"#)),
            ("license not a string", manifest(r#", "license": ["MIT"]"#)),
            ("author not an object", manifest(r#", "author": "me""#)),
            ("author with extra field", manifest(r#", "author": {"name": "a", "age": 3}"#)),
            ("author field not a string", manifest(r#", "author": {"name": 4}"#)),
            ("keywords not strings", manifest(r#", "keywords": ["a", 2]"#)),
            ("keywords not an array", manifest(r#", "keywords": "a""#)),
        ];
        for (what, text) in cases {
            assert!(
                parse_agent_plugin_manifest(&text).is_err(),
                "{what} was accepted"
            );
        }
    }

    #[test]
    fn claude_manifest_needs_a_valid_name() {
        assert_eq!(
            parse_claude_manifest(r#"{"name": "demo", "version": "1"}"#)
                .unwrap()
                .name,
            "demo"
        );
        assert!(parse_claude_manifest(r#"{"version": "1"}"#).is_err());
        assert!(parse_claude_manifest(r#"{"name": "Demo"}"#).is_err());
        assert!(parse_claude_manifest("[]").is_err());
    }
}

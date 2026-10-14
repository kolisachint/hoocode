//! Conformance: one fixture per rule of Agent Plugins §7.2 and §9 (`mcp.json`).

use std::path::PathBuf;

use hoocode_code_mcp::Transport;
use hoocode_code_plugins::agent_mcp::{parse_mcp, McpContext, McpParse};
use hoocode_code_plugins::manifest::AGENT_PLUGIN_SCHEMA;
use hoocode_code_plugins::{PluginDiagnostic, Severity};

use crate::common::fixtures;

fn parse_fixture(group: &str, file: &str) -> (McpParse, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("plug");
    let data = dir.path().join("data");
    std::fs::create_dir_all(root.join("bin")).unwrap();
    std::fs::create_dir_all(&data).unwrap();
    let root = root.canonicalize().unwrap();
    let data = data.canonicalize().unwrap();
    let mcp = root.join("mcp.json");
    let text = std::fs::read_to_string(fixtures().join("mcp").join(group).join(file)).unwrap();
    let ctx = McpContext {
        plugin: "plug",
        root: &root,
        data_dir: &data,
        mcp_path: &mcp,
    };
    let out = parse_mcp(&text, AGENT_PLUGIN_SCHEMA, &ctx);
    // The tempdir lives until here; the paths in the result are only compared as text.
    drop(dir);
    (out, root)
}

fn files(group: &str) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(fixtures().join("mcp").join(group))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert!(!names.is_empty());
    names
}

fn errors(diags: &[PluginDiagnostic]) -> usize {
    diags
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .count()
}

#[test]
fn valid_mcp_files_load_without_errors() {
    for file in files("valid") {
        let (out, _) = parse_fixture("valid", &file);
        assert_eq!(errors(&out.diagnostics), 0, "{file}: {:?}", out.diagnostics);
    }
}

#[test]
fn plugin_variables_expand_in_args_env_and_cwd_only() {
    let (out, root) = parse_fixture("valid", "plugin-variables.json");
    assert_eq!(out.servers.len(), 2);
    let validator = out
        .servers
        .iter()
        .find(|s| s.name == "local-validator")
        .unwrap();
    let Transport::Stdio {
        command,
        args,
        env,
        cwd,
    } = &validator.transport
    else {
        panic!("not stdio");
    };
    assert_eq!(command, &root.join("bin/validator").to_string_lossy());
    assert!(args[1].ends_with("/validator") && !args[1].contains("${"));
    assert_eq!(env["CONFIG"], format!("{}/config.json", root.display()));
    assert_eq!(cwd.as_deref(), Some(root.to_string_lossy().as_ref()));
}

#[test]
fn sse_is_skipped_with_a_warning_only() {
    let (out, _) = parse_fixture("valid", "sse-only.json");
    assert!(out.servers.is_empty());
    assert_eq!(out.diagnostics.len(), 1);
    assert_eq!(out.diagnostics[0].severity, Severity::Warning);
}

#[test]
fn every_invalid_mcp_file_is_refused_with_an_error() {
    for file in files("invalid") {
        let (out, _) = parse_fixture("invalid", &file);
        assert!(errors(&out.diagnostics) >= 1, "{file} gave no error");
        match file.as_str() {
            // One bad server: the other one still loads.
            "command-placeholder.json" => {
                let names: Vec<&str> = out.servers.iter().map(|s| s.name.as_str()).collect();
                assert_eq!(names, ["good"], "{file}");
            }
            _ => assert!(out.servers.is_empty(), "{file} kept a server"),
        }
    }
}

#[test]
fn file_level_errors_say_mcp_is_off() {
    for file in [
        "schema-version-mismatch.json",
        "missing-schema.json",
        "unknown-top-level-field.json",
        "servers-not-object.json",
    ] {
        let (out, _) = parse_fixture("invalid", file);
        assert!(
            out.diagnostics
                .iter()
                .any(|d| d.severity == Severity::Error && d.message.starts_with("MCP is off")),
            "{file}: {:?}",
            out.diagnostics
        );
    }
}

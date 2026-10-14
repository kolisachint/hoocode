//! Conformance: one fixture per rule of Agent Plugins §5 (`plugin.json`).

use std::path::PathBuf;

use hoocode_code_plugins::manifest::parse_agent_plugin_manifest;

use crate::common::fixtures;

/// Every `*.json` in `tests/fixtures/manifests/<dir>`, sorted by name, with its text.
fn cases(dir: &str) -> Vec<(PathBuf, String)> {
    let folder = fixtures().join("manifests").join(dir);
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&folder)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no fixtures in {}", folder.display());
    paths
        .into_iter()
        .map(|p| {
            let text = std::fs::read_to_string(&p).unwrap();
            (p, text)
        })
        .collect()
}

#[test]
fn valid_manifests_are_accepted() {
    for (path, text) in cases("valid") {
        let result = parse_agent_plugin_manifest(&text);
        assert!(
            result.is_ok(),
            "{} rejected: {:?}",
            path.display(),
            result.err()
        );
    }
}

#[test]
fn every_invalid_manifest_is_rejected() {
    for (path, text) in cases("invalid") {
        let result = parse_agent_plugin_manifest(&text);
        assert!(result.is_err(), "{} was accepted", path.display());
    }
}

#[test]
fn unknown_fields_and_a_non_object_extensions_are_warnings() {
    for (path, text) in cases("warnings") {
        let manifest = parse_agent_plugin_manifest(&text)
            .unwrap_or_else(|e| panic!("{} rejected: {e}", path.display()));
        assert_eq!(manifest.warnings.len(), 1, "{}", path.display());
    }
}

#[test]
fn the_full_manifest_keeps_its_fields_and_has_no_warnings() {
    let (_, text) = cases("valid")
        .into_iter()
        .find(|(p, _)| p.ends_with("full.json"))
        .unwrap();
    let manifest = parse_agent_plugin_manifest(&text).unwrap();
    assert_eq!(manifest.name, "plugin-name");
    assert_eq!(
        manifest.schema,
        "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json"
    );
    assert!(manifest.warnings.is_empty());
}

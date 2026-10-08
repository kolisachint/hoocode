//! MCP server discovery and trust for the hoocode coding agent (milestone 4 step 2 of
//! `docs/design/mcp.md`).
//!
//! This crate reads `mcp.json` files, applies name precedence (user, then project, then
//! plugins), keeps the folder and plugin trust store under the hoocode data directory, and
//! reports each server's `/mcp` state. It does not start servers and does not depend on rmcp.
//! The MCP client (in its own crate) takes the [`DiscoveredServer`]s whose state is `None`.
//!
//! Typical use:
//!
//! ```text
//! let sources = ConfigSources::new(cwd, plugin_files);
//! let config = load(&sources);
//! let trust = TrustStore::load(TrustStore::default_path())?;
//! let discovery = discover(&config, &trust);
//! // Ask for each discovery.prompts entry; on yes:
//! TrustStore::update(&trust_path, |s| s.grant(&prompt.key, config.declared(&prompt.source)))?;
//! ```

pub mod config;
pub mod discover;
pub mod trust;

pub use config::{
    load, parse_document, ConfigSources, Diagnostic, LoadedSource, McpConfig, PluginMcpFile,
    ServerDef, Severity, Source, Transport, AGENTS_DIR_NAME, MCP_FILE_NAME,
};
pub use discover::{discover, DiscoveredServer, Discovery, ServerState, TrustPrompt};
pub use trust::{
    diff, fingerprint, ApprovedServer, ServerDiff, TrustError, TrustStatus, TrustStore,
    TRUST_FILE_NAME,
};

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        folder: PathBuf,
        sources: ConfigSources,
        trust_path: PathBuf,
    }

    fn fixture(project_text: &str, plugin_text: Option<&str>) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let folder = root.join("repo");
        let user = root.join("home/.agents/mcp.json");
        write(&user, r#"{"mcpServers": {"mine": {"command": "mine"}}}"#);
        write(&folder.join(".agents/mcp.json"), project_text);
        let mut plugins = Vec::new();
        if let Some(text) = plugin_text {
            let path = root.join("plugins/p/mcp.json");
            write(&path, text);
            plugins.push(PluginMcpFile {
                id: "p".into(),
                path,
                marketplace_trusted: false,
            });
        }
        let sources = ConfigSources {
            user,
            project_folder: folder.clone(),
            plugins,
        };
        Fixture {
            trust_path: root.join("trust/mcp-trust.json"),
            _dir: dir,
            folder,
            sources,
        }
    }

    const PROJECT: &str = r#"{"mcpServers": {
        "repo-tool": {"command": "tool", "args": ["--x"]},
        "remote": {"url": "https://example.com/mcp"},
        "off": {"command": "o", "disabled": true}
    }}"#;

    fn state_of<'a>(d: &'a Discovery, name: &str) -> Option<&'a ServerState> {
        d.servers
            .iter()
            .find(|s| s.name == name)
            .unwrap()
            .state
            .as_ref()
    }

    #[test]
    fn untrusted_project_servers_are_not_trusted_and_get_a_prompt() {
        let f = fixture(PROJECT, None);
        let config = load(&f.sources);
        let trust = TrustStore::empty(&f.trust_path);
        let d = discover(&config, &trust);

        assert_eq!(state_of(&d, "mine"), None);
        assert_eq!(state_of(&d, "repo-tool"), Some(&ServerState::NotTrusted));
        assert_eq!(state_of(&d, "remote"), Some(&ServerState::NotTrusted));
        assert_eq!(state_of(&d, "off"), Some(&ServerState::Disabled));
        assert_eq!(d.prompts.len(), 1);
        assert_eq!(d.prompts[0].status, TrustStatus::Untrusted);
        assert_eq!(d.prompts[0].key, f.folder.display().to_string());
    }

    #[test]
    fn granting_the_prompt_makes_servers_startable() {
        let f = fixture(PROJECT, None);
        let config = load(&f.sources);
        let mut trust = TrustStore::empty(&f.trust_path);
        let d = discover(&config, &trust);
        let prompt = &d.prompts[0];
        trust.grant(&prompt.key, config.declared(&prompt.source));

        let d = discover(&config, &trust);
        assert!(d.prompts.is_empty());
        assert_eq!(state_of(&d, "repo-tool"), None);
        assert_eq!(state_of(&d, "remote"), None);
        // Disabled stays disabled even when trusted.
        assert_eq!(state_of(&d, "off"), Some(&ServerState::Disabled));
    }

    #[test]
    fn a_changed_project_file_asks_again_with_the_diff() {
        let f = fixture(PROJECT, None);
        let config = load(&f.sources);
        let mut trust = TrustStore::empty(&f.trust_path);
        trust.grant(
            &f.folder.display().to_string(),
            config.declared(&config.sources[1].source),
        );

        // A git pull adds a server and changes another one.
        write(
            &f.folder.join(".agents/mcp.json"),
            r#"{"mcpServers": {
                "repo-tool": {"command": "tool", "args": ["--y"]},
                "remote": {"url": "https://example.com/mcp"},
                "off": {"command": "o", "disabled": true},
                "added": {"command": "new"}
            }}"#,
        );
        let config = load(&f.sources);
        let d = discover(&config, &trust);
        assert_eq!(d.prompts.len(), 1);
        match &d.prompts[0].status {
            TrustStatus::Changed(diff) => {
                assert_eq!(diff.added, vec!["added".to_owned()]);
                assert_eq!(diff.changed, vec!["repo-tool".to_owned()]);
                assert!(diff.removed.is_empty());
            }
            other => panic!("expected Changed, got {other:?}"),
        }
        assert_eq!(state_of(&d, "repo-tool"), Some(&ServerState::NotTrusted));
        assert_eq!(state_of(&d, "remote"), Some(&ServerState::NotTrusted));
        assert_eq!(state_of(&d, "added"), Some(&ServerState::NotTrusted));
    }

    #[test]
    fn plugins_are_trusted_per_plugin_id() {
        let plugin = r#"{"mcpServers": {"pl": {"command": "pl"}}}"#;
        let f = fixture(r#"{"mcpServers": {}}"#, Some(plugin));
        let config = load(&f.sources);
        let trust = TrustStore::empty(&f.trust_path);
        let d = discover(&config, &trust);
        assert_eq!(state_of(&d, "pl"), Some(&ServerState::NotTrusted));
        assert_eq!(d.prompts.len(), 1);
        assert_eq!(d.prompts[0].key, "plugin:p");
    }

    #[test]
    fn shadowed_only_sources_are_not_prompted() {
        // The project file only repeats the user's server name, so nothing from it runs.
        let f = fixture(r#"{"mcpServers": {"mine": {"command": "other"}}}"#, None);
        let config = load(&f.sources);
        let trust = TrustStore::empty(&f.trust_path);
        let d = discover(&config, &trust);
        assert!(d.prompts.is_empty());
        assert_eq!(d.servers.len(), 1);
        assert_eq!(d.diagnostics.len(), 1);
    }

    #[test]
    fn store_on_disk_drives_discovery_across_loads() {
        let f = fixture(PROJECT, None);
        let config = load(&f.sources);
        TrustStore::update(&f.trust_path, |s| {
            s.grant(
                &f.folder.display().to_string(),
                config.declared(&config.sources[1].source),
            )
        })
        .unwrap();
        let trust = TrustStore::load(&f.trust_path).unwrap();
        let d = discover(&config, &trust);
        assert!(d.prompts.is_empty());
    }
}

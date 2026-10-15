//! Layouts: what an Agent Plugins folder and a Claude folder contribute, and what a broken one
//! does to the rest.

use hoocode_code_mcp::Transport;
use hoocode_code_mcp::TrustStore;
use hoocode_code_plugins::{PluginFormat, PluginScope, Severity};

use crate::common::{plugin, write, Sandbox};

fn empty_trust(sb: &Sandbox) -> TrustStore {
    TrustStore::empty(&sb.trust_path)
}

#[test]
fn an_agent_plugin_contributes_a_skill_and_mcp_servers() {
    let sb = Sandbox::new();
    let trust = empty_trust(&sb);
    let set = sb.load(&trust);
    let p = plugin(&set, "agent-demo");
    assert_eq!(p.format, PluginFormat::AgentPlugins);
    assert_eq!(p.scope, PluginScope::User);
    assert_eq!(p.skill_dirs, vec![p.root.join("skills/summarize")]);

    let mut names: Vec<&str> = p.mcp_servers.iter().map(|s| s.name.as_str()).collect();
    names.sort();
    assert_eq!(names, ["remote", "validator"]);
    let validator = p
        .mcp_servers
        .iter()
        .find(|s| s.name == "validator")
        .unwrap();
    let Transport::Stdio { env, cwd, .. } = &validator.transport else {
        panic!("validator is not stdio");
    };
    assert_eq!(env["PLUGIN_ROOT"], p.root.to_string_lossy());
    assert_eq!(env["PLUGIN_DATA"], p.data_dir.to_string_lossy());
    assert_eq!(cwd.as_deref(), Some(p.root.to_string_lossy().as_ref()));

    // PLUGIN_DATA exists before any server starts, and is in the data root.
    assert!(p.data_dir.is_dir());
    assert!(p.data_dir.starts_with(sb.data_root.canonicalize().unwrap()));
    assert!(!set
        .diagnostics
        .iter()
        .any(|d| d.plugin.as_deref() == Some("agent-demo") && d.severity == Severity::Error));
}

#[test]
fn a_claude_plugin_contributes_skills_agents_and_dot_mcp_json() {
    let sb = Sandbox::new();
    let trust = empty_trust(&sb);
    let set = sb.load(&trust);
    let p = plugin(&set, "claude-demo");
    assert_eq!(p.format, PluginFormat::Claude);
    assert_eq!(p.skill_dirs, vec![p.root.join("skills/review")]);
    // Only *.md files in agents/ are agents; notes.txt is not.
    assert_eq!(p.agent_files, vec![p.root.join("agents/reviewer.md")]);
    assert_eq!(
        p.mcp_path.as_deref(),
        Some(p.root.join(".mcp.json").as_path())
    );

    let docs = p.mcp_servers.iter().find(|s| s.name == "docs").unwrap();
    assert!(
        matches!(&docs.transport, Transport::Stdio { command, .. } if command == "docs-server")
    );
    let issues = p.mcp_servers.iter().find(|s| s.name == "issues").unwrap();
    assert!(
        matches!(&issues.transport, Transport::Http { url, .. } if url.starts_with("https://"))
    );

    // A Claude plugin has no PLUGIN_DATA; nothing is created for it.
    assert!(!p.data_dir.exists());
}

#[test]
fn a_claude_plugin_can_declare_its_mcp_servers_inline() {
    let sb = Sandbox::new();
    let trust = empty_trust(&sb);
    let set = sb.load(&trust);
    let p = plugin(&set, "claude-inline");
    assert_eq!(p.mcp_servers.len(), 1);
    assert_eq!(p.mcp_servers[0].name, "inline-docs");
    assert!(p
        .mcp_path
        .as_ref()
        .unwrap()
        .ends_with(".claude-plugin/plugin.json"));
}

#[test]
fn hooks_and_other_unsupported_folders_warn_and_the_plugin_still_loads() {
    let sb = Sandbox::new();
    let trust = empty_trust(&sb);
    let set = sb.load(&trust);
    plugin(&set, "with-unsupported");
    let warned = set.diagnostics.iter().any(|d| {
        d.plugin.as_deref() == Some("with-unsupported") && d.message.starts_with("hooks/")
    });
    assert!(warned, "{:?}", set.diagnostics);
}

#[test]
fn an_invalid_manifest_rejects_only_its_own_plugin() {
    let sb = Sandbox::new();
    let trust = empty_trust(&sb);
    let set = sb.load(&trust);
    assert!(set
        .plugins
        .iter()
        .all(|p| p.name != "Rejected-Name" && p.name != "never"));
    let rejected = set
        .diagnostics
        .iter()
        .find(|d| d.severity == Severity::Error && d.path.ends_with("rejected/plugin.json"))
        .expect("a rejection diagnostic");
    assert!(
        rejected.message.starts_with("plugin rejected:"),
        "{}",
        rejected.message
    );
    // The other plugins still load.
    plugin(&set, "agent-demo");
    plugin(&set, "claude-demo");
}

#[test]
fn a_bad_mcp_file_turns_mcp_off_and_keeps_the_skills() {
    let sb = Sandbox::new();
    let trust = empty_trust(&sb);
    let set = sb.load(&trust);
    let p = plugin(&set, "bad-mcp");
    assert!(p.mcp_servers.is_empty());
    assert_eq!(p.skill_dirs, vec![p.root.join("skills/still-loads")]);
    assert!(set.diagnostics.iter().any(|d| {
        d.plugin.as_deref() == Some("bad-mcp")
            && d.severity == Severity::Error
            && d.message.starts_with("MCP is off")
    }));
}

const MANIFEST: &str =
    r#"{"$schema": "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json", "name": "linky"}"#;

/// A user folder holding one plugin named `linky`, at `<base>/<folder>/linky`.
fn user_folder_with(sb: &Sandbox, folder: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let user = sb.repo.parent().unwrap().join(folder);
    let root = user.join("linky");
    write(&root.join("plugin.json"), MANIFEST);
    (user, root)
}

#[cfg(unix)]
#[test]
fn a_skill_folder_that_links_outside_the_plugin_is_skipped() {
    let sb = Sandbox::new();
    let outside = sb.repo.parent().unwrap().join("outside-skill");
    write(
        &outside.join("SKILL.md"),
        "---\nname: outside\ndescription: outside\n---\n",
    );
    let (user, root) = user_folder_with(&sb, "linky-user");
    write(
        &root.join("skills/real/SKILL.md"),
        "---\nname: real\ndescription: real\n---\n",
    );
    std::os::unix::fs::symlink(&outside, root.join("skills/escape")).unwrap();

    let trust = empty_trust(&sb);
    let mut options = sb.options(&trust);
    options.user_plugins_dir = user;
    let set = hoocode_code_plugins::load_plugins(&options);
    let p = plugin(&set, "linky");
    assert_eq!(p.skill_dirs, vec![p.root.join("skills/real")]);
    assert!(set
        .diagnostics
        .iter()
        .any(|d| d.message.contains("outside the plugin")));
}

#[test]
fn a_skills_entry_that_is_a_file_is_an_invalid_component() {
    let sb = Sandbox::new();
    let (user, root) = user_folder_with(&sb, "filey-user");
    write(&root.join("skills"), "not a folder");

    let trust = empty_trust(&sb);
    let mut options = sb.options(&trust);
    options.user_plugins_dir = user;
    let set = hoocode_code_plugins::load_plugins(&options);
    let p = plugin(&set, "linky");
    assert!(p.skill_dirs.is_empty());
    assert!(set
        .diagnostics
        .iter()
        .any(|d| d.message.starts_with("skills is not a folder")));
}

/// Claude `.mcp.json` servers: a stdio `command` or `cwd` must stay inside the plugin, as in
/// Agent Plugins. A `./` command is run from the plugin, not from the session's folder.
#[test]
fn claude_stdio_paths_are_held_to_the_plugin_root() {
    let sb = Sandbox::new();
    let (user, root) = user_folder_with(&sb, "claude-paths-user");
    write(
        &root.join(".claude-plugin/plugin.json"),
        r#"{"name": "linky"}"#,
    );
    std::fs::remove_file(root.join("plugin.json")).unwrap();
    write(
        &root.join(".mcp.json"),
        r#"{"mcpServers": {
            "bare": {"command": "npx", "args": ["-y", "x"]},
            "local": {"command": "./server.js", "cwd": "./sub"},
            "up": {"command": "../../bin/evil"},
            "absolute": {"command": "/bin/sh"},
            "placeholder": {"command": "${CLAUDE_PLUGIN_ROOT}/server.js"},
            "cwd-up": {"command": "npx", "cwd": "../.."},
            "cwd-absolute": {"command": "npx", "cwd": "/tmp"},
            "remote": {"type": "streamable-http", "url": "https://example.com/mcp"}
        }}"#,
    );

    let trust = empty_trust(&sb);
    let mut options = sb.options(&trust);
    options.user_plugins_dir = user;
    let set = hoocode_code_plugins::load_plugins(&options);
    let p = plugin(&set, "linky");
    let mut names: Vec<&str> = p.mcp_servers.iter().map(|s| s.name.as_str()).collect();
    names.sort();
    assert_eq!(names, ["bare", "local", "remote"]);

    let local = p.mcp_servers.iter().find(|s| s.name == "local").unwrap();
    let Transport::Stdio { command, .. } = &local.transport else {
        panic!("local is not stdio");
    };
    assert_eq!(command, &p.root.join("server.js").to_string_lossy());

    let errors: Vec<&str> = set
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error && d.plugin.as_deref() == Some("linky"))
        .filter_map(|d| d.server.as_deref())
        .collect();
    assert_eq!(errors.len(), 5, "{:?}", set.diagnostics);
    for name in ["up", "absolute", "placeholder", "cwd-up", "cwd-absolute"] {
        assert!(errors.contains(&name), "{name} was not refused");
    }
}

#[cfg(unix)]
#[test]
fn a_claude_command_that_links_outside_the_plugin_is_refused() {
    let sb = Sandbox::new();
    let outside = sb.repo.parent().unwrap().join("outside-bin");
    write(&outside.join("server"), "#!/bin/sh\n");
    let (user, root) = user_folder_with(&sb, "claude-link-user");
    write(
        &root.join(".claude-plugin/plugin.json"),
        r#"{"name": "linky"}"#,
    );
    std::fs::remove_file(root.join("plugin.json")).unwrap();
    std::os::unix::fs::symlink(outside.join("server"), root.join("server")).unwrap();
    write(
        &root.join(".mcp.json"),
        r#"{"mcpServers": {"linked": {"command": "./server"}}}"#,
    );

    let trust = empty_trust(&sb);
    let mut options = sb.options(&trust);
    options.user_plugins_dir = user;
    let set = hoocode_code_plugins::load_plugins(&options);
    let p = plugin(&set, "linky");
    assert!(p.mcp_servers.is_empty());
    assert!(set
        .diagnostics
        .iter()
        .any(|d| d.server.as_deref() == Some("linked") && d.message.contains("leaves")));
}

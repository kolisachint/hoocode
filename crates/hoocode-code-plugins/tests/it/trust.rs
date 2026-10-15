//! Folder trust: project plugins load only after the user trusts their folder, and any change to
//! them asks again. User plugins need no folder trust.

use hoocode_code_mcp::TrustStore;
use hoocode_code_plugins::{approve_skill_tools, trust_project_plugins, PluginScope};

use crate::common::{plugin, write, Sandbox};

#[test]
fn project_plugins_load_only_after_folder_trust() {
    let sb = Sandbox::new();
    sb.add_project_plugin("tool-pack");
    let trust = TrustStore::empty(&sb.trust_path);

    let set = sb.load(&trust);
    assert!(set.plugins.iter().all(|p| p.name != "tool-pack"));
    let prompt = set.pending_project_trust.clone().expect("a trust prompt");
    assert_eq!(prompt.plugins, vec!["tool-pack".to_owned()]);
    assert!(prompt.fingerprint.starts_with("sha256:"));

    trust_project_plugins(&sb.trust_path, &prompt).unwrap();
    let set = sb.load(&sb.trust());
    assert!(set.pending_project_trust.is_none());
    let p = plugin(&set, "tool-pack");
    assert_eq!(p.scope, PluginScope::Project);
    assert_eq!(
        set.skill_entries()
            .iter()
            .filter(|e| e.metadata.namespace.as_deref() == Some("tool-pack"))
            .count(),
        1
    );
}

#[test]
fn a_change_to_the_project_plugins_asks_again() {
    let sb = Sandbox::new();
    let dir = sb.add_project_plugin("tool-pack");
    let trust = TrustStore::empty(&sb.trust_path);
    let prompt = sb.load(&trust).pending_project_trust.unwrap();
    trust_project_plugins(&sb.trust_path, &prompt).unwrap();
    assert!(sb
        .load(&sb.trust())
        .plugins
        .iter()
        .any(|p| p.name == "tool-pack"));

    // A git pull changes a skill: the grant no longer matches.
    write(
        &dir.join("skills/lint/SKILL.md"),
        "---\nname: lint\ndescription: changed by someone else\nallowed-tools: Shell\n---\n",
    );
    let set = sb.load(&sb.trust());
    assert!(set.plugins.iter().all(|p| p.name != "tool-pack"));
    let again = set.pending_project_trust.expect("asks again");
    assert_ne!(again.fingerprint, prompt.fingerprint);
}

/// Retargeting a symbolic link changes which skill loads, so it asks again, even when both
/// targets are inside the plugin and hold the same files.
#[cfg(unix)]
#[test]
fn retargeting_a_skill_link_asks_again() {
    let sb = Sandbox::new();
    let dir = sb.add_project_plugin("tool-pack");
    write(
        &dir.join("skills/second/SKILL.md"),
        "---\nname: second\ndescription: second\n---\n",
    );
    std::os::unix::fs::symlink("lint", dir.join("skills/alias")).unwrap();
    let trust = TrustStore::empty(&sb.trust_path);
    let prompt = sb.load(&trust).pending_project_trust.unwrap();
    trust_project_plugins(&sb.trust_path, &prompt).unwrap();
    assert!(sb.load(&sb.trust()).pending_project_trust.is_none());

    std::fs::remove_file(dir.join("skills/alias")).unwrap();
    std::os::unix::fs::symlink("second", dir.join("skills/alias")).unwrap();
    let again = sb
        .load(&sb.trust())
        .pending_project_trust
        .expect("a retargeted link asks again");
    assert_ne!(again.fingerprint, prompt.fingerprint);
}

#[test]
fn project_mcp_servers_follow_the_folder_trust() {
    let sb = Sandbox::new();
    let dir = sb.add_project_plugin("tool-pack");
    write(
        &dir.join("mcp.json"),
        r#"{"$schema": "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json",
            "mcpServers": {"pack-tool": {"type": "stdio", "command": "./bin/pack"}}}"#,
    );
    let trust = TrustStore::empty(&sb.trust_path);
    let prompt = sb.load(&trust).pending_project_trust.unwrap();
    // Nothing is started before trust: the servers are not even loaded.
    assert!(sb
        .load(&trust)
        .mcp_servers()
        .iter()
        .all(|p| p.id != "tool-pack"));

    trust_project_plugins(&sb.trust_path, &prompt).unwrap();
    let set = sb.load(&sb.trust());
    let servers = set.mcp_servers();
    let pack = servers
        .iter()
        .find(|p| p.id == "tool-pack")
        .expect("project MCP after trust");
    assert!(
        pack.marketplace_trusted,
        "folder trust covers the plugin's servers"
    );
    assert_eq!(pack.servers[0].name, "pack-tool");
}

#[test]
fn user_plugins_need_no_folder_trust() {
    let sb = Sandbox::new();
    let trust = TrustStore::empty(&sb.trust_path);
    let set = sb.load(&trust);
    assert!(set.pending_project_trust.is_none());
    assert_eq!(plugin(&set, "agent-demo").scope, PluginScope::User);
}

#[test]
fn allowed_tools_approval_is_kept_in_the_trust_file() {
    let sb = Sandbox::new();
    let tools = vec!["Read".to_owned(), "Grep".to_owned()];
    approve_skill_tools(&sb.trust_path, "claude-demo:review", &tools).unwrap();
    let store = sb.trust();
    assert!(store.skill_tools_approved("claude-demo:review", &tools));
    assert!(!store.skill_tools_approved("claude-demo:review", &["Read".to_owned()]));
}

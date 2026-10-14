//! Wiring: the loaded plugins reach the skill loader (namespaced skills, agent files) and the
//! MCP config (`hoocode-code-mcp`), through the same calls the runtime makes.

use std::sync::{Arc, Mutex};

use hoocode_code_mcp::{
    discover, load_with_plugin_servers, ConfigSources, ServerState, TrustStore,
};
use hoocode_code_plugins::PluginSet;
use hoocode_code_resources::{DefaultResourceLoader, DefaultResourceLoaderOptions};
use hoocode_code_settings::SettingsManager;

use crate::common::Sandbox;

fn load_set(sb: &Sandbox) -> PluginSet {
    sb.load(&TrustStore::empty(&sb.trust_path))
}

#[test]
fn plugin_skills_load_as_plugin_namespaced_skills_and_agents_reach_the_registry() {
    let sb = Sandbox::new();
    let set = load_set(&sb);
    let base = sb.repo.parent().unwrap().to_path_buf();
    let options = DefaultResourceLoaderOptions {
        cwd: sb.repo.to_string_lossy().into_owned(),
        agent_dir: base.join("agent").to_string_lossy().into_owned(),
        settings: Some(Arc::new(Mutex::new(SettingsManager::in_memory(
            Default::default(),
        )))),
        home: Some(base.join("home").to_string_lossy().into_owned()),
        user_agents_dir: Some(base.join("home/.agents").to_string_lossy().into_owned()),
        plugin_skill_paths: set.skill_entries(),
        plugin_agent_paths: set.agent_files(),
        ..Default::default()
    };
    let mut loader = DefaultResourceLoader::new(options);
    loader.reload();

    let names: Vec<String> = loader.skills().skills.into_iter().map(|s| s.name).collect();
    assert!(
        names.contains(&"agent-demo:summarize".to_owned()),
        "{names:?}"
    );
    assert!(
        names.contains(&"claude-demo:review".to_owned()),
        "{names:?}"
    );
    assert!(loader
        .agent_paths()
        .iter()
        .any(|p| p.ends_with("claude-demo/agents/reviewer.md")));
}

#[test]
fn plugin_mcp_servers_reach_the_mcp_config_with_their_trust() {
    let sb = Sandbox::new();
    let set = load_set(&sb);
    let base = sb.repo.parent().unwrap().to_path_buf();
    let sources = ConfigSources {
        user: base.join("none/mcp.json"),
        project_folder: sb.repo.clone(),
        plugins: Vec::new(),
    };
    let config = load_with_plugin_servers(&sources, &set.mcp_servers());
    let names: Vec<&str> = config.servers.iter().map(|s| s.name.as_str()).collect();
    for expected in ["remote", "validator", "docs", "issues", "inline-docs"] {
        assert!(
            names.contains(&expected),
            "{expected} missing from {names:?}"
        );
    }

    // Plugins from the user folder need no grant: none of their servers is "not trusted".
    let discovery = discover(&config, &TrustStore::empty(&sb.trust_path));
    let validator = discovery
        .servers
        .iter()
        .find(|s| s.name == "validator")
        .unwrap();
    assert_eq!(validator.state, None);
    assert!(discovery
        .servers
        .iter()
        .all(|s| s.state != Some(ServerState::NotTrusted)));
}

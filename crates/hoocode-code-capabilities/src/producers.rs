//! Producers: turn the session's loaded skills and subagent definitions into
//! index entries. The same loaders the session uses (`load_skills`,
//! `load_agent_registry`) run here, so the index agrees with the prompt.

use std::path::PathBuf;

use hoocode_code_plugins::{load_plugins, LoadOptions, PluginSet};
use hoocode_code_resources::agent_frontmatter::AgentDefinition;
use hoocode_code_resources::agent_registry::{load_agent_registry, LoadAgentRegistryOptions};
use hoocode_code_resources::skills::{load_skills, LoadSkillsOptions, Skill};

use crate::{CapabilityEntry, CapabilityIndex, CapabilityKind};

/// Index entries for loaded skills (`kind: skill`, source = the SKILL.md path).
pub fn skill_entries(skills: &[Skill]) -> Vec<CapabilityEntry> {
    skills
        .iter()
        .map(|skill| CapabilityEntry {
            kind: CapabilityKind::Skill,
            name: skill.name.clone(),
            description: skill.description.clone(),
            source: skill.file_path.clone(),
        })
        .collect()
}

/// Index entries for subagent definitions (`kind: subagent`). Built-in agents
/// have no file and come out with source `builtin`.
pub fn subagent_entries(agents: &[AgentDefinition]) -> Vec<CapabilityEntry> {
    agents
        .iter()
        .map(|agent| CapabilityEntry {
            kind: CapabilityKind::Subagent,
            name: agent.name.clone(),
            description: agent.description.clone(),
            source: agent
                .file_path
                .clone()
                .unwrap_or_else(|| "builtin".to_string()),
        })
        .collect()
}

/// Index entries for loaded plugins (`kind: plugin`): one per plugin that loaded, with its
/// manifest description and its folder as the source. A project plugin that is not trusted
/// has not loaded, so it is not listed.
pub fn plugin_entries(plugins: &PluginSet) -> Vec<CapabilityEntry> {
    plugins
        .plugins
        .iter()
        .map(|plugin| CapabilityEntry {
            kind: CapabilityKind::Plugin,
            name: plugin.name.clone(),
            description: plugin.description.clone(),
            source: plugin.root.to_string_lossy().into_owned(),
        })
        .collect()
}

/// Build the index for a session rooted at `cwd`, loading skills, subagents and plugins from
/// disk the way the session does.
pub fn load_index(cwd: &str) -> CapabilityIndex {
    let agent_dir = hoocode_code_paths::agent_dir()
        .to_string_lossy()
        .into_owned();
    let skills = load_skills(&LoadSkillsOptions::new(cwd, agent_dir)).skills;
    let registry = load_agent_registry(&LoadAgentRegistryOptions::new(cwd));
    let trust = hoocode_code_plugins::load_default_trust();
    let plugins = load_plugins(&LoadOptions::new(PathBuf::from(cwd), &trust));

    let mut entries = skill_entries(&skills);
    entries.extend(subagent_entries(registry.list()));
    entries.extend(plugin_entries(&plugins));
    CapabilityIndex::new(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoocode_code_resources::agent_frontmatter::AgentSource;
    use hoocode_code_resources::source_info::{SourceInfo, SourceOrigin, SourceScope};

    fn info() -> SourceInfo {
        SourceInfo {
            path: "test".into(),
            source: "test".into(),
            scope: SourceScope::User,
            origin: SourceOrigin::TopLevel,
            base_dir: None,
        }
    }

    #[test]
    fn skills_become_skill_entries_with_their_file_as_source() {
        let skill = Skill {
            name: "pdf".into(),
            description: "Read PDF files.".into(),
            file_path: "/x/pdf/SKILL.md".into(),
            base_dir: "/x/pdf".into(),
            source_info: info(),
            disable_model_invocation: false,
            allowed_tools: None,
        };
        let entries = skill_entries(&[skill]);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, CapabilityKind::Skill);
        assert_eq!(entries[0].name, "pdf");
        assert_eq!(entries[0].description, "Read PDF files.");
        assert_eq!(entries[0].source, "/x/pdf/SKILL.md");
    }

    #[test]
    fn subagents_without_a_file_are_builtin() {
        let agent = AgentDefinition {
            name: "explore".into(),
            description: "Find code.".into(),
            tools: None,
            disallowed_tools: None,
            model: None,
            prompt: String::new(),
            source: AgentSource::Builtin,
            file_path: None,
            max_turns: None,
            token_budget: None,
            background: None,
            delegate: None,
            delegate_to: None,
            fork: None,
        };
        let entries = subagent_entries(&[agent]);
        assert_eq!(entries[0].kind, CapabilityKind::Subagent);
        assert_eq!(entries[0].source, "builtin");
    }

    #[test]
    fn each_loaded_plugin_is_a_plugin_entry() {
        use hoocode_code_plugins::{LoadedPlugin, PluginFormat, PluginScope};
        let plugin = LoadedPlugin {
            name: "lint-pack".into(),
            description: "Lint helpers.".into(),
            format: PluginFormat::AgentPlugins,
            scope: PluginScope::User,
            root: "/x/lint-pack".into(),
            marketplace_trusted: true,
            data_dir: "/x/data/lint-pack".into(),
            skill_dirs: Vec::new(),
            agent_files: Vec::new(),
            mcp_path: None,
            mcp_servers: Vec::new(),
        };
        let set = PluginSet {
            plugins: vec![plugin],
            ..PluginSet::default()
        };
        let entries = plugin_entries(&set);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, CapabilityKind::Plugin);
        assert_eq!(entries[0].name, "lint-pack");
        assert_eq!(entries[0].description, "Lint helpers.");
        assert_eq!(entries[0].source, "/x/lint-pack");
        assert!(plugin_entries(&PluginSet::default()).is_empty());
    }

    #[test]
    fn a_skill_entry_is_findable_by_its_name_and_description() {
        let skill = Skill {
            name: "artifact-design".into(),
            description: "Design guidance for artifacts.".into(),
            file_path: "/x/SKILL.md".into(),
            base_dir: "/x".into(),
            source_info: info(),
            disable_model_invocation: false,
            allowed_tools: None,
        };
        let index = CapabilityIndex::new(skill_entries(&[skill]));
        assert_eq!(
            index.search("artifact design", 5)[0].name,
            "artifact-design"
        );
    }
}

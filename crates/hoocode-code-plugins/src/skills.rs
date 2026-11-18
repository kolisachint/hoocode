//! Plugin skills and their `allowed-tools`. hoocode parses the list but never honours it, so a
//! plugin skill's list is dropped with a warning (`docs/design/plugins.md`, PL-5).

use std::path::{Path, PathBuf};

use hoocode_code_resources::{LoadSkillsResult, ResourceDiagnostic, Skill};

/// Drop the `allowed-tools` of every plugin skill and warn that it is ignored. A plugin skill is
/// one whose folder is in `plugin_skill_dirs`, the folders the plugin loader found. Every other
/// skill passes through unchanged. Nothing is recorded: there is no approval to give.
pub fn ignore_allowed_tools(
    mut result: LoadSkillsResult,
    plugin_skill_dirs: &[PathBuf],
) -> LoadSkillsResult {
    let plugin_dirs: Vec<PathBuf> = plugin_skill_dirs
        .iter()
        .map(|dir| hoocode_code_paths::canonicalize_path(dir))
        .collect();
    for skill in &mut result.skills {
        if skill.allowed_tools.is_none() || !is_plugin_skill(skill, &plugin_dirs) {
            continue;
        }
        skill.allowed_tools = None;
        result.diagnostics.push(ResourceDiagnostic::warning(
            format!(
                "allowed-tools of \"{}\" is ignored: hoocode does not honour a plugin skill's allowed-tools",
                skill.name
            ),
            Some(&skill.file_path),
        ));
    }
    result
}

/// Whether the skill's folder is one of the plugin skill folders (both canonical).
fn is_plugin_skill(skill: &Skill, plugin_dirs: &[PathBuf]) -> bool {
    let base = hoocode_code_paths::canonicalize_path(Path::new(&skill.base_dir));
    plugin_dirs.contains(&base)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoocode_code_resources::source_info::{SourceInfo, SourceOrigin, SourceScope};

    /// A skill in `base`, named `name`, with `tools` as its `allowed-tools`.
    fn skill(base: &Path, name: &str, tools: Option<Vec<String>>) -> Skill {
        Skill {
            name: name.to_owned(),
            description: "d".to_owned(),
            file_path: base.join("SKILL.md").to_string_lossy().into_owned(),
            base_dir: base.to_string_lossy().into_owned(),
            source_info: SourceInfo {
                path: String::new(),
                source: "path".to_owned(),
                scope: SourceScope::User,
                origin: SourceOrigin::TopLevel,
                base_dir: None,
            },
            disable_model_invocation: false,
            allowed_tools: tools,
        }
    }

    fn result(skills: Vec<Skill>) -> LoadSkillsResult {
        LoadSkillsResult {
            skills,
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn plugin_skill_tools_are_dropped_with_a_warning_and_others_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let ship = root.join("deploy/skills/ship");
        let elsewhere = root.join("notes/plain");
        for p in [&ship, &elsewhere] {
            std::fs::create_dir_all(p).unwrap();
        }
        let tools = vec!["Read".to_owned(), "Shell".to_owned()];

        let gated = ignore_allowed_tools(
            result(vec![
                skill(&ship, "deploy:ship", Some(tools.clone())),
                skill(&elsewhere, "plain", Some(tools.clone())),
            ]),
            std::slice::from_ref(&ship),
        );
        assert_eq!(gated.skills[0].allowed_tools, None);
        // Not a plugin skill: left as it is.
        assert_eq!(gated.skills[1].allowed_tools, Some(tools));
        assert_eq!(gated.diagnostics.len(), 1);
        assert!(gated.diagnostics[0].message.contains("deploy:ship"));
        assert!(gated.diagnostics[0].message.contains("is ignored"));
    }

    #[test]
    fn a_plugin_skill_without_allowed_tools_gets_no_warning() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let ship = root.join("deploy/skills/ship");
        std::fs::create_dir_all(&ship).unwrap();

        let gated = ignore_allowed_tools(
            result(vec![skill(&ship, "deploy:ship", None)]),
            std::slice::from_ref(&ship),
        );
        assert_eq!(gated.skills[0].allowed_tools, None);
        assert!(gated.diagnostics.is_empty());
    }

    #[test]
    fn a_repository_skill_is_not_treated_as_a_plugin_skill_by_its_name() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let plugin_skill = root.join("plugin/skills/ship");
        let repo_skill = root.join("repo/.agents/skills/deploy:ship");
        std::fs::create_dir_all(&plugin_skill).unwrap();
        std::fs::create_dir_all(&repo_skill).unwrap();
        let tools = vec!["Shell".to_owned()];

        let gated = ignore_allowed_tools(
            result(vec![skill(&repo_skill, "deploy:ship", Some(tools.clone()))]),
            &[plugin_skill],
        );
        assert_eq!(gated.skills[0].allowed_tools, Some(tools));
        assert!(gated.diagnostics.is_empty());
    }

    #[test]
    fn a_plugin_skill_with_allowed_tools_loads_with_the_warning_and_records_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let base = root.join("p/skills/s");
        std::fs::create_dir_all(&base).unwrap();
        let trust_file = root.join("home/mcp-trust.json");
        let before: Vec<_> = std::fs::read_dir(&root).unwrap().collect();

        let gated = ignore_allowed_tools(
            result(vec![skill(&base, "p:s", Some(vec!["Read".to_owned()]))]),
            std::slice::from_ref(&base),
        );

        assert_eq!(gated.skills.len(), 1, "the skill still loads");
        assert_eq!(gated.skills[0].allowed_tools, None);
        assert_eq!(gated.diagnostics.len(), 1);
        assert!(gated.diagnostics[0].message.contains("ignored"));
        // No approval is stored anywhere: the gate writes no file.
        assert!(!trust_file.exists());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), before.len());
    }
}

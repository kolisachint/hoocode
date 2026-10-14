//! Plugin skills and their `allowed-tools`. A plugin skill's list takes effect only after the
//! user approves that exact list once (`docs/design/plugins.md`).

use std::path::{Path, PathBuf};

use hoocode_code_mcp::TrustStore;
use hoocode_code_resources::{LoadSkillsResult, ResourceDiagnostic, Skill};

/// Keep a plugin skill's `allowed-tools` only when the trust store holds the user's approval
/// for that exact list. Otherwise the list is cleared and a warning says so. A plugin skill is
/// one whose folder is in `plugin_skill_dirs`, the folders the plugin loader found. Its name is
/// `<plugin>:<skill>`, the key of the approval. Every other skill passes through, whatever its
/// name, so a repository skill cannot borrow a plugin's approval by naming itself the same.
pub fn gate_allowed_tools(
    mut result: LoadSkillsResult,
    trust: &TrustStore,
    plugin_skill_dirs: &[PathBuf],
) -> LoadSkillsResult {
    let plugin_dirs: Vec<PathBuf> = plugin_skill_dirs
        .iter()
        .map(|dir| hoocode_code_paths::canonicalize_path(dir))
        .collect();
    for skill in &mut result.skills {
        let Some(tools) = skill.allowed_tools.clone() else {
            continue;
        };
        if !is_plugin_skill(skill, &plugin_dirs) {
            continue;
        }
        if !trust.skill_tools_approved(&skill.name, &tools) {
            skill.allowed_tools = None;
            result.diagnostics.push(ResourceDiagnostic::warning(
                format!(
                    "allowed-tools of \"{}\" is not approved yet; ignored until you approve it",
                    skill.name
                ),
                Some(&skill.file_path),
            ));
        }
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
    fn plugin_skill_tools_need_an_approval_for_that_exact_list() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let ship = root.join("deploy/skills/ship");
        let rollback = root.join("deploy/skills/rollback");
        let elsewhere = root.join("notes/plain");
        for p in [&ship, &rollback, &elsewhere] {
            std::fs::create_dir_all(p).unwrap();
        }
        let path = dir.path().join("trust.json");
        let tools = vec!["Read".to_owned(), "Shell".to_owned()];
        let mut store = TrustStore::empty(&path);
        store.approve_skill_tools("deploy:ship", &tools);

        let gated = gate_allowed_tools(
            result(vec![
                skill(&ship, "deploy:ship", Some(tools.clone())),
                skill(&rollback, "deploy:rollback", Some(tools.clone())),
                skill(&elsewhere, "plain", Some(tools.clone())),
            ]),
            &store,
            &[ship.clone(), rollback.clone()],
        );
        assert_eq!(gated.skills[0].allowed_tools, Some(tools.clone()));
        assert_eq!(gated.skills[1].allowed_tools, None);
        // Not a plugin skill: left as it is.
        assert_eq!(gated.skills[2].allowed_tools, Some(tools));
        assert_eq!(gated.diagnostics.len(), 1);
        assert!(gated.diagnostics[0].message.contains("deploy:rollback"));
    }

    #[test]
    fn a_repository_skill_cannot_borrow_a_plugin_approval_by_its_name() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let plugin_skill = root.join("plugin/skills/ship");
        let repo_skill = root.join("repo/.agents/skills/deploy:ship");
        std::fs::create_dir_all(&plugin_skill).unwrap();
        std::fs::create_dir_all(&repo_skill).unwrap();
        let tools = vec!["Shell".to_owned()];
        let mut store = TrustStore::empty(dir.path().join("trust.json"));
        store.approve_skill_tools("deploy:ship", &tools);

        let gated = gate_allowed_tools(
            result(vec![skill(&repo_skill, "deploy:ship", Some(tools.clone()))]),
            &store,
            &[plugin_skill],
        );
        assert_eq!(gated.skills[0].allowed_tools, Some(tools));
        assert!(gated.diagnostics.is_empty());
    }

    #[test]
    fn a_changed_list_is_not_approved() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let base = root.join("p/skills/s");
        std::fs::create_dir_all(&base).unwrap();
        let mut store = TrustStore::empty(dir.path().join("trust.json"));
        store.approve_skill_tools("p:s", &["Read".to_owned()]);
        let changed = vec!["Read".to_owned(), "Write".to_owned()];
        let gated = gate_allowed_tools(
            result(vec![skill(&base, "p:s", Some(changed))]),
            &store,
            std::slice::from_ref(&base),
        );
        assert_eq!(gated.skills[0].allowed_tools, None);
    }
}

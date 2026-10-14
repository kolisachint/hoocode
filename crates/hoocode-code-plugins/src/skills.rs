//! Plugin skills and their `allowed-tools`. A plugin skill's list takes effect only after the
//! user approves that exact list once (`docs/design/plugins.md`).

use hoocode_code_mcp::TrustStore;
use hoocode_code_resources::{LoadSkillsResult, ResourceDiagnostic};

/// Keep a plugin skill's `allowed-tools` only when the trust store holds the user's approval
/// for that exact list. Otherwise the list is cleared and a warning says so. Skills that come
/// from a plugin are the ones named `<plugin>:<skill>`; every other skill passes through.
pub fn gate_allowed_tools(mut result: LoadSkillsResult, trust: &TrustStore) -> LoadSkillsResult {
    for skill in &mut result.skills {
        if !skill.name.contains(':') {
            continue;
        }
        let Some(tools) = skill.allowed_tools.clone() else {
            continue;
        };
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

#[cfg(test)]
mod tests {
    use super::*;
    use hoocode_code_resources::source_info::{SourceInfo, SourceOrigin, SourceScope};
    use hoocode_code_resources::Skill;

    fn skill(name: &str, tools: Option<Vec<String>>) -> Skill {
        Skill {
            name: name.to_owned(),
            description: "d".to_owned(),
            file_path: format!("/p/{name}/SKILL.md"),
            base_dir: "/p".to_owned(),
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

    #[test]
    fn plugin_skill_tools_need_an_approval_for_that_exact_list() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trust.json");
        let tools = vec!["Read".to_owned(), "Shell".to_owned()];
        let mut store = TrustStore::empty(&path);
        store.approve_skill_tools("deploy:ship", &tools);

        let result = LoadSkillsResult {
            skills: vec![
                skill("deploy:ship", Some(tools.clone())),
                skill("deploy:rollback", Some(tools.clone())),
                skill("plain", Some(tools.clone())),
            ],
            diagnostics: Vec::new(),
        };
        let gated = gate_allowed_tools(result, &store);
        assert_eq!(gated.skills[0].allowed_tools, Some(tools.clone()));
        assert_eq!(gated.skills[1].allowed_tools, None);
        // Not a plugin skill: left as it is.
        assert_eq!(gated.skills[2].allowed_tools, Some(tools));
        assert_eq!(gated.diagnostics.len(), 1);
        assert!(gated.diagnostics[0].message.contains("deploy:rollback"));
    }

    #[test]
    fn a_changed_list_is_not_approved() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = TrustStore::empty(dir.path().join("trust.json"));
        store.approve_skill_tools("p:s", &["Read".to_owned()]);
        let changed = vec!["Read".to_owned(), "Write".to_owned()];
        let gated = gate_allowed_tools(
            LoadSkillsResult {
                skills: vec![skill("p:s", Some(changed))],
                diagnostics: Vec::new(),
            },
            &store,
        );
        assert_eq!(gated.skills[0].allowed_tools, None);
    }
}

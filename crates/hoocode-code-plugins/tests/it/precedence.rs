//! Precedence: the first plugin found wins a name. A project plugin is found only once its
//! folder is trusted, so it wins over the user's plugin of the same name only then.

use hoocode_code_mcp::TrustStore;
use hoocode_code_plugins::{trust_project_plugins, PluginScope, Severity};

use crate::common::{plugin, write, Sandbox};

fn write_plugin(root: &std::path::Path, name: &str, skill: &str) {
    write(
        &root.join("plugin.json"),
        &format!(
            r#"{{"$schema": "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json", "name": "{name}"}}"#
        ),
    );
    write(
        &root.join("skills").join(skill).join("SKILL.md"),
        &format!("---\nname: {skill}\ndescription: from {name}\n---\n"),
    );
}

#[test]
fn a_trusted_project_plugin_wins_a_name_over_the_user_plugin() {
    let sb = Sandbox::new();
    let project = sb.project_plugins().join("shared-project");
    write_plugin(&project, "shared", "from-project");
    let user = sb.data_root.parent().unwrap().join("user-plugins");
    write_plugin(&user.join("shared-user"), "shared", "from-user");

    // Trust the project folder, then load with the trust in place.
    let trust = TrustStore::empty(&sb.trust_path);
    let prompt = sb
        .load(&trust)
        .pending_project_trust
        .expect("project plugins need trust");
    trust_project_plugins(&sb.trust_path, &prompt).unwrap();
    let granted = sb.trust();
    let mut options = sb.options(&granted);
    options.user_plugins_dir = user;
    let set = hoocode_code_plugins::load_plugins(&options);

    let winner = plugin(&set, "shared");
    assert_eq!(winner.scope, PluginScope::Project);
    assert!(winner.root.ends_with("shared-project"));
    assert_eq!(set.plugins.iter().filter(|p| p.name == "shared").count(), 1);
    assert!(set
        .diagnostics
        .iter()
        .any(|d| { d.severity == Severity::Warning && d.message.contains("already loaded from") }));
}

#[test]
fn an_untrusted_project_plugin_does_not_take_the_name() {
    let sb = Sandbox::new();
    write_plugin(
        &sb.project_plugins().join("shared-project"),
        "shared",
        "from-project",
    );
    let user = sb.data_root.parent().unwrap().join("user-plugins");
    write_plugin(&user.join("shared-user"), "shared", "from-user");

    let trust = TrustStore::empty(&sb.trust_path);
    let mut options = sb.options(&trust);
    options.user_plugins_dir = user;
    let set = hoocode_code_plugins::load_plugins(&options);

    let winner = plugin(&set, "shared");
    assert_eq!(winner.scope, PluginScope::User);
    assert!(set.pending_project_trust.is_some());
}

#[test]
fn the_first_plugin_found_wins_within_one_folder() {
    let sb = Sandbox::new();
    let user = sb.data_root.parent().unwrap().join("user-plugins");
    write_plugin(&user.join("a-copy"), "dup", "first");
    write_plugin(&user.join("b-copy"), "dup", "second");

    let trust = TrustStore::empty(&sb.trust_path);
    let mut options = sb.options(&trust);
    options.user_plugins_dir = user;
    let set = hoocode_code_plugins::load_plugins(&options);

    let winner = plugin(&set, "dup");
    assert!(winner.root.ends_with("a-copy"));
    assert!(winner.skill_dirs.iter().all(|d| d.ends_with("first")));
    assert!(set
        .diagnostics
        .iter()
        .any(|d| d.message.contains("this copy is skipped")));
}

#[test]
fn a_rejected_plugin_does_not_claim_its_name() {
    let sb = Sandbox::new();
    let user = sb.data_root.parent().unwrap().join("user-plugins");
    // Sorts first, but its $schema is unsupported, so it is rejected.
    write(
        &user.join("a-bad/plugin.json"),
        r#"{"$schema": "https://agent-plugins.org/schemas/9.0.0/plugin.schema.json", "name": "dup"}"#,
    );
    write_plugin(&user.join("b-good"), "dup", "good");

    let trust = TrustStore::empty(&sb.trust_path);
    let mut options = sb.options(&trust);
    options.user_plugins_dir = user;
    let set = hoocode_code_plugins::load_plugins(&options);

    assert!(plugin(&set, "dup").root.ends_with("b-good"));
    assert!(set
        .diagnostics
        .iter()
        .any(|d| d.severity == Severity::Error && d.message.starts_with("plugin rejected")));
}

//! `ProposePlugin` (`propose.rs`): the package it writes passes the Agent Plugins loader, nothing
//! runs until the user grants it, and a package the loader would reject is never installed.
//! Offline: a temporary folder, no marketplace.

use std::path::PathBuf;

use hoocode_code_mcp::{TrustStatus, TrustStore};
use hoocode_code_plugins::install::{self, Paths, Scope};
use hoocode_code_plugins::manifest::AGENT_PLUGIN_SCHEMA;
use hoocode_code_plugins::propose::{self, Proposal, ProposedSkill, PROPOSED_MARKETPLACE};
use hoocode_code_plugins::{PluginSet, Severity};
use serde_json::{json, Map, Value};

struct Env {
    _dir: tempfile::TempDir,
    paths: Paths,
    trust: PathBuf,
}

impl Env {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let paths = Paths {
            cwd: base.join("repo"),
            user_plugins: base.join("home/.agents/plugins"),
            data_root: base.join("home/.agents/plugin-data"),
            state: base.join("home/.hoocode"),
        };
        std::fs::create_dir_all(&paths.cwd).unwrap();
        Self {
            trust: paths.state.join("mcp-trust.json"),
            paths,
            _dir: dir,
        }
    }

    fn load(&self) -> PluginSet {
        let trust =
            TrustStore::load(&self.trust).unwrap_or_else(|_| TrustStore::empty(&self.trust));
        hoocode_code_plugins::load_plugins(&self.paths.load_options(&trust))
    }
}

fn skill(name: &str) -> ProposedSkill {
    ProposedSkill {
        name: name.to_owned(),
        description: format!("Use {name} when the task calls for it: a test skill."),
        body: format!("# {name}\n\nDo the {name} thing.\n"),
    }
}

fn servers(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

fn proposal(name: &str, skills: Vec<ProposedSkill>, mcp: Option<Map<String, Value>>) -> Proposal {
    Proposal {
        name: name.to_owned(),
        description: format!("{name} for testing."),
        skills,
        mcp_servers: mcp,
    }
}

fn echo_server() -> Map<String, Value> {
    servers(json!({"echo": {"type": "stdio", "command": "./bin/echo", "args": ["--quiet"]}}))
}

#[test]
fn a_proposal_is_an_agent_plugins_package_the_loader_accepts() {
    let env = Env::new();
    let proposed = propose::propose(
        &env.paths,
        &proposal(
            "flaky-test-triage",
            vec![skill("triage")],
            Some(echo_server()),
        ),
        Scope::User,
    )
    .unwrap();
    assert_eq!(proposed.skills, vec!["triage".to_owned()]);
    assert_eq!(proposed.mcp_servers, vec!["echo".to_owned()]);

    let root = env.paths.user_plugins.join("flaky-test-triage");
    assert_eq!(
        proposed.root,
        hoocode_code_paths::canonicalize_path(&env.paths.user_plugins).join("flaky-test-triage")
    );

    // Only the Agent Plugins layout: no Claude or Copilot folders.
    assert!(!root.join(".claude-plugin").exists());
    assert!(!root.join(".github").exists());
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("plugin.json")).unwrap()).unwrap();
    assert_eq!(manifest["$schema"], AGENT_PLUGIN_SCHEMA);
    assert_eq!(manifest["name"], "flaky-test-triage");
    // hoocode's own fields live under its namespace.
    assert_eq!(
        manifest["extensions"]["hoocode"]["origin"],
        PROPOSED_MARKETPLACE
    );
    let mcp: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("mcp.json")).unwrap()).unwrap();
    assert_eq!(
        mcp["$schema"],
        "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json"
    );
    assert!(root.join("skills/triage/SKILL.md").is_file());

    // The loader accepts it with no error, and loads the skill and the server.
    let set = env.load();
    assert!(
        set.diagnostics
            .iter()
            .all(|d| d.severity != Severity::Error),
        "{:?}",
        set.diagnostics
    );
    let plugin = set
        .plugins
        .iter()
        .find(|p| p.name == "flaky-test-triage")
        .expect("the proposed plugin loads");
    assert_eq!(plugin.skill_dirs.len(), 1);
    assert_eq!(plugin.mcp_servers.len(), 1);
    assert_eq!(plugin.description, "flaky-test-triage for testing.");
}

#[test]
fn a_proposed_mcp_server_is_held_until_the_user_grants_it() {
    let env = Env::new();
    propose::propose(
        &env.paths,
        &proposal("held", vec![skill("hold")], Some(echo_server())),
        Scope::User,
    )
    .unwrap();

    // Not from a marketplace, so the record says untrusted and the loader marks the server.
    let record = install::records(&env.paths)
        .unwrap()
        .into_iter()
        .find(|r| r.name == "held")
        .expect("a record is kept");
    assert!(!record.trusted);
    assert_eq!(record.marketplace, PROPOSED_MARKETPLACE);

    let set = env.load();
    let plugin = set.plugins.iter().find(|p| p.name == "held").unwrap();
    assert!(!plugin.marketplace_trusted);
    // The skill loads at once; the server needs a grant before it can start.
    assert_eq!(plugin.skill_dirs.len(), 1);
    let servers = set.mcp_servers();
    let held = servers.iter().find(|p| p.id == "held").unwrap();
    let store = TrustStore::empty(&env.trust);
    assert_eq!(
        store.status(&held.servers[0].source, &held.servers),
        TrustStatus::Untrusted
    );
}

#[test]
fn a_skill_only_proposal_has_no_server_to_hold() {
    let env = Env::new();
    let proposed = propose::propose(
        &env.paths,
        &proposal("passive", vec![skill("look")], None),
        Scope::User,
    )
    .unwrap();
    assert!(proposed.mcp_servers.is_empty());
    assert!(!env.paths.user_plugins.join("passive/mcp.json").exists());
    assert!(env.load().mcp_servers().is_empty());
}

#[test]
fn a_package_the_loader_would_reject_is_refused_and_leaves_nothing_behind() {
    let env = Env::new();
    let bad_server = servers(json!({"odd": {"type": "carrier-pigeon", "url": "x"}}));
    let error = propose::propose(
        &env.paths,
        &proposal("broken", vec![skill("oops")], Some(bad_server)),
        Scope::User,
    )
    .unwrap_err();
    assert!(error.contains("mcp.json"), "{error}");
    assert!(!env.paths.user_plugins.join("broken").exists());
    assert!(!env
        .paths
        .state
        .join("plugin-staging/propose-broken")
        .exists());
    assert!(install::records(&env.paths).unwrap().is_empty());
}

#[test]
fn names_and_shapes_are_checked_before_anything_is_written() {
    let env = Env::new();
    let refuse = |p: Proposal, why: &str| {
        let error = propose::propose(&env.paths, &p, Scope::User).unwrap_err();
        assert!(error.contains(why), "wanted \"{why}\": {error}");
    };
    refuse(proposal("Bad Name", vec![skill("ok")], None), "plugin name");
    refuse(
        proposal("fine", vec![skill("Bad_Skill")], None),
        "skill name",
    );
    refuse(proposal("fine", vec![skill("-lead")], None), "skill name");
    let mut no_description = skill("nodesc");
    no_description.description = "  ".into();
    refuse(
        proposal("fine", vec![no_description], None),
        "needs a description",
    );
    let mut no_body = skill("nobody");
    no_body.body = String::new();
    refuse(proposal("fine", vec![no_body], None), "has no body");
    refuse(
        proposal("fine", vec![skill("twin"), skill("twin")], None),
        "listed twice",
    );
    refuse(
        proposal("../escape", vec![skill("ok")], None),
        "plugin name",
    );
    refuse(proposal("a/b", vec![skill("ok")], None), "plugin name");
    refuse(
        proposal("fine", vec![skill("../escape")], None),
        "skill name",
    );
    refuse(proposal("fine", vec![skill("a/b")], None), "skill name");
    refuse(proposal("fine", Vec::new(), None), "nothing to propose");
    let mut blank = proposal("fine", Vec::new(), Some(Map::new()));
    blank.description = String::new();
    refuse(blank, "needs a description");

    assert!(!env.paths.user_plugins.join("fine").exists());
    assert!(install::records(&env.paths).unwrap().is_empty());
}

#[test]
fn a_name_already_installed_in_the_scope_is_refused() {
    let env = Env::new();
    let p = proposal("twice", vec![skill("once")], None);
    propose::propose(&env.paths, &p, Scope::User).unwrap();
    let error = propose::propose(&env.paths, &p, Scope::User).unwrap_err();
    assert!(error.contains("already installed"), "{error}");
}

#[test]
fn a_folder_already_there_without_a_record_is_not_overwritten() {
    let env = Env::new();
    let hand_placed = env.paths.user_plugins.join("handmade");
    std::fs::create_dir_all(&hand_placed).unwrap();
    std::fs::write(hand_placed.join("keep.txt"), "mine").unwrap();
    let error = propose::propose(
        &env.paths,
        &proposal("handmade", vec![skill("help")], None),
        Scope::User,
    )
    .unwrap_err();
    assert!(error.contains("already installed"), "{error}");
    assert_eq!(
        std::fs::read_to_string(hand_placed.join("keep.txt")).unwrap(),
        "mine"
    );
    assert!(install::records(&env.paths).unwrap().is_empty());
}

#[test]
fn update_skips_a_proposed_plugin_with_a_reason_and_leaves_it_alone() {
    let env = Env::new();
    propose::propose(
        &env.paths,
        &proposal("written", vec![skill("help")], None),
        Scope::User,
    )
    .unwrap();
    let outcomes = install::update(&env.paths, None).unwrap();
    assert_eq!(outcomes.len(), 1, "{outcomes:?}");
    match &outcomes[0] {
        install::UpdateOutcome::Skipped { name, reason } => {
            assert_eq!(name, "written");
            assert!(reason.contains("ProposePlugin"), "{reason}");
        }
        other => panic!("wanted Skipped, got {other:?}"),
    }
    assert!(env.paths.user_plugins.join("written/plugin.json").is_file());
}

#[test]
fn a_project_proposal_waits_for_folder_trust_like_any_project_plugin() {
    let env = Env::new();
    propose::propose(
        &env.paths,
        &proposal("local-helper", vec![skill("help")], Some(echo_server())),
        Scope::Project,
    )
    .unwrap();
    assert!(env
        .paths
        .cwd
        .join(".agents/plugins/local-helper/plugin.json")
        .is_file());
    let set = env.load();
    assert!(!set.plugins.iter().any(|p| p.name == "local-helper"));
    let waiting = set.pending_project_trust.expect("waits for /plugin trust");
    assert_eq!(waiting.plugins, vec!["local-helper".to_owned()]);
}

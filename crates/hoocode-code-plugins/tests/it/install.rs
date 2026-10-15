//! Install, remove, update, marketplaces and the listing, against local test marketplaces: git
//! repositories made in a temporary folder. No network.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use hoocode_code_plugins::install::{self, Paths, Scope, UpdateOutcome};
use hoocode_code_plugins::marketplace::{Registry, REGISTRY_FILE};

use crate::common::write;

/// A sandbox: a project folder, the user plugin folder, the data folders, and a trust file.
struct Env {
    _dir: tempfile::TempDir,
    paths: Paths,
    trust: PathBuf,
    /// Where the test marketplaces are made.
    origin: PathBuf,
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
            origin: base.join("origin"),
            paths,
            _dir: dir,
        }
    }

    fn set_registry(&self, text: &str) {
        write(&self.paths.state.join(REGISTRY_FILE), text);
    }

    fn load(&self) -> hoocode_code_plugins::PluginSet {
        let trust = hoocode_code_mcp::TrustStore::load(&self.trust)
            .unwrap_or_else(|_| hoocode_code_mcp::TrustStore::empty(&self.trust));
        hoocode_code_plugins::load_plugins(&self.paths.load_options(&trust))
    }
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=test",
            "-c",
            "user.email=test@example.com",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed in {}", dir.display());
}

/// One plugin's files under `root/plugins/<name>`.
fn plugin_files(root: &Path, name: &str, version: &str, with_mcp: bool) {
    plugin_folder(&root.join("plugins").join(name), name, version, with_mcp);
}

/// A plugin folder: a manifest, a skill, and optionally MCP servers.
fn plugin_folder(folder: &Path, name: &str, version: &str, with_mcp: bool) {
    write(
        &folder.join("plugin.json"),
        &format!(
            r#"{{"$schema": "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json",
              "name": "{name}", "version": "{version}", "description": "{name} v{version}"}}"#
        ),
    );
    write(
        &folder.join("skills/hello/SKILL.md"),
        &format!("---\nname: hello\ndescription: say hello from {name} {version}\n---\nHi.\n"),
    );
    if with_mcp {
        write(
            &folder.join("mcp.json"),
            r#"{"$schema": "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json",
              "mcpServers": {"echo": {"type": "stdio", "command": "./bin/echo"}}}"#,
        );
    }
}

/// A marketplace repository named `name` offering `plugins` as `(name, version, with_mcp)`.
/// Returns its path. The repository is committed, so it can be cloned.
fn marketplace_repo(origin: &Path, name: &str, plugins: &[(&str, &str, bool)]) -> PathBuf {
    let root = origin.join(name);
    let entries: Vec<String> = plugins
        .iter()
        .map(|(p, v, _)| {
            format!(r#"{{"name": "{p}", "source": "./plugins/{p}", "version": "{v}", "description": "{p}"}}"#)
        })
        .collect();
    write(
        &root.join(".claude-plugin/marketplace.json"),
        &format!(
            r#"{{"name": "{name}", "plugins": [{}]}}"#,
            entries.join(",")
        ),
    );
    for (p, v, mcp) in plugins {
        plugin_files(&root, p, v, *mcp);
    }
    git(&root, &["init", "-q"]);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "initial"]);
    root
}

#[test]
fn add_lists_the_marketplace_and_refuses_a_duplicate() {
    let env = Env::new();
    let repo = marketplace_repo(&env.origin, "local-mk", &[("tool", "1.0.0", false)]);

    let name = install::marketplace_add(&env.paths, &repo.display().to_string(), false).unwrap();
    assert_eq!(name, "local-mk");

    let listed = install::marketplace_list(&env.paths).unwrap();
    let added = listed.iter().find(|m| m.name == "local-mk").unwrap();
    assert!(!added.trusted, "a marketplace the user adds is not trusted");
    assert!(listed
        .iter()
        .any(|m| m.name == "claude-plugins-official" && m.trusted));

    let again = install::marketplace_add(&env.paths, &repo.display().to_string(), false);
    assert!(again.unwrap_err().contains("already listed"));
}

#[test]
fn a_bad_marketplace_is_refused_and_leaves_nothing_behind() {
    let env = Env::new();
    // A repository with no marketplace index.
    let not_a_marketplace = env.origin.join("plain");
    write(&not_a_marketplace.join("README.md"), "not a marketplace\n");
    git(&not_a_marketplace, &["init", "-q"]);
    git(&not_a_marketplace, &["add", "-A"]);
    git(&not_a_marketplace, &["commit", "-q", "-m", "x"]);

    let error =
        install::marketplace_add(&env.paths, &not_a_marketplace.display().to_string(), false)
            .unwrap_err();
    assert!(error.contains("cannot add"), "{error}");
    assert!(install::marketplace_list(&env.paths)
        .unwrap()
        .iter()
        .all(|m| m.name == "claude-plugins-official"));

    // An index whose name is not a valid marketplace name.
    let bad_name = env.origin.join("bad-name");
    write(
        &bad_name.join(".claude-plugin/marketplace.json"),
        r#"{"name": "Bad Name", "plugins": []}"#,
    );
    git(&bad_name, &["init", "-q"]);
    git(&bad_name, &["add", "-A"]);
    git(&bad_name, &["commit", "-q", "-m", "x"]);
    assert!(install::marketplace_add(&env.paths, &bad_name.display().to_string(), false).is_err());
    assert!(!env
        .paths
        .state
        .join("plugin-marketplace-cache/Bad Name")
        .exists());
}

#[test]
fn installs_into_user_and_project_scopes() {
    let env = Env::new();
    let repo = marketplace_repo(
        &env.origin,
        "local-mk",
        &[("tool", "1.0.0", false), ("other", "2.0.0", false)],
    );
    install::marketplace_add(&env.paths, &repo.display().to_string(), false).unwrap();

    let user = install::install(&env.paths, "tool@local-mk", Scope::User).unwrap();
    assert_eq!(user.version, "1.0.0");
    assert!(env.paths.user_plugins.join("tool/plugin.json").is_file());
    assert!(
        !user.trusted,
        "an install from an untrusted marketplace is recorded as such"
    );

    let project = install::install(&env.paths, "other", Scope::Project).unwrap();
    assert_eq!(project.scope, Scope::Project);
    assert!(env
        .paths
        .cwd
        .join(".agents/plugins/other/plugin.json")
        .is_file());

    // The user plugin loads. The project plugin waits for `/plugin trust`.
    let set = env.load();
    assert!(set.plugins.iter().any(|p| p.name == "tool"));
    assert!(!set.plugins.iter().any(|p| p.name == "other"));
    let waiting = set
        .pending_project_trust
        .expect("the project plugin waits for trust");
    assert_eq!(waiting.plugins, vec!["other".to_owned()]);

    let trusted = install::trust_project(&env.paths, &env.trust).unwrap();
    assert_eq!(trusted, Some(vec!["other".to_owned()]));
    let set = env.load();
    assert!(set.plugins.iter().any(|p| p.name == "other"));
    assert_eq!(
        install::trust_project(&env.paths, &env.trust).unwrap(),
        None
    );
}

#[test]
fn an_install_from_an_untrusted_marketplace_needs_a_grant_for_its_mcp_servers() {
    let env = Env::new();
    let repo = marketplace_repo(&env.origin, "local-mk", &[("mcpy", "1.0.0", true)]);
    install::marketplace_add(&env.paths, &repo.display().to_string(), false).unwrap();
    install::install(&env.paths, "mcpy@local-mk", Scope::User).unwrap();

    let set = env.load();
    let plugin = set.plugins.iter().find(|p| p.name == "mcpy").unwrap();
    assert!(!plugin.marketplace_trusted);
    let servers = set.mcp_servers();
    assert_eq!(servers.len(), 1);
    assert!(!servers[0].marketplace_trusted);
}

#[test]
fn an_install_from_a_trusted_marketplace_needs_no_grant() {
    let env = Env::new();
    let repo = marketplace_repo(&env.origin, "trusted-mk", &[("mcpy", "1.0.0", true)]);
    // Trust is set in the registry by hand: the official directory is the only trusted one built
    // in, and a test cannot clone it.
    env.set_registry(&format!(
        r#"{{"marketplaces": [{{"name": "trusted-mk", "url": "{}", "trusted": true, "fetchedAt": 1}}]}}"#,
        repo.display()
    ));
    let installed = install::install(&env.paths, "mcpy@trusted-mk", Scope::User).unwrap();
    assert!(installed.trusted);

    let set = env.load();
    let plugin = set.plugins.iter().find(|p| p.name == "mcpy").unwrap();
    assert!(plugin.marketplace_trusted);
    assert!(set.mcp_servers()[0].marketplace_trusted);
}

#[test]
fn a_hand_placed_plugin_keeps_its_trust() {
    let env = Env::new();
    // No install record: the plugin is trusted as before.
    plugin_folder(&env.paths.user_plugins.join("hand"), "hand", "1.0.0", true);
    let set = env.load();
    let plugin = set.plugins.iter().find(|p| p.name == "hand").unwrap();
    assert!(plugin.marketplace_trusted);
}

#[test]
fn a_missing_plugin_and_a_missing_marketplace_are_errors() {
    let env = Env::new();
    let repo = marketplace_repo(&env.origin, "local-mk", &[("tool", "1.0.0", false)]);
    install::marketplace_add(&env.paths, &repo.display().to_string(), false).unwrap();

    let missing = install::install(&env.paths, "nope@local-mk", Scope::User).unwrap_err();
    assert!(
        missing.contains("has no plugin named \"nope\""),
        "{missing}"
    );

    let no_marketplace = install::install(&env.paths, "tool@ghost", Scope::User).unwrap_err();
    assert!(
        no_marketplace.contains("no marketplace named \"ghost\""),
        "{no_marketplace}"
    );

    let bad_spec = install::install(&env.paths, "Bad Name", Scope::User).unwrap_err();
    assert!(bad_spec.contains("plugin name"), "{bad_spec}");

    // Nothing was installed.
    assert!(!env.paths.user_plugins.join("tool").exists());
}

#[test]
fn installing_twice_in_one_scope_is_refused() {
    let env = Env::new();
    let repo = marketplace_repo(&env.origin, "local-mk", &[("tool", "1.0.0", false)]);
    install::marketplace_add(&env.paths, &repo.display().to_string(), false).unwrap();
    install::install(&env.paths, "tool@local-mk", Scope::User).unwrap();
    let again = install::install(&env.paths, "tool@local-mk", Scope::User).unwrap_err();
    assert!(again.contains("already installed"), "{again}");
}

#[test]
fn remove_deletes_the_folder_and_keeps_the_data() {
    let env = Env::new();
    let repo = marketplace_repo(&env.origin, "local-mk", &[("tool", "1.0.0", false)]);
    install::marketplace_add(&env.paths, &repo.display().to_string(), false).unwrap();
    install::install(&env.paths, "tool@local-mk", Scope::User).unwrap();
    write(&env.paths.data_root.join("tool/state.txt"), "kept\n");

    let removed = install::remove(&env.paths, "tool", None).unwrap();
    assert_eq!(removed.scope, Scope::User);
    assert!(!env.paths.user_plugins.join("tool").exists());
    assert!(removed.data_dir.join("state.txt").is_file());
    assert!(install::records(&env.paths).unwrap().is_empty());

    let again = install::remove(&env.paths, "tool", None).unwrap_err();
    assert!(again.contains("not installed"), "{again}");
}

#[test]
fn remove_asks_for_a_scope_when_both_scopes_have_it() {
    let env = Env::new();
    let repo = marketplace_repo(&env.origin, "local-mk", &[("tool", "1.0.0", false)]);
    install::marketplace_add(&env.paths, &repo.display().to_string(), false).unwrap();
    install::install(&env.paths, "tool@local-mk", Scope::User).unwrap();
    install::install(&env.paths, "tool@local-mk", Scope::Project).unwrap();

    let ambiguous = install::remove(&env.paths, "tool", None).unwrap_err();
    assert!(ambiguous.contains("both scopes"), "{ambiguous}");

    install::remove(&env.paths, "tool", Some(Scope::Project)).unwrap();
    assert!(env.paths.user_plugins.join("tool").is_dir());
    assert!(!env.paths.cwd.join(".agents/plugins/tool").exists());
}

#[test]
fn update_replaces_a_changed_plugin_and_leaves_an_unchanged_one() {
    let env = Env::new();
    let repo = marketplace_repo(&env.origin, "local-mk", &[("tool", "1.0.0", false)]);
    install::marketplace_add(&env.paths, &repo.display().to_string(), false).unwrap();
    install::install(&env.paths, "tool@local-mk", Scope::User).unwrap();

    let before = install::update(&env.paths, Some("tool")).unwrap();
    assert_eq!(
        before,
        vec![UpdateOutcome::UpToDate {
            name: "tool".into()
        }]
    );

    // The marketplace publishes version 1.1.0.
    plugin_files(&repo, "tool", "1.1.0", false);
    write(
        &repo.join(".claude-plugin/marketplace.json"),
        r#"{"name": "local-mk", "plugins": [{"name": "tool", "source": "./plugins/tool", "version": "1.1.0"}]}"#,
    );
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "1.1.0"]);

    let after = install::update(&env.paths, None).unwrap();
    assert_eq!(
        after,
        vec![UpdateOutcome::Updated {
            name: "tool".into(),
            from: "1.0.0".into(),
            to: "1.1.0".into(),
        }]
    );
    let text =
        std::fs::read_to_string(env.paths.user_plugins.join("tool/skills/hello/SKILL.md")).unwrap();
    assert!(text.contains("tool 1.1.0"), "{text}");
    assert_eq!(install::records(&env.paths).unwrap()[0].version, "1.1.0");
}

#[test]
fn update_of_an_unknown_plugin_is_an_error() {
    let env = Env::new();
    let error = install::update(&env.paths, Some("ghost")).unwrap_err();
    assert!(error.contains("not installed"), "{error}");
}

#[test]
fn a_plugin_missing_from_its_marketplace_now_fails_its_update_only() {
    let env = Env::new();
    let repo = marketplace_repo(
        &env.origin,
        "local-mk",
        &[("tool", "1.0.0", false), ("keep", "1.0.0", false)],
    );
    install::marketplace_add(&env.paths, &repo.display().to_string(), false).unwrap();
    install::install(&env.paths, "tool@local-mk", Scope::User).unwrap();
    install::install(&env.paths, "keep@local-mk", Scope::User).unwrap();

    // The marketplace drops `tool`.
    write(
        &repo.join(".claude-plugin/marketplace.json"),
        r#"{"name": "local-mk", "plugins": [{"name": "keep", "source": "./plugins/keep", "version": "1.0.0"}]}"#,
    );
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "drop tool"]);

    let outcomes = install::update(&env.paths, None).unwrap();
    assert_eq!(outcomes.len(), 2);
    assert!(outcomes.iter().any(|o| matches!(
        o,
        UpdateOutcome::Failed { name, reason } if name == "tool" && reason.contains("no longer")
    )));
    assert!(
        outcomes
            .iter()
            .any(|o| matches!(o, UpdateOutcome::UpToDate { name } if name == "keep")),
        "{outcomes:?}"
    );
    assert!(
        env.paths.user_plugins.join("tool").is_dir(),
        "a failed update leaves the plugin"
    );
}

#[test]
fn a_marketplace_with_an_unsupported_entry_skips_it_with_a_warning() {
    let env = Env::new();
    let repo = marketplace_repo(&env.origin, "local-mk", &[("tool", "1.0.0", false)]);
    write(
        &repo.join(".claude-plugin/marketplace.json"),
        r#"{"name": "local-mk", "plugins": [
            {"name": "tool", "source": "./plugins/tool", "version": "1.0.0"},
            {"name": "npmy", "source": {"source": "npm", "package": "x"}}
        ]}"#,
    );
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "add npm entry"]);
    install::marketplace_add(&env.paths, &repo.display().to_string(), false).unwrap();

    let error = install::install(&env.paths, "npmy@local-mk", Scope::User).unwrap_err();
    assert!(
        error.contains("source type \"npm\" is not supported"),
        "{error}"
    );
    install::install(&env.paths, "tool@local-mk", Scope::User).unwrap();
}

#[test]
fn refresh_reports_each_marketplace() {
    let env = Env::new();
    let repo = marketplace_repo(&env.origin, "local-mk", &[("tool", "1.0.0", false)]);
    install::marketplace_add(&env.paths, &repo.display().to_string(), false).unwrap();
    let lines = install::marketplace_refresh(&env.paths, Some("local-mk")).unwrap();
    assert_eq!(lines, vec!["local-mk: 1 plugin".to_owned()]);
    let missing = install::marketplace_refresh(&env.paths, Some("ghost")).unwrap_err();
    assert!(missing.contains("no marketplace named"));
}

#[test]
fn the_listing_shows_installs_and_what_was_skipped() {
    let env = Env::new();
    let repo = marketplace_repo(&env.origin, "local-mk", &[("tool", "1.0.0", false)]);
    install::marketplace_add(&env.paths, &repo.display().to_string(), false).unwrap();
    install::install(&env.paths, "tool@local-mk", Scope::User).unwrap();
    // A hook folder is not supported and is skipped with a warning.
    write(
        &env.paths.user_plugins.join("tool/hooks/hooks.json"),
        "{}\n",
    );

    let trust = hoocode_code_mcp::TrustStore::empty(&env.trust);
    let set = hoocode_code_plugins::load_plugins(&env.paths.load_options(&trust));
    let records = install::records(&env.paths).unwrap();
    let text = install::listing(&env.paths, &set, &records);
    assert!(text.contains("local-mk (not trusted"), "{text}");
    assert!(
        text.contains("tool (user, installed from local-mk)"),
        "{text}"
    );
    assert!(text.contains("hooks/ is not supported"), "{text}");
}

#[test]
fn an_unreadable_record_file_trusts_no_marketplace_install() {
    let env = Env::new();
    write(
        &env.paths.user_plugins.join("hand/plugin.json"),
        r#"{"$schema": "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json", "name": "hand", "version": "1.0.0", "description": "x"}"#,
    );
    write(&env.paths.state.join(install::INSTALLS_FILE), "not json");

    let set = env.load();
    let plugin = set.plugins.iter().find(|p| p.name == "hand").unwrap();
    assert!(!plugin.marketplace_trusted);
    assert!(set
        .diagnostics
        .iter()
        .any(|d| d.message.contains("install records cannot be read")));

    // The registry is read on its own: it did not change.
    let registry = Registry::load(&env.paths.state.join(REGISTRY_FILE)).unwrap();
    assert!(registry.added.is_empty());
}

#[cfg(unix)]
#[test]
fn a_symlink_in_a_plugin_that_leaves_the_marketplace_refuses_the_install() {
    let env = Env::new();
    let repo = marketplace_repo(&env.origin, "sym-mk", &[("leaky", "1.0.0", false)]);
    let secret = env.origin.join("secret.txt");
    write(&secret, "private\n");
    std::os::unix::fs::symlink(&secret, repo.join("plugins/leaky/leak.md")).unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "link"]);
    install::marketplace_add(&env.paths, &repo.display().to_string(), false).unwrap();

    let error = install::install(&env.paths, "leaky@sym-mk", Scope::User).unwrap_err();
    assert!(error.contains("leaves the marketplace"), "{error}");
    assert!(!env.paths.user_plugins.join("leaky").exists());
}

#[cfg(unix)]
#[test]
fn a_git_subdir_path_that_leaves_the_repository_is_refused() {
    let env = Env::new();
    let repo = marketplace_repo(&env.origin, "sub-mk", &[]);
    write(&env.origin.join("secret/SKILL.md"), "private\n");
    write(
        &repo.join(".claude-plugin/marketplace.json"),
        &format!(
            r#"{{"name": "sub-mk", "plugins": [{{"name": "esc", "source":
              {{"source": "git-subdir", "url": "{}", "path": "../../secret"}}}}]}}"#,
            repo.display()
        ),
    );
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "esc"]);
    install::marketplace_add(&env.paths, &repo.display().to_string(), false).unwrap();

    let error = install::install(&env.paths, "esc@sub-mk", Scope::User).unwrap_err();
    assert!(error.contains("leaves the repository"), "{error}");
    assert!(!env.paths.user_plugins.join("esc").exists());
}

#[test]
fn remove_refuses_a_record_that_names_a_folder_outside_the_plugin_folders() {
    let env = Env::new();
    let outside = env.origin.join("keep");
    write(&outside.join("important.txt"), "keep me\n");
    write(
        &env.paths.state.join(install::INSTALLS_FILE),
        &format!(
            r#"{{"installs": [{{"name": "tool", "root": "{}", "scope": "user"}}]}}"#,
            outside.display()
        ),
    );

    let error = install::remove(&env.paths, "tool", None).unwrap_err();
    assert!(error.contains("left alone"), "{error}");
    assert!(outside.join("important.txt").is_file());
}

#[test]
fn add_with_trust_marks_the_marketplace_and_its_installs_trusted() {
    let env = Env::new();
    let repo = marketplace_repo(&env.origin, "mine-mk", &[("mcpy", "1.0.0", true)]);
    let name = install::marketplace_add(&env.paths, &repo.display().to_string(), true).unwrap();
    assert_eq!(name, "mine-mk");
    let registry = Registry::load(&env.paths.state.join(REGISTRY_FILE)).unwrap();
    assert!(registry.find("mine-mk").unwrap().trusted);

    let installed = install::install(&env.paths, "mcpy@mine-mk", Scope::User).unwrap();
    assert!(installed.trusted);
    let set = env.load();
    assert!(set.plugins[0].marketplace_trusted);
}

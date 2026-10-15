//! The model's marketplace calls (`model.rs`): search across marketplaces, and an install only from
//! a trusted one. Offline: the official directory is a folder seeded in the cache and recorded as
//! fetched, and the user's marketplace is a local git repository.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use hoocode_code_plugins::install::{self, Paths, Scope};
use hoocode_code_plugins::marketplace::{now_secs, OFFICIAL_NAME, OFFICIAL_URL, REGISTRY_FILE};
use hoocode_code_plugins::model;

use crate::common::write;

struct Env {
    _dir: tempfile::TempDir,
    paths: Paths,
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
            origin: base.join("origin"),
            paths,
            _dir: dir,
        }
    }

    /// The official directory, as a folder in the cache, recorded as fetched just now (so no
    /// fetch is attempted). Trusted, as the official one always is.
    fn seed_official(&self, plugins: &[(&str, &str)]) {
        let root = hoocode_code_plugins::marketplace::cache_path(&self.paths.state, OFFICIAL_NAME);
        marketplace_files(&root, OFFICIAL_NAME, plugins);
        write(
            &self.paths.state.join(REGISTRY_FILE),
            &format!(
                r#"{{"marketplaces": [{{"name": "{OFFICIAL_NAME}", "url": "{OFFICIAL_URL}", "trusted": true, "fetchedAt": {}}}]}}"#,
                now_secs()
            ),
        );
    }

    /// A user-added marketplace: a git repository, listed (so, not trusted).
    fn add_user_marketplace(&self, name: &str, plugins: &[(&str, &str)]) {
        let repo = self.origin.join(name);
        marketplace_files(&repo, name, plugins);
        git(&repo, &["init", "-q"]);
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "-m", "initial"]);
        install::marketplace_add(&self.paths, &repo.display().to_string()).unwrap();
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

/// A marketplace folder: its index and one plugin folder per `(name, description)`.
fn marketplace_files(root: &Path, name: &str, plugins: &[(&str, &str)]) {
    let entries: Vec<String> = plugins
        .iter()
        .map(|(p, description)| {
            format!(
                r#"{{"name": "{p}", "source": "./plugins/{p}", "version": "1.0.0", "description": "{description}"}}"#
            )
        })
        .collect();
    write(
        &root.join(".claude-plugin/marketplace.json"),
        &format!(
            r#"{{"name": "{name}", "plugins": [{}]}}"#,
            entries.join(",")
        ),
    );
    for (p, description) in plugins {
        let folder = root.join("plugins").join(p);
        write(
            &folder.join("plugin.json"),
            &format!(
                r#"{{"$schema": "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json", "name": "{p}", "version": "1.0.0", "description": "{description}"}}"#
            ),
        );
        write(
            &folder.join("skills/hello/SKILL.md"),
            &format!("---\nname: hello\ndescription: say hello from {p}\n---\nHi.\n"),
        );
    }
}

#[test]
fn search_matches_names_and_descriptions_across_marketplaces() {
    let env = Env::new();
    env.seed_official(&[("alpha", "formats dates for reports")]);
    env.add_user_marketplace("local-mk", &[("beta", "parses dates from logs")]);

    let (matches, notes) = model::search(&env.paths, "dates").unwrap();
    let mut names: Vec<(String, bool)> = matches
        .iter()
        .map(|f| (f.entry.name.clone(), f.trusted))
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![("alpha".to_owned(), true), ("beta".to_owned(), false)]
    );
    assert!(notes.is_empty(), "{notes:?}");

    let (all, _) = model::search(&env.paths, "").unwrap();
    assert_eq!(all.len(), 2);
    let (none, _) = model::search(&env.paths, "nothing like this").unwrap();
    assert!(none.is_empty());
}

#[test]
fn an_install_from_a_trusted_marketplace_goes_through_without_a_grant() {
    let env = Env::new();
    env.seed_official(&[("alpha", "a trusted plugin")]);

    let installed = model::install_trusted(&env.paths, "alpha", Scope::User).unwrap();
    assert_eq!(installed.marketplace, OFFICIAL_NAME);
    assert!(installed.trusted);
    assert!(env.paths.user_plugins.join("alpha/plugin.json").is_file());
    let record = install::records(&env.paths)
        .unwrap()
        .into_iter()
        .find(|r| r.name == "alpha")
        .unwrap();
    assert!(record.trusted);
}

#[test]
fn an_install_from_an_untrusted_marketplace_is_refused_and_leaves_nothing() {
    let env = Env::new();
    env.seed_official(&[]);
    env.add_user_marketplace("local-mk", &[("beta", "an untrusted plugin")]);

    let error = model::install_trusted(&env.paths, "beta", Scope::User).unwrap_err();
    assert!(error.contains("not trusted"), "{error}");
    assert!(error.contains("/plugin install beta@local-mk"), "{error}");
    assert!(!env.paths.user_plugins.join("beta").exists());
    assert!(install::records(&env.paths).unwrap().is_empty());
}

#[test]
fn a_name_in_two_marketplaces_needs_the_marketplace_named() {
    let env = Env::new();
    env.seed_official(&[("alpha", "trusted copy")]);
    env.add_user_marketplace("local-mk", &[("alpha", "untrusted copy")]);

    let error = model::install_trusted(&env.paths, "alpha", Scope::User).unwrap_err();
    assert!(error.contains("several marketplaces"), "{error}");

    // Naming the untrusted one is refused; naming the trusted one installs it.
    let untrusted = model::install_trusted(&env.paths, "alpha@local-mk", Scope::User).unwrap_err();
    assert!(untrusted.contains("not trusted"), "{untrusted}");
    let trusted =
        model::install_trusted(&env.paths, "alpha@claude-plugins-official", Scope::User).unwrap();
    assert_eq!(trusted.marketplace, OFFICIAL_NAME);
}

#[test]
fn a_missing_plugin_or_marketplace_is_an_error() {
    let env = Env::new();
    env.seed_official(&[("alpha", "x")]);
    let missing = model::install_trusted(&env.paths, "ghost", Scope::User).unwrap_err();
    assert!(
        missing.contains("no marketplace has a plugin named"),
        "{missing}"
    );
    let no_market = model::install_trusted(&env.paths, "alpha@nowhere", Scope::User).unwrap_err();
    assert!(no_market.contains("has no plugin named"), "{no_market}");
}

//! The plugin tools (`plugins.rs`) called the way the agent loop calls them, against a temporary
//! folder and local marketplaces. Offline: the official directory is a seeded folder recorded as
//! fetched, and the user's marketplace is a local git repository.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use hoocode_agent_types::AgentToolResult;
use hoocode_ai_types::Content;
use hoocode_code_plugins::install::{Paths, Scope};
use hoocode_code_plugins::marketplace::{now_secs, OFFICIAL_NAME, OFFICIAL_URL, REGISTRY_FILE};
use hoocode_code_tool_api::ToolDefinition;
use hoocode_code_tools_optin::{create_plugin_tool_definitions, PluginToolHost, PLUGIN_TOOL_NAMES};
use serde_json::{json, Value};

struct Env {
    _dir: tempfile::TempDir,
    host: Arc<PluginToolHost>,
    origin: PathBuf,
}

impl Env {
    fn new(default_scope: Scope) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let paths = Paths {
            cwd: base.join("repo"),
            user_plugins: base.join("home/.agents/plugins"),
            data_root: base.join("home/.agents/plugin-data"),
            state: base.join("home/.hoocode"),
        };
        std::fs::create_dir_all(&paths.cwd).unwrap();
        let host = Arc::new(PluginToolHost {
            trust_path: paths.state.join("mcp-trust.json"),
            paths,
            default_scope,
        });
        Self {
            origin: base.join("origin"),
            host,
            _dir: dir,
        }
    }

    fn paths(&self) -> &Paths {
        &self.host.paths
    }

    fn tool(&self, name: &str) -> ToolDefinition {
        create_plugin_tool_definitions(self.host.clone())
            .into_iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("no tool {name}"))
    }

    /// Call `name` with `args`; the text of the reply, or the error text.
    fn call(&self, name: &str, args: Value) -> Result<String, String> {
        let tool = self.tool(name);
        (tool.execute)(format!("{name}-test"), args, None, None, None)
            .map(text_of)
            .map_err(|e| e.to_string())
    }

    /// The official directory: a folder in the cache, recorded as fetched just now.
    fn seed_official(&self, plugins: &[(&str, &str)]) {
        let root =
            hoocode_code_plugins::marketplace::cache_path(&self.paths().state, OFFICIAL_NAME);
        marketplace_files(&root, OFFICIAL_NAME, plugins);
        write(
            &self.paths().state.join(REGISTRY_FILE),
            &format!(
                r#"{{"marketplaces": [{{"name": "{OFFICIAL_NAME}", "url": "{OFFICIAL_URL}", "trusted": true, "fetchedAt": {}}}]}}"#,
                now_secs()
            ),
        );
    }

    /// A marketplace the user added: a git repository, so not trusted.
    fn add_user_marketplace(&self, name: &str, plugins: &[(&str, &str)]) {
        let repo = self.origin.join(name);
        marketplace_files(&repo, name, plugins);
        git(&repo, &["init", "-q"]);
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "-m", "initial"]);
        hoocode_code_plugins::install::marketplace_add(self.paths(), &repo.display().to_string())
            .unwrap();
    }
}

fn text_of(result: AgentToolResult) -> String {
    result
        .content
        .into_iter()
        .filter_map(|c| match c {
            Content::Text(t) => Some(t.text),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
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
fn there_are_five_plugin_tools_and_none_adds_a_marketplace() {
    let env = Env::new(Scope::User);
    let names: Vec<String> = create_plugin_tool_definitions(env.host.clone())
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert_eq!(names, PLUGIN_TOOL_NAMES.to_vec());
    assert!(names
        .iter()
        .all(|n| !n.to_lowercase().contains("marketplace")));
}

#[test]
fn search_install_list_and_uninstall_work_on_a_trusted_marketplace() {
    let env = Env::new(Scope::User);
    env.seed_official(&[("alpha", "formats dates for reports")]);

    let found = env
        .call("SearchPlugins", json!({"query": "dates"}))
        .unwrap();
    assert!(
        found.contains("alpha 1.0.0 (claude-plugins-official)"),
        "{found}"
    );

    let installed = env.call("InstallPlugin", json!({"name": "alpha"})).unwrap();
    assert!(
        installed.contains("Installed alpha 1.0.0 from claude-plugins-official"),
        "{installed}"
    );
    assert!(installed.contains("/reload"), "{installed}");
    assert!(env.paths().user_plugins.join("alpha/plugin.json").is_file());

    let again = env
        .call("SearchPlugins", json!({"query": "alpha"}))
        .unwrap();
    assert!(again.contains("installed"), "{again}");

    let listing = env.call("ListPlugins", json!({})).unwrap();
    assert!(
        listing.contains("alpha (user, installed from claude-plugins-official)"),
        "{listing}"
    );

    let removed = env
        .call("UninstallPlugin", json!({"name": "alpha"}))
        .unwrap();
    assert!(removed.contains("Removed alpha (user scope)"), "{removed}");
    assert!(removed.contains("kept at"), "{removed}");
    assert!(!env.paths().user_plugins.join("alpha").exists());
}

#[test]
fn install_refuses_a_marketplace_the_user_has_not_trusted() {
    let env = Env::new(Scope::User);
    env.seed_official(&[]);
    env.add_user_marketplace("local-mk", &[("beta", "an untrusted plugin")]);

    let error = env
        .call("InstallPlugin", json!({"name": "beta"}))
        .unwrap_err();
    assert!(error.contains("not trusted"), "{error}");
    assert!(error.contains("/plugin install beta@local-mk"), "{error}");
    assert!(!env.paths().user_plugins.join("beta").exists());
}

#[test]
fn the_default_scope_applies_unless_the_call_names_one() {
    let env = Env::new(Scope::Project);
    env.seed_official(&[("alpha", "a plugin"), ("gamma", "another plugin")]);

    let project = env.call("InstallPlugin", json!({"name": "alpha"})).unwrap();
    assert!(project.contains("(project scope)"), "{project}");
    assert!(env
        .paths()
        .cwd
        .join(".agents/plugins/alpha/plugin.json")
        .is_file());

    let user = env
        .call("InstallPlugin", json!({"name": "gamma", "scope": "user"}))
        .unwrap();
    assert!(user.contains("(user scope)"), "{user}");
    assert!(env.paths().user_plugins.join("gamma/plugin.json").is_file());

    let bad = env
        .call("InstallPlugin", json!({"name": "gamma", "scope": "global"}))
        .unwrap_err();
    assert!(bad.contains("scope must be"), "{bad}");
}

#[test]
fn propose_writes_a_package_and_says_its_servers_wait_for_approval() {
    let env = Env::new(Scope::User);
    let text = env
        .call(
            "ProposePlugin",
            json!({
                "name": "flaky-test-triage",
                "description": "Triage flaky tests: find, rerun and classify them.",
                "skills": [{
                    "name": "triage",
                    "description": "Use when a test fails intermittently.",
                    "body": "Rerun the test ten times and note the failures."
                }],
                "mcpServers": {"runner": {"type": "stdio", "command": "./bin/runner"}}
            }),
        )
        .unwrap();
    assert!(text.contains("Wrote flaky-test-triage"), "{text}");
    assert!(text.contains("Skills: triage"), "{text}");
    assert!(
        text.contains("MCP servers (runner) stay off until the user approves them in /mcp"),
        "{text}"
    );
    let manifest = std::fs::read_to_string(
        env.paths()
            .user_plugins
            .join("flaky-test-triage/plugin.json"),
    )
    .unwrap();
    assert!(manifest.contains("https://agent-plugins.org/schemas/1.0.0/plugin.schema.json"));

    let listing = env.call("ListPlugins", json!({})).unwrap();
    assert!(
        listing.contains(
            "flaky-test-triage (user, installed from ProposePlugin, MCP servers need trust"
        ),
        "{listing}"
    );
}

#[test]
fn a_skill_only_proposal_reports_no_held_servers() {
    let env = Env::new(Scope::User);
    let text = env
        .call(
            "ProposePlugin",
            json!({
                "name": "passive",
                "description": "Passive guidance.",
                "skills": [{"name": "look", "description": "Look first.", "body": "Look."}]
            }),
        )
        .unwrap();
    assert!(!text.contains("stay off"), "{text}");
}

#[test]
fn missing_and_malformed_arguments_name_the_field() {
    let env = Env::new(Scope::User);
    let missing = env.call("InstallPlugin", json!({})).unwrap_err();
    assert!(missing.contains("name is required"), "{missing}");
    let blank = env
        .call("SearchPlugins", json!({"query": "  "}))
        .unwrap_err();
    assert!(blank.contains("query is required"), "{blank}");
    let skills = env
        .call(
            "ProposePlugin",
            json!({"name": "x", "description": "y", "skills": "nope"}),
        )
        .unwrap_err();
    assert!(skills.contains("skills must be an array"), "{skills}");
    let servers = env
        .call(
            "ProposePlugin",
            json!({"name": "x", "description": "y", "skills": [], "mcpServers": []}),
        )
        .unwrap_err();
    assert!(
        servers.contains("mcpServers must be an object"),
        "{servers}"
    );
}

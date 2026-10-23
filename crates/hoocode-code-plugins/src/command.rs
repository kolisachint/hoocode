//! The plugin command, shared by `/plugin` in the interactive session and by the `hoocode
//! plugin` / `install` / `remove` / `list` subcommands of the CLI. This module parses the words
//! and runs them against [`install`]; the caller shows the text and, when [`Reply::changed`] is
//! set, makes the change live (the session reloads; the CLI has nothing to reload).

use crate::install::{self, Installed, Paths, Scope, UpdateOutcome};
use crate::PluginSet;

/// What a plugin command says, and whether it changed the installed plugins.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    pub text: String,
    pub changed: bool,
}

impl Reply {
    fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            changed: false,
        }
    }
}

/// The usage line, for the command spelled `prefix` (`/plugin` or `hoocode plugin`).
pub fn usage(prefix: &str) -> String {
    format!(
        "Usage: {prefix} list | marketplace add <source> [--trust] | marketplace list | \
marketplace refresh [name] | install <name[@marketplace]> [--scope user|project] | \
remove <name> [--scope user|project] | update [name] | trust"
    )
}

/// Run one plugin command. `words` are the words after `prefix`; `current` gives the plugin set
/// `list` describes (the session's loaded set, or a fresh load from disk).
pub fn run(
    paths: &Paths,
    words: &[&str],
    prefix: &str,
    current: impl FnOnce() -> PluginSet,
) -> Result<Reply, String> {
    match words {
        [] | ["list"] => {
            let records = install::records(paths)?;
            Ok(Reply::text(install::listing(paths, &current(), &records)))
        }
        ["marketplace", "add", rest @ ..] => {
            let (source, trust) = marketplace_add_args(rest, prefix)?;
            let name = install::marketplace_add(paths, &source, trust)?;
            let grant = if trust {
                "It is trusted, so its plugins' MCP servers need no grant."
            } else {
                "It is not trusted, so its plugins' MCP servers need a grant in /mcp."
            };
            Ok(Reply::text(format!(
                "Added marketplace {name}. {grant} Install with {prefix} install <plugin>@{name}."
            )))
        }
        ["marketplace", "list"] => Ok(Reply::text(marketplace_text(paths)?)),
        ["marketplace", "refresh", rest @ ..] => {
            let lines = install::marketplace_refresh(paths, rest.first().copied())?;
            Ok(Reply::text(lines.join("\n")))
        }
        ["install", rest @ ..] => {
            let (spec, scope) = spec_and_scope(rest, prefix)?;
            let installed = install::install(paths, &spec, scope.unwrap_or(Scope::User))?;
            Ok(Reply {
                text: installed_text(&installed, prefix),
                changed: true,
            })
        }
        ["remove" | "uninstall", rest @ ..] => {
            let (name, scope) = spec_and_scope(rest, prefix)?;
            let removed = install::remove(paths, &name, scope)?;
            Ok(Reply {
                text: format!(
                    "Removed {}. Its data folder {} is kept.",
                    removed.name,
                    removed.data_dir.display()
                ),
                changed: true,
            })
        }
        ["update", rest @ ..] => {
            let outcomes = install::update(paths, rest.first().copied())?;
            let changed = outcomes
                .iter()
                .any(|o| matches!(o, UpdateOutcome::Updated { .. }));
            let lines: Vec<String> = outcomes.iter().map(update_line).collect();
            Ok(Reply {
                text: if lines.is_empty() {
                    "No plugins are installed.".to_owned()
                } else {
                    lines.join("\n")
                },
                changed,
            })
        }
        ["trust"] => match install::trust_project(paths, &install::default_trust_path())? {
            Some(names) => Ok(Reply {
                text: format!("Trusted project plugins: {}.", names.join(", ")),
                changed: true,
            }),
            None => Ok(Reply::text("No project plugins are waiting for trust.")),
        },
        _ => Err(usage(prefix)),
    }
}

/// The plugin name (or `name@marketplace` spec) and the `--scope`, in either order.
pub fn spec_and_scope(words: &[&str], prefix: &str) -> Result<(String, Option<Scope>), String> {
    let mut positional = Vec::new();
    let mut scope = None;
    let mut rest = words.iter();
    while let Some(word) = rest.next() {
        if *word == "--scope" {
            let value = rest.next().ok_or("--scope needs user or project")?;
            scope = Some(
                Scope::parse(value)
                    .ok_or_else(|| format!("unknown scope \"{value}\": use user or project"))?,
            );
        } else if word.starts_with("--") {
            return Err(format!("unknown option {word}. {}", usage(prefix)));
        } else {
            positional.push(*word);
        }
    }
    match positional.as_slice() {
        [one] => Ok(((*one).to_owned(), scope)),
        _ => Err(usage(prefix)),
    }
}

/// The source of `marketplace add <source> [--trust]`, and whether `--trust` was given.
fn marketplace_add_args(words: &[&str], prefix: &str) -> Result<(String, bool), String> {
    let mut source = None;
    let mut trust = false;
    for word in words {
        match *word {
            "--trust" => trust = true,
            flag if flag.starts_with("--") => {
                return Err(format!("unknown option {flag}. {}", usage(prefix)))
            }
            value if source.is_none() => source = Some(value.to_owned()),
            _ => return Err(usage(prefix)),
        }
    }
    source.map(|s| (s, trust)).ok_or_else(|| usage(prefix))
}

fn marketplace_text(paths: &Paths) -> Result<String, String> {
    let list = install::marketplace_list(paths)?;
    let lines: Vec<String> = list
        .iter()
        .map(|m| {
            let trust = if m.trusted { "trusted" } else { "not trusted" };
            format!("{} ({trust}): {}", m.name, m.url)
        })
        .collect();
    Ok(lines.join("\n"))
}

fn installed_text(installed: &Installed, prefix: &str) -> String {
    let mut text = format!(
        "Installed {} {} from {} in {} scope: {}.",
        installed.name,
        installed.version,
        installed.marketplace,
        installed.scope.label(),
        installed.root.display()
    );
    if !installed.trusted {
        text.push_str(
            " The marketplace is not trusted, so the plugin's MCP servers wait for a grant in /mcp.",
        );
    }
    if installed.scope == Scope::Project {
        text.push_str(&format!(" A project plugin loads after {prefix} trust."));
    }
    for note in &installed.notes {
        text.push('\n');
        text.push_str(note);
    }
    text
}

fn update_line(outcome: &UpdateOutcome) -> String {
    match outcome {
        UpdateOutcome::Updated { name, from, to } => format!("Updated {name}: {from} to {to}."),
        UpdateOutcome::UpToDate { name } => format!("{name} is up to date."),
        UpdateOutcome::Failed { name, reason } => format!("Could not update {name}: {reason}"),
        UpdateOutcome::Skipped { name, reason } => format!("Skipped {name}: {reason}."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: &str = "/plugin";

    fn run_cmd(paths: &Paths, words: &[&str]) -> Result<Reply, String> {
        run(paths, words, P, || paths.load())
    }

    #[test]
    fn spec_and_scope_takes_the_flag_in_either_place() {
        assert_eq!(
            spec_and_scope(&["tool@mk", "--scope", "project"], P).unwrap(),
            ("tool@mk".to_owned(), Some(Scope::Project))
        );
        assert_eq!(
            spec_and_scope(&["--scope", "user", "tool"], P).unwrap(),
            ("tool".to_owned(), Some(Scope::User))
        );
        assert_eq!(
            spec_and_scope(&["tool"], P).unwrap(),
            ("tool".to_owned(), None)
        );
    }

    #[test]
    fn spec_and_scope_refuses_bad_input() {
        assert!(spec_and_scope(&[], P).unwrap_err().starts_with("Usage"));
        assert!(spec_and_scope(&["a", "b"], P)
            .unwrap_err()
            .starts_with("Usage"));
        assert!(spec_and_scope(&["a", "--scope"], P)
            .unwrap_err()
            .contains("needs"));
        assert!(spec_and_scope(&["a", "--scope", "team"], P)
            .unwrap_err()
            .contains("unknown scope"));
        assert!(spec_and_scope(&["a", "--force"], P)
            .unwrap_err()
            .contains("unknown option"));
    }

    #[test]
    fn the_usage_names_the_prefix() {
        assert!(usage("hoocode plugin").starts_with("Usage: hoocode plugin list | "));
        assert!(usage("/plugin").starts_with("Usage: /plugin list | "));
    }

    #[test]
    fn an_unknown_command_shows_the_usage() {
        let paths = Paths::for_cwd(std::path::Path::new("/nonexistent-hoocode-test"));
        let error = run(&paths, &["frobnicate"], P, PluginSet::default)
            .err()
            .unwrap();
        assert!(error.starts_with("Usage: /plugin"), "{error}");
    }

    #[test]
    fn marketplace_add_takes_the_trust_flag_in_either_place() {
        assert_eq!(
            marketplace_add_args(&["owner/repo"], P).unwrap(),
            ("owner/repo".to_owned(), false)
        );
        assert_eq!(
            marketplace_add_args(&["--trust", "owner/repo"], P).unwrap(),
            ("owner/repo".to_owned(), true)
        );
        assert!(marketplace_add_args(&[], P)
            .unwrap_err()
            .starts_with("Usage"));
        assert!(marketplace_add_args(&["a", "b"], P)
            .unwrap_err()
            .starts_with("Usage"));
        assert!(marketplace_add_args(&["a", "--yes"], P)
            .unwrap_err()
            .contains("unknown option"));
    }

    /// A paths sandbox in a temporary folder.
    struct Sandbox {
        dir: tempfile::TempDir,
        paths: Paths,
    }

    fn sandbox() -> Sandbox {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let paths = Paths {
            cwd: base.join("repo"),
            user_plugins: base.join("home/.agents/plugins"),
            data_root: base.join("home/.agents/plugin-data"),
            state: base.join("home/.hoocode"),
        };
        std::fs::create_dir_all(&paths.cwd).unwrap();
        Sandbox { dir, paths }
    }

    /// A local git marketplace named `name` with one plugin, `tool`, at `dir/name`.
    fn local_marketplace(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
        let root = dir.join(name);
        let put = |path: &str, text: &str| {
            let file = root.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, text).unwrap();
        };
        put(
            ".claude-plugin/marketplace.json",
            &format!(
                r#"{{"name": "{name}", "plugins": [{{"name": "tool", "source": "./plugins/tool", "version": "1.0.0", "description": "A tool"}}]}}"#
            ),
        );
        put(
            "plugins/tool/plugin.json",
            r#"{"$schema": "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json", "name": "tool", "version": "1.0.0", "description": "A tool"}"#,
        );
        put(
            "plugins/tool/skills/hello/SKILL.md",
            "---\nname: hello\ndescription: say hello\n---\nHi.\n",
        );
        for args in [
            vec!["init", "-q"],
            vec!["add", "-A"],
            vec!["commit", "-q", "-m", "initial"],
        ] {
            let status = std::process::Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(["-c", "user.name=test", "-c", "user.email=test@example.com"])
                .args(&args)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?}");
        }
        root
    }

    #[test]
    fn list_of_an_empty_sandbox_shows_no_plugins() {
        let sandbox = sandbox();
        let reply = run_cmd(&sandbox.paths, &["list"]).unwrap();
        assert!(!reply.changed);
        assert!(reply.text.contains("Marketplaces"), "{}", reply.text);
        assert!(reply.text.contains("none installed"), "{}", reply.text);
    }

    #[test]
    fn add_install_list_and_remove_from_a_local_marketplace() {
        let sandbox = sandbox();
        let repo = local_marketplace(&sandbox.dir.path().join("origin"), "local-mk");
        let paths = &sandbox.paths;

        // Added without --trust: not trusted, and the notice says its MCP servers need a grant.
        let added = run_cmd(paths, &["marketplace", "add", &repo.display().to_string()]).unwrap();
        assert!(
            added.text.contains("Added marketplace local-mk"),
            "{}",
            added.text
        );
        assert!(added.text.contains("not trusted"), "{}", added.text);
        assert!(added.text.contains("/mcp"), "{}", added.text);
        assert!(
            added.text.contains("/plugin install <plugin>@local-mk"),
            "{}",
            added.text
        );

        let listed = run_cmd(paths, &["marketplace", "list"]).unwrap();
        assert!(
            listed.text.contains("local-mk (not trusted)"),
            "{}",
            listed.text
        );

        // The install is a change, and says what it did.
        let installed = run_cmd(paths, &["install", "tool@local-mk"]).unwrap();
        assert!(installed.changed);
        assert!(
            installed
                .text
                .contains("Installed tool 1.0.0 from local-mk"),
            "{}",
            installed.text
        );
        assert!(installed.text.contains("/mcp"), "{}", installed.text);
        assert!(paths.user_plugins.join("tool/plugin.json").is_file());

        let listing = run_cmd(paths, &["list"]).unwrap();
        assert!(listing.text.contains("tool (user"), "{}", listing.text);
        assert!(listing.text.contains("installed"), "{}", listing.text);

        // `uninstall` is another name for `remove`.
        let removed = run_cmd(paths, &["uninstall", "tool"]).unwrap();
        assert!(removed.changed);
        assert!(
            removed.text.starts_with("Removed tool."),
            "{}",
            removed.text
        );
        assert!(!paths.user_plugins.join("tool").exists());
    }

    #[test]
    fn add_with_trust_says_the_marketplace_is_trusted() {
        let sandbox = sandbox();
        let repo = local_marketplace(&sandbox.dir.path().join("origin"), "mine-mk");
        let added = run_cmd(
            &sandbox.paths,
            &["marketplace", "add", &repo.display().to_string(), "--trust"],
        )
        .unwrap();
        assert!(added.text.contains("It is trusted"), "{}", added.text);
        assert!(!added.text.contains("not trusted"), "{}", added.text);
        let listed = run_cmd(&sandbox.paths, &["marketplace", "list"]).unwrap();
        assert!(listed.text.contains("mine-mk (trusted)"), "{}", listed.text);
    }
}

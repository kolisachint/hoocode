//! `/plugin`: the listing, marketplaces, installs, removes, updates and trust (part 2 of
//! `docs/design/plugins.md`). The work is in `hoocode-code-plugins`; this file parses the command,
//! and makes a change live: the MCP servers of the new plugin set start, and the resources reload
//! (the `/reload` path). A change during a turn waits for the turn to end.

use std::sync::Arc;

use hoocode_code_agent_session::mcp::McpHub;
use hoocode_code_plugins::install::{self, Installed, Paths, Scope, UpdateOutcome};
use hoocode_tui_components::{Spacer, Text};

use super::*;

const USAGE: &str = "Usage: /plugin list | marketplace add <source> | marketplace list | \
marketplace refresh [name] | install <name[@marketplace]> [--scope user|project] | \
remove <name> [--scope user|project] | update [name] | trust";

/// What a `/plugin` command says, and whether it changed the installed plugins.
struct Reply {
    text: String,
    changed: bool,
}

impl Reply {
    fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            changed: false,
        }
    }
}

impl Mode {
    /// `/plugin <command>`: run it, then show what it said. A change is made live (see
    /// [`Mode::apply_plugin_change`]).
    pub(super) fn handle_plugin_command(&mut self, command: &str) {
        let args = command
            .trim()
            .trim_start_matches('/')
            .strip_prefix("plugin")
            .unwrap_or("")
            .trim();
        let words: Vec<&str> = args.split_whitespace().collect();
        let paths = Paths::for_cwd(self.session.cwd());
        match run_plugin_command(&paths, &words) {
            Ok(reply) if reply.changed => {
                // The reload replays the transcript, so the text is shown after it.
                self.plugin_notices.push(reply.text);
                self.apply_plugin_change();
            }
            Ok(reply) => self.show_plugin_text(&reply.text),
            Err(error) => self.show_warning(&error),
        }
    }

    /// Make a change live now when idle, else when the turn ends.
    fn apply_plugin_change(&mut self) {
        if self.session.is_streaming() || self.session.is_compacting() {
            self.plugin_reload_pending = true;
            self.show_status("Plugin changes take effect when this turn ends.");
            return;
        }
        self.reload_plugins();
    }

    /// Reload the plugins: the current set, then the MCP servers of its plugins (only when this
    /// session runs MCP), then the resources. Skills and subagents are live from the next turn.
    pub(super) fn reload_plugins(&mut self) {
        let cwd = self.session.cwd().to_path_buf();
        let set = Paths::for_cwd(&cwd).load();
        hoocode_code_plugins::set_current(set.clone());
        if let Some(old) = self.session.mcp() {
            let hub = Arc::new(McpHub::for_folder_with_plugins(&cwd, &set.mcp_servers()));
            hub.set_elicitation(Arc::new(crate::mcp_elicitation::TuiElicitation));
            self.session.attach_mcp(hub.clone(), true);
            hub.start();
            old.shutdown();
            self.ask_mcp_trust();
        }
        self.handle_reload_command();
    }

    /// The notices of `/plugin` changes, shown after a reload.
    pub(super) fn show_plugin_notices(&mut self) {
        for notice in std::mem::take(&mut self.plugin_notices) {
            self.show_record(&notice);
        }
    }

    fn show_plugin_text(&mut self, text: &str) {
        self.add_to_chat(as_component(&handle(Spacer::new(1))));
        self.add_to_chat(as_component(&handle(Text::new(text.to_owned(), 1, 0))));
    }
}

/// Run one `/plugin` command. `words` are the words after `/plugin`.
fn run_plugin_command(paths: &Paths, words: &[&str]) -> Result<Reply, String> {
    match words {
        [] | ["list"] => {
            let records = install::records(paths)?;
            let set = hoocode_code_plugins::current();
            Ok(Reply::text(install::listing(paths, &set, &records)))
        }
        ["marketplace", "add", source] => {
            let name = install::marketplace_add(paths, source)?;
            Ok(Reply::text(format!(
                "Added marketplace {name}. It is not trusted, so its plugins' MCP servers need a grant in /mcp. Install with /plugin install <plugin>@{name}."
            )))
        }
        ["marketplace", "list"] => Ok(Reply::text(marketplace_text(paths)?)),
        ["marketplace", "refresh", rest @ ..] => {
            let lines = install::marketplace_refresh(paths, rest.first().copied())?;
            Ok(Reply::text(lines.join("\n")))
        }
        ["install", rest @ ..] => {
            let (spec, scope) = spec_and_scope(rest)?;
            let installed = install::install(paths, &spec, scope.unwrap_or(Scope::User))?;
            Ok(Reply {
                text: installed_text(&installed),
                changed: true,
            })
        }
        ["remove", rest @ ..] => {
            let (name, scope) = spec_and_scope(rest)?;
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
        _ => Err(USAGE.to_owned()),
    }
}

/// The plugin name (or `name@marketplace` spec) and the `--scope`, in either order.
fn spec_and_scope(words: &[&str]) -> Result<(String, Option<Scope>), String> {
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
            return Err(format!("unknown option {word}. {USAGE}"));
        } else {
            positional.push(*word);
        }
    }
    match positional.as_slice() {
        [one] => Ok(((*one).to_owned(), scope)),
        _ => Err(USAGE.to_owned()),
    }
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

fn installed_text(installed: &Installed) -> String {
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
        text.push_str(" A project plugin loads after /plugin trust.");
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_and_scope_takes_the_flag_in_either_place() {
        assert_eq!(
            spec_and_scope(&["tool@mk", "--scope", "project"]).unwrap(),
            ("tool@mk".to_owned(), Some(Scope::Project))
        );
        assert_eq!(
            spec_and_scope(&["--scope", "user", "tool"]).unwrap(),
            ("tool".to_owned(), Some(Scope::User))
        );
        assert_eq!(
            spec_and_scope(&["tool"]).unwrap(),
            ("tool".to_owned(), None)
        );
    }

    #[test]
    fn spec_and_scope_refuses_bad_input() {
        assert!(spec_and_scope(&[]).unwrap_err().starts_with("Usage"));
        assert!(spec_and_scope(&["a", "b"])
            .unwrap_err()
            .starts_with("Usage"));
        assert!(spec_and_scope(&["a", "--scope"])
            .unwrap_err()
            .contains("needs"));
        assert!(spec_and_scope(&["a", "--scope", "team"])
            .unwrap_err()
            .contains("unknown scope"));
        assert!(spec_and_scope(&["a", "--force"])
            .unwrap_err()
            .contains("unknown option"));
    }

    #[test]
    fn an_unknown_command_shows_the_usage() {
        let paths = Paths::for_cwd(std::path::Path::new("/nonexistent-hoocode-test"));
        let error = run_plugin_command(&paths, &["frobnicate"]).err().unwrap();
        assert!(error.starts_with("Usage"), "{error}");
    }
}

//! `hoocode plugin <command>` and its shortcuts `hoocode install`, `remove`, `uninstall` and
//! `list`: the `/plugin` command of the interactive session, run from the shell. The command
//! itself is `hoocode_code_plugins::command`; this file maps the subcommand to its words and
//! prints the reply. A running session picks up the change on `/reload` or its next start.

use std::io::Write;
use std::path::Path;

use hoocode_code_plugins::command;
use hoocode_code_plugins::install::Paths;

use crate::red;

/// How the full command is spelled, for its usage line and hints.
const PREFIX: &str = "hoocode plugin";

/// The subcommands this module answers.
pub(crate) const PLUGIN_SUBCOMMANDS: [&str; 5] =
    ["plugin", "install", "remove", "uninstall", "list"];

/// `hoocode plugin --help` (and `hoocode install --help`, ...).
pub(crate) const PLUGIN_HELP: &str = "\
Usage:
  hoocode install <name[@marketplace]> [--scope user|project]   Install a plugin
  hoocode remove <name> [--scope user|project]                  Remove a plugin (alias: uninstall)
  hoocode list                                                  List marketplaces and plugins
  hoocode plugin marketplace add <source> [--trust]             Add a git marketplace
  hoocode plugin marketplace list                               List marketplaces
  hoocode plugin marketplace refresh [name]                     Fetch marketplaces again
  hoocode plugin update [name]                                  Update installed plugins
  hoocode plugin trust                                          Trust this project's plugins

Plugins are Agent Plugins or Claude plugin folders that add skills, subagents and MCP
servers. User plugins go in ~/.agents/plugins (the default scope); project plugins go
in .agents/plugins and load once the project is trusted. Inside a session the same
command is /plugin, and the change is live at once; a session that is already running
picks up a change made here on /reload.
";

/// Run `argv` (its first word is one of [`PLUGIN_SUBCOMMANDS`]). Returns the exit code.
pub(crate) fn run(
    argv: &[String],
    cwd: &Path,
    out: &mut dyn Write,
    err: &mut dyn Write,
    color: bool,
) -> i32 {
    run_with(argv, &Paths::for_cwd(cwd), out, err, color)
}

pub(crate) fn run_with(
    argv: &[String],
    paths: &Paths,
    out: &mut dyn Write,
    err: &mut dyn Write,
    color: bool,
) -> i32 {
    let Some((first, rest)) = argv.split_first() else {
        return 2;
    };
    let rest: Vec<&str> = rest.iter().map(String::as_str).collect();
    if rest.iter().any(|w| *w == "--help" || *w == "-h") {
        let _ = write!(out, "{PLUGIN_HELP}");
        return 0;
    }
    // `hoocode install x` is `hoocode plugin install x`, and so on.
    let words: Vec<&str> = if first == "plugin" {
        rest
    } else {
        std::iter::once(first.as_str()).chain(rest).collect()
    };
    match command::run(paths, &words, PREFIX, || paths.load()) {
        Ok(reply) => {
            let _ = writeln!(out, "{}", reply.text);
            0
        }
        Err(error) => {
            let _ = writeln!(err, "{}", red(color, &format!("Error: {error}")));
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sandbox() -> (tempfile::TempDir, Paths) {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().to_path_buf();
        let paths = Paths {
            cwd: base.join("repo"),
            user_plugins: base.join("home/.agents/plugins"),
            data_root: base.join("home/.agents/plugin-data"),
            state: base.join("home/.hoocode"),
        };
        std::fs::create_dir_all(&paths.cwd).unwrap();
        (dir, paths)
    }

    fn call(paths: &Paths, argv: &[&str]) -> (i32, String, String) {
        let argv: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run_with(&argv, paths, &mut out, &mut err, false);
        (
            code,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }

    #[test]
    fn list_prints_the_listing() {
        let (_dir, paths) = sandbox();
        let (code, out, err) = call(&paths, &["list"]);
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("Marketplaces"), "{out}");
        assert!(out.contains("none installed"), "{out}");
    }

    #[test]
    fn plugin_list_is_the_same_command() {
        let (_dir, paths) = sandbox();
        assert_eq!(
            call(&paths, &["plugin", "list"]).1,
            call(&paths, &["list"]).1
        );
        assert_eq!(call(&paths, &["plugin"]).1, call(&paths, &["list"]).1);
    }

    #[test]
    fn help_after_any_subcommand_prints_the_plugin_help() {
        let (_dir, paths) = sandbox();
        for argv in [
            &["install", "--help"][..],
            &["remove", "-h"],
            &["list", "--help"],
            &["plugin", "--help"],
        ] {
            let (code, out, _) = call(&paths, argv);
            assert_eq!(code, 0);
            assert_eq!(out, PLUGIN_HELP, "{argv:?}");
        }
    }

    #[test]
    fn errors_go_to_stderr_with_the_usage() {
        let (_dir, paths) = sandbox();
        let (code, out, err) = call(&paths, &["install"]);
        assert_eq!(code, 1);
        assert!(out.is_empty());
        assert!(
            err.starts_with("Error: Usage: hoocode plugin list | "),
            "{err}"
        );

        let (code, _, err) = call(&paths, &["remove", "nothing-here"]);
        assert_eq!(code, 1);
        assert_eq!(err, "Error: nothing-here is not installed\n");

        let (code, _, err) = call(&paths, &["uninstall", "nothing-here"]);
        assert_eq!(code, 1);
        assert_eq!(err, "Error: nothing-here is not installed\n");
    }
}

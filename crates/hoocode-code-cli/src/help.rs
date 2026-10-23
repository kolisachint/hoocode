//! `hoocode --help`. The text is in `help_text.rs`, written by hand: it lists only the
//! commands, flags and variables this build handles.

use crate::help_text::HELP_SEGMENTS;
use std::io::Write;

/// One piece of the help template.
pub(crate) enum Seg {
    Plain(&'static str),
    /// `chalk.bold(...)`.
    Bold(&'static str),
    /// The `extensionFlagsText` slot (empty until extensions are ported).
    Ext,
}

/// Render the help text. `color` mirrors chalk: bold is emitted only when the
/// output supports color.
pub fn render_help(color: bool) -> String {
    let mut out = String::new();
    for seg in HELP_SEGMENTS {
        match seg {
            Seg::Plain(text) => out.push_str(text),
            Seg::Bold(text) if color => {
                out.push_str("\x1b[1m");
                out.push_str(text);
                out.push_str("\x1b[22m");
            }
            Seg::Bold(text) => out.push_str(text),
            Seg::Ext => {}
        }
    }
    out
}

/// `printHelp()`: `console.log` of the rendered text.
pub fn print_help(output: &mut dyn Write, color: bool) -> std::io::Result<()> {
    writeln!(output, "{}", render_help(color))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_lists_the_supported_flags() {
        let help = render_help(false);
        assert!(help.starts_with("hoocode - AI coding agent with Read, Shell, Edit, Write and CodeSearch tools\n\nUsage:\n  hoocode [options] [@files...] [messages...]\n"));
        for flag in [
            "--provider <name>",
            "--no-tools, -nt",
            "--no-slash-commands, -nsc",
            "--list-models [search]",
            "--offline",
            "--use-system-ca",
            "--enable-plugintools",
        ] {
            assert!(help.contains(flag), "missing {flag}");
        }
        assert!(
            help.ends_with("  CodeSearch - Ranked code search (read-only)\n"),
            "{help}"
        );
    }

    #[test]
    fn help_leaves_out_flags_that_are_not_implemented() {
        // Each of these fails with "not yet supported" (`unsupported_flags`, `--export`).
        let help = render_help(false);
        for flag in [
            "--export",
            "--extension",
            "--no-extensions",
            "--agent <",
            "--theme",
            "--no-themes",
            "--verbose",
            "--platform",
            "--enable-webtools",
            "--enable-todowrite",
            "--enable-semantic-index",
            "--print-token-surface",
        ] {
            assert!(!help.contains(flag), "help lists unimplemented {flag}");
        }
    }

    #[test]
    fn help_names_the_real_config_dir_and_commands() {
        let help = render_help(false);
        assert!(help.contains(
            "HOOCODE_CODING_AGENT_DIR         - Config directory (default: ~/.hoocode)\n"
        ));
        assert!(!help.contains("~/.hoocode/agent"));
        assert!(!help.contains(".ts"), "TypeScript-era text in help");
        assert!(!help.contains("hoocode-ts"));
        assert!(!help.to_lowercase().contains("extension"));
        // The commands that run.
        for command in [
            "hoocode install <name[@marketplace]>",
            "hoocode remove <name>",
            "hoocode list ",
            "hoocode plugin <command>",
            "hoocode config ",
            "hoocode update ",
            "hoocode migrate [--dry-run]",
            "hoocode app-server [--listen URL]",
        ] {
            assert!(help.contains(command), "missing {command}");
        }
        // `resources` is not a command.
        assert!(!help.contains("hoocode resources"));
    }

    #[test]
    fn every_listed_command_is_dispatched() {
        // A word after `hoocode ` in the Commands block must be one `main` handles.
        let help = render_help(false);
        let block = help
            .split("Commands:\n")
            .nth(1)
            .and_then(|rest| rest.split("\n\nOptions:").next())
            .unwrap();
        let handled = [
            "install",
            "remove",
            "list",
            "plugin",
            "config",
            "update",
            "migrate",
            "app-server",
            "<command>",
        ];
        for line in block.lines() {
            if let Some(rest) = line.trim_start().strip_prefix("hoocode ") {
                let word = rest.split_whitespace().next().unwrap();
                assert!(
                    handled.contains(&word),
                    "help lists unhandled `hoocode {word}`"
                );
            }
        }
        for word in crate::plugin_command::PLUGIN_SUBCOMMANDS {
            if word != "uninstall" {
                assert!(block.contains(&format!("hoocode {word} ")), "{word}");
            }
        }
    }

    #[test]
    fn bold_headers_only_with_color() {
        assert!(render_help(true).contains("\x1b[1mOptions:\x1b[22m"));
        assert!(!render_help(false).contains('\x1b'));
    }
}

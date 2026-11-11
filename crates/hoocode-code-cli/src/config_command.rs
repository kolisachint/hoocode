//! `handleConfigCommand` (package-manager-cli.ts): `hoocode config` opens the
//! resource list over everything the settings resolve to.
//!
//! Package sources (`packages`: npm / git) are not resolved yet (ledger
//! 12.2), so only settings entries and auto-discovered resources are listed.

use std::cell::RefCell;
use std::io::Write;
use std::rc::Rc;

use hoocode_code_resources::package_resolve::{home_dir, resolve_local_resources, ResolveOptions};
use hoocode_code_settings::SettingsManager;

/// `reportSettingsErrors`.
fn report_settings_errors(
    settings: &mut SettingsManager,
    context: &str,
    color: bool,
    err: &mut dyn Write,
) {
    for e in settings.drain_errors() {
        let line = format!(
            "Warning ({context}, {} settings): {}",
            e.scope.as_str(),
            e.error
        );
        let line = if color {
            format!("\x1b[33m{line}\x1b[39m")
        } else {
            line
        };
        let _ = writeln!(err, "{line}");
    }
}

const CONFIG_USAGE: &str = "Usage: hoocode config [--help]

Open a TUI to turn skills, prompts and other resources on or off.

  --help, -h    Show this usage
";

/// The arguments that end `hoocode config` before the picker opens: `--help` / `-h`
/// print the usage on stdout (exit 0); any other argument is an error on stderr with
/// the usage (exit 2). `None` means no such argument: the picker opens.
fn exit_before_picker(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> Option<i32> {
    if args.is_empty() {
        return None;
    }
    if let Some(bad) = args.iter().find(|a| !matches!(a.as_str(), "--help" | "-h")) {
        let _ = writeln!(err, "Error: Unknown option: {bad}\n\n{CONFIG_USAGE}");
        return Some(2);
    }
    let _ = write!(out, "{CONFIG_USAGE}");
    Some(0)
}

/// Run `hoocode config`. `args` are the words after `config`. Returns the exit code
/// (0 on close and on ctrl+c).
pub fn run_config_command(
    args: &[String],
    color: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    if let Some(code) = exit_before_picker(args, out, err) {
        return code;
    }
    let cwd = std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .to_string_lossy()
        .into_owned();
    let agent_dir = hoocode_code_paths::agent_dir()
        .to_string_lossy()
        .into_owned();
    let mut settings = SettingsManager::create(&cwd, &agent_dir);
    report_settings_errors(&mut settings, "config command", color, err);
    let resolved = resolve_local_resources(&ResolveOptions {
        cwd: &cwd,
        agent_dir: &agent_dir,
        home: home_dir(),
        global_settings: &settings.global_settings(),
        project_settings: &settings.project_settings(),
    });
    let settings = Rc::new(RefCell::new(settings));
    hoocode_code_tui_app::session_picker::select_config(&resolved, settings, &cwd, &agent_dir);
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn plain_config_opens_the_picker() {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        assert_eq!(exit_before_picker(&[], &mut out, &mut err), None);
        assert!(out.is_empty() && err.is_empty());
    }

    #[test]
    fn help_prints_usage_on_stdout_and_exits_0_without_the_picker() {
        for flag in ["--help", "-h"] {
            let (mut out, mut err) = (Vec::new(), Vec::new());
            assert_eq!(
                exit_before_picker(&args(&[flag]), &mut out, &mut err),
                Some(0),
                "{flag}"
            );
            let text = String::from_utf8(out).unwrap();
            assert!(text.starts_with("Usage: hoocode config"), "{text}");
            assert!(err.is_empty(), "{flag}");
        }
    }

    #[test]
    fn unknown_argument_is_an_error_with_usage_on_stderr_exit_2() {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        assert_eq!(
            exit_before_picker(&args(&["--bogus"]), &mut out, &mut err),
            Some(2)
        );
        assert!(out.is_empty());
        let text = String::from_utf8(err).unwrap();
        assert!(
            text.starts_with("Error: Unknown option: --bogus\n\nUsage: hoocode config"),
            "{text}"
        );
    }
}

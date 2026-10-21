//! CLI for the `hoocode` coding agent: argument parsing and mode dispatch.
//!
//! Port of hoocode `packages/coding-agent/src/cli/args.ts` (in [`args`]) and
//! the argument-handling half of `main.ts` (in [`run`]). Flags that parse but
//! whose behavior is not ported yet fail with a clear "not yet supported"
//! error instead of being silently ignored.

mod app_server;
pub mod args;
pub mod auth;
mod config_command;
mod help;
mod help_text;
pub mod initial_message;
mod list_models;
mod runtime;
mod session_flags;

pub use args::{
    is_valid_thinking_level, parse_args, Args, Diagnostic, DiagnosticKind, FlagValue, ListModels,
    Mode,
};
pub use help::{print_help, render_help};

use hoocode_code_print::PrintMode;
use hoocode_code_settings::SettingsManager;
use std::io::{IsTerminal, Read, Write};

/// `VERSION` printed by `--version`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Package-manager subcommands that hoocode handles before `parseArgs`
/// (`handlePackageCommand`, `handleResourcesCommand`). Not ported yet;
/// `config` (`handleConfigCommand`) and `update` (see `hoocode-code-update`) are.
const SUBCOMMANDS: [&str; 5] = ["install", "remove", "uninstall", "list", "resources"];

/// `resolveAppMode` in `main.ts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Interactive,
    Print,
    Json,
    Rpc,
}

pub fn resolve_app_mode(parsed: &Args, stdin_is_tty: bool) -> AppMode {
    match parsed.mode {
        Some(Mode::Rpc) => AppMode::Rpc,
        Some(Mode::Json) => AppMode::Json,
        _ if parsed.print == Some(true) || !stdin_is_tty => AppMode::Print,
        _ => AppMode::Interactive,
    }
}

/// Process environment the dispatcher depends on, injectable for tests.
#[derive(Debug, Clone, Copy)]
pub struct Env {
    pub stdin_is_tty: bool,
    /// Whether chalk-style color is enabled.
    pub color: bool,
}

impl Env {
    pub fn detect() -> Self {
        Self {
            stdin_is_tty: std::io::stdin().is_terminal(),
            color: color_enabled(),
        }
    }
}

/// chalk's default level comes from `supports-color` on stdout.
fn color_enabled() -> bool {
    let force = std::env::var("FORCE_COLOR").ok();
    let no_color = std::env::var("NO_COLOR").ok();
    let term = std::env::var("TERM").ok();
    color_from_env(
        force.as_deref(),
        no_color.as_deref(),
        std::io::stdout().is_terminal(),
        term.as_deref(),
    )
}

/// The colour decision from its inputs, so tests need no process env.
/// `FORCE_COLOR` wins over `NO_COLOR` when both are set (decisions-2026-10-14.md #18).
/// `NO_COLOR`: any non-empty value turns colour off (no-color.org).
fn color_from_env(
    force: Option<&str>,
    no_color: Option<&str>,
    stdout_tty: bool,
    term: Option<&str>,
) -> bool {
    match force {
        Some("0" | "false") => return false,
        Some(_) => return true,
        None => {}
    }
    if no_color.is_some_and(|v| !v.is_empty()) {
        return false;
    }
    stdout_tty && term != Some("dumb")
}

pub(crate) fn red(color: bool, text: &str) -> String {
    if color {
        format!("\x1b[31m{text}\x1b[39m")
    } else {
        text.to_string()
    }
}

fn yellow(env: Env, text: &str) -> String {
    if env.color {
        format!("\x1b[33m{text}\x1b[39m")
    } else {
        text.to_string()
    }
}

/// Flags from the pinned set that parse but are not implemented yet, in
/// `Args` field order.
pub fn unsupported_flags(a: &Args) -> Vec<&'static str> {
    let checks: [(bool, &'static str); 11] = [
        (a.todo_write.is_some(), "--enable-todowrite"),
        (a.enable_web_tools.is_some(), "--enable-webtools"),
        (a.enable_semantic_index.is_some(), "--enable-semantic-index"),
        (a.print_token_surface.is_some(), "--print-token-surface"),
        (a.platform.is_some(), "--platform"),
        (a.extensions.is_some(), "--extension"),
        (a.no_extensions.is_some(), "--no-extensions"),
        (a.agents.is_some(), "--agent"),
        (a.themes.is_some(), "--theme"),
        (a.no_themes.is_some(), "--no-themes"),
        (a.verbose.is_some(), "--verbose"),
    ];
    checks
        .into_iter()
        .filter(|(set, _)| *set)
        .map(|(_, flag)| flag)
        .collect()
}

/// Process entry point used by the `hoocode` binary. Returns the exit code.
pub fn main(argv: &[String]) -> i32 {
    // cli.ts: install the CA trust set before anything builds an HTTP client.
    // The two flags are read by an argv pre-scan, as in hoocode.
    hoocode_ai_util::tls::configure_global_tls(
        &hoocode_ai_util::tls::TlsSources::from_args_and_env(argv, |k| std::env::var(k).ok()),
    );
    // main.ts: offline mode is process-wide, so what reads it later sees it
    // too.
    if argv.iter().any(|a| a == "--offline") || hoocode_code_paths::is_offline_mode() {
        std::env::set_var("HOOCODE_OFFLINE", "1");
        std::env::set_var("HOOCODE_SKIP_VERSION_CHECK", "1");
    }
    let env = Env::detect();
    // Theme styles follow the same colour decision as the CLI's own output.
    hoocode_code_tui_theme::chalk::set_enabled(env.color);
    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));

    // naming-and-paths.md §3: the one-time merge of the pre-1.2 home folder runs at every
    // start, in every mode; `migrate` runs it on demand.
    if argv.first().is_some_and(|a| a == "migrate") {
        return hoocode_code_migrate::run_migrate_command(
            &argv[1..],
            &cwd,
            &mut stdout,
            &mut stderr,
        );
    }
    hoocode_code_migrate::run_startup(&cwd, &mut stderr);

    if argv.first().is_some_and(|a| a == "app-server") {
        return app_server::run(&argv[1..], &mut stderr);
    }
    if argv.first().is_some_and(|a| a == "config") {
        return config_command::run_config_command(env.color, &mut stderr);
    }
    if argv.first().is_some_and(|a| a == "update") {
        return hoocode_code_update::run_update_command(&mut stdout);
    }
    if let Some(cmd) = argv.first().filter(|a| SUBCOMMANDS.contains(&a.as_str())) {
        let _ = writeln!(
            stderr,
            "{}",
            red(
                env.color,
                &format!("Error: `hoocode {cmd}` is not yet supported by hoocode")
            )
        );
        return 1;
    }

    let parsed = parse_args(argv);
    match run(&parsed, env, &mut stdout, &mut stderr) {
        Ok(code) => code,
        Err(e) => {
            let _ = writeln!(stderr, "{}", red(env.color, &format!("Error: {e}")));
            1
        }
    }
}

/// The argument-handling part of `main()` in `main.ts`, then mode dispatch.
pub fn run(
    parsed: &Args,
    env: Env,
    output: &mut dyn Write,
    err: &mut dyn Write,
) -> std::io::Result<i32> {
    if !parsed.diagnostics.is_empty() {
        for d in &parsed.diagnostics {
            let line = match d.kind {
                DiagnosticKind::Error => red(env.color, &format!("Error: {}", d.message)),
                DiagnosticKind::Warning => yellow(env, &format!("Warning: {}", d.message)),
            };
            writeln!(err, "{line}")?;
        }
        if parsed
            .diagnostics
            .iter()
            .any(|d| d.kind == DiagnosticKind::Error)
        {
            return Ok(1);
        }
    }

    let app_mode = resolve_app_mode(parsed, env.stdin_is_tty);
    // `takeOverStdout`: outside interactive mode stdout is reserved for the
    // mode's own output, so what main.ts `console.log`s before the mode runs
    // (--version, --help, --list-models) goes to stderr.
    let take_over_stdout = app_mode != AppMode::Interactive;

    if parsed.version == Some(true) {
        let console: &mut dyn Write = if take_over_stdout { err } else { output };
        writeln!(console, "{VERSION}")?;
        return Ok(0);
    }

    if parsed.export.is_some() {
        writeln!(
            err,
            "{}",
            red(env.color, "Error: --export is not yet supported by hoocode")
        )?;
        return Ok(1);
    }

    if parsed.mode == Some(Mode::Rpc) && !parsed.file_args.is_empty() {
        writeln!(
            err,
            "{}",
            red(
                env.color,
                "Error: @file arguments are not supported in RPC mode"
            )
        )?;
        return Ok(1);
    }

    if parsed.help == Some(true) {
        let console: &mut dyn Write = if take_over_stdout { err } else { output };
        print_help(console, env.color)?;
        return Ok(0);
    }

    let unsupported = unsupported_flags(parsed);
    if !unsupported.is_empty() {
        for flag in &unsupported {
            writeln!(
                err,
                "{}",
                red(
                    env.color,
                    &format!("Error: {flag} is not yet supported by hoocode")
                )
            )?;
        }
        return Ok(1);
    }

    // No extensions are ported yet, so no extension registers flags: every
    // unknown long flag is reported the way `applyExtensionFlagValues` does.
    if !parsed.unknown_flags.is_empty() {
        let names: Vec<String> = parsed
            .unknown_flags
            .iter()
            .map(|(name, _)| format!("--{name}"))
            .collect();
        let plural = if names.len() == 1 { "" } else { "s" };
        writeln!(
            err,
            "{}",
            red(
                env.color,
                &format!("Error: Unknown option{plural}: {}", names.join(", "))
            )
        )?;
        return Ok(1);
    }

    // main.ts builds the runtime (auth.json + models.json) first, then lists.
    if let Some(list) = &parsed.list_models {
        let (auth, registry) = runtime::load_auth_and_registry();
        let search = match list {
            ListModels::Search(pattern) => Some(pattern.as_str()),
            ListModels::All => None,
        };
        if take_over_stdout {
            let mut table = Vec::new();
            list_models::list_models(&registry, auth.as_ref(), search, env.color, &mut table, err)?;
            err.write_all(&table)?;
        } else {
            list_models::list_models(&registry, auth.as_ref(), search, env.color, output, err)?;
        }
        return Ok(0);
    }

    // `readPipedStdin`: RPC mode owns stdin; otherwise a non-TTY stdin is read
    // whole and becomes the start of the initial message.
    // A subagent child (json + task id) keeps stdin for its parent's answers to
    // its clearance requests; its prompt is on the command line.
    let subagent_child = app_mode == AppMode::Json && parsed.task_id.is_some();
    let stdin_content = if app_mode != AppMode::Rpc && !subagent_child && !env.stdin_is_tty {
        let mut data = String::new();
        let _ = std::io::stdin().read_to_string(&mut data);
        initial_message::normalize_piped_stdin(&data)
    } else {
        None
    };

    let code = match app_mode {
        AppMode::Json => runtime::run_print_mode(
            parsed,
            PrintMode::Json,
            stdin_content,
            env.color,
            output,
            err,
        ),
        AppMode::Print => runtime::run_print_mode(
            parsed,
            PrintMode::Text,
            stdin_content,
            env.color,
            output,
            err,
        ),
        AppMode::Rpc => runtime::run_rpc_mode(parsed, env.color, err),
        AppMode::Interactive => match runtime::run_interactive_mode(parsed, output, err) {
            Ok(code) => Ok(code),
            Err(e) => {
                writeln!(err, "{e}")?;
                Ok(1)
            }
        },
    };
    // `process.once("exit", () => pool.dispose())`: no subagent outlives us.
    hoocode_code_subagents::instance::dispose_subagent_pool();
    code
}

/// The global + project settings for the current directory (`SettingsManager.create`).
/// A settings file that fails to parse is skipped (its errors stay in the
/// manager's `drain_errors`).
pub fn load_settings() -> SettingsManager {
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    SettingsManager::create_default(cwd)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TTY: Env = Env {
        stdin_is_tty: true,
        color: false,
    };

    fn run_with(argv: &[&str], env: Env) -> (i32, String, String) {
        let parsed = parse_args(&argv.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(&parsed, env, &mut out, &mut err).unwrap();
        (
            code,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }

    #[test]
    fn version_prints_bare_version() {
        assert_eq!(
            run_with(&["--version", "--help"], TTY),
            (0, format!("{VERSION}\n"), String::new())
        );
    }

    #[test]
    fn early_exit_output_goes_to_stderr_outside_interactive_mode() {
        let piped = Env {
            stdin_is_tty: false,
            color: false,
        };
        assert_eq!(
            run_with(&["--version"], piped),
            (0, String::new(), format!("{VERSION}\n"))
        );
        let (code, out, err) = run_with(&["--mode", "json", "-h"], TTY);
        assert_eq!((code, out.as_str()), (0, ""));
        assert!(err.contains("Usage:"));
    }

    #[test]
    fn help_prints_help() {
        let (code, out, _) = run_with(&["-h"], TTY);
        assert_eq!(code, 0);
        assert!(out.contains("Usage:"));
    }

    #[test]
    fn help_wins_over_unsupported_and_unknown_flags() {
        let (code, out, err) = run_with(&["--continue", "--future", "-h"], TTY);
        assert_eq!(code, 0);
        assert!(out.contains("Options:"));
        assert!(err.is_empty());
    }

    #[test]
    fn error_diagnostics_exit_1() {
        assert_eq!(
            run_with(&["-x"], TTY),
            (1, String::new(), "Error: Unknown option: -x\n".into())
        );
    }

    #[test]
    fn warnings_are_printed_and_colored() {
        let color = Env {
            stdin_is_tty: true,
            color: true,
        };
        let (code, _, err) = run_with(&["--thinking", "max", "--version"], color);
        assert_eq!(code, 0);
        assert!(err.starts_with("\x1b[33mWarning: Invalid thinking level \"max\""));
    }

    #[test]
    fn unsupported_flags_fail_clearly() {
        let (code, out, err) = run_with(&["--no-themes", "--theme", "t.json", "-p", "hi"], TTY);
        assert_eq!(code, 1);
        assert!(out.is_empty());
        assert_eq!(
            err,
            "Error: --theme is not yet supported by hoocode\n\
             Error: --no-themes is not yet supported by hoocode\n"
        );
    }

    #[test]
    fn export_is_not_yet_supported() {
        let (code, _, err) = run_with(&["--export", "s.jsonl"], TTY);
        assert_eq!(code, 1);
        assert_eq!(err, "Error: --export is not yet supported by hoocode\n");
    }

    #[test]
    fn unknown_long_flags_are_errors_without_extensions() {
        assert_eq!(
            run_with(&["--login", "anthropic", "-p", "hi"], TTY).2,
            "Error: Unknown option: --login\n"
        );
        assert_eq!(
            run_with(&["--a", "--b=1"], TTY).2,
            "Error: Unknown options: --a, --b\n"
        );
    }

    #[test]
    fn rpc_mode_rejects_file_args() {
        assert_eq!(
            run_with(&["--mode", "rpc", "@x.md"], TTY),
            (
                1,
                String::new(),
                "Error: @file arguments are not supported in RPC mode\n".into()
            )
        );
    }

    #[test]
    fn supported_flags_are_not_flagged() {
        let parsed = parse_args(
            &[
                "--offline",
                "--provider",
                "p",
                "--model",
                "m",
                "--api-key",
                "k",
                "--system-prompt",
                "s",
                "--no-session",
                "--task-id",
                "t",
                "--mode",
                "json",
                "-p",
                "hi",
                "@f",
            ]
            .map(String::from),
        );
        assert!(unsupported_flags(&parsed).is_empty());
        assert!(parsed.unknown_flags.is_empty());
    }

    #[test]
    fn plugintools_flag_is_supported() {
        let parsed = parse_args(&["--enable-plugintools".to_owned()]);
        assert!(unsupported_flags(&parsed).is_empty());
        assert_eq!(parsed.enable_plugin_tools, Some(true));
    }

    #[test]
    fn app_mode_resolution() {
        let p = |a: &[&str]| parse_args(&a.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(
            resolve_app_mode(&p(&["--mode", "rpc", "-p"]), true),
            AppMode::Rpc
        );
        assert_eq!(
            resolve_app_mode(&p(&["--mode", "json"]), true),
            AppMode::Json
        );
        assert_eq!(resolve_app_mode(&p(&["-p", "x"]), true), AppMode::Print);
        assert_eq!(resolve_app_mode(&p(&["x"]), false), AppMode::Print);
        assert_eq!(resolve_app_mode(&p(&["x"]), true), AppMode::Interactive);
        assert_eq!(
            resolve_app_mode(&p(&["--mode", "text"]), true),
            AppMode::Interactive
        );
    }

    #[test]
    fn color_follows_force_color() {
        // FORCE_COLOR on (any value but 0/false) turns colour on, even off a tty.
        assert!(color_from_env(Some("1"), None, false, Some("dumb")));
        assert!(color_from_env(Some("yes"), None, false, None));
        // FORCE_COLOR=0 or false turns it off, even on a tty.
        assert!(!color_from_env(Some("0"), None, true, Some("xterm")));
        assert!(!color_from_env(Some("false"), None, true, Some("xterm")));
    }

    #[test]
    fn no_color_turns_colour_off() {
        assert!(!color_from_env(None, Some("1"), true, Some("xterm")));
        assert!(!color_from_env(None, Some("anything"), true, Some("xterm")));
    }

    #[test]
    fn empty_no_color_is_ignored() {
        // no-color.org: only a non-empty value counts.
        assert!(color_from_env(None, Some(""), true, Some("xterm")));
        assert!(!color_from_env(None, Some(""), false, Some("xterm")));
    }

    #[test]
    fn force_color_wins_over_no_color() {
        assert!(color_from_env(Some("1"), Some("1"), false, None));
        assert!(color_from_env(Some("1"), Some("1"), true, None));
        // Both set and FORCE_COLOR off: off, as before.
        assert!(!color_from_env(Some("0"), Some("1"), true, Some("xterm")));
    }

    #[test]
    fn colour_without_overrides_needs_a_capable_tty() {
        assert!(color_from_env(None, None, true, Some("xterm-256color")));
        assert!(color_from_env(None, None, true, None));
        assert!(!color_from_env(None, None, false, Some("xterm")));
        assert!(!color_from_env(None, None, true, Some("dumb")));
    }
}

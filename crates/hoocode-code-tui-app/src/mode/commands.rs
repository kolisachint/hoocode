//! Built-in slash commands: the `BuiltinCommand` parser and dispatch, and the small commands
//! (`/hotkeys`, `/changelog`, `/perf`, `/debug`, Ctrl+Z).

use std::sync::mpsc::Receiver;

use hoocode_code_paths::VERSION;
use hoocode_code_resources::BUILTIN_SLASH_COMMANDS;
use hoocode_code_tui_selectors::oauth_selector::LoginMode;
use hoocode_code_tui_theme::theme;
use hoocode_code_tui_widgets::dynamic_border::DynamicBorder;
use hoocode_tui_components::{Markdown, Spacer, Text};
use hoocode_tui_render::{Tui, TuiEvent};
use hoocode_tui_util::visible_width;

use crate::changelog::{changelog_path, parse_changelog};
use crate::hotkeys::hotkeys_markdown;
use crate::suspend::{suspend_to_background, SuspendOps, SuspendOutcome};

use super::*;

/// The first version header in a changelog excerpt.
static CHANGELOG_VERSION: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"##\s+\[?(\d+\.\d+\.\d+)\]?").expect("static pattern")
});

/// The built-in slash commands this mode dispatches itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BuiltinCommand {
    Quit,
    Resume,
    Tree,
    Name,
    Session,
    New,
    Compact,
    Model,
    ScopedModels,
    Settings,
    Login,
    Logout,
    Copy,
    Hotkeys,
    Changelog,
    Debug,
    Perf,
    Mcp,
    Color,
    Chrome,
    Fork,
    Clone,
    Cd,
    Reload,
    Export,
    Import,
    Subagent,
    SubagentCancel,
    SubagentRetry,
    SubagentStats,
    /// A built-in whose handler lands with a later task.
    Pending(&'static str),
}

impl BuiltinCommand {
    pub(super) fn lookup(text: &str) -> Option<Self> {
        let name = text.strip_prefix('/')?;
        Some(match name {
            "quit" => Self::Quit,
            "resume" => Self::Resume,
            "tree" => Self::Tree,
            "name" => Self::Name,
            "session" => Self::Session,
            "new" => Self::New,
            "compact" => Self::Compact,
            "model" => Self::Model,
            "scoped-models" => Self::ScopedModels,
            "settings" => Self::Settings,
            "login" => Self::Login,
            "logout" => Self::Logout,
            "copy" => Self::Copy,
            "hotkeys" => Self::Hotkeys,
            "changelog" => Self::Changelog,
            "debug" => Self::Debug,
            "perf" => Self::Perf,
            "mcp" => Self::Mcp,
            "color" => Self::Color,
            "chrome" => Self::Chrome,
            "fork" => Self::Fork,
            "clone" => Self::Clone,
            "cd" => Self::Cd,
            "reload" => Self::Reload,
            "export" => Self::Export,
            "import" => Self::Import,
            "subagent" => Self::Subagent,
            "subagent-cancel" => Self::SubagentCancel,
            "subagent-retry" => Self::SubagentRetry,
            "subagent-stats" => Self::SubagentStats,
            _ => {
                let builtin = BUILTIN_SLASH_COMMANDS.iter().find(|c| c.name == name)?;
                Self::Pending(builtin.name)
            }
        })
    }

    /// Commands that also match "/name <args>".
    pub(super) fn with_args(self) -> bool {
        match self {
            Self::Name
            | Self::Compact
            | Self::Model
            | Self::Copy
            | Self::Color
            | Self::Chrome
            | Self::Cd
            | Self::Mcp
            | Self::Export
            | Self::Import
            | Self::Subagent
            | Self::SubagentCancel
            | Self::SubagentRetry
            | Self::SubagentStats => true,
            Self::Pending(_) => false,
            _ => false,
        }
    }
}

impl Mode {
    /// `createBuiltInSlashCommands`: run one.
    pub(super) fn run_builtin_command(&mut self, command: BuiltinCommand, text: &str) {
        self.editor.borrow_mut().editor.set_text("");
        match command {
            BuiltinCommand::Quit => self.exit_requested = true,
            BuiltinCommand::Resume => self.show_session_selector(),
            BuiltinCommand::Tree => self.show_tree_selector(None),
            BuiltinCommand::Name => self.handle_name_command(text),
            BuiltinCommand::Session => self.handle_session_command(),
            BuiltinCommand::New => self.handle_new_command(),
            BuiltinCommand::Compact => {
                let instructions = text
                    .strip_prefix("/compact ")
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .map(String::from);
                self.handle_compact_command(instructions);
            }
            BuiltinCommand::Model => {
                let search = text
                    .strip_prefix("/model ")
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .map(String::from);
                self.handle_model_command(search);
            }
            BuiltinCommand::ScopedModels => self.show_models_selector(),
            BuiltinCommand::Settings => self.show_settings_selector(),
            BuiltinCommand::Login => self.show_oauth_selector(LoginMode::Login),
            BuiltinCommand::Logout => self.show_oauth_selector(LoginMode::Logout),
            BuiltinCommand::Copy => self.handle_copy_command(text),
            BuiltinCommand::Hotkeys => self.handle_hotkeys_command(),
            BuiltinCommand::Changelog => self.handle_changelog_command(),
            BuiltinCommand::Debug => self.handle_debug_command(),
            BuiltinCommand::Perf => self.handle_perf_command(),
            BuiltinCommand::Mcp => self.handle_mcp_command(text),
            BuiltinCommand::Color => {
                // An argument sets the slot outright; bare `/color` opens the
                // swatches.
                if !self.handle_color_command(text) {
                    self.show_session_color_selector();
                }
            }
            BuiltinCommand::Chrome => self.handle_chrome_command(text),
            BuiltinCommand::Fork => self.show_user_message_selector(),
            BuiltinCommand::Clone => self.handle_clone_command(),
            BuiltinCommand::Cd => self.handle_change_directory(text),
            BuiltinCommand::Reload => self.handle_reload_command(),
            BuiltinCommand::Export => self.handle_export_command(text),
            BuiltinCommand::Import => self.handle_import_command(text),
            BuiltinCommand::Subagent => self.handle_subagent_command(text),
            BuiltinCommand::SubagentCancel => self.cancel_newest_subagent(text),
            BuiltinCommand::SubagentRetry => self.retry_newest_subagent(text),
            BuiltinCommand::SubagentStats => self.handle_subagent_stats_command(text),
            BuiltinCommand::Pending(name) => {
                // Wired by its own ledger task (see 11.4e).
                self.show_status(&format!("/{name} is not available yet"));
            }
        }
        self.dirty.set(true);
    }

    /// `handleHotkeys`.
    pub(super) fn handle_hotkeys_command(&mut self) {
        self.show_bordered_markdown("Keyboard Shortcuts", &hotkeys_markdown());
    }

    /// `handleChangelog`: every entry, oldest first.
    fn handle_changelog_command(&mut self) {
        let entries = parse_changelog(&changelog_path());
        if entries.is_empty() {
            self.add_to_chat(as_component(&handle(Spacer::new(1))));
            self.add_to_chat(as_component(&handle(Text::new(
                theme().fg("dim", "No changelog entries found."),
                1,
                0,
            ))));
            return;
        }
        let markdown = entries
            .iter()
            .rev()
            .map(|e| e.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        self.show_bordered_markdown("What's New", &markdown);
    }

    /// `showStartupNoticesIfNeeded`: the entries new since the last run,
    /// in full or as one line when the changelog is collapsed.
    pub(super) fn show_startup_notices(&mut self, changelog: Option<String>) {
        let Some(markdown) = changelog.filter(|m| !m.trim().is_empty()) else {
            return;
        };
        let has_rows = !self.transcript.chat.borrow().children.is_empty();
        if has_rows {
            self.add_to_chat(as_component(&handle(Spacer::new(1))));
        }
        self.add_to_chat(as_component(&handle(DynamicBorder::new(None))));
        let t = theme();
        let collapse = self.session.settings().collapse_changelog();
        if collapse {
            let latest = CHANGELOG_VERSION
                .captures(&markdown)
                .map_or_else(|| VERSION.to_string(), |c| c[1].to_string());
            self.add_to_chat(as_component(&handle(Text::new(
                format!(
                    "Updated to v{latest}. Use {} to view full changelog.",
                    t.bold("/changelog")
                ),
                1,
                0,
            ))));
        } else {
            self.add_to_chat(as_component(&handle(Text::new(
                t.bold(&t.fg("accent", "What's New")),
                1,
                0,
            ))));
            self.add_to_chat(as_component(&handle(Spacer::new(1))));
            self.add_to_chat(as_component(&handle(Markdown::new(
                markdown.trim(),
                1,
                0,
                (self.markdown_theme())(),
                None,
            ))));
        }
        self.add_to_chat(as_component(&handle(DynamicBorder::new(None))));
    }

    fn handle_perf_command(&mut self) {
        let text = crate::perf::format_report(&self.perf.snapshot());
        self.add_to_chat(as_component(&handle(Spacer::new(1))));
        self.add_to_chat(as_component(&handle(Text::new(text, 1, 0))));
    }

    /// `handleDebug`: every rendered line with its width, and the messages
    /// as JSON lines, written to the debug log.
    fn handle_debug_command(&mut self) {
        let (width, height) = self.size.get();
        let lines = self.tui.render(width);
        let mut data = vec![
            format!("Debug output at {}", iso_now()),
            format!("Terminal: {width}x{height}"),
            format!("Total lines: {}", lines.len()),
            String::new(),
            "=== All rendered lines with visible widths ===".to_string(),
        ];
        for (idx, line) in lines.iter().enumerate() {
            let escaped = serde_json::to_string(line).unwrap_or_default();
            data.push(format!("[{idx}] (w={}) {escaped}", visible_width(line)));
        }
        data.push(String::new());
        data.push("=== Agent messages (JSONL) ===".to_string());
        for message in self.session.messages() {
            data.push(serde_json::to_string(&message).unwrap_or_default());
        }
        data.push(String::new());

        let path = hoocode_code_paths::debug_log_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(error) = std::fs::write(&path, data.join("\n")) {
            self.show_error(&format!("Failed to write debug log: {error}"));
            return;
        }
        let t = theme();
        self.add_to_chat(as_component(&handle(Spacer::new(1))));
        self.add_to_chat(as_component(&handle(Text::new(
            format!(
                "{}\n{}",
                t.fg("accent", "✓ Debug log written"),
                t.fg("muted", &path.to_string_lossy())
            ),
            1,
            0,
        ))));
    }

    /// `handleCtrlZ`: suspend to the background; the TUI comes back on `fg`.
    pub(super) fn handle_ctrl_z(&mut self) {
        struct Ops<'a> {
            tui: &'a mut Tui,
            input: &'a mut Option<Receiver<TuiEvent>>,
            #[cfg(unix)]
            signals: crate::suspend::ProcessSignals,
        }
        impl SuspendOps for Ops<'_> {
            fn stop_ui(&mut self) {
                self.tui.stop();
            }
            fn restart_ui(&mut self) {
                *self.input = Some(self.tui.start());
                self.tui.request_render(true);
            }
            fn ignore_sigint(&mut self) {
                #[cfg(unix)]
                self.signals.ignore_sigint();
            }
            fn restore_sigint(&mut self) {
                #[cfg(unix)]
                self.signals.restore_sigint();
            }
            fn stop_process_group(&mut self) -> Result<(), String> {
                #[cfg(unix)]
                return self.signals.stop_process_group();
                #[cfg(not(unix))]
                Err("not supported".into())
            }
        }
        let mut ops = Ops {
            tui: &mut self.tui,
            input: &mut self.restarted_input,
            #[cfg(unix)]
            signals: Default::default(),
        };
        match suspend_to_background(&mut ops, cfg!(windows)) {
            SuspendOutcome::Resumed => {
                // The terminal may have been resized while stopped.
                let terminal = &self.tui.terminal;
                self.size.set((terminal.columns(), terminal.rows()));
                self.dirty.set(true);
            }
            SuspendOutcome::Unsupported(message) => self.show_status(message),
            SuspendOutcome::Failed(error) => {
                // hoocode rethrows here; the UI is brought back instead so
                // the session stays usable.
                self.restarted_input = Some(self.tui.start());
                self.tui.request_render(true);
                self.show_error(&format!("Suspend failed: {error}"));
            }
        }
    }
}

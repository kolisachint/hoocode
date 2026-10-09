//! The prompt: the custom editor and its keys (`Action`), submit, the `!` bash prefix, and the
//! autocomplete providers (`/` commands, `@file`, paths).

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use hoocode_ai_types::ImageContent;
use hoocode_ai_types::Model;
use hoocode_code_agent_session::{AgentSession, PromptOptions, StreamingBehavior};
use hoocode_code_paths::APP_NAME;
use hoocode_code_resources::BUILTIN_SLASH_COMMANDS;
use hoocode_code_settings::DoubleEscapeAction;
use hoocode_code_tui_widgets::task_panel::TaskPanelView;
use hoocode_tui_components::{
    ArgumentCompletionsFn, AutocompleteItem, CombinedAutocompleteProvider, CommandEntry, Editor,
    FileFinder, FileMatch, SlashCommand,
};
use hoocode_tui_keys::get_keybindings;
use hoocode_tui_render::Component;

use crate::extension_editor::EditorOutcome;
use crate::extension_selector::SelectorOutcome;
use crate::startup_progress;

use super::*;

/// App actions the prompt editor raises; handled by the mode after the
/// keystroke (the editor is borrowed while it dispatches).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Action {
    Interrupt,
    Clear,
    Exit,
    Suspend,
    ToolsExpand,
    ChromeForward,
    ChromeBackward,
    ThinkingForward,
    ThinkingBackward,
    ThinkingToggle,
    ExternalEditor,
    SessionNew,
    SessionTree,
    ModeForward,
    ModeBackward,
    ModelForward,
    ModelBackward,
    ModelSelect,
    SettingsOpen,
    ViewForward,
    ViewBackward,
    /// The editor submitted this text (it has already cleared itself).
    Submit(String),
    AutocompleteVisibility(bool),
    /// The extension selector closed.
    SelectorDone(SelectorOutcome),
    /// `app.session.resume`: open the session selector.
    ResumeSession,
    /// The session selector closed, with the chosen file.
    SessionSelectorDone(Option<PathBuf>),
    /// The options pane closed: the answers, or `None` when skipped.
    AskOptionsDone(Option<Vec<String>>),
    /// The multi-line editor dialog closed.
    EditorDialogDone(EditorOutcome),
    /// The prompt text changed (`onChange`).
    EditorChanged(String),
    /// `app.clipboard.copyMessage`: `/copy` with no argument.
    CopyMessage,
    /// `app.clipboard.pasteImage`.
    PasteImage,
    /// `app.hotkeys.open`: `/hotkeys`.
    HotkeysOpen,
    /// `app.session.color.cycleForward` / `cycleBackward`.
    SessionColorForward,
    SessionColorBackward,
    /// The colour picker moved to this slot (live preview).
    SessionColorPreview(u8),
    /// The colour picker closed: the chosen slot, or `None` when cancelled.
    SessionColorDone(Option<u8>),
    /// `app.session.changeDirectory`: prefill `/cd `.
    ChangeDirectoryPrefill,
    /// `app.session.fork`: `/fork`.
    ForkOpen,
    /// `app.message.followUp`: queue the prompt as a follow-up.
    FollowUp,
    /// `app.message.dequeue`: every queued message back to the prompt.
    Dequeue,
    /// `app.tasks.cycleForward` / `cycleBackward`: step the task panel lens.
    TasksForward,
    TasksBackward,
}

/// Bindings the prompt answers to, and their action (`CustomEditor.onAction`).
const EDITOR_ACTIONS: [(&str, Action); 29] = [
    ("app.tasks.cycleForward", Action::TasksForward),
    ("app.tasks.cycleBackward", Action::TasksBackward),
    ("app.message.followUp", Action::FollowUp),
    ("app.message.dequeue", Action::Dequeue),
    (
        "app.session.changeDirectory",
        Action::ChangeDirectoryPrefill,
    ),
    ("app.session.fork", Action::ForkOpen),
    (
        "app.session.color.cycleForward",
        Action::SessionColorForward,
    ),
    (
        "app.session.color.cycleBackward",
        Action::SessionColorBackward,
    ),
    ("app.hotkeys.open", Action::HotkeysOpen),
    ("app.clipboard.copyMessage", Action::CopyMessage),
    ("app.view.cycleForward", Action::ViewForward),
    ("app.view.cycleBackward", Action::ViewBackward),
    ("app.clear", Action::Clear),
    ("app.suspend", Action::Suspend),
    ("app.tools.expand", Action::ToolsExpand),
    ("app.chrome.cycleForward", Action::ChromeForward),
    ("app.chrome.cycleBackward", Action::ChromeBackward),
    ("app.thinking.cycleForward", Action::ThinkingForward),
    ("app.model.cycleForward", Action::ModelForward),
    ("app.model.cycleBackward", Action::ModelBackward),
    ("app.model.select", Action::ModelSelect),
    ("app.settings.open", Action::SettingsOpen),
    ("app.session.resume", Action::ResumeSession),
    ("app.thinking.toggle", Action::ThinkingToggle),
    ("app.editor.external", Action::ExternalEditor),
    ("app.session.new", Action::SessionNew),
    ("app.session.tree", Action::SessionTree),
    ("app.mode.cycleForward", Action::ModeForward),
    ("app.mode.cycleBackward", Action::ModeBackward),
];

/// The prompt editor with the app's key dispatch in front of it
/// (`CustomEditor`).
pub(super) struct CustomEditor {
    pub(super) editor: Editor,
    pub(super) actions: Rc<RefCell<Vec<Action>>>,
}

/// The `@file` finder: `hoocode_code_tools::file_finder` (fd's rules, in
/// process, so no `fd` binary is needed).
pub fn at_file_finder() -> FileFinder {
    Box::new(|base, query, max_results| {
        hoocode_code_tools::file_finder::find_paths(base, query, max_results)
            .into_iter()
            .map(|found| FileMatch {
                path: found.path,
                is_directory: found.is_directory,
            })
            .collect()
    })
}

fn is_plain_text(data: &str) -> bool {
    !data.is_empty() && data.chars().all(|c| (c as u32) >= 32 && c as u32 != 127)
}

impl Component for CustomEditor {
    fn render(&mut self, width: u16) -> Vec<String> {
        self.editor.render(width)
    }

    fn handle_input(&mut self, data: &str) {
        if is_plain_text(data) {
            self.editor.handle_input(data);
            return;
        }
        let kb = get_keybindings();
        if kb.matches(data, "app.clipboard.pasteImage") {
            self.actions.borrow_mut().push(Action::PasteImage);
            return;
        }
        if kb.matches(data, "app.interrupt") {
            if !self.editor.is_showing_autocomplete() {
                self.actions.borrow_mut().push(Action::Interrupt);
                return;
            }
            self.editor.handle_input(data);
            return;
        }
        if kb.matches(data, "app.exit") && self.editor.get_text().is_empty() {
            self.actions.borrow_mut().push(Action::Exit);
            return;
        }
        let extra = [("app.thinking.cycleBackward", Action::ThinkingBackward)];
        for (id, action) in EDITOR_ACTIONS.iter().chain(extra.iter()) {
            if kb.matches(data, id) {
                self.actions.borrow_mut().push(action.clone());
                return;
            }
        }
        self.editor.handle_input(data);
    }

    fn invalidate(&mut self) {
        self.editor.invalidate();
    }

    fn is_focusable(&self) -> bool {
        true
    }

    fn set_focused(&mut self, focused: bool) {
        self.editor.set_focused(focused);
    }
}

/// `/model <prefix>` completions: the scoped models, else the available
/// ones, fuzzy-matched on id and provider (so "opus anthropic" matches).
fn model_argument_completions(
    session: &AgentSession,
    prefix: &str,
) -> Option<Vec<AutocompleteItem>> {
    let scoped = session.scoped_models();
    let models: Vec<Model> = if scoped.is_empty() {
        session.get_available_models()
    } else {
        scoped.into_iter().map(|s| s.model).collect()
    };
    if models.is_empty() {
        return None;
    }
    let filtered =
        hoocode_tui_fuzzy::fuzzy_filter(&models, prefix, |m| format!("{} {}", m.id, m.provider));
    if filtered.is_empty() {
        return None;
    }
    Some(
        filtered
            .into_iter()
            .map(|m| AutocompleteItem {
                value: format!("{}/{}", m.provider, m.id),
                label: m.id,
                description: Some(m.provider),
            })
            .collect(),
    )
}

/// `getPathArgument`: the first argument after `command`, quoted or not.
pub fn command_path_argument(text: &str, command: &str) -> Option<String> {
    let args = text.strip_prefix(command)?.strip_prefix(' ')?.trim_start();
    let first = args.chars().next()?;
    if first == '"' || first == '\'' {
        let rest = hoocode_tui_util::text_slice::suffix_from(args, 1);
        return rest
            .find(first)
            .map(|end| hoocode_tui_util::text_slice::prefix(rest, end).to_string());
    }
    Some(
        args.split(char::is_whitespace)
            .next()
            .unwrap_or(args)
            .to_string(),
    )
}

/// `os.homedir()`.
pub(super) fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// `path.resolve(base, path)`: absolute and normalized, without touching
/// the filesystem.
pub(super) fn resolve_path(base: &Path, path: &Path) -> PathBuf {
    let joined = base.join(path);
    let mut out = PathBuf::new();
    for component in joined.components() {
        match component {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// `getChangeDirectoryCompletions`: directories for `/cd <prefix>`, plus
/// `-` and `~` when nothing is typed yet. At most 50.
fn change_directory_completions(
    cwd: &Path,
    previous_cwd: Option<&Path>,
    home: &Path,
    prefix: &str,
) -> Vec<AutocompleteItem> {
    let expanded = match prefix.strip_prefix("~/") {
        Some(rest) => format!("{}/{rest}", home.to_string_lossy()),
        None => prefix.to_string(),
    };
    let ends_with_sep = expanded.ends_with('/') || expanded.ends_with(std::path::MAIN_SEPARATOR);
    let (base, partial) = if ends_with_sep {
        (expanded.clone(), String::new())
    } else {
        let path = Path::new(&expanded);
        let base = match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_string_lossy().into_owned(),
            _ if expanded.starts_with('/') => "/".to_string(),
            _ => ".".to_string(),
        };
        let partial = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        (base, partial)
    };
    let search_dir = if Path::new(&base).is_absolute() {
        PathBuf::from(&base)
    } else {
        resolve_path(cwd, Path::new(if base.is_empty() { "." } else { &base }))
    };

    let mut completions = Vec::new();
    if prefix.is_empty() {
        if let Some(previous) = previous_cwd {
            completions.push(AutocompleteItem {
                value: "-".into(),
                label: format!("- ({})", previous.display()),
                description: None,
            });
        }
        completions.push(AutocompleteItem {
            value: "~".into(),
            label: format!("~ ({})", home.display()),
            description: None,
        });
    }
    let Ok(entries) = std::fs::read_dir(&search_dir) else {
        return completions;
    };
    // Node's readdirSync (libuv scandir) returns names sorted.
    let mut dirs: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_dir()))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    dirs.sort();
    for name in dirs {
        if !partial.is_empty() && !name.starts_with(&partial) {
            continue;
        }
        if partial.is_empty() && name.starts_with('.') {
            continue;
        }
        let joined = if !ends_with_sep {
            let dir = Path::new(prefix)
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            match dir.as_str() {
                "" | "." => name.clone(),
                "/" => format!("/{name}"),
                _ => format!("{dir}/{name}"),
            }
        } else {
            format!("{prefix}{name}")
        };
        completions.push(AutocompleteItem {
            value: format!("{joined}/"),
            label: format!("{name}/"),
            description: None,
        });
    }
    completions.truncate(50);
    completions
}

/// `resolveVoiceSilenceMs`: `VOICETOOLS_SILENCE_MS` (clamped to
/// 300-10000) wins over the setting.
pub(super) fn resolve_voice_silence_ms(setting: u64) -> u64 {
    std::env::var("VOICETOOLS_SILENCE_MS")
        .ok()
        .and_then(|raw| raw.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v > 0.0)
        .map(|v| (v.floor() as u64).clamp(300, 10000))
        .unwrap_or(setting)
}

/// `getAutocompleteSourceTag`: u/p/t, plus the package for npm and git.
fn autocomplete_source_tag(source_info: &serde_json::Value) -> Option<String> {
    let scope = source_info.get("scope")?.as_str()?;
    let prefix = match scope {
        "user" => "u",
        "project" => "p",
        _ => "t",
    };
    let source = source_info
        .get("source")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .trim();
    if matches!(source, "auto" | "local" | "cli") {
        return Some(prefix.to_string());
    }
    if source.starts_with("npm:") {
        return Some(format!("{prefix}:{source}"));
    }
    if let Some(git) = hoocode_code_paths::git::parse_git_url(source) {
        let git_ref = git.git_ref.map(|r| format!("@{r}")).unwrap_or_default();
        return Some(format!("{prefix}:git:{}/{}{git_ref}", git.host, git.path));
    }
    Some(prefix.to_string())
}

/// `prefixAutocompleteDescription`.
fn prefix_autocomplete_description(
    description: Option<String>,
    source_info: &serde_json::Value,
) -> Option<String> {
    let Some(tag) = autocomplete_source_tag(source_info) else {
        return description;
    };
    Some(match description {
        Some(d) if !d.is_empty() => format!("[{tag}] {d}"),
        _ => format!("[{tag}]"),
    })
}

impl Mode {
    /// The editor's `onChange`: entering or leaving bash mode.
    fn on_editor_change(&mut self, text: &str) {
        let was_bash_mode = self.is_bash_mode;
        self.is_bash_mode = text.trim_start().starts_with('!');
        if was_bash_mode != self.is_bash_mode {
            self.update_editor_border_color();
            self.update_editor_prompt_prefix();
        }
    }

    pub(super) fn prompt(&mut self, text: String) {
        self.prompt_with_images(text, Vec::new());
    }

    pub(super) fn prompt_with_images(&mut self, text: String, images: Vec<ImageContent>) {
        startup_progress::clear();
        let session = self.session.clone();
        let tx = self.tx.clone();
        self.runtime.spawn(async move {
            let result = session
                .prompt(
                    &text,
                    PromptOptions {
                        expand_prompt_templates: true,
                        images,
                        ..Default::default()
                    },
                )
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(AppEvent::PromptDone(result));
        });
    }

    pub(super) fn submit(&mut self, text: String) {
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }
        // A session change is running: the text goes back to the prompt.
        if let Some(running) = self.session_op {
            self.show_warning(&format!(
                "{running} is still running; wait for it to finish."
            ));
            self.editor.borrow_mut().editor.set_text(&text);
            return;
        }
        // Built-in slash commands; `with_args` ones also take "/name <args>".
        let (name, has_args) = match text.find(' ') {
            Some(i) => (hoocode_tui_util::text_slice::prefix(&text, i), true),
            None => (text.as_str(), false),
        };
        if let Some(command) = BuiltinCommand::lookup(name) {
            if !has_args || command.with_args() {
                self.run_builtin_command(command, &text);
                return;
            }
        }
        // `!` runs bash; `!!` keeps it out of the model's context.
        if let Some(rest) = text.strip_prefix('!') {
            let exclude_from_context = rest.starts_with('!');
            let command = if exclude_from_context {
                hoocode_tui_util::text_slice::suffix_from(rest, 1)
            } else {
                rest
            }
            .trim();
            if !command.is_empty() {
                if self.session.is_bash_running() {
                    self.show_warning(
                        "A bash command is already running. Press Esc to cancel it first.",
                    );
                    self.editor.borrow_mut().editor.set_text(&text);
                    return;
                }
                self.editor.borrow_mut().editor.add_to_history(&text);
                self.handle_bash_command(command.to_string(), exclude_from_context);
                self.leave_bash_mode();
                return;
            }
        }
        // Typed during a compaction: held until it ends.
        if self.session.is_compacting() {
            self.queue_compaction_message(text, StreamingBehavior::Steer);
            return;
        }
        // Typed while the agent works: steers the current run.
        if self.session.is_streaming() {
            self.editor.borrow_mut().editor.add_to_history(&text);
            self.queue_prompt(text, StreamingBehavior::Steer);
            return;
        }
        self.flush_pending_bash_components();
        self.editor.borrow_mut().editor.add_to_history(&text);
        self.prompt(text);
    }

    /// Escape twice within 500ms on an empty prompt: the tree, the fork
    /// picker or nothing, as `doubleEscapeAction` says.
    fn handle_double_escape(&mut self) {
        let action = self.session.settings().double_escape_action();
        if action == DoubleEscapeAction::None {
            return;
        }
        let now = Instant::now();
        if self
            .last_escape
            .is_some_and(|at| now.duration_since(at) < Duration::from_millis(500))
        {
            self.last_escape = None;
            // `fork` opens the fork picker, which is wired with /fork (11.4).
            if action == DoubleEscapeAction::Tree {
                self.show_tree_selector(None);
            }
        } else {
            self.last_escape = Some(now);
        }
    }

    /// `cycleAgentMode`: step the agent mode through `/mode`'s argument
    /// completions (ask → plan → build → debug → ask) by running `/mode
    /// <next>`, then name the mode the session landed in.
    fn cycle_agent_mode(&mut self, forward: bool) {
        let extensions = self.session.extensions().clone();
        if !extensions.has_command("mode") {
            self.show_warning("Modes are not available in this session");
            return;
        }
        let modes = extensions
            .argument_completions("mode", "")
            .unwrap_or_default();
        if modes.is_empty() {
            self.show_warning("No modes are configured");
            return;
        }
        let current = self.footer_data.get_active_mode();
        // An active mode missing from the list steps to the first mode going
        // forward and the last going back.
        let len = modes.len() as isize;
        let index = modes
            .iter()
            .position(|m| *m == current)
            .map_or(-1, |i| i as isize);
        let step = if forward { 1 } else { -1 };
        let next = modes[((index + step + len) % len) as usize].clone();
        if !self.session_op_free() {
            return;
        }
        self.start_session_op("Switching mode");
        let session = self.session.clone();
        let text = format!("/mode {next}");
        let tx = self.tx.clone();
        self.runtime.spawn(async move {
            let result = session
                .prompt(
                    &text,
                    PromptOptions {
                        expand_prompt_templates: true,
                        ..Default::default()
                    },
                )
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(AppEvent::SessionOp(Box::new(SessionOpDone {
                runtime: None,
                outcome: SessionOutcome::Mode {
                    forward,
                    next,
                    result,
                },
            })));
        });
    }

    /// `createBaseAutocompleteProvider` + `setupAutocompleteProvider`: the
    /// built-in commands, then prompt templates, then skill commands. `@`
    /// file completion uses [`at_file_finder`], an in-process walk, so it
    /// works without `fd` installed.
    pub(super) fn setup_autocomplete_provider(&mut self) {
        let mut commands: Vec<CommandEntry> = BUILTIN_SLASH_COMMANDS
            .iter()
            .map(|c| {
                CommandEntry::Slash(SlashCommand {
                    name: c.name.to_string(),
                    description: Some(c.description.to_string()),
                    argument_hint: (c.name == "cd").then(|| "<path>".to_string()),
                    get_argument_completions: if c.name == "model" {
                        let session = self.session.clone();
                        Some(Box::new(move |prefix: &str| {
                            model_argument_completions(&session, prefix)
                        }) as ArgumentCompletionsFn)
                    } else {
                        (c.name == "cd").then(|| {
                            let session = self.session.clone();
                            let previous = self.previous_cwd.clone();
                            Box::new(move |prefix: &str| {
                                let completions = change_directory_completions(
                                    session.cwd(),
                                    previous.borrow().as_deref(),
                                    &home_dir(),
                                    prefix,
                                );
                                (!completions.is_empty()).then_some(completions)
                            }) as ArgumentCompletionsFn
                        })
                    },
                })
            })
            .collect();
        let skill_commands = self.session.settings().enable_skill_commands();
        let (templates, skills): (Vec<_>, Vec<_>) = self
            .session
            .resource_loader()
            .slash_commands()
            .into_iter()
            .partition(|info| info.source != "skill");
        for info in templates {
            commands.push(CommandEntry::Slash(SlashCommand {
                description: prefix_autocomplete_description(info.description, &info.source_info),
                name: info.name,
                argument_hint: None,
                get_argument_completions: None,
            }));
        }
        // Extension commands a built-in does not shadow, with their own
        // argument completions.
        let extensions = self.session.extensions().clone();
        for info in extensions.commands() {
            if BUILTIN_SLASH_COMMANDS.iter().any(|c| c.name == info.name) {
                continue;
            }
            let completer = extensions.clone();
            let name = info.name.clone();
            commands.push(CommandEntry::Slash(SlashCommand {
                description: prefix_autocomplete_description(info.description, &info.source_info),
                name: info.name,
                argument_hint: None,
                get_argument_completions: Some(Box::new(move |prefix: &str| {
                    let values = completer.argument_completions(&name, prefix)?;
                    Some(
                        values
                            .into_iter()
                            .map(|value| AutocompleteItem {
                                label: value.clone(),
                                value,
                                description: None,
                            })
                            .collect(),
                    )
                }) as ArgumentCompletionsFn),
            }));
        }
        for info in skills {
            if !skill_commands {
                continue;
            }
            commands.push(CommandEntry::Slash(SlashCommand {
                description: prefix_autocomplete_description(info.description, &info.source_info),
                name: info.name,
                argument_hint: None,
                get_argument_completions: None,
            }));
        }
        let provider = CombinedAutocompleteProvider::new(
            commands,
            self.session.cwd().to_path_buf(),
            Some(at_file_finder()),
        );
        self.editor
            .borrow_mut()
            .editor
            .set_autocomplete_provider(Box::new(provider));
    }

    /// `openExternalEditor`: edit the prompt in `$VISUAL` / `$EDITOR`.
    fn open_external_editor(&mut self) {
        let editor_cmd = std::env::var("VISUAL")
            .ok()
            .filter(|v| !v.is_empty())
            .or_else(|| std::env::var("EDITOR").ok().filter(|v| !v.is_empty()));
        let Some(editor_cmd) = editor_cmd else {
            self.show_warning("No editor configured. Set $VISUAL or $EDITOR environment variable.");
            return;
        };
        let current = self.editor.borrow().editor.get_expanded_text();
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        let tmp_file =
            std::env::temp_dir().join(format!("{APP_NAME}-editor-{millis}.{APP_NAME}.md"));
        if std::fs::write(&tmp_file, &current).is_ok() {
            self.tui.stop();
            // Split on spaces for editor arguments (e.g. "code --wait").
            let mut parts = editor_cmd.split(' ').filter(|p| !p.is_empty());
            let program = parts.next().unwrap_or_default().to_string();
            let status = std::process::Command::new(&program)
                .args(parts)
                .arg(&tmp_file)
                .status();
            if status.is_ok_and(|s| s.success()) {
                if let Ok(content) = std::fs::read_to_string(&tmp_file) {
                    let content = content.strip_suffix('\n').unwrap_or(&content);
                    self.editor.borrow_mut().editor.set_text(content);
                }
            }
            self.restarted_input = Some(self.tui.start());
            // The editor used the alternate screen: redraw everything.
            self.tui.request_render(true);
        }
        let _ = std::fs::remove_file(&tmp_file);
        self.dirty.set(true);
    }

    pub(super) fn handle_action(&mut self, action: Action) {
        match action {
            Action::Submit(text) => self.submit(text),
            Action::Interrupt => {
                if self.tree_navigation.is_some() {
                    // Escape cancels a branch summary while it runs.
                    self.session.abort_branch_summary();
                } else if self.compaction_loader.is_some() {
                    self.session.abort_compaction();
                } else if self.session.is_streaming() {
                    // Queued messages come back to the prompt, then the run stops.
                    self.restore_queued_messages_to_editor(true);
                } else if self.session.is_bash_running() {
                    self.session.abort_bash();
                } else if self.is_bash_mode {
                    self.editor.borrow_mut().editor.set_text("");
                    self.leave_bash_mode();
                } else if self.editor.borrow().editor.get_text().trim().is_empty() {
                    self.handle_double_escape();
                }
            }
            Action::Clear => {
                let now = Instant::now();
                if self
                    .last_sigint
                    .is_some_and(|at| now.duration_since(at) < Duration::from_millis(500))
                {
                    self.exit_requested = true;
                } else {
                    self.editor.borrow_mut().editor.set_text("");
                    self.last_sigint = Some(now);
                    self.dirty.set(true);
                }
            }
            Action::Exit => self.exit_requested = true,
            Action::Suspend => self.handle_ctrl_z(),
            Action::ToolsExpand => self.jump_to_full_view(),
            Action::ViewForward | Action::ViewBackward => {
                self.cycle_tool_output_view(action == Action::ViewForward)
            }
            Action::ChromeForward | Action::ChromeBackward => {
                let density = self.chrome.cycle_density(action == Action::ChromeForward);
                let mut settings = self.session.settings();
                settings.set_chrome_density(density);
                drop(settings);
                self.show_dial_step("app.chrome.cycleBackward", &format!("Chrome: {density}"));
            }
            Action::ThinkingForward | Action::ThinkingBackward => {
                let direction = if action == Action::ThinkingForward {
                    hoocode_code_agent_session::CycleDirection::Forward
                } else {
                    hoocode_code_agent_session::CycleDirection::Backward
                };
                match self.session.cycle_thinking_level(direction) {
                    None => self.show_status("Current model does not support thinking"),
                    Some(level) => {
                        self.update_editor_border_color();
                        self.show_dial_step(
                            "app.thinking.cycleBackward",
                            &format!("Thinking level: {}", level.as_str()),
                        );
                    }
                }
            }
            Action::ModelForward | Action::ModelBackward => {
                self.cycle_model(action == Action::ModelForward)
            }
            Action::ModelSelect => self.show_model_selector(None),
            Action::SettingsOpen => self.show_settings_selector(),
            Action::SelectorDone(outcome) => self.close_selector(outcome),
            Action::ResumeSession => self.show_session_selector(),
            Action::ThinkingToggle => self.toggle_thinking_block_visibility(),
            Action::ExternalEditor => self.open_external_editor(),
            Action::SessionNew => self.handle_new_command(),
            Action::SessionTree => self.show_tree_selector(None),
            Action::ModeForward => self.cycle_agent_mode(true),
            Action::ModeBackward => self.cycle_agent_mode(false),
            Action::EditorChanged(text) => self.on_editor_change(&text),
            Action::CopyMessage => self.handle_copy_command(""),
            Action::PasteImage => self.handle_clipboard_image_paste(),
            Action::HotkeysOpen => self.handle_hotkeys_command(),
            Action::SessionColorForward | Action::SessionColorBackward => {
                self.cycle_session_color(action == Action::SessionColorForward)
            }
            Action::SessionColorPreview(slot) => self.preview_session_color(slot),
            Action::SessionColorDone(slot) => self.close_session_color_selector(slot),
            Action::ChangeDirectoryPrefill => {
                // Prefill rather than act: the editor's path completion is the
                // way to name the target.
                self.editor.borrow_mut().editor.set_text("/cd ");
                self.dirty.set(true);
            }
            Action::ForkOpen => self.show_user_message_selector(),
            Action::FollowUp => self.handle_follow_up(),
            Action::Dequeue => self.handle_dequeue(),
            Action::TasksForward | Action::TasksBackward => {
                let forward = action == Action::TasksForward;
                let view = self.task_panel.borrow_mut().cycle_view(forward);
                let label = match view {
                    TaskPanelView::Plan => "tasks",
                    TaskPanelView::Subagents => "subagents",
                };
                self.show_dial_step(
                    if forward {
                        "app.tasks.cycleBackward"
                    } else {
                        "app.tasks.cycleForward"
                    },
                    &format!("Task panel: {label}"),
                );
            }
            Action::AskOptionsDone(answers) => self.hide_ask_options(answers),
            Action::EditorDialogDone(outcome) => self.close_editor_dialog(outcome),
            Action::SessionSelectorDone(path) => {
                self.close_session_selector();
                if let Some(path) = path {
                    self.handle_resume_session(path, None);
                }
            }
            Action::AutocompleteVisibility(visible) => {
                if self.chrome.set_autocomplete_open(visible) {
                    self.dirty.set(true);
                }
            }
        }
    }
}

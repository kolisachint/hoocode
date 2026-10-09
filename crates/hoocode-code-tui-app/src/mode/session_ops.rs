//! Session operations: new, resume, fork, clone, name, compact, `/cd`, reload, export and import.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self};
use std::time::Instant;

use hoocode_agent_types::AgentEvent;
use hoocode_code_agent_session::format::group_digits;
use hoocode_code_agent_session::runtime::{format_missing_session_cwd_prompt, RuntimeError};
use hoocode_code_agent_session::stats::sum_assistant_usage;
use hoocode_code_agent_session::{AgentSessionEvent, ForkPosition, NewSessionRequest};
use hoocode_code_session::SessionManager;
use hoocode_code_settings::EditorBorder;
use hoocode_code_tui_keybindings::{key_display_text, AppKeybindingsManager};
use hoocode_code_tui_selectors::session_selector::{
    SessionSelectorComponent, SessionSelectorOptions,
};
use hoocode_code_tui_selectors::user_message_selector::{
    UserMessageEvent, UserMessageItem, UserMessageSelectorComponent,
};
use hoocode_code_tui_theme::{set_theme, theme};
use hoocode_code_tui_widgets::dynamic_border::DynamicBorder;
use hoocode_tui_components::{FrameBorderStyle, Spacer, Text};
use hoocode_tui_render::{Component, Container};

use crate::expandable_text::Expandable;
use crate::input_frame::set_input_frame_border;
use crate::session_picker;

use super::*;

impl Mode {
    pub(super) fn subscribe(&mut self) -> hoocode_code_agent_session::SessionSubscription {
        let tx = self.tx.clone();
        let session = self.session.clone();
        self.session.subscribe(move |event| {
            let anchor =
                matches!(event, AgentSessionEvent::Agent(AgentEvent::AgentStart)).then(|| {
                    let totals = sum_assistant_usage(session.session_manager().entries());
                    (totals, Instant::now())
                });
            let _ = tx.send(AppEvent::Session(Box::new(event.clone()), anchor));
        })
    }

    /// `showUserMessageSelector`: `/fork` picks a user message; the branch
    /// before it becomes a new session and its text returns to the prompt.
    pub(super) fn show_user_message_selector(&mut self) {
        let messages = self.session.get_user_messages_for_forking();
        let Some(last) = messages.last() else {
            self.show_status("No messages to fork from");
            return;
        };
        let initial = last.entry_id.clone();
        let items = messages
            .into_iter()
            .map(|m| UserMessageItem {
                id: m.entry_id,
                text: m.text,
                timestamp: None,
            })
            .collect();
        let selector = handle(UserMessageSelectorComponent::new(items, Some(&initial)));
        self.show_in_editor_slot(as_component(&selector));
        self.fork_selector = Some(selector);
    }

    pub(super) fn poll_fork_selector(&mut self) {
        let Some(selector) = &self.fork_selector else {
            return;
        };
        let Some(event) = selector
            .borrow_mut()
            .poll(Instant::now())
            .into_iter()
            .next()
        else {
            return;
        };
        self.fork_selector = None;
        self.restore_editor();
        if let UserMessageEvent::Select(entry_id) = event {
            self.fork_session(&entry_id, ForkPosition::Before);
        }
    }

    /// `runtimeHost.fork`, then the new session's transcript. `Before` puts
    /// the forked message's text back in the prompt (`/fork`); `At` copies
    /// the branch whole (`/clone`).
    fn fork_session(&mut self, entry_id: &str, position: ForkPosition) {
        self.stop_working_loader();
        let handle_rt = self.runtime.clone();
        let Some(runtime) = self.session_runtime.as_mut() else {
            return;
        };
        match handle_rt.block_on(runtime.fork(entry_id, position)) {
            Ok(result) if result.cancelled => self.dirty.set(true),
            Ok(result) => {
                self.rebind_current_session();
                self.render_current_session_state();
                self.editor
                    .borrow_mut()
                    .editor
                    .set_text(result.selected_text.as_deref().unwrap_or(""));
                self.show_status(if position == ForkPosition::At {
                    "Cloned to new session"
                } else {
                    "Forked to new session"
                });
            }
            Err(error) => self.show_error(&error.to_string()),
        }
    }

    /// `handleClone`: the whole current branch into a new session.
    pub(super) fn handle_clone_command(&mut self) {
        let leaf = self.session.session_manager().leaf_id().map(str::to_string);
        match leaf {
            Some(leaf) => self.fork_session(&leaf, ForkPosition::At),
            None => self.show_status("Nothing to clone yet"),
        }
    }

    /// `handleChangeDirectory`: `/cd [path|~|-]` moves the runtime to a new
    /// session in the target directory.
    pub(super) fn handle_change_directory(&mut self, text: &str) {
        let raw = text.strip_prefix("/cd").unwrap_or(text).trim();
        let previous_cwd = self.session.cwd().to_path_buf();
        let home = home_dir();
        let target = if raw.is_empty() || raw == "~" {
            home.clone()
        } else if raw == "-" {
            let Some(previous) = self.previous_cwd.borrow().clone() else {
                self.show_warning("No previous directory to return to");
                return;
            };
            previous
        } else {
            let expanded = match raw.strip_prefix("~/") {
                Some(rest) => home.join(rest),
                None => PathBuf::from(raw),
            };
            resolve_path(&previous_cwd, &expanded)
        };
        if resolve_path(Path::new("/"), &target) == resolve_path(Path::new("/"), &previous_cwd) {
            self.show_status(&format!("Already in {}", target.display()));
            return;
        }

        self.stop_working_loader();
        let handle_rt = self.runtime.clone();
        let Some(runtime) = self.session_runtime.as_mut() else {
            return;
        };
        match handle_rt.block_on(runtime.change_directory(&target)) {
            Ok(result) if result.cancelled => {}
            Ok(result) => {
                *self.previous_cwd.borrow_mut() = Some(previous_cwd.clone());
                self.rebind_current_session();
                self.render_current_session_state();
                let t = theme();
                let line = format!(
                    "{} {}\n{}",
                    t.fg("accent", "✓ Working directory"),
                    t.fg("muted", &result.cwd.to_string_lossy()),
                    t.fg(
                        "dim",
                        &format!(
                            "New session started here. {} reopens the session you left in {}.",
                            key_display_text("app.session.resume"),
                            previous_cwd.display()
                        )
                    )
                );
                self.add_to_chat(as_component(&handle(Spacer::new(1))));
                self.add_to_chat(as_component(&handle(Text::new(line, 1, 0))));
            }
            Err(RuntimeError::ChangeDirectory { message, .. }) => self.show_error(&message),
            Err(error) => {
                // `handleFatalRuntimeError`.
                self.show_error(&format!(
                    "Failed to change directory to {}: {error}",
                    target.display()
                ));
                self.exit_requested = true;
            }
        }
        self.dirty.set(true);
    }

    /// `handleReloadCommand`: re-read keybindings, settings, resources and
    /// themes, then replay the transcript with the listing below it.
    pub(super) fn handle_reload_command(&mut self) {
        if self.session.is_streaming() {
            self.show_warning("Wait for the current response to finish before reloading.");
            return;
        }
        if self.session.is_compacting() {
            self.show_warning("Wait for compaction to finish before reloading.");
            return;
        }
        let t = theme();
        let mut reload_box = Container::new();
        reload_box.add_child(as_component(&handle(DynamicBorder::new(Some(Box::new(
            |s: &str| theme().fg("border", s),
        ))))));
        reload_box.add_child(as_component(&handle(Text::new(
            t.fg(
                "muted",
                "Reloading keybindings, extensions, skills, prompts, themes...",
            ),
            1,
            0,
        ))));
        reload_box.add_child(as_component(&handle(DynamicBorder::new(Some(Box::new(
            |s: &str| theme().fg("border", s),
        ))))));
        self.show_in_editor_slot(as_component(&handle(reload_box)));
        self.tui.request_render(true);

        let session = self.session.clone();
        self.runtime.block_on(async move { session.reload().await });
        self.apply_runtime_settings();
        self.apply_session_theme();
        self.update_editor_border_color();
        self.setup_autocomplete_provider();
        self.reset_transcript_view();
        // From the session file, as the pin replays it: messages recorded
        // only there (a `/subagent` answer) show too.
        let messages = self.session.session_manager().build_context().messages;
        self.render_session_context(&messages, false);
        self.restore_editor();
        // Below the replayed transcript: /reload is asked for to see what
        // just loaded.
        self.render_resources();
        self.update_available_provider_count();
        if let Some(error) = self.session.model_registry().error() {
            self.show_error(&format!("models.json error: {error}"));
        }
        self.show_status("Reloaded keybindings, extensions, skills, prompts, themes");
    }

    /// `handleExport`: `/export <path.jsonl>` writes the branch as JSONL.
    /// The HTML export (every other path) is export-html, owned by 12.7.
    pub(super) fn handle_export_command(&mut self, text: &str) {
        let output = command_path_argument(text, "/export");
        match output.as_deref() {
            Some(path) if path.ends_with(".jsonl") => {
                match self.session.export_to_jsonl(Some(Path::new(path))) {
                    Ok(file) => {
                        self.show_record(&format!("Session exported to: {}", file.display()))
                    }
                    Err(error) => {
                        self.show_error(&format!("Failed to export session: {error}"))
                    }
                }
            }
            _ => self.show_error(
                "Failed to export session: HTML export is not available yet; use /export <file.jsonl>",
            ),
        }
    }

    /// `handleImport`: `/import <path.jsonl>` replaces the session, after a
    /// confirm.
    pub(super) fn handle_import_command(&mut self, text: &str) {
        let Some(input) = command_path_argument(text, "/import") else {
            self.show_error("Usage: /import <path.jsonl>");
            return;
        };
        let (reply, answer) = mpsc::channel();
        self.show_selector(
            &format!("Import session\nReplace current session with {input}?"),
            vec!["Yes".into(), "No".into()],
            reply,
        );
        self.pending_import = Some((input, None, answer));
    }

    /// An `/import` confirm answered.
    pub(super) fn poll_import_confirm(&mut self) {
        let Some((_, _, answer)) = &self.pending_import else {
            return;
        };
        let confirmed = match answer.try_recv() {
            Ok(choice) => choice.as_deref() == Some("Yes"),
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => false,
        };
        let Some((input, fallback, _)) = self.pending_import.take() else {
            return;
        };
        if confirmed {
            self.import_session(input, fallback);
        } else {
            self.show_status("Import cancelled");
        }
    }

    /// `runtimeHost.importFromJsonl`; a stored cwd that is gone asks first.
    fn import_session(&mut self, input: String, cwd_override: Option<String>) {
        self.stop_working_loader();
        let overridden = cwd_override.is_some();
        let handle_rt = self.runtime.clone();
        let Some(runtime) = self.session_runtime.as_mut() else {
            return;
        };
        match handle_rt.block_on(runtime.import_from_jsonl(Path::new(&input), cwd_override)) {
            Ok(result) if result.cancelled => self.show_status("Import cancelled"),
            Ok(_) => {
                self.rebind_current_session();
                self.render_current_session_state();
                self.show_record(&format!("Session imported from: {input}"));
            }
            Err(RuntimeError::MissingSessionCwd(issue)) if !overridden => {
                let (reply, answer) = mpsc::channel();
                let title = format!(
                    "Session cwd not found\n{}",
                    format_missing_session_cwd_prompt(&issue)
                );
                self.show_selector(&title, vec!["Yes".into(), "No".into()], reply);
                self.pending_import = Some((input, Some(issue.fallback_cwd), answer));
            }
            Err(error @ RuntimeError::ImportFileNotFound(_)) => {
                self.show_error(&format!("Failed to import session: {error}"))
            }
            Err(error) => {
                // `handleFatalRuntimeError`.
                self.show_error(&format!("Failed to import session: {error}"));
                self.exit_requested = true;
            }
        }
        self.dirty.set(true);
    }

    /// `handleName`.
    pub(super) fn handle_name_command(&mut self, text: &str) {
        let name = text.strip_prefix("/name").unwrap_or("").trim();
        let t = theme();
        if name.is_empty() {
            let label = if self.session.session_name().is_some() {
                "Session name:"
            } else {
                "Session name (auto):"
            };
            let line = format!(
                "{} {}  {}",
                t.fg("dim", label),
                self.current_chip(),
                t.fg("dim", "/name <name> to change")
            );
            self.add_to_chat(as_component(&handle(Spacer::new(1))));
            self.add_to_chat(as_component(&handle(Text::new(line, 1, 0))));
            return;
        }
        self.session.set_session_name(name);
        let message = format!(
            "{} {}",
            t.fg("dim", "Session name set:"),
            self.current_chip()
        );
        self.show_status(&message);
    }

    /// `handleSession`: the session's facts in the transcript.
    pub(super) fn handle_session_command(&mut self) {
        let stats = self.session.get_session_stats();
        let t = theme();
        let dim = |s: &str| t.fg("dim", s);
        let mut info = format!("{}\n\n", t.bold("Session Info"));
        let name_label = if self.session.session_name().is_some() {
            "Name:"
        } else {
            "Name (auto):"
        };
        info += &format!("{} {}\n", dim(name_label), self.current_chip());
        if let Some(branch) = self.session.session_manager().session_branch() {
            info += &format!("{} {branch}\n", dim("Branch:"));
        }
        info += &format!(
            "{} {}\n",
            dim("File:"),
            stats.session_file.as_deref().unwrap_or("In-memory")
        );
        info += &format!("{} {}\n\n", dim("ID:"), stats.session_id);
        info += &format!("{}\n", t.bold("Messages"));
        info += &format!("{} {}\n", dim("User:"), stats.user_messages);
        info += &format!("{} {}\n", dim("Assistant:"), stats.assistant_messages);
        info += &format!("{} {}\n", dim("Tool Calls:"), stats.tool_calls);
        info += &format!("{} {}\n", dim("Tool Results:"), stats.tool_results);
        info += &format!("{} {}\n\n", dim("Total:"), stats.total_messages);
        info += &format!("{}\n", t.bold("Tokens"));
        info += &format!("{} {}\n", dim("Input:"), group_digits(stats.tokens.input));
        info += &format!("{} {}\n", dim("Output:"), group_digits(stats.tokens.output));
        if stats.tokens.cache_read > 0 {
            info += &format!(
                "{} {}\n",
                dim("Cache Read:"),
                group_digits(stats.tokens.cache_read)
            );
        }
        if stats.tokens.cache_write > 0 {
            info += &format!(
                "{} {}\n",
                dim("Cache Write:"),
                group_digits(stats.tokens.cache_write)
            );
        }
        info += &format!("{} {}\n", dim("Total:"), group_digits(stats.tokens.total));
        if stats.cost > 0.0 {
            info += &format!("\n{}\n", t.bold("Cost"));
            info += &format!("{} {:.4}", dim("Total:"), stats.cost);
        }
        self.add_to_chat(as_component(&handle(Spacer::new(1))));
        self.add_to_chat(as_component(&handle(Text::new(info, 1, 0))));
    }

    /// `handleClear` (`/new`): a fresh session in the same directory.
    pub(super) fn handle_new_command(&mut self) {
        self.stop_working_loader();
        let handle_rt = self.runtime.clone();
        let Some(runtime) = self.session_runtime.as_mut() else {
            return;
        };
        match handle_rt.block_on(runtime.new_session(NewSessionRequest::default())) {
            Ok(result) if result.cancelled => {}
            Ok(_) => {
                self.rebind_current_session();
                self.render_current_session_state();
                self.add_to_chat(as_component(&handle(Spacer::new(1))));
                let line = theme().fg("accent", "✓ New session started");
                self.add_to_chat(as_component(&handle(Text::new(line, 1, 0))));
            }
            Err(error) => {
                // `handleFatalRuntimeError`.
                self.show_error(&format!("Failed to create session: {error}"));
                self.exit_requested = true;
            }
        }
    }

    /// `handleCompactCommand`: the session reports the outcome through its
    /// compaction events.
    pub(super) fn handle_compact_command(&mut self, instructions: Option<String>) {
        self.stop_working_loader();
        let session = self.session.clone();
        self.runtime.spawn(async move {
            let _ = session.compact(instructions.as_deref()).await;
        });
    }

    /// `showSessionSelector`: the session picker in the editor's slot.
    pub(super) fn show_session_selector(&mut self) {
        if self.session_runtime.is_none() || self.session_selector.is_some() {
            return;
        }
        let (session_dir, current_file) = {
            let manager = self.session.session_manager();
            let dir = manager.session_dir().to_path_buf();
            let dir = if dir.as_os_str().is_empty() {
                hoocode_code_session::default_session_dir(manager.cwd())
            } else {
                dir
            };
            (dir, manager.session_file().map(Path::to_path_buf))
        };
        let on_select = self.actions.clone();
        let on_cancel = self.actions.clone();
        let selector = handle(SessionSelectorComponent::new(
            session_picker::current_sessions_loader(session_dir),
            session_picker::all_sessions_loader(),
            Box::new(move |path| {
                on_select
                    .borrow_mut()
                    .push(Action::SessionSelectorDone(Some(path)))
            }),
            Box::new(move || {
                on_cancel
                    .borrow_mut()
                    .push(Action::SessionSelectorDone(None))
            }),
            SessionSelectorOptions {
                rename_session: Some(Box::new(|path: &Path, next: &str| {
                    let next = next.trim();
                    if next.is_empty() {
                        return Ok(());
                    }
                    let mut manager = SessionManager::open(path, None, None);
                    manager.append_session_info(Some(next), None);
                    Ok(())
                })),
                show_rename_hint: Some(true),
                keybindings: None,
            },
            current_file.as_deref(),
        ));
        {
            let mut container = self.editor_container.borrow_mut();
            container.clear();
            container.add_child(as_component(&selector));
        }
        self.tui.set_focus(Some(as_component(&selector)));
        self.session_selector = Some(selector);
        self.dirty.set(true);
    }

    pub(super) fn close_session_selector(&mut self) {
        if self.session_selector.take().is_none() {
            return;
        }
        {
            let mut container = self.editor_container.borrow_mut();
            container.clear();
            container.add_child(as_component(&self.editor));
        }
        self.tui.set_focus(Some(as_component(&self.editor)));
        self.dirty.set(true);
    }

    /// `handleResumeSession`: swap to `path`; a stored cwd that is gone asks
    /// first (`promptForMissingSessionCwd`).
    pub(super) fn handle_resume_session(&mut self, path: PathBuf, cwd_override: Option<String>) {
        if self.session_runtime.is_none() {
            return;
        }
        self.stop_working_loader();
        let overridden = cwd_override.is_some();
        let handle = self.runtime.clone();
        let Some(runtime) = self.session_runtime.as_mut() else {
            return;
        };
        let result = handle.block_on(runtime.switch_session(&path, cwd_override));
        match result {
            Ok(result) if result.cancelled => {}
            Ok(_) => {
                self.rebind_current_session();
                self.render_current_session_state();
                self.show_status(if overridden {
                    "Resumed session in current cwd"
                } else {
                    "Resumed session"
                });
            }
            Err(RuntimeError::MissingSessionCwd(issue)) if !overridden => {
                let (reply, answer) = mpsc::channel();
                let title = format!(
                    "Session cwd not found\n{}",
                    format_missing_session_cwd_prompt(&issue)
                );
                self.show_selector(&title, vec!["Yes".into(), "No".into()], reply);
                self.pending_cwd_prompt = Some((path, issue.fallback_cwd, answer));
            }
            Err(error) => {
                // `handleFatalRuntimeError`.
                self.show_error(&format!("Failed to resume session: {error}"));
                self.exit_requested = true;
            }
        }
    }

    /// The missing-cwd confirm answered: resume in the fallback cwd, or not.
    pub(super) fn poll_cwd_prompt(&mut self) {
        let Some((_, _, answer)) = &self.pending_cwd_prompt else {
            return;
        };
        let confirmed = match answer.try_recv() {
            Ok(choice) => choice.as_deref() == Some("Yes"),
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => false,
        };
        let Some((path, fallback, _)) = self.pending_cwd_prompt.take() else {
            return;
        };
        if confirmed {
            self.handle_resume_session(path, Some(fallback));
        } else {
            self.show_status("Resume cancelled");
        }
    }

    /// `rebindCurrentSession`: point the mode at the runtime's session.
    pub(super) fn rebind_current_session(&mut self) {
        let Some(runtime) = &self.session_runtime else {
            return;
        };
        self.subscription = None;
        self.session = runtime.session().clone();
        self.footer.borrow_mut().set_source(Box::new(SessionFooter {
            session: self.session.clone(),
            is_oauth: self.is_oauth.clone(),
        }));
        self.footer_data.set_cwd(self.session.cwd());
        self.subscription = Some(self.subscribe());
        self.update_editor_border_color();
        self.update_session_chip();
        self.update_terminal_title();
        self.setup_autocomplete_provider();
        self.update_available_provider_count();
        self.apply_runtime_settings();
        self.apply_session_theme();
    }

    /// `applySessionTheme`: load the theme the settings name and rebuild the
    /// banner, whose text carries its escapes. As the pin renders it, the
    /// banner is rebuilt before the new theme loads: a theme edited on disk
    /// reaches the banner on the next rebuild (a second /new or /reload),
    /// everything else at once.
    fn apply_session_theme(&mut self) {
        let expanded = self.verbose || self.expanded;
        self.header.borrow_mut().set_expanded(expanded);
        let theme_name = self.session.settings().theme();
        if let Some(name) = theme_name {
            if let Err(error) = set_theme(&name, true) {
                self.show_error(&format!(
                    "Failed to load theme \"{name}\": {error}\nFell back to dark theme."
                ));
            }
        }
        self.tui.invalidate();
        self.dirty.set(true);
    }

    /// `applyRuntimeSettings`: push the current settings and session onto
    /// the chrome (keybindings, footer, editor, cursor). Startup, a session
    /// swap and /reload all rebuild these from disk, so they repaint alike;
    /// a `compaction.enabled` edit used to leave the footer promising
    /// auto-compaction after /new or /reload.
    fn apply_runtime_settings(&mut self) {
        AppKeybindingsManager::create(None).install();
        let settings = self.session.settings();
        let (show_hardware_cursor, clear_on_shrink, hide_thinking_block) = (
            settings.show_hardware_cursor(),
            settings.clear_on_shrink(),
            settings.hide_thinking_block(),
        );
        let (border, padding_x, autocomplete_max_visible) = (
            match settings.editor_border() {
                EditorBorder::Box => FrameBorderStyle::Box,
                EditorBorder::Rule => FrameBorderStyle::Rule,
            },
            settings.editor_padding_x() as usize,
            settings.autocomplete_max_visible() as usize,
        );
        drop(settings);
        {
            let mut footer = self.footer.borrow_mut();
            footer.set_auto_compact_enabled(self.session.auto_compaction_enabled());
            footer.set_tool_output_view(self.tool_output_view);
            footer.invalidate();
        }
        self.footer_data.set_cwd(self.session.cwd());
        self.footer_data.set_subagent_enabled(
            self.session
                .get_active_tool_names()
                .iter()
                .any(|t| t == "Agent"),
        );
        // `ctx.ui.setMode` from the mode system's `session_start`.
        if let Some(mode) = self.session.extensions().active_mode() {
            self.footer_data.set_active_mode(&mode);
            self.footer.borrow_mut().invalidate();
        }
        self.hide_thinking_block = hide_thinking_block;
        self.tui.set_show_hardware_cursor(show_hardware_cursor);
        self.tui.set_clear_on_shrink(clear_on_shrink);
        set_input_frame_border(border);
        {
            let mut editor = self.editor.borrow_mut();
            editor.editor.set_border(border);
            editor.editor.set_padding_x(padding_x);
            editor
                .editor
                .set_autocomplete_max_visible(autocomplete_max_visible);
        }
        self.dirty.set(true);
    }
}

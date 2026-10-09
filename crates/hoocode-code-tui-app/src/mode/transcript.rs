//! The chat transcript: rendering messages and tool blocks, streaming, the status and notice
//! rows, and the tool output views.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use hoocode_agent_types::AgentMessage;
use hoocode_ai_types::{AssistantMessage, Content, StopReason};
use hoocode_ai_util::is_long_retry_delay_error;
use hoocode_code_agent_session::format::{format_duration_secs, format_tokens};
use hoocode_code_agent_session::stats::sum_assistant_usage;
use hoocode_code_settings::ToolOutputView;
use hoocode_code_tool_api::{truncate_tail, TruncationOptions, TruncationResult};
use hoocode_code_tui_keybindings::key_text;
use hoocode_code_tui_theme::{apply_block_fill, BlockFill};
use hoocode_code_tui_theme::{get_markdown_theme, theme};
use hoocode_code_tui_widgets::bash_execution::BashExecutionComponent;
use hoocode_code_tui_widgets::custom_message::{
    BranchSummaryMessageComponent, CompactionSummaryMessageComponent, CustomMessageComponent,
};
use hoocode_code_tui_widgets::dynamic_border::DynamicBorder;
use hoocode_code_tui_widgets::tool_chain::ToolChainComponent;
use hoocode_code_tui_widgets::tool_chain_summary::ChainState;
use hoocode_code_tui_widgets::tool_execution::{ToolExecutionComponent, ToolExecutionOptions};
use hoocode_code_tui_widgets::tool_output_view::{
    cycle_tool_output_view, DEFAULT_TOOL_OUTPUT_VIEW, MAX_TOOL_OUTPUT_VIEW,
};
use hoocode_code_tui_widgets::tool_signal::ToolResult;
use hoocode_code_tui_widgets::tools::registered_tool_definition;
use hoocode_code_tui_widgets::{AssistantMessageComponent, ThinkingDisplay, UserMessageComponent};
use hoocode_tui_components::BoxComponent;
use hoocode_tui_components::{Loader, Markdown, MarkdownTheme, Spacer, Text};
use hoocode_tui_render::ComponentHandle;

use crate::expandable_text::Expandable;
use crate::notification_panel::NotificationKind;
use crate::resource_display::show_loaded_resources;

use super::*;

/// The chat transcript: the chat `Container` and the components that track it. Reset
/// (`reset_transcript_view`) replaces the whole value, keeping only the chat container.
pub(super) struct Transcript {
    pub(super) chat: Rc<RefCell<Container>>,
    pub(super) streaming: Option<Rc<RefCell<AssistantMessageComponent>>>,
    pub(super) streaming_message: Option<AssistantMessage>,
    /// Tool blocks by call id, until their execution ends.
    pub(super) pending_tools: HashMap<String, Rc<RefCell<ToolExecutionComponent>>>,
    /// The chain collecting tool calls, if the agent is mid-run.
    pub(super) open_chain: Option<Rc<RefCell<ToolChainComponent>>>,
    /// Every chain and assistant message in the transcript, in order.
    pub(super) chains: Vec<Rc<RefCell<ToolChainComponent>>>,
    pub(super) assistant_components: Vec<Rc<RefCell<AssistantMessageComponent>>>,
    pub(super) latest_block: Option<Rc<RefCell<ToolExecutionComponent>>>,
    pub(super) latest_chain: Option<Rc<RefCell<ToolChainComponent>>>,
    /// The last status line, updated in place when nothing followed it.
    pub(super) last_status: RecordRows,
    /// Every `!` row in the transcript, for the expand sweep.
    pub(super) bash_components: Vec<Rc<RefCell<BashExecutionComponent>>>,
    /// Every branch summary in the transcript, for the expand sweep.
    pub(super) branch_summaries: Vec<Rc<RefCell<BranchSummaryMessageComponent>>>,
    /// Every compaction summary in the transcript, for the expand sweep.
    pub(super) compaction_summaries: Vec<Rc<RefCell<CompactionSummaryMessageComponent>>>,
}

impl Transcript {
    /// An empty transcript over `chat`, the container the TUI tree shows.
    pub(super) fn new(chat: Rc<RefCell<Container>>) -> Self {
        Self {
            chat,
            streaming: None,
            streaming_message: None,
            pending_tools: HashMap::new(),
            open_chain: None,
            chains: Vec::new(),
            assistant_components: Vec::new(),
            latest_block: None,
            latest_chain: None,
            last_status: RecordRows::default(),
            bash_components: Vec::new(),
            branch_summaries: Vec::new(),
            compaction_summaries: Vec::new(),
        }
    }
}

/// A `{ truncated: true }` result for a `!` row: the row only reads the flag.
pub(super) fn truncated_marker(content: &str) -> TruncationResult {
    TruncationResult {
        truncated: true,
        ..truncate_tail(content, TruncationOptions::default())
    }
}

impl Mode {
    pub(super) fn add_to_chat(&mut self, component: ComponentHandle) {
        self.transcript.chat.borrow_mut().add_child(component);
        self.dirty.set(true);
    }

    /// `showError`: a filled error block in the chat, headline over detail.
    pub(super) fn show_error(&mut self, message: &str) {
        let mut lines = message.split('\n');
        let title = format!("Error: {}", lines.next().unwrap_or(""));
        let mut body: Vec<&str> = lines.collect();
        // A wait measured in days is a dead end: say what to do about it.
        if is_long_retry_delay_error(Some(message)) {
            body.push(
                "Switch to another model with /model, or wait for the provider's quota to reset.",
            );
        }
        self.show_block(BlockFill::ToolErrorBg, "error", &title, &body);
    }

    /// The startup/reload listing and the session's own state.
    pub(super) fn render_resources(&mut self) {
        let mut listing = (self.listing)(&self.session);
        listing.columns = Some(self.size.get().0 as usize);
        listing.verbose = self.verbose;
        listing.expanded = self.expanded;
        for component in show_loaded_resources(&listing, false, true) {
            self.add_to_chat(component);
        }
    }

    /// `getMarkdownThemeWithSettings`.
    pub(super) fn markdown_theme(&self) -> Rc<dyn Fn() -> MarkdownTheme> {
        let indent = self.code_block_indent.clone();
        Rc::new(move || MarkdownTheme {
            code_block_indent: Some(indent.clone()),
            ..get_markdown_theme()
        })
    }

    /// `thinkingDisplayForView`: radar drops traces outright.
    pub(super) fn thinking_display(&self) -> ThinkingDisplay {
        if self.tool_output_view == ToolOutputView::Radar {
            ThinkingDisplay::Omit
        } else if self.hide_thinking_block {
            ThinkingDisplay::Label
        } else {
            ThinkingDisplay::Full
        }
    }

    pub(super) fn create_working_loader(&self) -> Rc<RefCell<Loader>> {
        let mut loader = Loader::new(
            Box::new(|s: &str| theme().fg("accent", s)),
            Box::new(|s: &str| theme().fg("muted", s)),
            DEFAULT_WORKING_MESSAGE,
            None,
        );
        loader.start();
        handle(loader)
    }

    pub(super) fn stop_working_loader(&mut self) {
        if let Some(loader) = self.loader.take() {
            loader.borrow_mut().stop();
        }
        self.status.borrow_mut().clear();
    }

    /// `addMessageToChat` for the roles this transcript draws so far.
    pub(super) fn add_message_to_chat(&mut self, message: &AgentMessage, populate_history: bool) {
        match message {
            AgentMessage::User(user) => {
                let text: String = user
                    .content
                    .blocks()
                    .iter()
                    .filter_map(|c| match c {
                        Content::Text(t) => Some(t.text.as_str()),
                        _ => None,
                    })
                    .collect();
                if text.is_empty() {
                    return;
                }
                if !self.transcript.chat.borrow().children.is_empty() {
                    self.add_to_chat(as_component(&handle(Spacer::new(1))));
                }
                let component = UserMessageComponent::with_theme(&text, (self.markdown_theme())());
                self.add_to_chat(as_component(&handle(component)));
                if populate_history {
                    self.editor.borrow_mut().editor.add_to_history(&text);
                }
            }
            AgentMessage::Assistant(assistant) => {
                let component = handle(AssistantMessageComponent::with_theme(
                    Some(assistant),
                    self.thinking_display(),
                    self.markdown_theme(),
                    DEFAULT_HIDDEN_THINKING_LABEL,
                ));
                self.transcript.assistant_components.push(component.clone());
                self.add_to_chat(as_component(&component));
            }
            AgentMessage::Custom(custom) => {
                if custom.display {
                    let component =
                        CustomMessageComponent::new(custom.clone(), self.markdown_theme());
                    self.add_to_chat(as_component(&handle(component)));
                }
            }
            AgentMessage::CompactionSummary(summary) => {
                self.add_to_chat(as_component(&handle(Spacer::new(1))));
                let component = handle(CompactionSummaryMessageComponent::new(
                    summary.clone(),
                    self.markdown_theme(),
                ));
                component.borrow_mut().set_expanded(self.expanded);
                self.transcript.compaction_summaries.push(component.clone());
                self.add_to_chat(as_component(&component));
            }
            AgentMessage::BranchSummary(summary) => {
                self.add_to_chat(as_component(&handle(Spacer::new(1))));
                let component = handle(BranchSummaryMessageComponent::new(
                    summary.clone(),
                    self.markdown_theme(),
                ));
                component.borrow_mut().set_expanded(self.expanded);
                self.transcript.branch_summaries.push(component.clone());
                self.add_to_chat(as_component(&component));
            }
            AgentMessage::BashExecution(bash) => {
                let exclude = bash.exclude_from_context.unwrap_or(false);
                let mut component = BashExecutionComponent::new(&bash.command, exclude);
                if !bash.output.is_empty() {
                    component.append_output(&bash.output);
                }
                component.set_complete(
                    bash.exit_code.map(|c| c as i32),
                    bash.cancelled,
                    bash.truncated.then(|| truncated_marker("")),
                    bash.full_output_path.clone(),
                );
                let component = handle(component);
                self.transcript.bash_components.push(component.clone());
                self.add_to_chat(as_component(&component));
            }
            _ => {}
        }
    }

    /// `renderSessionContext`: the transcript of messages already in the
    /// session, with tool calls drawn as the live path draws them.
    pub(super) fn render_session_context(
        &mut self,
        messages: &[AgentMessage],
        populate_history: bool,
    ) {
        self.transcript.pending_tools.clear();
        let mut rendered_pending: Vec<(String, Rc<RefCell<ToolExecutionComponent>>)> = Vec::new();
        let mut last_stop_reason = None;
        for message in messages {
            match message {
                AgentMessage::Assistant(assistant) => {
                    // The live path's chain boundary, so a rebuilt history shows
                    // the chains it lived through.
                    if self.opens_new_chain(assistant) {
                        self.close_open_chain(ChainState::Done);
                    }
                    last_stop_reason = Some(assistant.stop_reason);
                    self.add_message_to_chat(message, populate_history);
                    for content in &assistant.content {
                        let Content::ToolCall(call) = content else {
                            continue;
                        };
                        let block =
                            self.new_tool_block(&call.name, &call.id, call.arguments.clone());
                        self.attach_tool_block(block.clone());
                        if matches!(
                            assistant.stop_reason,
                            StopReason::Aborted | StopReason::Error
                        ) {
                            let error = if assistant.stop_reason == StopReason::Aborted {
                                let attempt = self.session.retry_attempt();
                                if attempt > 0 {
                                    format!(
                                        "Aborted after {attempt} retry attempt{}",
                                        if attempt > 1 { "s" } else { "" }
                                    )
                                } else {
                                    "Operation aborted".to_string()
                                }
                            } else {
                                assistant
                                    .error_message
                                    .clone()
                                    .filter(|m| !m.is_empty())
                                    .unwrap_or_else(|| "Error".into())
                            };
                            block.borrow_mut().update_result(
                                ToolResult {
                                    content: vec![Content::text(error)],
                                    details: serde_json::Value::Null,
                                    is_error: true,
                                },
                                false,
                            );
                        } else {
                            block.borrow_mut().set_args_complete();
                            rendered_pending.push((call.id.clone(), block));
                        }
                    }
                }
                AgentMessage::ToolResult(result) => {
                    if let Some(i) = rendered_pending
                        .iter()
                        .position(|(id, _)| *id == result.tool_call_id)
                    {
                        let (_, block) = rendered_pending.remove(i);
                        block.borrow_mut().update_result(
                            ToolResult {
                                content: result.content.clone(),
                                details: result.details.clone().unwrap_or(serde_json::Value::Null),
                                is_error: result.is_error,
                            },
                            false,
                        );
                    }
                }
                _ => self.add_message_to_chat(message, populate_history),
            }
        }
        // History has no live state: a chain still open is finished, unless
        // calls still wait for results (a resumed run in flight).
        if rendered_pending.is_empty() {
            self.close_open_chain(if last_stop_reason == Some(StopReason::Stop) {
                ChainState::Done
            } else {
                ChainState::Interrupted
            });
        }
        self.transcript.pending_tools.extend(rendered_pending);
        self.dirty.set(true);
    }

    /// `renderInitialMessages`: the loaded session's transcript, and how often
    /// it was compacted.
    pub(super) fn render_initial_messages(&mut self) {
        let messages = self.session.messages();
        self.update_editor_border_color();
        self.render_session_context(&messages, true);
        let compactions = self
            .session
            .session_manager()
            .entries()
            .iter()
            .filter(|e| matches!(e, hoocode_code_session::FileEntry::Compaction { .. }))
            .count();
        if compactions > 0 {
            let times = if compactions == 1 {
                "1 time".to_string()
            } else {
                format!("{compactions} times")
            };
            self.show_status(&format!("Session compacted {times}"));
        }
    }

    /// Re-render the in-flight message now, or once the throttle window passes.
    pub(super) fn schedule_streaming_render(&mut self) {
        let due = self
            .stream_render_at
            .is_none_or(|at| at.elapsed() >= STREAM_RENDER_THROTTLE);
        if due {
            self.run_streaming_render();
        } else {
            self.stream_render_pending = true;
        }
    }

    pub(super) fn run_streaming_render(&mut self) {
        self.stream_render_pending = false;
        self.stream_render_at = Some(Instant::now());
        if let (Some(component), Some(message)) = (
            &self.transcript.streaming,
            &self.transcript.streaming_message,
        ) {
            component.borrow_mut().update_content(message, true);
            self.dirty.set(true);
        }
    }

    pub(super) fn new_tool_block(
        &self,
        name: &str,
        id: &str,
        args: serde_json::Value,
    ) -> Rc<RefCell<ToolExecutionComponent>> {
        // A registered tool always has a definition, renderers or not; only an
        // unknown name gets the bare text rendering.
        let definition = self
            .session
            .get_tool_definition(name)
            .map(|_| registered_tool_definition(name));
        handle(ToolExecutionComponent::new(
            name,
            id,
            args,
            ToolExecutionOptions {
                show_images: self.show_images,
                image_width_cells: self.image_width_cells,
                view: self.tool_output_view,
            },
            definition,
            &self.session.cwd().to_string_lossy(),
        ))
    }

    /// `attachToolBlock`: into the open chain (a new one when none is open),
    /// marking the newest call and run.
    pub(super) fn attach_tool_block(&mut self, block: Rc<RefCell<ToolExecutionComponent>>) {
        let chain = match &self.transcript.open_chain {
            Some(chain) if chain.borrow().is_open() => chain.clone(),
            _ => {
                let chain = handle(ToolChainComponent::new(self.tool_output_view));
                self.add_to_chat(as_component(&chain));
                self.transcript.chains.push(chain.clone());
                self.transcript.open_chain = Some(chain.clone());
                chain
            }
        };
        chain.borrow_mut().add(block.clone());
        if let Some(previous) = self.transcript.latest_block.replace(block.clone()) {
            previous.borrow_mut().set_latest(false);
        }
        block.borrow_mut().set_latest(true);
        let same = self
            .transcript
            .latest_chain
            .as_ref()
            .is_some_and(|c| Rc::ptr_eq(c, &chain));
        if !same {
            if let Some(previous) = self.transcript.latest_chain.replace(chain.clone()) {
                previous.borrow_mut().set_latest(false);
            }
            chain.borrow_mut().set_latest(true);
        }
        self.dirty.set(true);
    }

    /// `closeOpenChain`: settle the chain collecting calls.
    pub(super) fn close_open_chain(&mut self, outcome: ChainState) {
        let Some(chain) = self.transcript.open_chain.take() else {
            return;
        };
        if chain.borrow().is_empty() {
            let h = as_component(&chain);
            self.transcript.chat.borrow_mut().remove_child(&h);
            self.transcript.chains.retain(|c| !Rc::ptr_eq(c, &chain));
        } else {
            chain.borrow_mut().close(outcome);
        }
        self.dirty.set(true);
    }

    /// `opensNewChain`: speaking ends the run, and so does a drawn trace.
    pub(super) fn opens_new_chain(&self, message: &AssistantMessage) -> bool {
        let draws_thinking = self.thinking_display() != ThinkingDisplay::Omit;
        message.content.iter().any(|c| match c {
            Content::Text(t) => !t.text.trim().is_empty(),
            Content::Thinking(t) => draws_thinking && !t.thinking.trim().is_empty(),
            _ => false,
        })
    }

    /// `trimTranscriptMemory`: freeze all but the newest live tool blocks.
    pub(super) fn trim_transcript_memory(&mut self) {
        let freezable: Vec<_> = self
            .transcript
            .chains
            .iter()
            .flat_map(|c| c.borrow().tool_blocks().to_vec())
            .filter(|b| b.borrow().is_freezable())
            .collect();
        let excess = freezable.len().saturating_sub(LIVE_TOOL_WINDOW);
        for block in &freezable[..excess] {
            block.borrow_mut().freeze();
        }
    }

    /// `showNotice`: a filled warning block in the chat, for warnings that
    /// cost money if ignored (`showBlock`).
    pub(super) fn show_notice(&mut self, title: &str, body: &[&str]) {
        self.show_block(BlockFill::WarningBg, "warning", title, body);
    }

    /// `showBlock`: a filled block in the chat, bold title over muted lines.
    fn show_block(&mut self, fill: BlockFill, fg: &str, title: &str, body: &[&str]) {
        let t = theme();
        let mut block = BoxComponent::new(1, 1, None);
        apply_block_fill(&mut block, fill);
        block.add_child(as_component(&handle(Text::new(
            t.bold(&t.fg(fg, title)),
            0,
            0,
        ))));
        for line in body {
            block.add_child(as_component(&handle(Text::new(t.fg("muted", line), 0, 0))));
        }
        self.add_to_chat(as_component(&handle(Spacer::new(1))));
        self.add_to_chat(as_component(&handle(block)));
    }

    /// Every tool block in the transcript.
    pub(super) fn tool_blocks(&self) -> Vec<Rc<RefCell<ToolExecutionComponent>>> {
        let mut blocks: Vec<_> = self
            .transcript
            .chains
            .iter()
            .flat_map(|c| c.borrow().tool_blocks().to_vec())
            .collect();
        for block in self.transcript.pending_tools.values() {
            if !blocks.iter().any(|b| Rc::ptr_eq(b, block)) {
                blocks.push(block.clone());
            }
        }
        blocks
    }

    /// A bordered page in the chat: accent title over markdown
    /// (`handleHotkeys`, `handleChangelog`, the startup "What's New").
    pub(super) fn show_bordered_markdown(&mut self, title: &str, markdown: &str) {
        let t = theme();
        self.add_to_chat(as_component(&handle(Spacer::new(1))));
        self.add_to_chat(as_component(&handle(DynamicBorder::new(None))));
        self.add_to_chat(as_component(&handle(Text::new(
            t.bold(&t.fg("accent", title)),
            1,
            0,
        ))));
        self.add_to_chat(as_component(&handle(Spacer::new(1))));
        self.add_to_chat(as_component(&handle(Markdown::new(
            markdown,
            1,
            0,
            (self.markdown_theme())(),
            None,
        ))));
        self.add_to_chat(as_component(&handle(DynamicBorder::new(None))));
    }

    /// `toggleThinkingBlockVisibility`: hide or show thinking traces, saved,
    /// and the transcript rebuilt with the new display.
    pub(super) fn toggle_thinking_block_visibility(&mut self) {
        self.hide_thinking_block = !self.hide_thinking_block;
        self.session
            .settings()
            .set_hide_thinking_block(self.hide_thinking_block);
        let streaming = self.transcript.streaming.clone();
        let streaming_message = self.transcript.streaming_message.clone();
        self.reset_transcript_view();
        let messages = self.session.messages();
        self.render_session_context(&messages, false);
        // A message still streaming goes back on with the new display.
        if let (Some(component), Some(message)) = (streaming, streaming_message) {
            {
                let mut c = component.borrow_mut();
                c.set_thinking_display(self.thinking_display());
                c.update_content(&message, true);
            }
            self.add_to_chat(as_component(&component));
            self.transcript.streaming = Some(component);
            self.transcript.streaming_message = Some(message);
        }
        // Radar overrides the setting: say so rather than claim a change the
        // screen does not show.
        let status = if !self.hide_thinking_block && self.tool_output_view == ToolOutputView::Radar
        {
            "Thinking blocks: visible (radar hides them)"
        } else if self.hide_thinking_block {
            "Thinking blocks: hidden"
        } else {
            "Thinking blocks: visible"
        };
        self.show_status(status);
    }

    /// `resetTranscriptView`: drop every view reference into the transcript.
    pub(super) fn reset_transcript_view(&mut self) {
        let chat = self.transcript.chat.clone();
        chat.borrow_mut().clear();
        self.transcript = Transcript::new(chat);
    }

    /// `renderCurrentSessionState`: the transcript of the session just
    /// swapped in, after its resource listing.
    pub(super) fn render_current_session_state(&mut self) {
        self.reset_transcript_view();
        self.render_resources();
        self.render_initial_messages();
    }

    /// `showStatus`: a passing status in the notification band above the
    /// prompt (the first line is the title, the rest its body).
    pub(super) fn show_status(&mut self, message: &str) {
        self.notify(NotificationKind::Info, message);
    }

    /// `showWarning`: on the notification band, not in the transcript.
    pub(super) fn show_warning(&mut self, message: &str) {
        self.notify(NotificationKind::Warning, message);
    }

    pub(super) fn notify(&mut self, kind: NotificationKind, message: &str) {
        let mut lines = message.split('\n');
        let title = lines.next().unwrap_or("");
        let body: Vec<&str> = lines.collect();
        self.notifications
            .borrow_mut()
            .notify(kind, title, &body, None, None, None);
        self.dirty.set(true);
    }

    /// `showRecord`: a dim line in the chat that stays; back-to-back records
    /// update the previous line instead of stacking.
    pub(super) fn show_record(&mut self, message: &str) {
        let styled = if message.contains("\x1b[") {
            message.to_string()
        } else {
            theme().fg("dim", message)
        };
        self.transcript
            .last_status
            .show(&mut self.transcript.chat.borrow_mut(), styled);
        self.dirty.set(true);
    }

    /// `showDialStep`: the stop a dial landed on, and (the first time) how to
    /// step back.
    pub(super) fn show_dial_step(&mut self, backward: &'static str, message: &str) {
        let taught = !self.dial_reverse_taught.insert(backward);
        let topic = backward
            .strip_suffix(".cycleForward")
            .or_else(|| backward.strip_suffix(".cycleBackward"))
            .unwrap_or(backward);
        let note = (!taught).then(|| format!("{} steps back", key_text(backward)));
        self.notifications.borrow_mut().notify(
            NotificationKind::Info,
            message,
            &[],
            note.as_deref(),
            None,
            Some(topic),
        );
        self.dirty.set(true);
    }

    /// `applyToolOutputView`: move every block and chain to `view`.
    pub(super) fn apply_tool_output_view(&mut self, view: ToolOutputView, persist: bool) {
        let previous_thinking = self.thinking_display();
        let was_expanded = self.tool_output_view == MAX_TOOL_OUTPUT_VIEW;
        self.tool_output_view = view;
        if persist {
            self.session.settings().set_tool_output_view(view);
            self.view_before_jump = None;
        }
        self.footer.borrow_mut().set_tool_output_view(view);
        let thinking = self.thinking_display();
        for chain in &self.transcript.chains {
            chain.borrow_mut().set_view(view);
        }
        if thinking != previous_thinking {
            for component in &self.transcript.assistant_components {
                component.borrow_mut().set_thinking_display(thinking);
            }
        }
        let expanded = view == MAX_TOOL_OUTPUT_VIEW;
        if expanded != was_expanded {
            // "full" holds nothing back: the header opens with it.
            self.expanded = expanded;
            self.header.borrow_mut().set_expanded(expanded);
            for component in &self.transcript.bash_components {
                component.borrow_mut().set_expanded(expanded);
            }
            for component in &self.transcript.branch_summaries {
                component.borrow_mut().set_expanded(expanded);
            }
            for component in &self.transcript.compaction_summaries {
                component.borrow_mut().set_expanded(expanded);
            }
        }
        self.dirty.set(true);
    }

    /// `jumpToFullView`: to `full`, or back to where the jump started.
    pub(super) fn jump_to_full_view(&mut self) {
        if self.tool_output_view == MAX_TOOL_OUTPUT_VIEW {
            let back = self
                .view_before_jump
                .take()
                .unwrap_or(DEFAULT_TOOL_OUTPUT_VIEW);
            self.apply_tool_output_view(back, false);
            return;
        }
        self.view_before_jump = Some(self.tool_output_view);
        self.apply_tool_output_view(MAX_TOOL_OUTPUT_VIEW, false);
    }

    /// `cycleToolOutputView`: one stop on the dial, saved.
    pub(super) fn cycle_tool_output_view(&mut self, forward: bool) {
        let next = cycle_tool_output_view(self.tool_output_view, forward);
        self.apply_tool_output_view(next, true);
        self.show_dial_step(
            if forward {
                "app.view.cycleBackward"
            } else {
                "app.view.cycleForward"
            },
            &format!("Tool output: {next}"),
        );
    }

    /// `showTurnCost`: this request's own tokens, time and cost.
    pub(super) fn show_turn_cost(&mut self) {
        let Some((anchor, at)) = self.turn_cost_anchor.take() else {
            return;
        };
        let now = sum_assistant_usage(self.session.session_manager().entries());
        let input = now.input as i64 - anchor.input as i64;
        let output = now.output as i64 - anchor.output as i64;
        let cost = now.cost - anchor.cost;
        // Nothing was accounted: stay silent rather than print zeroes.
        if input <= 0 && output <= 0 {
            return;
        }
        let t = theme();
        let mut segs = vec![
            format!(
                "{}{}{}{}",
                t.fg("dim", "↑"),
                t.fg("muted", &format_tokens(input.max(0) as u64)),
                t.fg("dim", " ↓"),
                t.fg("muted", &format_tokens(output.max(0) as u64)),
            ),
            t.fg("muted", &format_duration_secs(at.elapsed().as_secs_f64())),
        ];
        if cost > 0.0 {
            segs.push(t.fg(
                "muted",
                &format!(
                    "${}",
                    hoocode_code_agent_session::format::js_to_fixed(cost, 3)
                ),
            ));
        }
        let separator = t.fg("dim", " · ");
        self.add_to_chat(as_component(&handle(Spacer::new(1))));
        self.add_to_chat(as_component(&handle(Text::new(
            segs.join(&separator),
            1,
            0,
        ))));
    }
}

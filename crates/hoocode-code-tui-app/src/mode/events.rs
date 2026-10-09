//! Agent and session events: the `AgentSessionEvent` and `AgentEvent` handlers, compaction, and the
//! scheduler tick.

use std::time::Instant;

use hoocode_agent_compaction::CompactionResult;
use hoocode_agent_types::{AgentEvent, AgentMessage, CompactionSummaryMessage};
use hoocode_ai_types::{Content, StopReason};
use hoocode_code_agent_session::{
    AgentSessionEvent, CompactionReason, ExtensionUiRequest, NotifyLevel, StreamingBehavior,
};
use hoocode_code_task_store::{task_store, TaskStatus};
use hoocode_code_tools_optin::todo::settle_dangling_main_tasks;
use hoocode_code_tui_keybindings::key_text;
use hoocode_code_tui_theme::theme;
use hoocode_code_tui_widgets::tool_chain_summary::ChainState;
use hoocode_code_tui_widgets::tool_signal::ToolResult;
use hoocode_code_tui_widgets::AssistantMessageComponent;
use hoocode_tui_components::{Loader, Spacer, Text};
use hoocode_tui_render::Component;

use crate::startup_progress;

use super::*;

/// What dangling plan rows settle to when a request ends: done after a clean
/// stop, cancelled after an abort, error or length stop; nothing while
/// messages are queued (the request continues).
pub fn plan_settle_outcome(
    stop_reason: Option<StopReason>,
    pending_messages: usize,
) -> Option<TaskStatus> {
    if pending_messages > 0 {
        return None;
    }
    Some(if stop_reason == Some(StopReason::Stop) {
        TaskStatus::Done
    } else {
        TaskStatus::Cancelled
    })
}

impl Mode {
    /// Carry out what extension command handlers asked of the UI
    /// (`ctx.ui.notify`, `ctx.reload()`, `ctx.newSession`, `sendUserMessage`).
    pub(super) fn drain_extension_ui_requests(&mut self) {
        loop {
            let requests = self.session.extensions().take_ui_requests();
            if requests.is_empty() {
                return;
            }
            for request in requests {
                match request {
                    ExtensionUiRequest::Notify(message, level) => match level {
                        NotifyLevel::Error => self.show_error(&message),
                        NotifyLevel::Warning => self.show_warning(&message),
                        NotifyLevel::Info => self.show_status(&message),
                    },
                    ExtensionUiRequest::Reload => self.handle_reload_command(),
                    ExtensionUiRequest::SendFollowUp(text) => self.send_user_follow_up(text),
                    ExtensionUiRequest::NewSessionWithMessage(text) => {
                        self.handle_extension_new_session(text)
                    }
                }
            }
        }
    }

    /// `TaskScheduler.tick`: every [`hoocode_code_scheduler::TICK_INTERVAL`],
    /// while the agent is idle, submit the prompts of the tasks due this minute
    /// as follow-up messages (`fire` in `extensions/core/loop.ts`).
    pub(super) fn tick_scheduler(&mut self) {
        let now = Instant::now();
        if now < self.scheduler_tick_at {
            return;
        }
        self.scheduler_tick_at = now + hoocode_code_scheduler::TICK_INTERVAL;
        if self.session.is_streaming() || self.session.is_compacting() {
            return;
        }
        let due = hoocode_code_scheduler::claim_due_now(self.session.cwd());
        // The first prompt starts a turn; the rest queue behind it, as the
        // follow-up they would be in TS once the first has begun.
        for (index, prompt) in due.into_iter().enumerate() {
            if index == 0 {
                self.send_user_follow_up(prompt);
            } else {
                self.queue_prompt(prompt, StreamingBehavior::FollowUp);
            }
        }
    }

    pub(super) fn handle_session_event(&mut self, event: AgentSessionEvent) {
        match event {
            AgentSessionEvent::Agent(event) => self.handle_agent_event(event),
            AgentSessionEvent::ThinkingLevelChanged { .. } => self.update_editor_border_color(),
            AgentSessionEvent::CompactionStart { reason } => self.on_compaction_start(reason),
            AgentSessionEvent::QueueUpdate { .. } => self.update_pending_messages_display(),
            AgentSessionEvent::CompactionEnd {
                reason,
                result,
                aborted,
                will_retry,
                error_message,
            } => {
                self.on_compaction_end(reason, result, aborted, error_message);
                self.flush_compaction_queue(will_retry);
            }
            AgentSessionEvent::SessionInfoChanged { .. } => {
                self.update_session_chip();
                self.update_terminal_title();
            }
            _ => {}
        }
        self.dirty.set(true);
    }

    /// `settleDanglingPlanItems`: plan rows the model left in progress settle
    /// once the request is over: done after a clean stop, else cancelled.
    /// Skipped while messages are queued (the request continues).
    pub(super) fn settle_dangling_plan_items(&mut self) {
        let Some(outcome) =
            plan_settle_outcome(self.turn_stop_reason, self.session.pending_message_count())
        else {
            return;
        };
        if settle_dangling_main_tasks(task_store(), outcome) > 0 {
            self.dirty.set(true);
        }
    }

    /// `compaction_start`: the spinner with its cancel hint; Escape aborts
    /// the compaction meanwhile.
    fn on_compaction_start(&mut self, reason: CompactionReason) {
        if self.session.settings().show_terminal_progress() {
            self.tui.terminal.set_progress(true);
        }
        self.status.borrow_mut().clear();
        let cancel_hint = format!("({} to cancel)", key_text("app.interrupt"));
        let label = match reason {
            CompactionReason::Manual => format!("Compacting context... {cancel_hint}"),
            CompactionReason::Overflow => {
                format!("Context overflow detected, Auto-compacting... {cancel_hint}")
            }
            CompactionReason::Threshold => format!("Auto-compacting... {cancel_hint}"),
        };
        let mut loader = Loader::new(
            Box::new(|s: &str| theme().fg("accent", s)),
            Box::new(|s: &str| theme().fg("muted", s)),
            &label,
            None,
        );
        loader.start();
        let loader = handle(loader);
        self.status.borrow_mut().add_child(as_component(&loader));
        self.compaction_loader = Some(loader);
    }

    /// `compaction_end`: the chat rebuilt around the summary, or why not.
    fn on_compaction_end(
        &mut self,
        reason: CompactionReason,
        result: Option<CompactionResult>,
        aborted: bool,
        error_message: Option<String>,
    ) {
        if self.session.settings().show_terminal_progress() {
            self.tui.terminal.set_progress(false);
        }
        if let Some(loader) = self.compaction_loader.take() {
            loader.borrow_mut().stop();
            self.status.borrow_mut().clear();
        }
        let manual = reason == CompactionReason::Manual;
        if aborted {
            if manual {
                self.show_error("Compaction cancelled");
            } else {
                self.show_status("Auto-compaction cancelled");
            }
        } else if let Some(result) = result {
            self.reset_transcript_view();
            let messages = self.session.session_manager().build_context().messages;
            self.render_session_context(&messages, false);
            let summary = AgentMessage::CompactionSummary(CompactionSummaryMessage {
                summary: result.summary,
                tokens_before: result.tokens_before,
                tokens_after: result.tokens_after,
                timestamp: hoocode_ai_types::now_ms(),
            });
            self.add_message_to_chat(&summary, false);
            self.footer.borrow_mut().invalidate();
        } else if let Some(error) = error_message {
            if manual {
                self.show_error(&error);
            } else {
                self.add_to_chat(as_component(&handle(Spacer::new(1))));
                self.add_to_chat(as_component(&handle(Text::new(
                    theme().fg("error", &error),
                    1,
                    0,
                ))));
            }
        }
    }

    fn handle_agent_event(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::AgentStart => {
                self.tips.on_turn_start();
                self.stop_working_loader();
                let loader = self.create_working_loader();
                self.status.borrow_mut().add_child(as_component(&loader));
                self.loader = Some(loader);
            }
            AgentEvent::MessageStart { message } => match &message {
                AgentMessage::User(_) => {
                    // A new turn drops the finished tasks of the last one.
                    task_store().reset();
                    startup_progress::clear();
                    self.turn_stop_reason = None;
                    self.add_message_to_chat(&message, false);
                    self.update_pending_messages_display();
                }
                AgentMessage::Custom(_) => self.add_message_to_chat(&message, false),
                AgentMessage::Assistant(assistant) => {
                    let component = handle(AssistantMessageComponent::with_theme(
                        None,
                        self.thinking_display(),
                        self.markdown_theme(),
                        DEFAULT_HIDDEN_THINKING_LABEL,
                    ));
                    self.add_to_chat(as_component(&component));
                    component.borrow_mut().update_content(assistant, false);
                    self.assistant_components.push(component.clone());
                    self.chain_closed_for_current_message = false;
                    self.streaming = Some(component);
                    self.streaming_message = Some(assistant.clone());
                }
                _ => {}
            },
            AgentEvent::MessageUpdate { message, .. } => {
                if let (Some(_), AgentMessage::Assistant(assistant)) = (&self.streaming, message) {
                    self.schedule_streaming_render();
                    // Whatever this message first puts on screen ends the run
                    // its previous calls formed.
                    if !self.chain_closed_for_current_message && self.opens_new_chain(&assistant) {
                        self.chain_closed_for_current_message = true;
                        self.close_open_chain(ChainState::Done);
                    }
                    for content in &assistant.content {
                        let Content::ToolCall(call) = content else {
                            continue;
                        };
                        match self.pending_tools.get(&call.id) {
                            Some(block) => block.borrow_mut().update_args(call.arguments.clone()),
                            None => {
                                let block = self.new_tool_block(
                                    &call.name,
                                    &call.id,
                                    call.arguments.clone(),
                                );
                                self.attach_tool_block(block.clone());
                                self.pending_tools.insert(call.id.clone(), block);
                            }
                        }
                    }
                    self.streaming_message = Some(assistant);
                }
            }
            AgentEvent::MessageEnd { message } => {
                let AgentMessage::Assistant(mut assistant) = message else {
                    return;
                };
                self.turn_stop_reason = Some(assistant.stop_reason);
                if let Some(component) = self.streaming.take() {
                    if assistant.stop_reason == StopReason::Aborted {
                        let attempt = self.session.retry_attempt();
                        assistant.error_message = Some(if attempt > 0 {
                            format!(
                                "Aborted after {attempt} retry attempt{}",
                                if attempt > 1 { "s" } else { "" }
                            )
                        } else {
                            "Operation aborted".to_string()
                        });
                    }
                    component.borrow_mut().update_content(&assistant, false);
                    if matches!(
                        assistant.stop_reason,
                        StopReason::Aborted | StopReason::Error
                    ) {
                        let error = assistant
                            .error_message
                            .clone()
                            .filter(|m| !m.is_empty())
                            .unwrap_or_else(|| "Error".into());
                        for block in self.pending_tools.values() {
                            block.borrow_mut().update_result(
                                ToolResult {
                                    content: vec![Content::text(error.clone())],
                                    details: serde_json::Value::Null,
                                    is_error: true,
                                },
                                false,
                            );
                        }
                        self.pending_tools.clear();
                    } else {
                        // Args are complete: edit blocks compute their diffs.
                        for block in self.pending_tools.values() {
                            block.borrow_mut().set_args_complete();
                        }
                    }
                    self.streaming_message = None;
                    self.stream_render_pending = false;
                }
            }
            AgentEvent::ToolExecutionStart {
                tool_call_id,
                tool_name,
                args,
            } => {
                let block = match self.pending_tools.get(&tool_call_id) {
                    Some(block) => block.clone(),
                    None => {
                        let block = self.new_tool_block(&tool_name, &tool_call_id, args);
                        self.attach_tool_block(block.clone());
                        self.pending_tools.insert(tool_call_id, block.clone());
                        block
                    }
                };
                block.borrow_mut().mark_execution_started();
            }
            AgentEvent::ToolExecutionUpdate {
                tool_call_id,
                partial_result,
                ..
            } => {
                if let Some(block) = self.pending_tools.get(&tool_call_id) {
                    block.borrow_mut().update_result(
                        ToolResult {
                            content: partial_result.content,
                            details: partial_result.details,
                            is_error: false,
                        },
                        true,
                    );
                }
            }
            AgentEvent::ToolExecutionEnd {
                tool_call_id,
                result,
                is_error,
                ..
            } => {
                if let Some(block) = self.pending_tools.remove(&tool_call_id) {
                    block.borrow_mut().update_result(
                        ToolResult {
                            content: result.content,
                            details: result.details,
                            is_error,
                        },
                        false,
                    );
                    self.trim_transcript_memory();
                }
            }
            AgentEvent::AgentEnd { .. } => {
                self.pending_tools.clear();
                self.stop_working_loader();
                if let Some(component) = self.streaming.take() {
                    let handle = as_component(&component);
                    self.chat.borrow_mut().remove_child(&handle);
                    self.streaming_message = None;
                }
            }
            _ => {}
        }
    }
}

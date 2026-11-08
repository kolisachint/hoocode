//! Agent and session events: the `AgentSessionEvent` and `AgentEvent` handlers, compaction, and the
//! scheduler tick.

use std::time::Instant;

use hoocode_agent_compaction::CompactionResult;
use hoocode_agent_types::{AgentEvent, AgentMessage, CompactionSummaryMessage};
use hoocode_ai_types::{Content, StopReason};
use hoocode_code_agent_session::{
    AgentSessionEvent, CompactionReason, ExtensionUiRequest, NotifyLevel,
};
use hoocode_code_task_store::{task_store, TaskStatus};
use hoocode_code_tools_optin::todo::settle_dangling_main_tasks;
use hoocode_code_tui_keybindings::key_text;
use hoocode_code_tui_theme::theme;
use hoocode_code_tui_widgets::tool_chain::ChainState;
use hoocode_code_tui_widgets::tool_signal::ToolResult;
use hoocode_code_tui_widgets::AssistantMessageComponent;
use hoocode_tui_components::{Loader, Spacer, Text};

use crate::herdr::AgentState;
use crate::startup_progress;

use super::retry::retry_notice_text;
use super::retry::RETRY_CANCELLED;
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
        // Requests wait while a session operation runs: `/mode` asks for a
        // reload from inside its own prompt, and that reload would be refused
        // behind the operation. They are taken when it finishes.
        if self.session_op.is_some() {
            return;
        }
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

    /// The startup update notice, once, when the version check reports a newer release.
    pub(super) fn poll_update_notice(&mut self) {
        let Some(receiver) = &self.update_notice else {
            return;
        };
        match receiver.try_recv() {
            Ok(update) => {
                self.update_notice = None;
                let (title, body) = hoocode_code_update::notice_text(&update);
                let body: Vec<&str> = body.iter().map(String::as_str).collect();
                self.notifications.borrow_mut().notify(
                    NotificationKind::Info,
                    &title,
                    &body,
                    None,
                    Some(Duration::from_secs(15)),
                    Some("update"),
                );
                self.dirty.set(true);
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.update_notice = None,
        }
    }

    /// Rings the bell when the setting allows and the debounce lets it through.
    pub(super) fn ring_chime(&mut self) {
        if !self.session.settings().chime_on_turn_complete() {
            return;
        }
        if self.chime.allow(Instant::now()) {
            self.tui.terminal.write(BELL);
        }
    }

    /// The turn that just ended (`PromptDone`): chime when it ran long enough.
    pub(super) fn finish_turn_chime(&mut self) {
        if self
            .turn_timer
            .borrow_mut()
            .end(Instant::now(), self.turn_stop_reason)
        {
            self.ring_chime();
        }
    }

    /// Reports the agent's state to Herdr (once per loop pass; the reporter
    /// sends only changes): blocked while a clearance card, a permission
    /// question or an options pane waits for the user, working while a
    /// prompt, run or compaction is in flight, else idle. The session id and
    /// file go with it, so a `/new`, resume or fork re-reports the resume
    /// command.
    pub(super) fn sync_herdr(&mut self) {
        if self.herdr.is_none() {
            return;
        }
        let (state, message) = if self.clearance.is_some() {
            (
                AgentState::Blocked,
                Some("Waiting for a permission decision"),
            )
        } else if self.dialog_select.get() > 0
            && matches!(self.picker, Some(ActivePicker::Selector(..)))
        {
            (AgentState::Blocked, Some("Waiting for a decision"))
        } else if self.ask_options.is_some() {
            (AgentState::Blocked, Some("Waiting for an answer"))
        } else if self.agent_busy() || self.compaction_loader.is_some() {
            (AgentState::Working, None)
        } else {
            (AgentState::Idle, None)
        };
        // Borrowed from the session manager: nothing is copied unless the
        // reporter has a change to send.
        let manager = self.session.session_manager();
        if let Some(herdr) = self.herdr.as_mut() {
            herdr.update(state, message, manager.session_id(), manager.session_file());
        }
    }

    /// `TaskScheduler.tick`: every [`hoocode_code_scheduler::TICK_INTERVAL`],
    /// while the agent is free (`agent_busy`), claim the tasks due this minute
    /// and queue their prompts in `scheduled_backlog`; one is sent per free
    /// pass (`send_next_scheduled`), as a fresh prompt, so a slash or extension
    /// command works and no fire is claimed while a start is in flight.
    pub(super) fn tick_scheduler(&mut self) {
        self.send_next_scheduled();
        let now = Instant::now();
        if now < self.scheduler_tick_at {
            return;
        }
        self.scheduler_tick_at = now + hoocode_code_scheduler::TICK_INTERVAL;
        // Nothing is claimed while busy: a claim moves the task's last-run
        // minute (and deletes a one-shot), so a fire that could not be sent
        // would be lost. The next free tick claims it, inside the late window.
        if self.agent_busy() {
            return;
        }
        // The tick's store read also feeds the Schedule rows (no extra read here).
        // The claim is for this session: it fires the tasks it owns and adopts
        // the ones whose owner has exited.
        let session_id = self.session.session_id();
        let tick = hoocode_code_scheduler::tick_now(self.session.cwd(), Some(&session_id));
        let claimed = tick.claimed;
        self.scheduled_runs.publish_tick(
            task_store(),
            tick.jobs.as_deref(),
            &claimed.fired_jobs,
            &chrono::Local::now(),
        );
        self.report_scheduled_prompts(&claimed);
        self.scheduled_backlog
            .extend(claimed.fired.iter().map(|fired| fired.message()));
        self.send_next_scheduled();
    }

    /// Wakes an idle agent when a background shell job finished (the same as a
    /// background subagent's result starting a turn). The wake is a short
    /// generic prompt; its model call carries the notices (`JobNotices::for_call`),
    /// so nothing is taken here: a completion reaches the model once, and a
    /// wake that fails before a model call loses none. It waits while the agent
    /// is busy, a `!` command runs, `/loop auto` runs or the user is typing: the
    /// notices then ride on the next model call. It is not the user's prompt
    /// (`start_prompt` with `typed = false`), and a wake that failed is not
    /// retried until the notices are gone.
    pub(super) fn tick_job_wake(&mut self) {
        if self.agent_busy()
            || self.session.is_bash_running()
            || self.auto_loop.is_some()
            || !self.editor.borrow().editor.get_text().trim().is_empty()
        {
            return;
        }
        if !self.session.job_wake_pending() {
            self.job_wake_failed = false;
            return;
        }
        if self.job_wake_failed {
            return;
        }
        self.start_prompt(
            hoocode_code_agent_session::JOB_WAKE_MESSAGE.to_string(),
            Vec::new(),
            false,
        );
    }

    pub(super) fn handle_session_event(&mut self, event: AgentSessionEvent) {
        match event {
            AgentSessionEvent::Agent(event) => self.handle_agent_event(event),
            AgentSessionEvent::ThinkingLevelChanged { .. } => self.dirty.set(true),
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
                self.update_terminal_title();
            }
            AgentSessionEvent::AutoRetryStart {
                attempt,
                max_attempts,
                error_message,
                ..
            } => {
                self.transcript.start_retry(&error_message);
                self.comms_lost.set(false);
                self.transient
                    .borrow_mut()
                    .set_retry_notice(Some(retry_notice_text(
                        attempt,
                        max_attempts,
                        &error_message,
                    )));
            }
            AgentSessionEvent::ModelFallback {
                from,
                to,
                error_message,
            } => {
                // The auto-picked model was refused; the session already
                // switched and continues the turn.
                let reason = error_message.lines().next().unwrap_or("").trim();
                self.show_warning(&format!(
                    "{from} is not available on this account ({reason}). Switched to {to}; /model picks another."
                ));
            }
            AgentSessionEvent::AutoRetryEnd {
                success,
                attempt,
                final_error,
            } => {
                self.transcript
                    .end_retry(success, attempt, final_error.as_deref());
                // COMMS LOST (M8e1): the last retry gave up after a real
                // streak. A lone unretried error, a cancellation and a
                // success leave the frame alone.
                let lost =
                    !success && final_error.as_deref() != Some(RETRY_CANCELLED) && attempt >= 2;
                self.comms_lost.set(lost);
                self.transient.borrow_mut().set_retry_notice(None);
            }
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
                format!("Context overflow detected, Auto-compacting… {cancel_hint}")
            }
            CompactionReason::Threshold => format!("Auto-compacting… {cancel_hint}"),
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
                self.show_status("Compaction cancelled");
            } else {
                self.show_status("Auto-compaction cancelled");
            }
        } else if let Some(result) = result {
            // The missions this compaction folds, for the JETTISONED row
            // (M8e6): read before the reset drops the transcript's ranges.
            let mission_range = self.transcript.jettisoned_range();
            self.reset_transcript_view();
            let messages = self.session.session_manager().build_context().messages;
            self.render_session_context(&messages, false);
            let summary = AgentMessage::CompactionSummary(CompactionSummaryMessage {
                summary: result.summary,
                tokens_before: result.tokens_before,
                tokens_after: result.tokens_after,
                timestamp: hoocode_ai_types::now_ms(),
                mission_range,
            });
            self.add_message_to_chat(&summary, false);
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

    pub(super) fn handle_agent_event(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::AgentStart => {
                self.tips.on_turn_start();
                self.stop_working_loader();
                let loader = self.create_working_loader();
                self.status.borrow_mut().add_child(as_component(&loader));
                self.loader = Some(loader);
            }
            AgentEvent::MessageStart { message } => match &message {
                AgentMessage::User(user) => {
                    // A scheduled prompt's row says running once its own message begins.
                    if let Some(text) = user.content.as_str() {
                        self.scheduled_runs.run_started(task_store(), text);
                    }
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
                    component
                        .borrow_mut()
                        .set_outline_mission(self.transcript.current_mission());
                    self.transcript.note_assistant(&component);
                    self.add_to_chat(as_component(&component));
                    component.borrow_mut().update_content(assistant, false);
                    self.transcript.assistant_components.push(component.clone());
                    self.chain_closed_for_current_message = false;
                    self.transcript.streaming = Some(component);
                    self.transcript.streaming_message = Some(Rc::new(assistant.clone()));
                }
                _ => {}
            },
            AgentEvent::MessageUpdate { message, .. } => {
                // The session's own copy is usually gone by now, so this takes
                // the message out of its Arc without a copy.
                let message = Arc::try_unwrap(message).unwrap_or_else(|shared| (*shared).clone());
                if let (Some(_), AgentMessage::Assistant(assistant)) =
                    (&self.transcript.streaming, message)
                {
                    // The render scheduled here reads `streaming_message`, so the
                    // new message is stored before it, not after.
                    let assistant = Rc::new(assistant);
                    self.transcript.streaming_message = Some(assistant.clone());
                    self.schedule_streaming_render();
                    // Whatever this message first puts on screen ends the run
                    // its previous calls formed.
                    if !self.chain_closed_for_current_message && self.opens_new_chain(&assistant) {
                        self.chain_closed_for_current_message = true;
                        self.release_streaming_trace();
                        self.close_open_chain(ChainState::Done);
                    }
                    for content in &assistant.content {
                        let Content::ToolCall(call) = content else {
                            continue;
                        };
                        match self.transcript.pending_tools.get(&call.id) {
                            Some(block) => block.borrow_mut().update_args(call.arguments.clone()),
                            None => {
                                if let Some(trace) = self.transcript.streaming.clone() {
                                    self.place_trace_in_chain(&trace);
                                }
                                let block = self.new_tool_block(
                                    &call.name,
                                    &call.id,
                                    call.arguments.clone(),
                                );
                                self.attach_tool_block(block.clone());
                                self.transcript.pending_tools.insert(call.id.clone(), block);
                            }
                        }
                    }
                }
            }
            AgentEvent::MessageEnd { message } => {
                let AgentMessage::Assistant(mut assistant) = message else {
                    return;
                };
                self.turn_stop_reason = Some(assistant.stop_reason);
                self.observe_auto_loop_reply(&assistant.content);
                if let Some(component) = self.transcript.streaming.take() {
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
                    // A failed attempt is taken out again if a retry follows
                    // (`retry.rs`): an error, or an abort during a retry.
                    let failed_attempt = assistant.stop_reason == StopReason::Error
                        || (assistant.stop_reason == StopReason::Aborted
                            && self.session.retry_attempt() > 0);
                    if failed_attempt {
                        let tools = self.transcript.pending_tools.values().cloned().collect();
                        self.transcript.keep_attempt_error(component.clone(), tools);
                    }
                    if matches!(
                        assistant.stop_reason,
                        StopReason::Aborted | StopReason::Error
                    ) {
                        let error = assistant
                            .error_message
                            .clone()
                            .filter(|m| !m.is_empty())
                            .unwrap_or_else(|| "Error".into());
                        for block in self.transcript.pending_tools.values() {
                            block.borrow_mut().update_result(
                                ToolResult {
                                    content: vec![Content::text(error.clone())],
                                    details: serde_json::Value::Null,
                                    is_error: true,
                                },
                                false,
                            );
                            // The user's abort is not the call's own failure.
                            if assistant.stop_reason == StopReason::Aborted {
                                block.borrow_mut().set_interrupted();
                            }
                        }
                        self.transcript.pending_tools.clear();
                        // Aborted and errored turns settle their steps at once.
                        self.refresh_flight_stats();
                    } else {
                        // Args are complete: edit blocks compute their diffs.
                        for block in self.transcript.pending_tools.values() {
                            block.borrow_mut().set_args_complete();
                        }
                    }
                    self.transcript.streaming_message = None;
                    self.stream_render_pending = false;
                }
            }
            AgentEvent::ToolExecutionStart {
                tool_call_id,
                tool_name,
                args,
            } => {
                let block = match self.transcript.pending_tools.get(&tool_call_id) {
                    Some(block) => block.clone(),
                    None => {
                        let block = self.new_tool_block(&tool_name, &tool_call_id, args);
                        self.attach_tool_block(block.clone());
                        self.transcript
                            .pending_tools
                            .insert(tool_call_id, block.clone());
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
                if let Some(block) = self.transcript.pending_tools.get(&tool_call_id) {
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
                if let Some(block) = self.transcript.pending_tools.remove(&tool_call_id) {
                    block.borrow_mut().update_result(
                        ToolResult {
                            content: result.content,
                            details: result.details,
                            is_error,
                        },
                        false,
                    );
                    // A settled result can flip the mission's hit count.
                    self.refresh_flight_stats();
                    self.trim_transcript_memory();
                }
            }
            AgentEvent::AgentEnd { .. } => {
                self.transcript.pending_tools.clear();
                self.stop_working_loader();
                // The run ended with a message still streaming: its end never
                // came. Settle what was streamed, so the text (and the reason,
                // when it was cut off) stays on screen instead of vanishing.
                if let Some(component) = self.transcript.streaming.take() {
                    if let Some(message) = self.transcript.streaming_message.take() {
                        self.turn_stop_reason.get_or_insert(message.stop_reason);
                        component.borrow_mut().update_shared(message, false);
                    }
                    self.stream_render_pending = false;
                    self.refresh_flight_stats();
                    self.dirty.set(true);
                }
            }
            _ => {}
        }
    }
}

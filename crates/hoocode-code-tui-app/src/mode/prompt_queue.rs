//! Prompts queued while the agent streams or compacts, and follow-up / dequeue keys.

use hoocode_code_agent_session::{AgentSession, PromptOptions, StreamingBehavior};
use hoocode_code_tui_keybindings::key_display_text;
use hoocode_code_tui_theme::theme;
use hoocode_tui_components::Spacer;
use hoocode_tui_components::TruncatedText;

use super::*;

impl Mode {
    /// `sendUserMessage(text, { deliverAs: "followUp" })`: queued behind a
    /// running turn, else sent now.
    pub(super) fn send_user_follow_up(&mut self, text: String) {
        if self.session.is_streaming() || self.session.is_compacting() {
            self.queue_prompt(text, StreamingBehavior::FollowUp);
        } else {
            self.prompt(text);
        }
    }

    /// `ctx.newSession({ withSession })` from a command: a fresh session
    /// (no "New session started" line), then the message to it.
    pub(super) fn handle_extension_new_session(&mut self, text: String) {
        self.start_new_session(false, Some(text));
    }

    /// `session.prompt(text, {streamingBehavior})` for a message typed while
    /// the agent works: it only queues, so it has no turn of its own to settle.
    pub(super) fn queue_prompt(&mut self, text: String, behavior: StreamingBehavior) {
        let session = self.session.clone();
        let tx = self.tx.clone();
        self.runtime.spawn(async move {
            let result = session
                .prompt(
                    &text,
                    PromptOptions {
                        expand_prompt_templates: true,
                        streaming_behavior: Some(behavior),
                        ..Default::default()
                    },
                )
                .await;
            if let Err(error) = result {
                let _ = tx.send(AppEvent::QueueError(error.to_string()));
            }
        });
        self.update_pending_messages_display();
    }

    /// `updatePendingMessagesDisplay`: the queued steering and follow-up
    /// messages above the prompt, with the dequeue hint.
    pub(super) fn update_pending_messages_display(&mut self) {
        let mut steering = self.session.get_steering_messages();
        let mut follow_up = self.session.get_follow_up_messages();
        for (text, behavior) in &self.compaction_queue {
            match behavior {
                StreamingBehavior::Steer => steering.push(text.clone()),
                StreamingBehavior::FollowUp => follow_up.push(text.clone()),
            }
        }
        let mut pending = self.pending_messages.borrow_mut();
        pending.clear();
        if steering.is_empty() && follow_up.is_empty() {
            self.dirty.set(true);
            return;
        }
        let t = theme();
        pending.add_child(as_component(&handle(Spacer::new(1))));
        let rows = steering
            .iter()
            .map(|m| format!("Steering: {m}"))
            .chain(follow_up.iter().map(|m| format!("Follow-up: {m}")))
            .chain(std::iter::once(format!(
                "↳ {} to edit all queued messages",
                key_display_text("app.message.dequeue")
            )));
        for row in rows {
            pending.add_child(as_component(&handle(TruncatedText::new(
                t.fg("dim", &row),
                1,
                0,
            ))));
        }
        self.dirty.set(true);
    }

    /// `restoreQueuedMessagesToEditor`: every queued message, oldest first,
    /// back into the prompt ahead of what is typed; `abort` then stops the run.
    pub(super) fn restore_queued_messages_to_editor(&mut self, abort: bool) -> usize {
        let (steering, follow_up) = self.session.clear_queue();
        let mut queued: Vec<String> = steering;
        let mut compaction_follow_up = Vec::new();
        for (text, behavior) in std::mem::take(&mut self.compaction_queue) {
            match behavior {
                StreamingBehavior::Steer => queued.push(text),
                StreamingBehavior::FollowUp => compaction_follow_up.push(text),
            }
        }
        queued.extend(follow_up);
        queued.extend(compaction_follow_up);
        let count = queued.len();
        if count > 0 {
            let current = self.editor.borrow().editor.get_text();
            let combined = [queued.join("\n\n"), current]
                .into_iter()
                .filter(|t| !t.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n\n");
            self.editor.borrow_mut().editor.set_text(&combined);
        }
        self.update_pending_messages_display();
        if abort {
            self.session.agent().abort();
        }
        count
    }

    /// `queueCompactionMessage`.
    pub(super) fn queue_compaction_message(&mut self, text: String, behavior: StreamingBehavior) {
        {
            let mut editor = self.editor.borrow_mut();
            editor.editor.add_to_history(&text);
            editor.editor.set_text("");
        }
        self.compaction_queue.push((text, behavior));
        self.update_pending_messages_display();
        self.show_status("Queued message for after compaction");
    }

    /// `flushCompactionQueue`: once a compaction ends, the first held message
    /// becomes the prompt and the rest queue behind it (all of them queue
    /// when a retry is pending).
    pub(super) fn flush_compaction_queue(&mut self, will_retry: bool) {
        if self.compaction_queue.is_empty() {
            return;
        }
        let queued = std::mem::take(&mut self.compaction_queue);
        self.update_pending_messages_display();
        let queue_rest = |session: &AgentSession, rest: &[(String, StreamingBehavior)]| {
            for (text, behavior) in rest {
                let queued = match behavior {
                    StreamingBehavior::FollowUp => session.follow_up(text, &[]),
                    StreamingBehavior::Steer => session.steer(text, &[]),
                };
                queued.map_err(|e| e.to_string())?;
            }
            Ok::<(), String>(())
        };
        let failed = |error: String, count: usize| {
            format!(
                "Failed to send queued message{}: {error}",
                if count > 1 { "s" } else { "" }
            )
        };
        if will_retry {
            if let Err(error) = queue_rest(&self.session, &queued) {
                let count = queued.len();
                let _ = self.tx.send(AppEvent::CompactionQueueFailed(
                    queued,
                    failed(error, count),
                ));
            }
            self.update_pending_messages_display();
            return;
        }
        let (first, rest) = (queued[0].0.clone(), queued[1..].to_vec());
        self.prompt(first);
        if let Err(error) = queue_rest(&self.session, &rest) {
            let count = queued.len();
            let _ = self.tx.send(AppEvent::CompactionQueueFailed(
                queued,
                failed(error, count),
            ));
        }
        self.update_pending_messages_display();
    }

    /// `handleFollowUp`: alt+enter queues the prompt as a follow-up while the
    /// agent works (held during a compaction), else submits it.
    pub(super) fn handle_follow_up(&mut self) {
        let text = self.editor.borrow().editor.get_text().trim().to_string();
        if text.is_empty() {
            return;
        }
        if self.session.is_compacting() {
            self.queue_compaction_message(text, StreamingBehavior::FollowUp);
            return;
        }
        {
            let mut editor = self.editor.borrow_mut();
            editor.editor.set_text("");
        }
        if self.session.is_streaming() {
            self.editor.borrow_mut().editor.add_to_history(&text);
            self.queue_prompt(text, StreamingBehavior::FollowUp);
        } else {
            self.submit(text);
        }
    }

    /// `handleDequeue`.
    pub(super) fn handle_dequeue(&mut self) {
        match self.restore_queued_messages_to_editor(false) {
            0 => self.show_status("No queued messages to restore"),
            1 => self.show_status("Restored 1 queued message to editor"),
            n => self.show_status(&format!("Restored {n} queued messages to editor")),
        }
    }
}

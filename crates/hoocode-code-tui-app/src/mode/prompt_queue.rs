//! Prompts queued while the agent streams or compacts, and follow-up / dequeue keys.

use hoocode_code_agent_session::{AgentSession, PromptOptions, StreamingBehavior};

use super::*;

/// Delivers `texts` in order. All but the first are queued as follow-ups first,
/// then the first is started, so it begins a turn and the rest run after it,
/// each as its own turn. Queueing the rest through a second `session.prompt`
/// would race the first: both are spawned, and the loser fails with "Agent is
/// already processing a prompt" after its fire was claimed. Returns the errors
/// of the follow-ups that could not be queued.
pub(super) fn deliver_in_order(
    texts: Vec<String>,
    mut queue_follow_up: impl FnMut(&str) -> Result<(), String>,
    start: impl FnOnce(String),
) -> Vec<String> {
    let mut texts = texts.into_iter();
    let Some(first) = texts.next() else {
        return Vec::new();
    };
    let mut failures = Vec::new();
    for text in texts {
        if let Err(error) = queue_follow_up(&text) {
            failures.push(error);
        }
    }
    start(first);
    failures
}

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

    /// Delivers the prompts of the tasks that fired in one tick, in order, none
    /// lost: see [`deliver_in_order`]. Each text is queued or sent as it is.
    pub(super) fn deliver_scheduled_prompts(&mut self, texts: Vec<String>) {
        if texts.is_empty() {
            return;
        }
        let session = self.session.clone();
        let failures = deliver_in_order(
            texts,
            |text| session.follow_up(text, &[]).map_err(|e| e.to_string()),
            |first| self.send_user_follow_up(first),
        );
        self.update_pending_messages_display();
        for failure in failures {
            self.show_warning(&format!(
                "A scheduled prompt could not be queued behind the first: {failure}"
            ));
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
    /// messages, oldest first. They draw inside the comms frame (spec §4 c4),
    /// which reads them from the snapshot; the pending container keeps only
    /// parked `!` rows. The dequeue hint lives in the frame's foot.
    pub(super) fn update_pending_messages_display(&mut self) {
        let mut steering = self.session.get_steering_messages();
        let mut follow_up = self.session.get_follow_up_messages();
        for (text, behavior) in &self.compaction_queue {
            match behavior {
                StreamingBehavior::Steer => steering.push(text.clone()),
                StreamingBehavior::FollowUp => follow_up.push(text.clone()),
            }
        }
        *self.queue_snapshot.borrow_mut() = (steering, follow_up);
        self.dirty.set(true);
    }

    /// `restoreQueuedMessagesToEditor`: every queued message, oldest first,
    /// back into the prompt ahead of what is typed; `abort` then stops the run.
    pub(super) fn restore_queued_messages_to_editor(&mut self, abort: bool) -> usize {
        let (steering, follow_up) = self.session.clear_queue();
        // The queue is gone: a scheduled prompt waiting in it will not run.
        self.scheduled_runs
            .drop_waiting(task_store(), &chrono::Local::now());
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
            // A retry in progress ends with the run (`retry.rs`).
            self.session.abort_retry();
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn the_rest_are_queued_in_order_before_the_first_starts() {
        let log = RefCell::new(Vec::new());
        let failures = deliver_in_order(
            vec!["a".into(), "b".into(), "c".into()],
            |text| {
                log.borrow_mut().push(format!("queue {text}"));
                Ok(())
            },
            |first| log.borrow_mut().push(format!("start {first}")),
        );
        assert!(failures.is_empty());
        assert_eq!(*log.borrow(), ["queue b", "queue c", "start a"]);
    }

    #[test]
    fn a_single_prompt_just_starts_and_nothing_means_nothing() {
        let log = RefCell::new(Vec::new());
        deliver_in_order(
            vec!["only".into()],
            |text| {
                log.borrow_mut().push(format!("queue {text}"));
                Ok(())
            },
            |first| log.borrow_mut().push(format!("start {first}")),
        );
        assert_eq!(*log.borrow(), ["start only"]);
        deliver_in_order(
            Vec::new(),
            |_| panic!("nothing to queue"),
            |_| panic!("nothing to start"),
        );
    }

    #[test]
    fn a_follow_up_that_cannot_be_queued_is_reported_and_the_rest_still_go() {
        let log = RefCell::new(Vec::new());
        let failures = deliver_in_order(
            vec!["a".into(), "b".into(), "c".into()],
            |text| {
                if text == "b" {
                    return Err("nope".into());
                }
                log.borrow_mut().push(format!("queue {text}"));
                Ok(())
            },
            |first| log.borrow_mut().push(format!("start {first}")),
        );
        assert_eq!(failures, ["nope"]);
        assert_eq!(*log.borrow(), ["queue c", "start a"]);
    }
}

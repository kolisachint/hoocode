//! `/loop`: scheduled prompts (`/loop <interval|"cron"> <prompt>`, `once`,
//! `list`, `delete`, `stop`) and the autonomous `/loop auto` run. The forms are
//! parsed in `hoocode_code_scheduler::loop_command`; this file carries them out.

use hoocode_ai_types::{Content, StopReason};
use hoocode_code_scheduler::loop_command::{
    parse_loop_args, AutoEnd, AutoLoop, AutoNext, LoopCommand, Schedule, DONE_TOKEN,
};
use hoocode_code_scheduler::{Claimed, TaskStore, LATE_FIRE_MINUTES};

use crate::auto_loop;

use super::*;

/// A `/loop auto` run in progress.
#[derive(Debug)]
pub(super) struct LoopRun {
    state: AutoLoop,
    /// The next prompt, held while the user is typing or the agent has queued
    /// messages. It goes out on the first idle tick after that.
    next: Option<String>,
}

/// A scheduled prompt `/loop` created in this session, with the store it is in.
pub(super) type LoopTask = (TaskStore, String);

/// Removes the scheduled prompts `/loop` created in this session. Returns how
/// many were removed; one already fired and removed, or deleted, counts zero.
pub(super) fn delete_loop_tasks(tasks: Vec<LoopTask>) -> usize {
    tasks
        .iter()
        .filter(|(store, id)| store.delete(id).unwrap_or(false))
        .count()
}

/// What `/loop auto` does after a turn ends. A halt from `AskUserQuestion` ends
/// the run first, then a turn that was aborted or failed (`failed` is the
/// request's own result), then the reply and the turn cap.
pub(super) fn next_auto_step(
    state: &mut AutoLoop,
    halt: Option<String>,
    stop: Option<StopReason>,
    failed: bool,
) -> AutoNext {
    if let Some(reason) = halt {
        return AutoNext::Stop(AutoEnd::Halted(reason));
    }
    if failed || matches!(stop, Some(StopReason::Aborted | StopReason::Error)) {
        return AutoNext::Stop(AutoEnd::Interrupted);
    }
    state.turn_ended()
}

/// The text of an assistant message's blocks, one block per line.
pub(super) fn reply_text(content: &[Content]) -> String {
    content
        .iter()
        .filter_map(|block| match block {
            Content::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A prompt's first line, cut to 60 characters, for a list or a notice.
pub(super) fn preview(prompt: &str) -> String {
    let line = prompt.lines().next().unwrap_or_default();
    if line.chars().count() > 60 {
        format!("{}…", line.chars().take(59).collect::<String>())
    } else {
        line.to_string()
    }
}

fn kind(recurring: bool) -> &'static str {
    if recurring {
        "recurring"
    } else {
        "once"
    }
}

impl Mode {
    /// `/loop <args>`: the text after the command name.
    pub(super) fn handle_loop_command(&mut self, args: &str) {
        match parse_loop_args(args) {
            Err(message) => self.show_warning(&message),
            Ok(LoopCommand::Create { schedule, prompt }) => {
                self.create_scheduled_prompt(schedule, &prompt)
            }
            Ok(LoopCommand::List) => self.show_scheduled_prompts(),
            Ok(LoopCommand::Delete(id)) => self.delete_scheduled_prompt(&id),
            Ok(LoopCommand::Stop) => self.stop_loops(),
            Ok(LoopCommand::Auto { max_turns, task }) => self.start_auto_loop(max_turns, &task),
        }
    }

    /// The store the tick fires from: the session's current directory.
    fn scheduled_prompts(&self) -> TaskStore {
        TaskStore::for_cwd(self.session.cwd())
    }

    fn create_scheduled_prompt(&mut self, schedule: Schedule, prompt: &str) {
        let store = self.scheduled_prompts();
        match store.create(&schedule.cron, prompt, schedule.recurring) {
            Ok(task) => {
                self.show_status(&format!(
                    "Scheduled {} ({}, {}): {}",
                    task.id,
                    kind(schedule.recurring),
                    schedule.cron,
                    preview(prompt)
                ));
                self.loop_tasks.push((store, task.id));
            }
            Err(error) => self.show_error(&format!("Could not save the schedule: {error}")),
        }
    }

    fn show_scheduled_prompts(&mut self) {
        let tasks = self.scheduled_prompts().list();
        if tasks.is_empty() {
            self.show_status("No scheduled prompts.");
            return;
        }
        let lines: Vec<String> = tasks
            .iter()
            .map(|t| {
                format!(
                    "{}  {}  {}  {}",
                    t.id,
                    t.cron,
                    kind(t.recurring),
                    preview(&t.prompt)
                )
            })
            .collect();
        self.show_record(&lines.join("\n"));
    }

    fn delete_scheduled_prompt(&mut self, id: &str) {
        match self.scheduled_prompts().delete(id) {
            Ok(true) => {
                self.loop_tasks.retain(|(_, known)| known != id);
                self.show_status(&format!("Deleted {id}."));
            }
            Ok(false) => {
                self.show_warning(&format!("No scheduled prompt {id}. /loop list shows them."))
            }
            Err(error) => self.show_error(&format!("Could not change the schedule: {error}")),
        }
    }

    /// `/loop stop`: end `/loop auto`, and remove the scheduled prompts `/loop`
    /// created in this session. Prompts from `/loop` in an earlier session, or
    /// made by the model's Cron tools, stay; `/loop delete` removes those.
    fn stop_loops(&mut self) {
        let was_auto = self.auto_loop.is_some();
        self.end_auto_loop(AutoEnd::Stopped);
        let removed = delete_loop_tasks(std::mem::take(&mut self.loop_tasks));
        if removed > 0 {
            self.show_status(&format!(
                "Removed {removed} scheduled prompt(s) created by /loop in this session."
            ));
        } else if !was_auto {
            self.show_status("Nothing to stop.");
        }
    }

    fn start_auto_loop(&mut self, max_turns: u64, task: &str) {
        if self.auto_loop.is_some() {
            self.show_warning("/loop auto is already running. /loop stop ends it.");
            return;
        }
        if self.session.is_streaming() || self.session.is_compacting() {
            self.show_warning("Wait for the agent to finish, then run /loop auto.");
            return;
        }
        let (state, first) = AutoLoop::start(task, max_turns);
        self.auto_loop = Some(LoopRun { state, next: None });
        auto_loop::set_active(true);
        self.show_status(&format!(
            "/loop auto: up to {max_turns} turn(s), ending when a reply contains {DONE_TOKEN}. /loop stop ends it."
        ));
        self.prompt(first);
    }

    /// Records the text of an assistant message of the running turn.
    pub(super) fn observe_auto_loop_reply(&mut self, content: &[Content]) {
        if let Some(run) = self.auto_loop.as_mut() {
            run.state.observe_reply(&reply_text(content));
        }
    }

    /// `PromptDone` of a turn while `/loop auto` runs: the next prompt, or the
    /// end of the run. `failed` is whether the request itself returned an error.
    pub(super) fn auto_loop_turn_done(&mut self, failed: bool) {
        let Some(run) = self.auto_loop.as_mut() else {
            return;
        };
        let next = next_auto_step(
            &mut run.state,
            auto_loop::take_halt(),
            self.turn_stop_reason,
            failed,
        );
        match next {
            AutoNext::Prompt(text) => {
                if let Some(run) = self.auto_loop.as_mut() {
                    run.next = Some(text);
                }
                self.advance_auto_loop();
            }
            AutoNext::Stop(end) => self.end_auto_loop(end),
        }
    }

    /// Sends the held prompt of `/loop auto` once the user is idle. The loop
    /// yields while the agent works, messages are queued, or the editor has text.
    pub(super) fn advance_auto_loop(&mut self) {
        let Some(run) = self.auto_loop.as_ref() else {
            return;
        };
        if run.next.is_none() {
            return;
        }
        let typing = !self.editor.borrow().editor.get_text().trim().is_empty();
        if typing
            || self.session.is_streaming()
            || self.session.is_compacting()
            || self.session.pending_message_count() > 0
        {
            return;
        }
        if let Some(text) = self.auto_loop.as_mut().and_then(|run| run.next.take()) {
            self.prompt(text);
        }
    }

    fn end_auto_loop(&mut self, end: AutoEnd) {
        let Some(run) = self.auto_loop.take() else {
            return;
        };
        auto_loop::set_active(false);
        let turns = run.state.turns();
        let message = match &end {
            AutoEnd::Done => {
                format!("/loop auto finished in {turns} turn(s): a reply contained {DONE_TOKEN}.")
            }
            AutoEnd::TurnCap => format!(
                "/loop auto stopped at the {}-turn cap without {DONE_TOKEN}.",
                run.state.max_turns()
            ),
            AutoEnd::Halted(reason) => {
                format!("/loop auto stopped after {turns} turn(s): {reason}")
            }
            AutoEnd::Interrupted => {
                format!("/loop auto stopped after {turns} turn(s): the turn was interrupted.")
            }
            AutoEnd::Stopped => format!("/loop auto stopped after {turns} turn(s)."),
        };
        if end == AutoEnd::Done {
            self.show_status(&message);
        } else {
            self.show_warning(&message);
        }
    }

    /// `tick_scheduler`'s notices: the prompts that ran late and the ones skipped.
    pub(super) fn report_scheduled_prompts(&mut self, claimed: &Claimed) {
        for prompt in &claimed.skipped {
            self.show_warning(&format!(
                "Scheduled prompt skipped: it was due more than {LATE_FIRE_MINUTES} minutes ago, while the agent was busy: {}",
                preview(prompt)
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reply_is_its_text_blocks_one_per_line() {
        let content = vec![Content::text("Drafted."), Content::text("LOOP_DONE")];
        assert_eq!(reply_text(&content), "Drafted.\nLOOP_DONE");
        assert_eq!(reply_text(&[]), "");
    }

    #[test]
    fn a_failed_or_aborted_turn_ends_the_auto_loop() {
        let (mut run, _) = AutoLoop::start("task", 10);
        assert_eq!(
            next_auto_step(&mut run, None, None, true),
            AutoNext::Stop(AutoEnd::Interrupted)
        );
        assert_eq!(run.turns(), 1, "a failed turn starts no next turn");
        let (mut run, _) = AutoLoop::start("task", 10);
        assert_eq!(
            next_auto_step(&mut run, None, Some(StopReason::Aborted), false),
            AutoNext::Stop(AutoEnd::Interrupted)
        );
        assert_eq!(
            next_auto_step(&mut run, None, Some(StopReason::Error), false),
            AutoNext::Stop(AutoEnd::Interrupted)
        );
    }

    #[test]
    fn a_halt_wins_and_a_clean_turn_continues() {
        let (mut run, _) = AutoLoop::start("task", 10);
        assert_eq!(
            next_auto_step(
                &mut run,
                Some("no default".into()),
                Some(StopReason::Stop),
                false
            ),
            AutoNext::Stop(AutoEnd::Halted("no default".into()))
        );
        let (mut run, _) = AutoLoop::start("task", 10);
        assert!(matches!(
            next_auto_step(&mut run, None, Some(StopReason::Stop), false),
            AutoNext::Prompt(_)
        ));
        assert_eq!(run.turns(), 2);
    }

    #[test]
    fn delete_loop_tasks_removes_only_the_listed_ones() {
        let dir = tempfile::tempdir().unwrap();
        let store = TaskStore::for_cwd(dir.path());
        let mine = store.create("*/5 * * * *", "mine", true).unwrap();
        let theirs = store.create("*/5 * * * *", "theirs", true).unwrap();
        let removed = delete_loop_tasks(vec![
            (store.clone(), mine.id.clone()),
            // Already removed, as a fired one-shot would be.
            (store.clone(), "task_gone".into()),
        ]);
        assert_eq!(removed, 1);
        let left = store.list();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].id, theirs.id);
    }

    #[test]
    fn a_long_prompt_is_previewed_by_its_first_line_and_cut() {
        assert_eq!(preview("check the build\nthen report"), "check the build");
        let long = "x".repeat(80);
        let shown = preview(&long);
        assert_eq!(shown.chars().count(), 60);
        assert!(shown.ends_with('…'));
    }
}

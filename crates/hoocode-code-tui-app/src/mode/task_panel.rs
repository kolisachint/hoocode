//! The task panel's focus and events: what the app does with what the panel
//! asks for. The panel itself is `hoocode_code_tui_widgets::task_panel`, and
//! its keys are the `app.panel.*` bindings.

use hoocode_code_permissions::PermissionUi as _;
use hoocode_code_subagents::instance::get_subagent_pool;
use hoocode_code_task_store::{task_store, ActionNotifier, RowAction, TaskKind};
use hoocode_code_tui_widgets::task_panel::{PanelEvent, TaskPanelView};

use crate::end_shift::{EndShiftChoice, EndShiftJobsChoice, ShiftDebrief, ShiftJob};

use super::*;

/// Where the quit prompt for running shell jobs stands (T5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum QuitJobs {
    /// No quit has asked yet, or the last one was cancelled.
    Unasked,
    /// The prompt is on screen.
    Asking,
    /// Answered: the quit goes ahead.
    Decided,
}

/// Where the end-of-shift card stands (M6, spec §6.2): a user-initiated
/// quit (`/quit`, ctrl+c twice, ctrl+d) with nothing running shows the
/// amber card. `enter` ends the shift, `esc` stays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EndShift {
    /// No quit has asked yet, or the last one was cancelled.
    Unasked,
    /// The card is on screen.
    Asking,
    /// Answered: the quit goes ahead.
    Decided,
}

impl Mode {
    /// `app.panel.focus` (Ctrl+G): the task panel takes the keyboard. With no
    /// rows there is nothing to pick, so the prompt keeps the keys.
    pub(super) fn focus_task_panel(&mut self) {
        if !self.task_panel.borrow_mut().can_focus() {
            self.show_status("No tasks to show.");
            return;
        }
        self.tui.set_focus(Some(as_component(&self.task_panel)));
        self.dirty.set(true);
    }

    /// Back to the prompt. `set_focus` also clears the panel's focus state.
    fn leave_task_panel(&mut self) {
        self.tui.set_focus(Some(self.prompt_component()));
        self.dirty.set(true);
    }

    /// Runs the events the panel queued while it had the keyboard. Called
    /// after each input pass, before the queued actions.
    pub(super) fn drain_task_panel_events(&mut self) {
        let events = self.task_panel.borrow_mut().take_events();
        for event in events {
            match event {
                PanelEvent::Open { view, task_id } => self.open_task_panel_row(view, task_id),
                PanelEvent::Perform { task_id, action } => {
                    self.perform_task_panel_row(task_id, action)
                }
                PanelEvent::Cancel { run_id } => self.cancel_subagent_run(&run_id),
                PanelEvent::Exit => self.leave_task_panel(),
            }
        }
    }

    /// Enter on a row. A subagent row attaches to its run (T3.3), a shell row
    /// to its job's output (T5.3). Other rows have nothing to open, so Enter
    /// does nothing there.
    fn open_task_panel_row(&mut self, _view: TaskPanelView, task_id: u64) {
        let Some(task) = task_store().list().into_iter().find(|t| t.id == task_id) else {
            return;
        };
        match task.kind() {
            TaskKind::Subagent => {
                if let Some(run_id) = task.agent.clone() {
                    let shown = (
                        task.subagent_mode
                            .clone()
                            .unwrap_or_else(|| "subagent".into()),
                        task.title.clone(),
                    );
                    self.attach_run(run_id, Some(shown));
                }
            }
            TaskKind::Shell => {
                if let Some(key) = task.key.clone() {
                    self.attach_job(key);
                }
            }
            TaskKind::Plan | TaskKind::Schedule | TaskKind::Mcp => {}
        }
    }

    /// A user-initiated quit (`/quit`, ctrl+c twice, ctrl+d): the loop
    /// shows the end-of-shift card (or the satellites card) instead of
    /// leaving at once. Session errors exit without asking.
    pub(super) fn request_user_quit(&mut self) {
        self.exit_requested = true;
        self.user_quit = true;
    }

    /// Whether the event loop ends. A quit while shell jobs run shows the
    /// satellites card (T5.3, spec §4 c8): `k` brings every job down (the
    /// old default), `l` leaves them. A user-initiated quit with nothing
    /// running shows the end-of-shift card with the shift's debrief (M6,
    /// spec §6.2): `enter` ends, `esc` stays. Anything else (a fatal
    /// session error) leaves at once. Esc keeps the app open, and the next
    /// quit asks again. A quit asked again while a card is up changes nothing.
    pub(super) fn ready_to_quit(&mut self) -> bool {
        if !self.exit_requested {
            return false;
        }
        match self.quit_jobs {
            QuitJobs::Decided => true,
            QuitJobs::Asking => {
                self.exit_requested = false;
                false
            }
            QuitJobs::Unasked => match self.end_shift {
                EndShift::Decided => true,
                EndShift::Asking => {
                    self.exit_requested = false;
                    false
                }
                EndShift::Unasked => {
                    if hoocode_code_tool_bash::jobs::running_job_count() > 0 {
                        self.exit_requested = false;
                        self.quit_jobs = QuitJobs::Asking;
                        let jobs = hoocode_code_tool_bash::jobs::running_jobs()
                            .into_iter()
                            .map(|info| ShiftJob {
                                id: info.id,
                                command: info.command,
                                running: info.state.is_running(),
                            })
                            .collect();
                        self.show_end_shift_jobs(jobs);
                        return false;
                    }
                    if self.user_quit {
                        self.exit_requested = false;
                        self.end_shift = EndShift::Asking;
                        self.show_end_shift(self.shift_debrief());
                        return false;
                    }
                    true
                }
            },
        }
    }

    /// The shift's debrief for the end-of-shift card, from the session's
    /// real totals: missions (user messages), steps (tool calls), tokens,
    /// spend. Hits and landings are not tracked, so the card does not claim them.
    fn shift_debrief(&self) -> ShiftDebrief {
        let stats = self.session.get_session_stats();
        ShiftDebrief {
            missions: stats.user_messages,
            steps: stats.tool_calls,
            input_tokens: stats.tokens.input,
            output_tokens: stats.tokens.output,
            cost: stats.cost,
        }
    }

    /// The end-of-shift card's answer. `enter` ends the shift; `esc` stays.
    pub(super) fn close_end_shift(&mut self, choice: EndShiftChoice) {
        self.end_shift_card = None;
        self.restore_editor();
        match choice {
            EndShiftChoice::End => {
                self.end_shift = EndShift::Decided;
                self.exit_requested = true;
            }
            EndShiftChoice::Stay => {
                self.end_shift = EndShift::Unasked;
                self.user_quit = false;
            }
        }
        self.dirty.set(true);
    }

    /// The satellites card's answer. "Bring it down" stops every job's
    /// process group before the app exits (the old default); "leave it"
    /// exits and leaves them. Esc stays in the app.
    pub(super) fn close_end_shift_jobs(&mut self, choice: EndShiftJobsChoice) {
        self.end_shift_jobs_card = None;
        self.restore_editor();
        match choice {
            EndShiftJobsChoice::KillAll => {
                hoocode_code_tool_bash::jobs::kill_all_jobs();
                self.quit_jobs = QuitJobs::Decided;
                self.exit_requested = true;
            }
            EndShiftJobsChoice::LeaveRunning => {
                self.quit_jobs = QuitJobs::Decided;
                self.exit_requested = true;
            }
            EndShiftJobsChoice::Stay => self.quit_jobs = QuitJobs::Unasked,
        }
        self.dirty.set(true);
    }

    /// Enter on a row with an action (Login), or `x` twice on an Abort row:
    /// the store hands it to the row's producer. Its text goes to the user as
    /// a notice; an error shows in the status line.
    fn perform_task_panel_row(&mut self, task_id: u64, action: RowAction) {
        let notify: ActionNotifier = Arc::new(|message: String| {
            crate::dialog_bridge::TuiPermissionUi.notify(&message);
        });
        if let Err(message) = task_store().perform(task_id, action, notify) {
            self.show_status(&message);
        }
    }

    /// `x` twice on a running subagent row: stop that run through the pool,
    /// as `/subagent-cancel` does.
    fn cancel_subagent_run(&mut self, run_id: &str) {
        // Reading the models and making the pool may do I/O: both happen on
        // the tools pool, and the outcome comes back as a status line.
        let cwd = self.session.cwd().to_path_buf();
        let session = self.session.clone();
        let tx = self.tx.clone();
        let run_id = run_id.to_string();
        self.runtime.spawn(async move {
            let models = session.get_available_models();
            let pool = get_subagent_pool(&cwd, &models);
            let message = if pool.cancel(&run_id) {
                format!("Cancelling subagent {run_id}…")
            } else {
                "That subagent has already ended.".to_string()
            };
            let _ = tx.send(AppEvent::Status(message));
        });
    }
}

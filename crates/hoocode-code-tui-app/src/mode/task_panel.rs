//! The task panel's focus and events: what the app does with what the panel
//! asks for. The panel itself is `hoocode_code_tui_widgets::task_panel`, and
//! its keys are the `app.panel.*` bindings.

use hoocode_code_permissions::PermissionUi as _;
use hoocode_code_subagents::instance::get_subagent_pool;
use hoocode_code_task_store::{task_store, ActionNotifier, RowAction, TaskKind};
use hoocode_code_tui_widgets::task_panel::{PanelEvent, TaskPanelView};

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

    /// Whether the event loop ends. A quit while shell jobs run asks once
    /// (T5.3): "Kill all" (the default) or "Leave running". Esc keeps the app
    /// open, and the next quit asks again.
    pub(super) fn ready_to_quit(&mut self) -> bool {
        if !self.exit_requested {
            return false;
        }
        match self.quit_jobs {
            QuitJobs::Decided => true,
            // A quit asked again while the prompt is up changes nothing.
            QuitJobs::Asking => {
                self.exit_requested = false;
                false
            }
            QuitJobs::Unasked => {
                let running = hoocode_code_tool_bash::jobs::running_job_count();
                if running == 0 {
                    return true;
                }
                self.exit_requested = false;
                self.quit_jobs = QuitJobs::Asking;
                let noun = if running == 1 { "job" } else { "jobs" };
                let title = format!("{running} background shell {noun} still running. Quit?");
                let answer = self.answer_to(AppEvent::QuitJobsAnswer);
                self.show_selector(
                    &title,
                    vec!["Kill all".into(), "Leave running".into()],
                    answer,
                );
                false
            }
        }
    }

    /// The quit prompt's answer. "Kill all" stops every job of the process
    /// before the app exits; "Leave running" exits and leaves them.
    pub(super) fn quit_jobs_answered(&mut self, choice: Option<String>) {
        match choice.as_deref() {
            Some("Kill all") => {
                hoocode_code_tool_bash::jobs::kill_all_jobs();
                self.quit_jobs = QuitJobs::Decided;
                self.exit_requested = true;
            }
            Some("Leave running") => {
                self.quit_jobs = QuitJobs::Decided;
                self.exit_requested = true;
            }
            // Esc: stay in the app.
            _ => self.quit_jobs = QuitJobs::Unasked,
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

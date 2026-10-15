//! The task panel's focus and events: what the app does with what the panel
//! asks for. The panel itself is `hoocode_code_tui_widgets::task_panel`, and
//! its keys are the `app.panel.*` bindings.

use hoocode_code_permissions::PermissionUi as _;
use hoocode_code_subagents::instance::get_subagent_pool;
use hoocode_code_task_store::{task_store, ActionNotifier, RowAction, TaskKind};
use hoocode_code_tui_widgets::task_panel::{PanelEvent, TaskPanelView};

use super::*;

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

    /// Enter on a row. A subagent row attaches to its run (T3.3); other rows
    /// have nothing to open yet, so Enter does nothing there.
    fn open_task_panel_row(&mut self, _view: TaskPanelView, task_id: u64) {
        let run = task_store()
            .list()
            .into_iter()
            .find(|task| task.id == task_id)
            .filter(|task| task.kind() == TaskKind::Subagent)
            .and_then(|task| {
                let shown = (
                    task.subagent_mode
                        .clone()
                        .unwrap_or_else(|| "subagent".into()),
                    task.title.clone(),
                );
                Some((task.agent?, shown))
            });
        if let Some((run_id, shown)) = run {
            self.attach_run(run_id, Some(shown));
        }
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
        let cwd = self.session.cwd().to_path_buf();
        let models = self.session.get_available_models();
        let pool = get_subagent_pool(&cwd, &models);
        if pool.cancel(run_id) {
            self.show_status(&format!("Cancelling subagent {run_id}…"));
        } else {
            self.show_status("That subagent has already ended.");
        }
    }
}

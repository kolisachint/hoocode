//! The task panel's focus and events: what the app does with what the panel
//! asks for. The panel itself is `hoocode_code_tui_widgets::task_panel`, and
//! its keys are the `app.panel.*` bindings.

use hoocode_code_subagents::instance::get_subagent_pool;
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
        self.tui.set_focus(Some(as_component(&self.editor)));
        self.dirty.set(true);
    }

    /// Runs the events the panel queued while it had the keyboard. Called
    /// after each input pass, before the queued actions.
    pub(super) fn drain_task_panel_events(&mut self) {
        let events = self.task_panel.borrow_mut().take_events();
        for event in events {
            match event {
                PanelEvent::Open { view, task_id } => self.open_task_panel_row(view, task_id),
                PanelEvent::Cancel { run_id } => self.cancel_subagent_run(&run_id),
                PanelEvent::Exit => self.leave_task_panel(),
            }
        }
    }

    /// Enter on a row. The attach view (T3.3) fills this in; until then the
    /// panel only moves the pick.
    fn open_task_panel_row(&mut self, _view: TaskPanelView, _task_id: u64) {}

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

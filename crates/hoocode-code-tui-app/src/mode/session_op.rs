//! Session operations that run off the UI thread (TUI plan N14).
//!
//! The UI loop never waits on the runtime. An operation moves the
//! `AgentSessionRuntime` (or only the session) onto the runtime, and the
//! result comes back as `AppEvent::SessionOp`; [`Mode::finish_session_op`]
//! applies it. While one runs, a loader shows and the prompt refuses input;
//! a second operation says the first must finish.

use std::path::PathBuf;

use hoocode_code_agent_session::runtime::{ChangeDirectoryResult, RuntimeError};
use hoocode_code_agent_session::{NavigateTreeResult, ReplaceResult};

use super::*;

/// What a session operation found out.
pub(super) enum SessionOutcome {
    /// `/mode <next>` sent to the session as a prompt (the Shift+Tab dial).
    Mode {
        forward: bool,
        next: String,
        result: Result<(), String>,
    },
    /// `/tree` to a point without a summary.
    Tree {
        entry_id: String,
        result: Box<Result<NavigateTreeResult, String>>,
    },
    /// `/new`, or a command's `newSession`; `follow_up` is sent to the new session.
    New {
        announce: bool,
        follow_up: Option<String>,
        result: Result<ReplaceResult, RuntimeError>,
    },
    /// `/cd`: the target and the directory left.
    ChangeDirectory {
        target: PathBuf,
        previous_cwd: PathBuf,
        result: Result<ChangeDirectoryResult, RuntimeError>,
    },
    /// `/reload`: the session re-read its resources (the screen is applied after).
    Reload,
}

/// A finished session operation. The runtime comes back with it when the
/// operation used it.
pub(super) struct SessionOpDone {
    pub(super) runtime: Option<AgentSessionRuntime>,
    pub(super) outcome: SessionOutcome,
}

impl Mode {
    /// False, with a warning, when another session operation is running.
    pub(super) fn session_op_free(&mut self) -> bool {
        match self.session_op {
            None => true,
            Some(running) => {
                self.show_warning(&format!(
                    "{running} is still running; wait for it to finish."
                ));
                false
            }
        }
    }

    /// Marks `label` as running; the caller shows its own loader.
    pub(super) fn mark_session_op(&mut self, label: &'static str) {
        self.session_op = Some(label);
        self.dirty.set(true);
    }

    /// Marks `label` as running and shows its loader.
    pub(super) fn start_session_op(&mut self, label: &'static str) {
        self.mark_session_op(label);
        self.stop_working_loader();
        let mut loader = Loader::new(
            Box::new(|s: &str| theme().fg("accent", s)),
            Box::new(|s: &str| theme().fg("muted", s)),
            format!("{label}..."),
            None,
        );
        loader.start();
        let loader = handle(loader);
        self.status.borrow_mut().add_child(as_component(&loader));
        self.loader = Some(loader);
        self.dirty.set(true);
    }

    /// The runtime for an operation that replaces the session. None when
    /// there is none (nothing to do) or another operation is running.
    pub(super) fn take_session_runtime(
        &mut self,
        label: &'static str,
    ) -> Option<AgentSessionRuntime> {
        if !self.session_op_free() {
            return None;
        }
        let runtime = self.session_runtime.take()?;
        self.start_session_op(label);
        Some(runtime)
    }

    /// Applies a finished operation on the UI thread.
    pub(super) fn finish_session_op(&mut self, done: SessionOpDone) {
        self.session_op = None;
        self.stop_working_loader();
        if let Some(runtime) = done.runtime {
            self.session_runtime = Some(runtime);
        }
        match done.outcome {
            SessionOutcome::Mode {
                forward,
                next,
                result,
            } => {
                if let Err(error) = result {
                    self.show_error(&error);
                } else {
                    self.drain_extension_ui_requests();
                    let landed = self.footer_data.get_active_mode();
                    let landed = if landed.is_empty() { next } else { landed };
                    self.show_dial_step(
                        if forward {
                            "app.mode.cycleBackward"
                        } else {
                            "app.mode.cycleForward"
                        },
                        &format!("Mode: {landed}"),
                    );
                }
            }
            SessionOutcome::Tree { entry_id, result } => {
                self.finish_tree_navigation(entry_id, *result)
            }
            SessionOutcome::New {
                announce,
                follow_up,
                result,
            } => self.finish_new_session(announce, follow_up, result),
            SessionOutcome::ChangeDirectory {
                target,
                previous_cwd,
                result,
            } => self.finish_change_directory(target, previous_cwd, result),
            SessionOutcome::Reload => self.finish_reload(),
        }
        self.dirty.set(true);
    }
}

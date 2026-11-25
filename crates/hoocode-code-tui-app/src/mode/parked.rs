//! A call a detached run parked (SA-12 row 4b), asked of the user when its session opens.
//!
//! The clearance card shows the call (rows: yes, approve and detach, no). Approve and deny
//! are answered on the session, off the UI thread (`AppEvent::Parked`): the call runs once
//! or is denied, the other calls of its message are answered `Not run`, and the run goes on
//! in the background. "Approve and detach" also takes the runtime (a session operation,
//! `session_op.rs`): this window opens a new empty session (which lets go of the parked
//! file), and a detached process continues the run.

use std::path::{Path, PathBuf};
use std::time::Duration;

use hoocode_code_agent_session::{
    AgentSession, AgentSessionRuntime, NewSessionRequest, ParkedCall, ParkedDecision, ParkedOutcome,
};
use hoocode_code_permissions::{describe_tool, reach_for, ClearanceChoice, ClearanceInfo};
use hoocode_code_session::lock::lock_holder;
use hoocode_code_subagents::detached_child::{
    child_command_continue, log_path_for, spawn_detached,
};
use hoocode_tui_components::Text;

use super::session_op::{SessionOpDone, SessionOutcome};
use super::*;
use crate::clearance_card::ClearanceCard;

/// The open parked-call card.
pub(super) type OpenParkedCard = Rc<RefCell<ClearanceCard>>;

/// How the user answered the card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ParkedChoice {
    /// Run the call once, then continue the run here.
    Approve,
    /// Deny the call, then continue the run here.
    Deny,
    /// Run the call once, then continue the run in a detached process.
    ApproveAndDetach,
}

/// A run handed to a detached process.
pub(super) struct DetachedRun {
    file: PathBuf,
    cwd: PathBuf,
    /// The provider and model id of the run, when it has a model.
    model: Option<(String, String)>,
}

/// What answering the call came to.
pub(super) struct ParkedReport {
    tool: String,
    /// The file that changed after the call was parked (the call did not run).
    changed: Option<PathBuf>,
    /// Set when the run was handed to a detached process.
    detached: Option<DetachedRun>,
}

impl Mode {
    /// Asks about the parked call of the session that is now shown, if it has one. A
    /// target file that changed since the call was parked is not asked about: the call is
    /// recorded as not run and the model is told.
    pub(super) fn check_parked_call(&mut self) {
        // A followed session's run is alive, so its call is not parked (SA-12 row 5).
        if self.parked_call.is_some() || self.session_op.is_some() || self.is_following() {
            return;
        }
        let Some(call) = self.session.open_parked_call() else {
            return;
        };
        if self.session.parked_call_target_changed(&call) {
            self.start_parked_op(call, ParkedChoice::Approve);
            return;
        }
        self.open_parked_card(call);
    }

    /// Puts the parked-call card in the editor's slot, focused.
    fn open_parked_card(&mut self, call: ParkedCall) {
        let info = ClearanceInfo {
            tool: call.tool_name.clone(),
            detail: describe_tool(&call.tool_name, &call.input),
            mode: self.footer_data.get_active_mode(),
            step: None,
            reach: reach_for(&call.tool_name, &call.input),
            from: None,
        };
        let on_done = self.actions.clone();
        let on_detach = self.actions.clone();
        let card = handle(ClearanceCard::parked(
            info,
            Box::new(move |choice| {
                let choice = match choice {
                    ClearanceChoice::Once => ParkedChoice::Approve,
                    _ => ParkedChoice::Deny,
                };
                on_done.borrow_mut().push(Action::ParkedDone(choice));
            }),
            Box::new(move || {
                on_detach
                    .borrow_mut()
                    .push(Action::ParkedDone(ParkedChoice::ApproveAndDetach));
            }),
        ));
        {
            let mut container = self.editor_container.borrow_mut();
            container.clear();
            container.add_child(as_component(&card));
        }
        self.tui.set_focus(Some(as_component(&card)));
        self.parked_card = Some(card);
        self.parked_call = Some(call);
        self.dirty.set(true);
    }

    /// The card's answer: the card goes and the prompt returns, then the call is answered.
    pub(super) fn answer_parked(&mut self, choice: ParkedChoice) {
        self.parked_card = None;
        {
            let mut container = self.editor_container.borrow_mut();
            container.clear();
            container.add_child(self.prompt_component());
        }
        self.tui.set_focus(Some(self.prompt_component()));
        self.dirty.set(true);
        if let Some(call) = self.parked_call.take() {
            self.start_parked_op(call, choice);
        }
    }

    /// Answers `call` off the UI thread. Approve and deny need only the session; approve
    /// and detach also takes the runtime, which comes back with the result.
    fn start_parked_op(&mut self, call: ParkedCall, choice: ParkedChoice) {
        if choice != ParkedChoice::ApproveAndDetach {
            let session = self.session.clone();
            let tx = self.tx.clone();
            self.runtime.spawn(async move {
                let report = resolve_parked_call(&session, &call, choice).await;
                let _ = tx.send(AppEvent::Parked(Ok(report)));
            });
            return;
        }
        let Some(runtime) = self.take_session_runtime("Detaching the run") else {
            return;
        };
        let tx = self.tx.clone();
        self.runtime.spawn(async move {
            let mut runtime = runtime;
            let result = detach_parked_call(&mut runtime, call).await;
            let _ = tx.send(AppEvent::SessionOp(Box::new(SessionOpDone {
                runtime: Some(runtime),
                outcome: SessionOutcome::Parked { result },
            })));
        });
    }

    /// Applies a finished answer on the UI thread: notices, then the run goes on here or
    /// in a detached process.
    pub(super) fn finish_parked(&mut self, result: Result<ParkedReport, String>) {
        let report = match result {
            Ok(report) => report,
            Err(message) => {
                self.show_error(&message);
                return;
            }
        };
        if let Some(path) = &report.changed {
            self.show_status(&format!(
                "Parked {} not run: {} changed. The model was told.",
                report.tool,
                path.display()
            ));
        }
        match report.detached {
            None => self.start_parked_continue(),
            Some(run) => {
                self.rebind_current_session();
                self.render_current_session_state();
                self.show_detached_notice(&run.file);
                self.spawn_detached_run(run);
            }
        }
    }

    /// The run goes on in the background, as a turn does (its end comes as `PromptDone`).
    fn start_parked_continue(&mut self) {
        self.prompt_pending = true;
        self.wake_in_flight = true;
        self.turn_timer.borrow_mut().begin(Instant::now());
        let session = self.session.clone();
        let mut done = PromptDoneGuard::new(self.tx.clone());
        self.runtime.spawn(async move {
            let result = session.continue_run().await.map_err(|e| e.to_string());
            done.send(result);
        });
    }

    fn show_detached_notice(&mut self, file: &Path) {
        let code = theme().fg("accent", &format!("hoocode --session {}", file.display()));
        let line = format!(
            "{} {} Open it with {}",
            theme().fg("success", "●"),
            theme().bold("Detached: the run goes on without this window."),
            code
        );
        self.add_to_chat(as_component(&handle(Spacer::new(1))));
        self.add_to_chat(as_component(&handle(Text::new(line, 1, 0))));
    }

    /// Starts the detached process once this window has let go of the file. The window
    /// waits (bounded) for the lock to drop, then starts the child; a failure is a status line.
    fn spawn_detached_run(&mut self, run: DetachedRun) {
        let tx = self.tx.clone();
        self.runtime.spawn(async move {
            for _ in 0..50 {
                if lock_holder(&run.file).is_none() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            if let Err(error) = start_detached_process(&run) {
                let _ = tx.send(AppEvent::Status(format!(
                    "Could not start the detached run: {error}"
                )));
            }
        });
    }
}

/// Answers the parked call on `session`: the call runs once (approve) or is denied, and
/// the other calls of its message that never ran are answered `Not run`. The file is
/// flushed. A target that changed since the call was parked is not run.
async fn resolve_parked_call(
    session: &AgentSession,
    call: &ParkedCall,
    choice: ParkedChoice,
) -> ParkedReport {
    let decision = match choice {
        ParkedChoice::Deny => ParkedDecision::Deny,
        ParkedChoice::Approve | ParkedChoice::ApproveAndDetach => ParkedDecision::Approve,
    };
    let outcome = session.resolve_parked_call(call, decision).await;
    let changed = match outcome {
        ParkedOutcome::TargetChanged => call.target.as_ref().map(|(path, _)| path.clone()),
        ParkedOutcome::Ran | ParkedOutcome::Denied => None,
    };
    ParkedReport {
        tool: call.tool_name.clone(),
        changed,
        detached: None,
    }
}

/// Approve and detach: resolves the call (approve), then lets go of the parked file by
/// giving the runtime a new empty session, and records the run to hand to a detached
/// process. The process is started by [`Mode::spawn_detached_run`] once the lock is free.
async fn detach_parked_call(
    runtime: &mut AgentSessionRuntime,
    call: ParkedCall,
) -> Result<ParkedReport, String> {
    let session = runtime.session().clone();
    let mut report = resolve_parked_call(&session, &call, ParkedChoice::ApproveAndDetach).await;
    let file = session
        .session_file()
        .ok_or_else(|| "The parked session has no file.".to_string())?;
    let run = DetachedRun {
        file,
        cwd: session.cwd().to_path_buf(),
        model: session.model().map(|m| (m.provider.clone(), m.id.clone())),
    };
    drop(session);
    runtime
        .new_session(NewSessionRequest::default())
        .await
        .map_err(|e| e.to_string())?;
    report.detached = Some(run);
    Ok(report)
}

/// Starts `hoocode --session <file>` detached, with its log beside the file, and reaps it
/// off this thread (it outlives this process).
fn start_detached_process(run: &DetachedRun) -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    let model = run
        .model
        .as_ref()
        .map(|(provider, id)| (provider.as_str(), id.as_str()));
    let mut command = child_command_continue(&exe, &run.file, &run.cwd, model);
    let mut child = spawn_detached(&mut command, &log_path_for(&run.file))?;
    let _ = hoocode_runtime::spawn_named_thread("hoocode-detached-reaper", move || {
        let _ = child.wait();
    });
    Ok(())
}

//! How code off the UI thread asks the interactive mode something: the
//! permission gate's `select`/`notify` (the pin's `ctx.ui.select` /
//! `ctx.ui.notify`), answered by the selector dialog, and the `ask_options`
//! tool's questions (`ctx.ui.askOptions`), answered by the options pane.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::sync::{Mutex, OnceLock};

use std::time::Duration;

use hoocode_ai_types::AbortSignal;
use hoocode_code_permissions::{
    ApprovalChannel, ClearanceChoice, ClearanceInfo, ClearanceRelay, HooPermissionGate,
    PermissionUi, RelayRequest,
};
use hoocode_code_tools_optin::{AskOptionsHost, AskQuestion};

use crate::clearance_queue::OWN_CALL;

/// A request for the interactive mode.
pub enum DialogRequest {
    /// Show a selector; the answer (`None` = cancelled) goes back on `reply`.
    Select {
        title: String,
        options: Vec<String>,
        reply: mpsc::Sender<Option<String>>,
    },
    /// An info notification (`showStatus`).
    Notify(String),
    /// Show the options pane; one answer per question, or `None` when skipped.
    /// `chime` is set for the `ask_options` tool only (the completion chime rings
    /// when the agent asks a question), not for MCP elicitation forms.
    /// `esc_answers_first` is set for the tool too (Mission Control: esc
    /// picks option 1); elicitation keeps esc-as-cancel.
    AskOptions {
        questions: Vec<AskQuestion>,
        reply: mpsc::Sender<Option<Vec<String>>>,
        chime: bool,
        esc_answers_first: bool,
    },
    /// Show the clearance card for a gated call; the answer goes back on `reply`.
    /// `id` is what [`DialogRequest::HideClearance`] withdraws it by
    /// ([`OWN_CALL`] for the main agent's calls, which are never withdrawn). A
    /// card already up keeps its place and this one waits behind it.
    Clearance {
        id: u64,
        info: ClearanceInfo,
        reply: mpsc::Sender<ClearanceChoice>,
    },
    /// The asker stopped waiting (its subagent ended): take the card down, or
    /// out of the queue, unanswered.
    HideClearance(u64),
    /// The asker's signal fired: take the options pane down unanswered.
    HideAskOptions,
}

type Sink = Box<dyn Fn(DialogRequest) -> bool + Send>;

fn sink() -> &'static Mutex<Option<Sink>> {
    static SINK: OnceLock<Mutex<Option<Sink>>> = OnceLock::new();
    SINK.get_or_init(|| Mutex::new(None))
}

/// Route requests to a running interactive mode (`None` detaches it). The
/// sink returns false when the mode is gone.
pub fn set_dialog_sink(new: Option<Sink>) {
    *sink().lock().unwrap_or_else(|e| e.into_inner()) = new;
}

fn send(request: DialogRequest) -> bool {
    match sink().lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        Some(send) => send(request),
        None => false,
    }
}

/// The permission gate's UI in interactive mode. With no mode attached a
/// question reads as cancelled.
pub struct TuiPermissionUi;

impl PermissionUi for TuiPermissionUi {
    fn select(&self, title: &str, options: &[&str]) -> Option<String> {
        let (reply, answer) = mpsc::channel();
        let sent = send(DialogRequest::Select {
            title: title.to_string(),
            options: options.iter().map(|o| o.to_string()).collect(),
            reply,
        });
        if !sent {
            return None;
        }
        answer.recv().ok().flatten()
    }

    fn notify(&self, message: &str) {
        send(DialogRequest::Notify(message.to_string()));
    }

    /// The clearance card in interactive mode. With no mode attached the
    /// call reads as denied (never a silent allow).
    fn select_clearance(&self, info: &ClearanceInfo) -> ClearanceChoice {
        ask_clearance(OWN_CALL, info.clone(), None)
    }
}

/// Shows the clearance card and waits for the answer. With `cancelled`, a
/// flag that goes up takes the card down and reads as a denial.
fn ask_clearance(id: u64, info: ClearanceInfo, cancelled: Option<&AtomicBool>) -> ClearanceChoice {
    let deny = |reason: Option<&str>| ClearanceChoice::Deny {
        reason: reason.map(str::to_string),
    };
    let (reply, answer) = mpsc::channel();
    if !send(DialogRequest::Clearance { id, info, reply }) {
        return deny(None);
    }
    let Some(cancelled) = cancelled else {
        return answer.recv().unwrap_or_else(|_| deny(None));
    };
    loop {
        match answer.recv_timeout(Duration::from_millis(50)) {
            Ok(choice) => return choice,
            Err(mpsc::RecvTimeoutError::Disconnected) => return deny(None),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if cancelled.load(Ordering::SeqCst) {
                    send(DialogRequest::HideClearance(id));
                    return deny(Some("the subagent ended"));
                }
            }
        }
    }
}

/// The clearance card for a call a subagent asked about: the TUI's card, named
/// for the subagent, that goes away if the subagent does.
struct RelayedUi {
    id: u64,
    via: String,
    cancelled: Arc<AtomicBool>,
}

impl PermissionUi for RelayedUi {
    fn select(&self, title: &str, options: &[&str]) -> Option<String> {
        TuiPermissionUi.select(title, options)
    }

    fn notify(&self, message: &str) {
        TuiPermissionUi.notify(message);
    }

    fn select_clearance(&self, info: &ClearanceInfo) -> ClearanceChoice {
        let mut info = info.clone();
        info.from = Some(self.via.clone());
        ask_clearance(self.id, info, Some(&self.cancelled))
    }
}

/// Decides the gated calls subagents ask about, with this session's own gate:
/// its config rules and session grants apply, and a call that needs a person
/// gets a card that names the subagent. Install it with
/// `hoocode_code_subagents::instance::set_clearance_relay`.
pub struct TuiClearanceRelay {
    cwd: PathBuf,
}

impl TuiClearanceRelay {
    pub fn new(cwd: PathBuf) -> Self {
        Self { cwd }
    }
}

impl ClearanceRelay for TuiClearanceRelay {
    fn ask(&self, request: &RelayRequest) -> Option<String> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let ui = RelayedUi {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            via: request.via.clone().unwrap_or_else(|| "subagent".into()),
            cancelled: request.cancelled.clone(),
        };
        HooPermissionGate::new(self.cwd.clone(), ApprovalChannel::Ui, Some(Arc::new(ui)))
            .check(&request.tool, &request.input)
    }
}

/// The `ask_options` tool's UI in interactive mode (`showAskOptions`). It
/// blocks the tool until the pane answers, and an abort takes the pane down.
pub struct TuiAskOptionsHost;

impl TuiAskOptionsHost {
    /// The MCP elicitation form: the same pane, without the AskUserQuestion chime.
    pub fn ask_form(&self, questions: &[AskQuestion]) -> Option<Vec<Option<String>>> {
        ask_pane(questions, None, false, false, &send)
    }
}

impl AskOptionsHost for TuiAskOptionsHost {
    fn has_ui(&self) -> bool {
        true
    }

    /// During `/loop auto` the tool answers from recommended options (or halts)
    /// without a pane.
    fn auto_loop_active(&self) -> bool {
        crate::auto_loop::is_active()
    }

    fn halt_loop(&self, reason: &str) {
        crate::auto_loop::halt(reason);
    }

    fn ask_options(
        &self,
        questions: &[AskQuestion],
        signal: Option<AbortSignal>,
    ) -> Option<Vec<Option<String>>> {
        ask_pane(questions, signal, true, true, &send)
    }
}

/// Shows the options pane through `send` and waits for the answer.
fn ask_pane(
    questions: &[AskQuestion],
    signal: Option<AbortSignal>,
    chime: bool,
    esc_answers_first: bool,
    send: &dyn Fn(DialogRequest) -> bool,
) -> Option<Vec<Option<String>>> {
    let aborted = || signal.as_ref().is_some_and(AbortSignal::aborted);
    if questions.is_empty() || aborted() {
        return None;
    }
    let (reply, answer) = mpsc::channel();
    let sent = send(DialogRequest::AskOptions {
        questions: questions.to_vec(),
        reply,
        chime,
        esc_answers_first,
    });
    if !sent {
        return None;
    }
    loop {
        match answer.recv_timeout(Duration::from_millis(50)) {
            Ok(answers) => return answers.map(|a| a.into_iter().map(Some).collect()),
            Err(mpsc::RecvTimeoutError::Disconnected) => return None,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if aborted() {
                    send(DialogRequest::HideAskOptions);
                    return None;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn question() -> AskQuestion {
        AskQuestion {
            question: "Which?".into(),
            short: None,
            detail: None,
            options: Vec::new(),
            allow_custom: true,
        }
    }

    #[test]
    fn only_the_ask_options_tool_chimes() {
        // The tool path passes `chime: true`; the elicitation path must not.
        let questions = [question()];
        let tool = Cell::new(None);
        let tool_esc = Cell::new(None);
        let _ = ask_pane(&questions, None, true, true, &|r| {
            if let DialogRequest::AskOptions {
                chime,
                esc_answers_first,
                ..
            } = r
            {
                tool.set(Some(chime));
                tool_esc.set(Some(esc_answers_first));
            }
            false
        });
        assert_eq!(tool.get(), Some(true));
        // The tool's card answers with option 1 on esc; elicitation skips.
        assert_eq!(tool_esc.get(), Some(true));

        let form = Cell::new(None);
        let form_esc = Cell::new(None);
        let _ = ask_pane(&questions, None, false, false, &|r| {
            if let DialogRequest::AskOptions {
                chime,
                esc_answers_first,
                ..
            } = r
            {
                form.set(Some(chime));
                form_esc.set(Some(esc_answers_first));
            }
            false
        });
        assert_eq!(form.get(), Some(false));
        assert_eq!(form_esc.get(), Some(false));
    }
}

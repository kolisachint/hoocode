//! How code off the UI thread asks the interactive mode something: the
//! permission gate's `select`/`notify` (the pin's `ctx.ui.select` /
//! `ctx.ui.notify`), answered by the selector dialog, and the `ask_options`
//! tool's questions (`ctx.ui.askOptions`), answered by the options pane.

use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};

use std::time::Duration;

use hoocode_ai_types::AbortSignal;
use hoocode_code_permissions::PermissionUi;
use hoocode_code_tools_optin::{AskOptionsHost, AskQuestion};

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
    AskOptions {
        questions: Vec<AskQuestion>,
        reply: mpsc::Sender<Option<Vec<String>>>,
        chime: bool,
    },
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
}

/// The `ask_options` tool's UI in interactive mode (`showAskOptions`). It
/// blocks the tool until the pane answers, and an abort takes the pane down.
pub struct TuiAskOptionsHost;

impl TuiAskOptionsHost {
    /// The MCP elicitation form: the same pane, without the AskUserQuestion chime.
    pub fn ask_form(&self, questions: &[AskQuestion]) -> Option<Vec<Option<String>>> {
        ask_pane(questions, None, false, &send)
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
        ask_pane(questions, signal, true, &send)
    }
}

/// Shows the options pane through `send` and waits for the answer.
fn ask_pane(
    questions: &[AskQuestion],
    signal: Option<AbortSignal>,
    chime: bool,
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
        let _ = ask_pane(&questions, None, true, &|r| {
            if let DialogRequest::AskOptions { chime, .. } = r {
                tool.set(Some(chime));
            }
            false
        });
        assert_eq!(tool.get(), Some(true));

        let form = Cell::new(None);
        let _ = ask_pane(&questions, None, false, &|r| {
            if let DialogRequest::AskOptions { chime, .. } = r {
                form.set(Some(chime));
            }
            false
        });
        assert_eq!(form.get(), Some(false));
    }
}

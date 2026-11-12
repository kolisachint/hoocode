//! Port of the pin's `test/interactive-mode-suspend.test.ts`
//! (`InteractiveMode.handleCtrlZ`) over a recording [`SuspendOps`].
//! hoocode's keep-alive interval has no counterpart: `kill` returns once the
//! process is continued, and the restore follows it. The SIGCONT watch covers
//! stops hoocode did not ask for (`resume_after_external_stop`).

use hoocode_code_tui_app::suspend::{
    clear_external_resume, note_sigcont, resume_after_external_stop, suspend_to_background,
    take_external_resume, SuspendOps, SuspendOutcome,
};

#[derive(Default)]
struct Recorder {
    calls: Vec<&'static str>,
    fail: bool,
}

impl SuspendOps for Recorder {
    fn stop_ui(&mut self) {
        self.calls.push("stop_ui");
    }
    fn restart_ui(&mut self) {
        self.calls.push("restart_ui");
    }
    fn ignore_sigint(&mut self) {
        self.calls.push("ignore_sigint");
    }
    fn restore_sigint(&mut self) {
        self.calls.push("restore_sigint");
    }
    fn stop_process_group(&mut self) -> Result<(), String> {
        self.calls.push("SIGTSTP");
        if self.fail {
            Err("suspend failed".into())
        } else {
            Ok(())
        }
    }
}

#[test]
fn shows_a_status_message_and_skips_suspend_on_windows() {
    let mut ops = Recorder::default();
    assert_eq!(
        suspend_to_background(&mut ops, true),
        SuspendOutcome::Unsupported("Suspend to background is not supported on Windows")
    );
    assert!(ops.calls.is_empty());
}

#[test]
fn ignores_sigint_while_suspended_and_restores_the_tui_on_resume() {
    let mut ops = Recorder::default();
    assert_eq!(
        suspend_to_background(&mut ops, false),
        SuspendOutcome::Resumed
    );
    assert_eq!(
        ops.calls,
        [
            "ignore_sigint",
            "stop_ui",
            "SIGTSTP",
            "restore_sigint",
            "restart_ui"
        ]
    );
}

#[test]
fn cleans_up_the_temporary_handlers_if_suspension_fails() {
    let mut ops = Recorder {
        fail: true,
        ..Default::default()
    };
    assert_eq!(
        suspend_to_background(&mut ops, false),
        SuspendOutcome::Failed("suspend failed".into())
    );
    assert_eq!(
        ops.calls,
        ["ignore_sigint", "stop_ui", "SIGTSTP", "restore_sigint"]
    );
}

/// After a stop hoocode did not ask for, the UI is torn down and started
/// again (Kitty flags popped and pushed once, raw mode back, full redraw);
/// SIGINT and the process group are left alone.
#[test]
fn an_external_resume_restarts_the_ui() {
    let mut ops = Recorder::default();
    resume_after_external_stop(&mut ops);
    assert_eq!(ops.calls, ["stop_ui", "restart_ui"]);
}

/// A SIGCONT hoocode did not cause is reported once to the UI loop; a
/// restart that already happened (the external editor's) can drop it.
#[test]
fn a_sigcont_from_outside_is_taken_once() {
    clear_external_resume();
    assert!(!take_external_resume(), "nothing pending at first");
    note_sigcont();
    assert!(take_external_resume(), "an outside SIGCONT is reported");
    assert!(!take_external_resume(), "and only once");
    note_sigcont();
    clear_external_resume();
    assert!(!take_external_resume(), "cleared by a restart");
}

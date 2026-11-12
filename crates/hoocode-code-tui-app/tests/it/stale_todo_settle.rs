//! The InteractiveMode half of the pin's
//! `suite/regressions/79-stale-todo-settle.test.ts`: what dangling plan rows
//! settle to when the request ends. hoocode settles on the prompt's
//! completion (`AppEvent::PromptDone`), after retries and continuations, so
//! the TS `settleRequestOnIdle` busy checks (pending retry, streaming,
//! compacting) have no separate idle hook to guard here.

use hoocode_ai_types::StopReason;
use hoocode_code_task_store::TaskStatus;
use hoocode_code_tui_app::mode::plan_settle_outcome;

#[test]
fn a_clean_stop_settles_to_done_an_abort_or_error_to_cancelled() {
    for (stop, expected) in [
        (StopReason::Stop, TaskStatus::Done),
        (StopReason::Aborted, TaskStatus::Cancelled),
        (StopReason::Error, TaskStatus::Cancelled),
        (StopReason::Length, TaskStatus::Cancelled),
    ] {
        assert_eq!(
            plan_settle_outcome(Some(stop), 0),
            Some(expected),
            "{stop:?}"
        );
    }
}

#[test]
fn does_not_settle_while_a_follow_up_or_steer_is_queued() {
    assert_eq!(plan_settle_outcome(Some(StopReason::Stop), 1), None);
}

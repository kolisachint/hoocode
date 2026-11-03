//! A prompt in flight (spawned, `PromptDone` not yet handled) is a busy agent:
//! the scheduler, the job wake, `/loop auto` and typed text wait on it, so no
//! second prompt races the first. Also the scheduled backlog and the job wake.
//!
//! The test agent has no model, so every prompt fails at preflight and sends
//! `PromptDone(Err)`: the tests feed that event to `on_prompt_done` the way the
//! UI loop does.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hoocode_code_agent_session::services::JobNotices;
use hoocode_code_scheduler::TaskStore;
use hoocode_code_tool_bash::{get_shell_env, JobRegistry, LocalBashOperations};

use super::prompt_queue::next_scheduled;
use super::test_support::test_mode_in;
use super::*;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "flight-{tag}-{}-{}",
        std::process::id(),
        hoocode_code_session::create_session_id()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir.canonicalize().expect("canonical temp dir")
}

/// Waits for the next app event the prompt task sends.
fn next_event(mode: &Mode) -> AppEvent {
    mode.rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the prompt task reports")
}

/// No further app event arrives soon (a second spawned prompt would send one).
fn no_more_events(mode: &Mode) -> bool {
    mode.rx.recv_timeout(Duration::from_millis(300)).is_err()
}

/// Handles the pending prompt's `PromptDone`, as the UI loop does.
fn finish_prompt(mode: &mut Mode) -> Result<(), String> {
    let AppEvent::PromptDone(result) = next_event(mode) else {
        panic!("expected PromptDone");
    };
    mode.on_prompt_done(result.clone(), &mut Vec::new());
    result
}

/// Makes the next `tick_scheduler` run its claim now.
fn tick_due(mode: &mut Mode) {
    mode.scheduler_tick_at = Instant::now();
    mode.tick_scheduler();
}

fn follow_ups(mode: &Mode) -> Vec<String> {
    mode.session.get_follow_up_messages()
}

fn notice_titles(mode: &Mode) -> Vec<String> {
    let panel = mode.notifications.borrow();
    panel
        .showing()
        .into_iter()
        .chain(panel.pending())
        .map(|n| n.title.clone())
        .collect()
}

/// A mode working in a fresh folder with one task due every minute.
fn mode_with_due_task(tag: &str) -> (Mode, TaskStore) {
    let dir = temp_dir(tag);
    let mode = test_mode_in(None, dir.clone());
    let store = TaskStore::for_cwd(&dir);
    store.create("* * * * *", "ping", true).expect("task");
    (mode, store)
}

#[test]
fn a_prompt_in_flight_blocks_the_scheduler_until_its_prompt_done() {
    let (mut mode, store) = mode_with_due_task("sched");
    assert!(!mode.agent_busy());
    mode.prompt("hello".into());
    assert!(mode.agent_busy(), "spawned, not streaming yet");
    assert!(!mode.session.is_streaming());

    tick_due(&mut mode);
    assert_eq!(
        store.list()[0].last_run_minute,
        None,
        "nothing is claimed while a prompt starts"
    );
    assert!(mode.scheduled_backlog.is_empty());

    assert!(finish_prompt(&mut mode).is_err(), "no model: preflight");
    assert!(!mode.agent_busy());
    tick_due(&mut mode);
    assert!(store.list()[0].last_run_minute.is_some(), "claimed now");
    // The fire went out as a prompt of its own.
    assert!(mode.scheduled_backlog.is_empty());
    assert!(mode.prompt_pending);
    assert!(follow_ups(&mode).is_empty());
    assert!(finish_prompt(&mut mode).is_err());
    assert!(no_more_events(&mode));
}

#[test]
fn a_second_prompt_while_one_is_in_flight_is_queued_not_raced() {
    let mut mode = test_mode_in(None, temp_dir("race"));
    mode.prompt("first".into());
    mode.prompt("second".into());
    assert_eq!(follow_ups(&mode), ["second"], "queued behind the first");
    // One prompt task, one PromptDone: the second cannot fail with "already
    // processing" and settle the first's turn.
    assert!(finish_prompt(&mut mode).is_err());
    assert!(no_more_events(&mode), "only one prompt was spawned");
    // The first never ran a turn, so the queued text goes back to the editor.
    assert!(follow_ups(&mode).is_empty());
    assert_eq!(mode.editor.borrow().editor.get_text(), "second");
}

#[test]
fn text_typed_while_a_prompt_starts_is_queued_behind_it() {
    let mut mode = test_mode_in(None, temp_dir("typed"));
    mode.prompt("first".into());
    mode.submit("typed meanwhile".into());
    assert_eq!(follow_ups(&mode), ["typed meanwhile"]);
    assert!(finish_prompt(&mut mode).is_err());
    assert!(no_more_events(&mode));
    assert_eq!(mode.editor.borrow().editor.get_text(), "typed meanwhile");
}

#[test]
fn a_prompt_task_that_ends_without_a_result_still_frees_the_agent() {
    let mut mode = test_mode_in(None, temp_dir("guard"));
    mode.prompt_pending = true;
    drop(PromptDoneGuard::new(mode.tx.clone()));
    let AppEvent::PromptDone(result) = next_event(&mode) else {
        panic!("expected PromptDone");
    };
    assert!(result.is_err());
    mode.on_prompt_done(result, &mut Vec::new());
    assert!(!mode.prompt_pending);
}

#[test]
fn scheduled_prompts_leave_the_backlog_oldest_first_and_only_when_free() {
    let mut backlog: VecDeque<String> = ["a", "b", "c"].map(String::from).into();
    assert_eq!(next_scheduled(&mut backlog, true), None);
    assert_eq!(backlog.len(), 3);
    assert_eq!(next_scheduled(&mut backlog, false).as_deref(), Some("a"));
    assert_eq!(next_scheduled(&mut backlog, false).as_deref(), Some("b"));
    assert_eq!(next_scheduled(&mut backlog, false).as_deref(), Some("c"));
    assert_eq!(next_scheduled(&mut backlog, false), None);
}

#[test]
fn a_first_fire_that_starts_no_turn_does_not_strand_the_rest() {
    // An extension command runs without a turn: its prompt ends at once, with
    // nothing streaming. Each fire still goes out after the one before it.
    let mut mode = test_mode_in(None, temp_dir("strand"));
    mode.scheduled_backlog = ["/ext-command", "plain one", "plain two"]
        .map(String::from)
        .into();
    mode.send_next_scheduled();
    assert!(mode.prompt_pending);
    assert_eq!(mode.scheduled_backlog.len(), 2, "one at a time");
    // Another pass while the first is in flight sends nothing.
    mode.send_next_scheduled();
    assert_eq!(mode.scheduled_backlog.len(), 2);
    finish_prompt(&mut mode).ok();
    assert!(mode.prompt_pending, "the second went out");
    assert_eq!(mode.scheduled_backlog.len(), 1);
    finish_prompt(&mut mode).ok();
    assert_eq!(mode.scheduled_backlog.len(), 0);
    finish_prompt(&mut mode).ok();
    assert!(!mode.agent_busy());
    assert!(follow_ups(&mode).is_empty(), "no follow-up queueing");
    assert!(no_more_events(&mode));
}

#[test]
fn every_fire_of_a_tick_reaches_the_backlog_in_order() {
    let dir = temp_dir("order");
    let mut mode = test_mode_in(None, dir.clone());
    let store = TaskStore::for_cwd(&dir);
    store.create("* * * * *", "one", true).unwrap();
    store.create("* * * * *", "two", true).unwrap();
    tick_due(&mut mode);
    // The first is sent, the second waits its turn.
    assert!(mode.prompt_pending);
    assert_eq!(mode.scheduled_backlog.len(), 1);
    assert!(mode.scheduled_backlog[0].contains("two"));
    assert!(finish_prompt(&mut mode).is_err());
    assert!(mode.scheduled_backlog.is_empty(), "the second went out");
    assert!(finish_prompt(&mut mode).is_err());
}

#[test]
fn a_session_switch_drops_the_backlog_with_a_notice() {
    let mut mode = test_mode_in(None, temp_dir("switch"));
    mode.scheduled_backlog = ["a", "b"].map(String::from).into();
    mode.drop_scheduled_backlog();
    assert!(mode.scheduled_backlog.is_empty());
    assert!(
        notice_titles(&mode)
            .iter()
            .any(|t| t == "2 scheduled prompt(s) dropped by the session switch"),
        "{:?}",
        notice_titles(&mode)
    );
    // Nothing queued, nothing said.
    let before = notice_titles(&mode).len();
    mode.drop_scheduled_backlog();
    assert_eq!(notice_titles(&mode).len(), before);
}

/// A mode whose session has one background job that finished on its own.
fn mode_with_finished_job(tag: &str) -> Mode {
    let dir = temp_dir(tag);
    let mode = test_mode_in(None, dir.clone());
    let jobs = JobRegistry::new(dir.join("jobs"), LocalBashOperations::new(None));
    let id = jobs.spawn("true", &dir, get_shell_env()).expect("spawn").id;
    let deadline = Instant::now() + Duration::from_secs(15);
    while jobs.info(&id).is_some_and(|i| i.state.is_running()) {
        assert!(Instant::now() < deadline, "job did not finish");
        std::thread::sleep(Duration::from_millis(10));
    }
    mode.session
        .set_job_notices(Arc::new(JobNotices::new(jobs)));
    mode
}

#[test]
fn the_job_wake_waits_for_a_prompt_in_flight_and_takes_nothing() {
    let mut mode = mode_with_finished_job("wake-busy");
    mode.prompt("hello".into());
    mode.tick_job_wake();
    assert!(follow_ups(&mode).is_empty(), "no wake queued or started");
    assert!(!mode.wake_in_flight);
    assert!(finish_prompt(&mut mode).is_err());
    assert!(no_more_events(&mode));
    // Idle: now it wakes the agent. The notices were not taken meanwhile.
    assert!(mode.session.job_wake_pending());
    mode.tick_job_wake();
    assert!(mode.prompt_pending && mode.wake_in_flight);
    assert!(finish_prompt(&mut mode).is_err());
}

#[test]
fn the_job_wake_is_not_the_users_prompt() {
    let mut mode = mode_with_finished_job("wake-quiet");
    mode.comms_lost.set(true);
    mode.tick_job_wake();
    assert!(mode.prompt_pending && mode.wake_in_flight);
    assert!(
        mode.transcript.last_prompt.is_none(),
        "an empty enter must not send the notice again"
    );
    assert!(mode.comms_lost.get(), "COMMS LOST stays");
    assert!(finish_prompt(&mut mode).is_err());
}

#[test]
fn a_wake_that_failed_before_a_model_call_loses_nothing_and_is_not_retried() {
    let mut mode = mode_with_finished_job("wake-failed");
    mode.tick_job_wake();
    assert!(finish_prompt(&mut mode).is_err(), "no model: preflight");
    assert!(mode.job_wake_failed);
    // Still due: the notices were never taken, the next model call carries them.
    assert!(mode.session.job_wake_pending());
    mode.tick_job_wake();
    assert!(!mode.prompt_pending, "no retry loop");
    assert!(no_more_events(&mode));
}

#[test]
fn the_job_wake_waits_while_the_user_types_or_a_loop_runs() {
    let mut mode = mode_with_finished_job("wake-typing");
    mode.editor.borrow_mut().editor.set_text("half a thought");
    mode.tick_job_wake();
    assert!(!mode.prompt_pending, "the user is typing");
    mode.editor.borrow_mut().editor.set_text("");

    let (state, _first) = hoocode_code_scheduler::loop_command::AutoLoop::start("task", 3);
    mode.auto_loop = Some(loop_command::LoopRun { state, next: None });
    mode.tick_job_wake();
    assert!(!mode.prompt_pending, "/loop auto is running");
    mode.auto_loop = None;
    assert!(mode.session.job_wake_pending());
    mode.tick_job_wake();
    assert!(mode.prompt_pending);
    assert!(finish_prompt(&mut mode).is_err());
}

#[test]
fn a_prompt_that_ends_without_a_turn_runs_what_was_queued_behind_it() {
    // An extension command (e.g. `/grill`) ends at once with no turn, and may
    // send its own follow-up while the prompt still shows as in flight. That
    // follow-up must run, not bounce back to the editor.
    let mut mode = test_mode_in(None, temp_dir("gap-ok"));
    mode.prompt("first".into());
    // Queued behind the prompt while it starts, as `/grill`'s follow-up is.
    mode.prompt("grill message".into());
    assert_eq!(follow_ups(&mode), ["grill message"]);
    // The spawned task's real result is replaced: the extension path ends
    // `Ok` with no turn.
    let AppEvent::PromptDone(_) = next_event(&mode) else {
        panic!("expected PromptDone");
    };
    mode.on_prompt_done(Ok(()), &mut Vec::new());
    assert!(mode.prompt_pending, "the queued text started a turn");
    assert!(follow_ups(&mode).is_empty());
    assert!(
        mode.editor.borrow().editor.get_text().is_empty(),
        "nothing bounced to the editor"
    );
    // The started prompt still fails preflight, as every test prompt does.
    assert!(finish_prompt(&mut mode).is_err());
    assert!(no_more_events(&mode));
}

#[test]
fn a_wake_ended_before_its_first_model_call_does_not_rewake_at_once() {
    // Esc before the wake's first model call ends `Ok` with the notices
    // still pending: no immediate re-wake, the next call carries them.
    let mut mode = mode_with_finished_job("wake-abort");
    mode.tick_job_wake();
    assert!(mode.prompt_pending && mode.wake_in_flight);
    // Aborted before any model call: Ok, notices still pending. Drain the
    // spawned task's real result first; the abort replaces it.
    let AppEvent::PromptDone(_) = next_event(&mode) else {
        panic!("expected PromptDone");
    };
    mode.on_prompt_done(Ok(()), &mut Vec::new());
    assert!(mode.session.job_wake_pending());
    assert!(mode.job_wake_failed, "no immediate re-wake");
    mode.tick_job_wake();
    assert!(!mode.prompt_pending);
    assert!(no_more_events(&mode));
}

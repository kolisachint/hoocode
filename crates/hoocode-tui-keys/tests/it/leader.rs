//! Leader state machine: pending, timeout, Esc, a second leader, an unbound
//! key, swallowed keys, and printable leaders. Time is passed in as an
//! `Instant`, so nothing sleeps.

use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use hoocode_tui_keys::{
    set_kitty_protocol_active, CancelReason, LeaderError, LeaderKey, LeaderMachine, LeaderOutcome,
    SequenceError, DEFAULT_LEADER_TIMEOUT,
};

/// The Kitty flag is a process global. Every test here runs in legacy mode.
static GLOBALS: Mutex<()> = Mutex::new(());

fn legacy() -> MutexGuard<'static, ()> {
    let guard = GLOBALS.lock().unwrap_or_else(|e| e.into_inner());
    set_kitty_protocol_active(false);
    guard
}

const CTRL_X: &str = "\x18";

fn machine() -> (LeaderMachine, Instant) {
    let leader = LeaderKey::new("ctrl+x").expect("ctrl+x is a valid leader");
    let mut m = LeaderMachine::new(leader, DEFAULT_LEADER_TIMEOUT);
    m.bind("ctrl+x m", "app.model.cycleForward").unwrap();
    m.bind("ctrl+x M", "app.model.cycleBackward").unwrap();
    m.bind("ctrl+x g t", "app.test.threeSteps").unwrap();
    m.bind("ctrl+x enter", "app.newline").unwrap();
    (m, Instant::now())
}

fn ms(t0: Instant, millis: u64) -> Instant {
    t0 + Duration::from_millis(millis)
}

#[test]
fn pending_since_is_the_time_the_leader_was_pressed() {
    let _g = legacy();
    let (mut m, t0) = machine();
    assert_eq!(m.pending_since(), None);
    m.handle(t0, CTRL_X);
    assert_eq!(m.pending_since(), Some(t0));
    // A key that extends the sequence keeps the original start.
    m.handle(ms(t0, 100), "g");
    assert_eq!(m.pending_since(), Some(t0));
    assert_eq!(
        m.deadline(),
        Some(ms(t0, 100) + DEFAULT_LEADER_TIMEOUT),
        "the timeout restarts at each key"
    );
}

#[test]
fn default_timeout_is_1500_ms() {
    assert_eq!(DEFAULT_LEADER_TIMEOUT, Duration::from_millis(1500));
}

#[test]
fn leader_enters_pending_and_is_swallowed() {
    let _g = legacy();
    let (mut m, t0) = machine();
    assert_eq!(m.handle(t0, CTRL_X), LeaderOutcome::Pending);
    assert!(m.is_pending());
    assert_eq!(m.pending_depth(), 1);
    assert_eq!(m.deadline(), Some(ms(t0, 1500)));
}

#[test]
fn a_bound_sequence_runs_its_action() {
    let _g = legacy();
    let (mut m, t0) = machine();
    m.handle(t0, CTRL_X);
    assert_eq!(
        m.handle(ms(t0, 10), "m"),
        LeaderOutcome::Run("app.model.cycleForward".into())
    );
    assert!(!m.is_pending(), "a finished sequence leaves pending");
}

#[test]
fn a_shifted_letter_step_matches_only_the_capital() {
    let _g = legacy();
    let (mut m, t0) = machine();
    m.handle(t0, CTRL_X);
    assert_eq!(
        m.handle(ms(t0, 10), "M"),
        LeaderOutcome::Run("app.model.cycleBackward".into())
    );
    m.handle(ms(t0, 20), CTRL_X);
    assert_eq!(
        m.handle(ms(t0, 30), "m"),
        LeaderOutcome::Run("app.model.cycleForward".into())
    );
}

#[test]
fn a_three_step_sequence_stays_pending_until_its_last_step() {
    let _g = legacy();
    let (mut m, t0) = machine();
    m.handle(t0, CTRL_X);
    assert_eq!(m.handle(ms(t0, 10), "g"), LeaderOutcome::Pending);
    assert_eq!(m.pending_depth(), 2);
    assert_eq!(
        m.handle(ms(t0, 20), "t"),
        LeaderOutcome::Run("app.test.threeSteps".into())
    );
}

#[test]
fn enter_is_a_valid_second_step() {
    let _g = legacy();
    let (mut m, t0) = machine();
    m.handle(t0, CTRL_X);
    assert_eq!(
        m.handle(ms(t0, 10), "\r"),
        LeaderOutcome::Run("app.newline".into())
    );
}

#[test]
fn each_step_gets_the_full_timeout() {
    let _g = legacy();
    let (mut m, t0) = machine();
    m.handle(t0, CTRL_X);
    assert_eq!(m.handle(ms(t0, 1400), "g"), LeaderOutcome::Pending);
    // The second step at 1400 ms restarted the clock. At 2800 ms the
    // sequence is past the first deadline (1500 ms) but still inside the second.
    assert_eq!(m.poll(ms(t0, 2800)), None);
    assert_eq!(
        m.handle(ms(t0, 2800), "t"),
        LeaderOutcome::Run("app.test.threeSteps".into())
    );
}

#[test]
fn timeout_cancels_on_poll_and_only_reports_once() {
    let _g = legacy();
    let (mut m, t0) = machine();
    m.handle(t0, CTRL_X);
    assert_eq!(m.poll(ms(t0, 1499)), None, "one ms early still pending");
    assert!(m.is_pending());
    assert_eq!(m.poll(ms(t0, 1500)), Some(CancelReason::Timeout));
    assert!(!m.is_pending());
    assert_eq!(m.poll(ms(t0, 1600)), None, "reported once");
}

#[test]
fn a_key_after_the_timeout_is_a_fresh_key() {
    let _g = legacy();
    let (mut m, t0) = machine();
    m.handle(t0, CTRL_X);
    // "m" arrives after the timeout. It is no longer part of the sequence,
    // so the editor gets it.
    assert_eq!(m.handle(ms(t0, 2000), "m"), LeaderOutcome::Unhandled);
    assert!(!m.is_pending());
}

#[test]
fn a_custom_timeout_is_used() {
    let _g = legacy();
    let leader = LeaderKey::new("ctrl+x").unwrap();
    let mut m = LeaderMachine::new(leader, Duration::from_millis(200));
    m.bind("ctrl+x m", "a").unwrap();
    let t0 = Instant::now();
    m.handle(t0, CTRL_X);
    assert_eq!(m.poll(ms(t0, 199)), None);
    assert_eq!(m.poll(ms(t0, 200)), Some(CancelReason::Timeout));
}

#[test]
fn esc_cancels_and_is_swallowed() {
    let _g = legacy();
    let (mut m, t0) = machine();
    m.handle(t0, CTRL_X);
    assert_eq!(
        m.handle(ms(t0, 10), "\x1b"),
        LeaderOutcome::Cancelled(CancelReason::Escape)
    );
    assert!(!m.is_pending());
}

#[test]
fn a_second_leader_cancels_without_starting_again() {
    let _g = legacy();
    let (mut m, t0) = machine();
    m.handle(t0, CTRL_X);
    assert_eq!(
        m.handle(ms(t0, 10), CTRL_X),
        LeaderOutcome::Cancelled(CancelReason::LeaderAgain)
    );
    assert!(
        !m.is_pending(),
        "the second leader cancels, it does not re-arm"
    );
}

#[test]
fn an_unbound_next_key_cancels_with_no_binding() {
    let _g = legacy();
    let (mut m, t0) = machine();
    m.handle(t0, CTRL_X);
    assert_eq!(
        m.handle(ms(t0, 10), "q"),
        LeaderOutcome::Cancelled(CancelReason::Unbound)
    );
    assert!(!m.is_pending());
}

#[test]
fn an_unbound_step_deep_in_a_sequence_cancels() {
    let _g = legacy();
    let (mut m, t0) = machine();
    m.handle(t0, CTRL_X);
    m.handle(ms(t0, 10), "g");
    assert_eq!(
        m.handle(ms(t0, 20), "x"),
        LeaderOutcome::Cancelled(CancelReason::Unbound),
        "ctrl+x g x has no binding"
    );
}

#[test]
fn a_pending_key_never_reaches_the_editor() {
    let _g = legacy();
    let (mut m, t0) = machine();
    // Every key in pending is Pending, Run, or Cancelled. None is Unhandled.
    assert_ne!(m.handle(t0, CTRL_X), LeaderOutcome::Unhandled);
    assert_ne!(m.handle(ms(t0, 10), "m"), LeaderOutcome::Unhandled);

    m.handle(ms(t0, 20), CTRL_X);
    assert_ne!(m.handle(ms(t0, 30), "z"), LeaderOutcome::Unhandled);

    m.handle(ms(t0, 40), CTRL_X);
    assert_ne!(m.handle(ms(t0, 50), "\x1b"), LeaderOutcome::Unhandled);
}

#[test]
fn keys_outside_a_sequence_pass_to_the_editor() {
    let _g = legacy();
    let (mut m, t0) = machine();
    assert_eq!(m.handle(t0, "a"), LeaderOutcome::Unhandled);
    assert_eq!(
        m.handle(t0, "\x1b"),
        LeaderOutcome::Unhandled,
        "idle Esc is not swallowed"
    );
    assert!(!m.is_pending());
}

#[test]
fn a_leader_with_no_bindings_still_enters_pending_and_reports_no_binding() {
    let _g = legacy();
    let leader = LeaderKey::new("ctrl+x").unwrap();
    let mut m = LeaderMachine::new(leader, DEFAULT_LEADER_TIMEOUT);
    let t0 = Instant::now();
    assert_eq!(m.handle(t0, CTRL_X), LeaderOutcome::Pending);
    assert_eq!(
        m.handle(ms(t0, 1), "m"),
        LeaderOutcome::Cancelled(CancelReason::Unbound)
    );
}

#[test]
fn rebinding_a_sequence_replaces_its_action() {
    let _g = legacy();
    let (mut m, t0) = machine();
    m.bind("ctrl+x m", "app.other").unwrap();
    m.handle(t0, CTRL_X);
    assert_eq!(
        m.handle(ms(t0, 10), "m"),
        LeaderOutcome::Run("app.other".into())
    );
}

#[test]
fn printable_leaders_are_rejected() {
    for key in ["m", "/", "?", "1", "shift+a", "space", "A"] {
        assert!(
            matches!(LeaderKey::new(key), Err(LeaderError::Printable(_))),
            "{key} should be rejected as a leader"
        );
    }
}

#[test]
fn named_and_modified_leaders_are_accepted() {
    for key in ["ctrl+x", "ctrl+g", "alt+x", "enter", "tab", "ctrl+space"] {
        assert!(LeaderKey::new(key).is_ok(), "{key} should be accepted");
    }
}

#[test]
fn an_empty_leader_is_not_a_key() {
    assert!(matches!(LeaderKey::new(""), Err(LeaderError::NotAKey(_))));
    assert!(matches!(
        LeaderKey::new("ctrl+"),
        Err(LeaderError::NotAKey(_))
    ));
}

#[test]
fn a_binding_must_start_with_the_leader() {
    let (mut m, _) = machine();
    assert!(matches!(
        m.bind("alt+m", "a"),
        Err(SequenceError::NoLeader(_))
    ));
}

#[test]
fn a_binding_needs_a_key_after_the_leader() {
    let (mut m, _) = machine();
    assert!(matches!(
        m.bind("ctrl+x", "a"),
        Err(SequenceError::LeaderOnly(_))
    ));
}

#[test]
fn esc_and_the_leader_cannot_be_a_later_step() {
    let (mut m, _) = machine();
    assert!(matches!(
        m.bind("ctrl+x escape", "a"),
        Err(SequenceError::ReservedStep(_))
    ));
    assert!(matches!(
        m.bind("ctrl+x ctrl+x", "a"),
        Err(SequenceError::ReservedStep(_))
    ));
}

#[test]
fn a_bad_step_is_an_error_not_a_panic() {
    let (mut m, _) = machine();
    assert!(matches!(
        m.bind("ctrl+x ctrl+", "a"),
        Err(SequenceError::BadStep(_))
    ));
    assert!(matches!(m.bind("   ", "a"), Err(SequenceError::Empty)));
}

#[test]
fn reset_ends_a_pending_sequence_without_a_report() {
    let _g = legacy();
    let (mut m, t0) = machine();
    m.handle(t0, CTRL_X);
    m.reset();
    assert!(!m.is_pending());
    assert_eq!(m.poll(ms(t0, 5000)), None);
}

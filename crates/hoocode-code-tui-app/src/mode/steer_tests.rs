//! The keys while a turn runs: Enter steers it, leader `f` (Ctrl+X f) queues a
//! follow-up for after it, and Up on an empty prompt or leader `d` takes the last
//! queued message back, whichever kind it was.
//!
//! The turn is a faux model that answers only when the test opens its gate, so
//! the session stays streaming while the keys are pressed.

use std::sync::Arc;
use std::time::{Duration, Instant};

use hoocode_code_agent_session::PromptOptions;
use hoocode_code_tui_keybindings::AppKeybindingsManager;
use hoocode_tui_render::Component;

use super::input::Action;
use super::leader::LeaderState;
use super::test_support::streaming_mode;
use super::*;

const CTRL_X: &str = "\x18";
const ENTER: &str = "\r";
const UP: &str = "\x1b[A";

/// A mode with one turn running and the default keymap.
fn running() -> Mode {
    AppKeybindingsManager::default().install();
    let mut mode = streaming_mode(Arc::new(tokio::sync::Notify::new()));
    mode.leader = LeaderState::new(&AppKeybindingsManager::default());
    let session = mode.session.clone();
    mode.runtime.spawn(async move {
        let _ = session.prompt("start", PromptOptions::default()).await;
    });
    wait_for(|| mode.session.is_streaming());
    mode
}

/// Polls until `ready` holds. The turn and the queue fill on the runtime, so a
/// test waits for them rather than assuming an order.
fn wait_for(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready() {
        assert!(
            Instant::now() < deadline,
            "the run or the queue never filled"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Sends one key the way the screen does: the leader first, then the prompt
/// editor, then the actions the editor queued, each handled in order.
fn press(mode: &mut Mode, data: &str) {
    if !mode.leader_takes_key_at(data, Instant::now()) {
        mode.editor.borrow_mut().render(80);
        mode.editor.borrow_mut().handle_input(data);
    }
    let actions: Vec<Action> = mode.actions.borrow_mut().drain(..).collect();
    for action in actions {
        mode.handle_action(action);
    }
}

fn type_text(mode: &mut Mode, text: &str) {
    for c in text.chars() {
        press(mode, &c.to_string());
    }
}

fn prompt_text(mode: &Mode) -> String {
    mode.editor.borrow().editor.get_text()
}

/// The queued `(steering, follow-up)` messages.
fn queues(mode: &Mode) -> (Vec<String>, Vec<String>) {
    (
        mode.session.get_steering_messages(),
        mode.session.get_follow_up_messages(),
    )
}

/// Waits until the agent holds a queued message too, which is when a take-back
/// can find it. Then refreshes the queue display, as the screen does on the
/// session's `QueueUpdate` (no event loop runs here), and reads the queues.
fn queued(mode: &mut Mode) -> (Vec<String>, Vec<String>) {
    wait_for(|| mode.session.agent().has_queued_messages());
    mode.update_pending_messages_display();
    queues(mode)
}

#[test]
fn enter_while_a_turn_runs_steers_it() {
    let mut mode = running();
    type_text(&mut mode, "change course");
    press(&mut mode, ENTER);
    assert_eq!(
        queued(&mut mode),
        (vec!["change course".to_string()], vec![])
    );
    assert_eq!(prompt_text(&mode), "");
}

#[test]
fn leader_f_while_a_turn_runs_queues_a_follow_up() {
    let mut mode = running();
    type_text(&mut mode, "after this");
    press(&mut mode, CTRL_X);
    press(&mut mode, "f");
    assert_eq!(queued(&mut mode), (vec![], vec!["after this".to_string()]));
    assert_eq!(prompt_text(&mode), "");
}

#[test]
fn up_on_an_empty_prompt_takes_back_a_steer() {
    let mut mode = running();
    type_text(&mut mode, "steer");
    press(&mut mode, ENTER);
    queued(&mut mode);
    press(&mut mode, UP);
    assert_eq!(prompt_text(&mode), "steer");
    assert_eq!(queues(&mode), (vec![], vec![]));
}

#[test]
fn up_on_an_empty_prompt_takes_back_a_follow_up() {
    let mut mode = running();
    type_text(&mut mode, "later");
    press(&mut mode, CTRL_X);
    press(&mut mode, "f");
    queued(&mut mode);
    press(&mut mode, UP);
    assert_eq!(prompt_text(&mode), "later");
    assert_eq!(queues(&mode), (vec![], vec![]));
}

#[test]
fn leader_d_takes_back_a_steer() {
    let mut mode = running();
    type_text(&mut mode, "steer");
    press(&mut mode, ENTER);
    queued(&mut mode);
    press(&mut mode, CTRL_X);
    press(&mut mode, "d");
    assert_eq!(prompt_text(&mode), "steer");
    assert_eq!(queues(&mode), (vec![], vec![]));
}

#[test]
fn leader_d_takes_back_a_follow_up() {
    let mut mode = running();
    type_text(&mut mode, "later");
    press(&mut mode, CTRL_X);
    press(&mut mode, "f");
    queued(&mut mode);
    press(&mut mode, CTRL_X);
    press(&mut mode, "d");
    assert_eq!(prompt_text(&mode), "later");
    assert_eq!(queues(&mode), (vec![], vec![]));
}

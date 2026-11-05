//! The leader at the prompt, through the Mode's key path: a pending leader
//! runs its action, swallows the key after it, and cancels cleanly. The
//! keymap comes from a file's text, so the tests do not read the user's own.

use std::time::{Duration, Instant};

use hoocode_code_tui_keybindings::AppKeybindingsManager;

use super::leader::LeaderState;
use super::test_support::test_mode;
use super::*;

const KEYMAP: &str = r#"{"bindings": [{"context": "Prompt", "bindings": {
    "ctrl+x m": "app.model.cycleForward",
    "ctrl+x b": "app.thinking.toggle"
}}]}"#;

const CTRL_X: &str = "\x18";
const ESC: &str = "\x1b";

/// A mode whose leader has the test keymap.
fn leader_mode() -> Mode {
    let mut mode = test_mode(None);
    let keys = AppKeybindingsManager::from_json_str(KEYMAP);
    mode.leader = LeaderState::new(&keys);
    mode
}

/// Sends one key press that arrived at `now`, through the leader and then the screen.
fn press(mode: &mut Mode, data: &str, now: Instant) {
    if !mode.leader_takes_key_at(data, now) {
        mode.tui
            .accept_event(TuiEvent::Input(data.to_string(), now));
    }
}

fn queued(mode: &Mode) -> Vec<Action> {
    mode.actions.borrow().clone()
}

fn prompt_text(mode: &Mode) -> String {
    mode.editor.borrow().editor.get_text()
}

fn footer(mode: &Mode) -> Vec<String> {
    mode.transient.borrow().lines(80)
}

#[test]
fn a_leader_then_a_bound_key_runs_its_action() {
    let mut mode = leader_mode();
    let t0 = Instant::now();
    press(&mut mode, CTRL_X, t0);
    assert!(mode.leader.is_pending());
    press(&mut mode, "m", t0);
    assert!(!mode.leader.is_pending());
    assert_eq!(queued(&mode), vec![Action::ModelForward]);
    assert_eq!(
        prompt_text(&mode),
        "",
        "the bound key never reaches the prompt"
    );
}

#[test]
fn the_keys_of_a_leader_sequence_are_swallowed() {
    let mut mode = leader_mode();
    let t0 = Instant::now();
    press(&mut mode, CTRL_X, t0);
    press(&mut mode, "b", t0);
    assert_eq!(queued(&mode), vec![Action::ThinkingToggle]);
    assert_eq!(prompt_text(&mode), "");
}

#[test]
fn esc_cancels_the_leader_without_interrupting_the_turn() {
    let mut mode = leader_mode();
    let t0 = Instant::now();
    press(&mut mode, CTRL_X, t0);
    press(&mut mode, ESC, t0);
    assert!(!mode.leader.is_pending());
    assert!(
        !queued(&mode).contains(&Action::Interrupt),
        "the Esc that cancels a leader must not interrupt the turn"
    );
    assert!(queued(&mode).is_empty());
}

#[test]
fn a_second_leader_key_cancels_without_running_anything() {
    let mut mode = leader_mode();
    let t0 = Instant::now();
    press(&mut mode, CTRL_X, t0);
    press(&mut mode, CTRL_X, t0);
    assert!(!mode.leader.is_pending());
    assert!(queued(&mode).is_empty());
    assert_eq!(prompt_text(&mode), "");
}

#[test]
fn an_unbound_key_cancels_with_a_no_binding_note_and_is_swallowed() {
    let mut mode = leader_mode();
    let t0 = Instant::now();
    press(&mut mode, CTRL_X, t0);
    press(&mut mode, "q", t0);
    assert!(!mode.leader.is_pending());
    assert!(queued(&mode).is_empty());
    assert_eq!(prompt_text(&mode), "", "the unbound key is swallowed");
    mode.poll_leader_at(t0);
    let rows = footer(&mode);
    assert!(
        rows.iter().any(|row| row.contains("Ctrl+X … no binding")),
        "{rows:?}"
    );
}

#[test]
fn the_timeout_ends_the_leader_and_the_next_key_is_an_ordinary_key() {
    let mut mode = leader_mode();
    let t0 = Instant::now();
    press(&mut mode, CTRL_X, t0);
    // The loop's pass after the timeout ends the pending state, with no key press.
    mode.poll_leader_at(t0 + Duration::from_millis(2000));
    assert!(!mode.leader.is_pending());
    assert!(footer(&mode).iter().all(|row| !row.contains("Ctrl+X")));
    press(&mut mode, "m", t0 + Duration::from_millis(2000));
    assert!(
        queued(&mode).is_empty(),
        "after the timeout, m is typed text"
    );
    assert_eq!(prompt_text(&mode), "m");
}

#[test]
fn the_footer_shows_the_configured_leader_while_pending() {
    let mut mode = leader_mode();
    let t0 = Instant::now();
    press(&mut mode, CTRL_X, t0);
    mode.poll_leader_at(t0);
    assert!(
        footer(&mode).iter().any(|row| row.contains("Ctrl+X …")),
        "{:?}",
        footer(&mode)
    );
}

#[test]
fn the_which_key_popup_waits_for_its_delay_then_lists_the_keymap() {
    let mut mode = leader_mode();
    let t0 = Instant::now();
    press(&mut mode, CTRL_X, t0);
    mode.poll_leader_at(t0 + Duration::from_millis(399));
    assert!(
        footer(&mode)
            .iter()
            .all(|row| !row.contains("Cycle to next model")),
        "the popup waits 400 ms"
    );
    mode.poll_leader_at(t0 + Duration::from_millis(400));
    let rows = footer(&mode);
    assert!(
        rows.iter().any(|row| row.contains("Cycle to next model")),
        "{rows:?}"
    );
    assert!(
        rows.iter()
            .any(|row| row.contains("Toggle thinking blocks")),
        "{rows:?}"
    );
}

/// A mode whose leader has the built-in defaults (no keybindings file).
fn default_mode() -> Mode {
    let mut mode = test_mode(None);
    mode.leader = LeaderState::new(&AppKeybindingsManager::default());
    mode
}

#[test]
fn the_default_keymap_runs_its_leader_actions() {
    let mut mode = default_mode();
    let t0 = Instant::now();
    press(&mut mode, CTRL_X, t0);
    press(&mut mode, "m", t0);
    assert_eq!(queued(&mode), vec![Action::ModelForward]);
    press(&mut mode, CTRL_X, t0);
    press(&mut mode, "M", t0);
    assert_eq!(
        queued(&mode),
        vec![Action::ModelForward, Action::ModelBackward]
    );
}

#[test]
fn ctrl_x_g_is_the_task_panel_show_or_hide() {
    // K-4: the task panel's show or hide is leader g (it was Ctrl+G).
    let mut mode = default_mode();
    let t0 = Instant::now();
    press(&mut mode, CTRL_X, t0);
    press(&mut mode, "g", t0);
    assert_eq!(queued(&mode), vec![Action::PanelToggle]);
}

#[test]
fn ctrl_x_then_enter_inserts_a_newline_in_the_prompt() {
    let mut mode = default_mode();
    let t0 = Instant::now();
    press(&mut mode, "a", t0);
    press(&mut mode, CTRL_X, t0);
    press(&mut mode, "\r", t0);
    assert_eq!(
        prompt_text(&mode),
        "a\n",
        "the leader's enter is the newline"
    );
    assert!(queued(&mode).is_empty());
}

#[test]
fn ctrl_x_slash_is_swallowed_and_opens_the_transcript_search() {
    let mut mode = default_mode();
    let t0 = Instant::now();
    press(&mut mode, CTRL_X, t0);
    press(&mut mode, "/", t0);
    assert!(!mode.leader.is_pending());
    assert_eq!(prompt_text(&mode), "", "/ is not typed into the prompt");
    assert!(queued(&mode).is_empty());
}

#[test]
fn ctrl_r_opens_the_history_search_and_types_nothing() {
    // K-7: Ctrl+R is the prompt-history search. Transcript search is leader `/`.
    let mut mode = default_mode();
    let t0 = Instant::now();
    press(&mut mode, "a", t0);
    press(&mut mode, "\x12", t0);
    assert_eq!(queued(&mode), vec![Action::HistorySearchOpen]);
    assert_eq!(
        prompt_text(&mode),
        "a",
        "Ctrl+R types nothing into the prompt"
    );
}

#[test]
fn a_key_that_is_a_release_is_not_a_leader_press() {
    let mut mode = leader_mode();
    let t0 = Instant::now();
    // Kitty release report for ctrl+x (event type 3).
    assert!(!mode.leader_takes_key_at("\x1b[120;5:3u", t0));
    assert!(!mode.leader.is_pending());
}

//! K-7 at the prompt: the Ctrl+R history search, and the queue keys that take
//! the last queued message back (Up on an empty prompt, leader `d`).
//!
//! Keys go through the prompt editor and then the actions it queues, the same
//! route the screen takes. A streaming turn cannot run in a test (the test agent
//! has no model), so Enter-while-running is covered by the follow-up queue it uses.

use hoocode_code_agent_session::StreamingBehavior;
use hoocode_code_tui_keybindings::AppKeybindingsManager;
use hoocode_tui_render::Component;

use super::history_search::{find_match, HistorySearch};
use super::input::Action;
use super::test_support::test_mode;
use super::*;

const CTRL_R: &str = "\x12";
const ENTER: &str = "\r";
const ESC: &str = "\x1b";
const UP: &str = "\x1b[A";
const BACKSPACE: &str = "\x7f";

/// A mode whose prompt history is, newest first: "alpha three", "beta two", "alpha one".
fn mode_with_history() -> Mode {
    AppKeybindingsManager::default().install();
    let mode = test_mode(None);
    for text in ["alpha one", "beta two", "alpha three"] {
        mode.editor.borrow_mut().editor.add_to_history(text);
    }
    mode
}

/// Sends one key the way the screen does: the prompt editor, then the actions it
/// queued, each handled in order.
fn press(mode: &mut Mode, data: &str) {
    mode.editor.borrow_mut().render(80);
    mode.editor.borrow_mut().handle_input(data);
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

/// The history entry the open search shows, if any.
fn shown(mode: &Mode) -> Option<String> {
    let editor = mode.editor.borrow();
    mode.history_search
        .as_ref()
        .and_then(|search| search.matched(editor.editor.history()).map(str::to_string))
}

fn footer(mode: &Mode) -> String {
    mode.transient.borrow().lines(80).join("\n")
}

#[test]
fn matching_is_newest_first_and_ignores_case() {
    let history: Vec<String> = ["alpha three", "beta two", "alpha one"]
        .map(String::from)
        .to_vec();
    assert_eq!(find_match(&history, "ALPHA", 0), Some(0));
    assert_eq!(
        find_match(&history, "alpha", 1),
        Some(2),
        "from the next older"
    );
    assert_eq!(find_match(&history, "gamma", 0), None);
    assert_eq!(
        find_match(&history, "", 0),
        Some(0),
        "empty matches the newest"
    );
    assert_eq!(find_match(&[], "", 0), None);
}

#[test]
fn an_empty_query_shows_the_newest_prompt_and_older_steps_back() {
    let history: Vec<String> = ["b", "a"].map(String::from).to_vec();
    let mut search = HistorySearch::start(&history);
    assert_eq!(search.matched(&history), Some("b"));
    search.older(&history);
    assert_eq!(search.matched(&history), Some("a"));
    search.older(&history);
    assert_eq!(search.matched(&history), Some("a"), "the oldest stays");
}

#[test]
fn ctrl_r_opens_the_search_and_typing_narrows_to_the_newest_match() {
    let mut mode = mode_with_history();
    press(&mut mode, CTRL_R);
    assert!(mode.history_search.is_some());
    assert_eq!(shown(&mode).as_deref(), Some("alpha three"));
    type_text(&mut mode, "beta");
    assert_eq!(shown(&mode).as_deref(), Some("beta two"));
    press(&mut mode, BACKSPACE);
    press(&mut mode, BACKSPACE);
    press(&mut mode, BACKSPACE);
    press(&mut mode, BACKSPACE);
    assert_eq!(shown(&mode).as_deref(), Some("alpha three"), "empty again");
}

#[test]
fn a_query_with_no_match_says_so_and_enter_does_nothing() {
    let mut mode = mode_with_history();
    press(&mut mode, CTRL_R);
    type_text(&mut mode, "zzz");
    assert_eq!(shown(&mode), None);
    assert!(footer(&mode).contains("no match"));
    press(&mut mode, ENTER);
    assert!(mode.history_search.is_none(), "Enter closes the search");
    assert_eq!(prompt_text(&mode), "", "nothing was put in the prompt");
}

#[test]
fn ctrl_r_again_goes_to_the_next_older_match_and_stops_at_the_oldest() {
    let mut mode = mode_with_history();
    press(&mut mode, CTRL_R);
    type_text(&mut mode, "alpha");
    assert_eq!(shown(&mode).as_deref(), Some("alpha three"));
    press(&mut mode, CTRL_R);
    assert_eq!(shown(&mode).as_deref(), Some("alpha one"));
    press(&mut mode, CTRL_R);
    assert_eq!(shown(&mode).as_deref(), Some("alpha one"), "no older match");
}

#[test]
fn enter_puts_the_match_in_the_prompt_for_editing_and_does_not_submit() {
    let mut mode = mode_with_history();
    press(&mut mode, CTRL_R);
    type_text(&mut mode, "beta");
    press(&mut mode, ENTER);
    assert!(mode.history_search.is_none());
    assert_eq!(prompt_text(&mode), "beta two");
    assert!(!mode.prompt_pending, "not submitted: no turn started");
    assert!(mode.session.get_follow_up_messages().is_empty());
}

#[test]
fn escape_cancels_and_leaves_the_prompt_as_it_was() {
    let mut mode = mode_with_history();
    mode.editor.borrow_mut().editor.set_text("draft");
    press(&mut mode, CTRL_R);
    type_text(&mut mode, "beta");
    press(&mut mode, ESC);
    assert!(mode.history_search.is_none());
    assert_eq!(prompt_text(&mode), "draft");
    assert!(!mode.history_search_active.get());
}

#[test]
fn while_the_search_is_open_keys_do_not_reach_the_prompt() {
    let mut mode = mode_with_history();
    mode.editor.borrow_mut().editor.set_text("draft");
    press(&mut mode, CTRL_R);
    type_text(&mut mode, "x");
    assert_eq!(
        prompt_text(&mode),
        "draft",
        "typed into the query, not the prompt"
    );
    assert_eq!(shown(&mode), None);
}

#[test]
fn the_search_rows_show_the_query_and_the_keys_from_the_keymap() {
    let mut mode = mode_with_history();
    press(&mut mode, CTRL_R);
    type_text(&mut mode, "beta");
    let rows = footer(&mode);
    assert!(rows.contains("(reverse-i-search)"), "{rows}");
    assert!(rows.contains("`beta': beta two"), "{rows}");
    assert!(rows.contains("ctrl+r older"), "{rows}");
    assert!(rows.contains("enter edit"), "{rows}");
    assert!(rows.contains("esc cancel"), "{rows}");
    press(&mut mode, ESC);
    assert!(
        !footer(&mode).contains("reverse-i-search"),
        "rows go with the search"
    );
}

#[test]
fn up_on_an_empty_prompt_takes_the_last_queued_message_back() {
    let mut mode = mode_with_history();
    mode.session.follow_up("first", &[]).expect("queued");
    mode.session.follow_up("second", &[]).expect("queued");
    mode.update_pending_messages_display();
    press(&mut mode, UP);
    assert_eq!(prompt_text(&mode), "second");
    assert_eq!(mode.session.get_follow_up_messages(), ["first"]);
}

#[test]
fn up_with_nothing_queued_is_not_taken_back_and_does_not_type() {
    // Plain Up keeps its own meaning (the cursor, or the task panel); only Shift+Up
    // walks the prompt history (d55758f). With nothing queued, Up takes nothing back.
    let mut mode = mode_with_history();
    mode.update_pending_messages_display();
    press(&mut mode, UP);
    assert_eq!(prompt_text(&mode), "");
    assert!(mode.session.get_follow_up_messages().is_empty());
}

#[test]
fn leader_d_takes_only_the_last_queued_message_back() {
    let mut mode = mode_with_history();
    mode.session.follow_up("first", &[]).expect("queued");
    mode.session.follow_up("second", &[]).expect("queued");
    mode.handle_dequeue();
    assert_eq!(prompt_text(&mode), "second");
    assert_eq!(mode.session.get_follow_up_messages(), ["first"]);
}

#[test]
fn a_taken_back_message_goes_ahead_of_what_is_typed() {
    let mut mode = mode_with_history();
    mode.session.follow_up("first", &[]).expect("queued");
    mode.editor.borrow_mut().editor.set_text("draft");
    mode.handle_dequeue();
    assert_eq!(prompt_text(&mode), "first\n\ndraft");
}

#[test]
fn with_nothing_in_the_session_queue_a_held_compaction_message_comes_back() {
    let mut mode = mode_with_history();
    mode.compaction_queue
        .push(("held".into(), StreamingBehavior::FollowUp));
    assert!(mode.take_back_last_queued());
    assert_eq!(prompt_text(&mode), "held");
    assert!(mode.compaction_queue.is_empty());
    assert!(!mode.take_back_last_queued(), "nothing left");
}

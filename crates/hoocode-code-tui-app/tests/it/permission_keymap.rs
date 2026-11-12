//! The permission card's keys come from the keymap (`app.permission.*`): a
//! rebound key acts, the card's hints show its new name, and the old default
//! no longer answers. Each test rebinds one key and restores the default on
//! drop.

use std::cell::RefCell;
use std::rc::Rc;

use hoocode_code_permissions::{reach_for, ClearanceChoice, ClearanceInfo};
use hoocode_code_tui_app::clearance_card::ClearanceCard;
use hoocode_code_tui_keybindings::AppKeybindingsManager;
use hoocode_tui_render::golden::render_golden;
use hoocode_tui_render::Component;
use serde_json::json;

use crate::support::{lock, plain};

type Answered = Rc<RefCell<Option<ClearanceChoice>>>;

/// Installs a keymap given as flat `keybindings.json` text; the default comes
/// back when the guard drops.
struct Rebound;

impl Rebound {
    fn with(text: &str) -> Self {
        AppKeybindingsManager::from_json_str(text).install();
        Rebound
    }
}

impl Drop for Rebound {
    fn drop(&mut self) {
        AppKeybindingsManager::default().install();
    }
}

fn shell_info(detail: &str) -> ClearanceInfo {
    let input = json!({"command": detail.trim_start_matches("$ ").trim_start()});
    ClearanceInfo {
        tool: "Shell".to_string(),
        detail: detail.to_string(),
        mode: "build".to_string(),
        step: None,
        reach: reach_for("Shell", &input),
        from: None,
    }
}

fn card(info: ClearanceInfo) -> (ClearanceCard, Answered) {
    let answered: Answered = Rc::default();
    let sink = answered.clone();
    let card = ClearanceCard::new(
        info,
        Box::new(move |choice| *sink.borrow_mut() = Some(choice)),
    );
    (card, answered)
}

/// The card's plain rows at 100 columns.
fn rows(card: &mut ClearanceCard) -> Vec<String> {
    plain(&render_golden(card, 100))
        .lines()
        .take_while(|line| *line != "--- styles ---")
        .map(str::to_string)
        .collect()
}

fn shows(rows: &[String], text: &str) -> bool {
    rows.iter().any(|row| row.contains(text))
}

#[test]
fn rebound_edit_key_opens_the_editor_and_the_hint_shows_it() {
    let _g = lock(Some("dark"));
    let _k = Rebound::with(r#"{"app.permission.edit": "x"}"#);
    let (mut card, answered) = card(shell_info("$ echo hi"));
    let before = rows(&mut card);
    assert!(shows(&before, "x edit"), "{before:?}");
    assert!(!shows(&before, "e edit"), "{before:?}");
    card.handle_input("x");
    card.handle_input("!");
    card.handle_input("\r");
    assert_eq!(
        *answered.borrow(),
        Some(ClearanceChoice::Revise("echo hi!".to_string())),
        "x opens the editor"
    );
}

#[test]
fn rebound_pick1_answers_option_one_and_the_hint_shows_it() {
    let _g = lock(Some("dark"));
    let _k = Rebound::with(r#"{"app.permission.pick1": "q"}"#);
    let (mut card, answered) = card(shell_info("$ echo hi"));
    let before = rows(&mut card);
    assert!(shows(&before, "q–3 pick"), "{before:?}");
    assert!(shows(&before, "q  yes"), "{before:?}");
    card.handle_input("q");
    assert_eq!(*answered.borrow(), Some(ClearanceChoice::Once));
}

#[test]
fn rebound_pick2_answers_option_two_and_its_row_shows_it() {
    let _g = lock(Some("dark"));
    let _k = Rebound::with(r#"{"app.permission.pick2": "w"}"#);
    let (mut card, answered) = card(shell_info("$ echo hi"));
    let before = rows(&mut card);
    assert!(shows(&before, "w  yes, always"), "{before:?}");
    card.handle_input("w");
    assert_eq!(*answered.borrow(), Some(ClearanceChoice::Always));
}

#[test]
fn rebound_pick3_answers_option_three_and_its_row_shows_it() {
    let _g = lock(Some("dark"));
    let _k = Rebound::with(r#"{"app.permission.pick3": "z"}"#);
    let (mut card, answered) = card(shell_info("$ echo hi"));
    let before = rows(&mut card);
    assert!(shows(&before, "z  no, tell it why"), "{before:?}");
    card.handle_input("z");
    assert_eq!(
        *answered.borrow(),
        Some(ClearanceChoice::Deny { reason: None })
    );
}

#[test]
fn the_default_digit_no_longer_answers_once_rebound() {
    // "1" is an ordinary letter now: it joins the reply, so nothing is chosen.
    let _g = lock(Some("dark"));
    let _k = Rebound::with(r#"{"app.permission.pick1": "q"}"#);
    let (mut card, answered) = card(shell_info("$ echo hi"));
    card.handle_input("1");
    assert_eq!(*answered.borrow(), None);
    assert!(shows(&rows(&mut card), "1"), "1 types into the reply");
}

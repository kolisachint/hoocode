//! The clearance card (M6, spec §4 c1, r5): the warn frame over a gated
//! call, the REACH line from the call's real data, and each key's answer.
//! The theme lock pins the palette so the goldens hold.

use std::cell::RefCell;
use std::rc::Rc;

use hoocode_code_permissions::{reach_for, ClearanceChoice, ClearanceInfo};
use hoocode_code_tui_app::clearance_card::ClearanceCard;
use hoocode_tui_render::assert_golden;
use hoocode_tui_render::golden::render_golden;
use hoocode_tui_render::Component;
use serde_json::json;

use crate::support::{lock, look, plain, Look};

type Answered = Rc<RefCell<Option<ClearanceChoice>>>;

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

/// The card's plain rows at `width` columns.
fn rows(card: &mut ClearanceCard, width: u16) -> Vec<String> {
    plain(&render_golden(card, width))
        .lines()
        .take_while(|line| *line != "--- styles ---")
        .map(str::to_string)
        .collect()
}

#[test]
fn a_subagents_card_names_the_subagent() {
    let _g = lock(Some("dark"));
    let mut info = shell_info("$ ls");
    info.from = Some("explore task-3".to_string());
    let (mut card, _) = card(info);
    let rows = rows(&mut card, 100);
    assert!(rows[0].contains("Allow Shell?"), "row 0: {:?}", rows[0]);
    assert!(
        rows[0].contains("asked by explore task-3"),
        "row 0: {:?}",
        rows[0]
    );
    assert!(rows[1].contains("$ ls"), "row 1: {:?}", rows[1]);
}

#[test]
fn shell_card_shows_title_command_access_and_options() {
    let _g = lock(Some("dark"));
    let (mut card, _) = card(shell_info("$ printf 'alpha\\nbeta\\ngamma\\n' | sort -r"));
    let rows = rows(&mut card, 100);
    assert_eq!(rows.len(), 8, "{rows:?}");
    assert!(rows[0].contains("Allow Shell?"), "row 0: {:?}", rows[0]);
    assert!(rows[1].contains("$ printf"), "row 1: {:?}", rows[1]);
    assert!(
        rows[2].contains("access: pipes 2 programs"),
        "row 2: {:?}",
        rows[2]
    );
    assert!(rows[3].contains("› 1  yes"), "row 3: {:?}", rows[3]);
    assert!(
        rows[4].contains("2  yes, always for printf this session"),
        "row 4: {:?}",
        rows[4]
    );
    assert!(
        rows[5].contains("3  no, tell it why"),
        "row 5: {:?}",
        rows[5]
    );
    assert!(rows[6].contains("or type a reply"), "row 6: {:?}", rows[6]);
    assert!(
        rows[7].contains("1–3 pick · e edit first · or type a reply"),
        "row 7: {:?}",
        rows[7]
    );
    // Plain words: no REACH row, no y/a/n key hints.
    assert!(!rows.iter().any(|row| row.contains("REACH")), "{rows:?}");
}

#[test]
fn shell_card_golden() {
    let _g = lock(Some("dark"));
    let (mut card, _) = card(shell_info("$ printf 'alpha\\nbeta\\ngamma\\n' | sort -r"));
    assert_golden!("clearance_shell_100", render_golden(&mut card, 100));
}

#[test]
fn a_plain_read_has_no_access_row() {
    let _g = lock(Some("dark"));
    let (mut card, _) = card(shell_info("$ wc -l src/wrap.rs"));
    let rows = rows(&mut card, 100);
    assert!(!rows.iter().any(|row| row.contains("access:")), "{rows:?}");
    assert!(rows[2].contains("› 1  yes"), "row 2: {:?}", rows[2]);
}

#[test]
fn a_write_or_network_call_has_an_access_row() {
    let _g = lock(Some("dark"));
    let (mut write_card, _) = card(shell_info("$ echo hi > out.txt"));
    let write_rows = rows(&mut write_card, 100);
    assert!(
        write_rows[2].contains("access: writes out.txt"),
        "{write_rows:?}"
    );
    let (mut net_card, _) = card(shell_info("$ curl https://example.com"));
    let net_rows = rows(&mut net_card, 100);
    assert!(net_rows[2].contains("access: network"), "{net_rows:?}");
}

#[test]
fn edit_card_names_the_real_path() {
    let _g = lock(Some("dark"));
    let input = json!({"path": "src/app.ts"});
    let (mut card, _) = card(ClearanceInfo {
        tool: "Edit".to_string(),
        detail: "Edit src/app.ts".to_string(),
        mode: "build".to_string(),
        step: None,
        reach: reach_for("Edit", &input),
        from: None,
    });
    let rows = rows(&mut card, 100);
    assert!(rows[0].contains("Allow Edit?"), "row 0: {:?}", rows[0]);
    assert!(rows[1].contains("Edit src/app.ts"), "row 1: {:?}", rows[1]);
    assert!(
        rows[2].contains("access: writes src/app.ts"),
        "row 2: {:?}",
        rows[2]
    );
    assert!(
        rows[4].contains("yes, always for Edit in build"),
        "row 4: {:?}",
        rows[4]
    );
}

#[test]
fn wrapper_omits_always_and_two_is_no() {
    let _g = lock(Some("dark"));
    hoocode_code_tui_keybindings::AppKeybindingsManager::default().install();
    let (mut card, answered) = card(shell_info("$ sudo rm -rf /tmp/x"));
    let rows = rows(&mut card, 100);
    assert!(
        !rows.iter().any(|row| row.contains("always")),
        "must omit always: {rows:?}"
    );
    assert!(
        rows.iter().any(|row| row.contains("2  no, tell it why")),
        "{rows:?}"
    );
    card.handle_input("3");
    assert_eq!(*answered.borrow(), None, "3 is not an option here");
    card.handle_input("\u{7f}");
    card.handle_input("2");
    assert_eq!(
        *answered.borrow(),
        Some(ClearanceChoice::Deny { reason: None })
    );
}

#[test]
fn plan_mode_omits_always() {
    let _g = lock(Some("dark"));
    for mode in ["plan", " plan "] {
        let mut info = shell_info("$ printf y");
        info.mode = mode.to_string();
        let (mut card, _) = card(info);
        let rows = rows(&mut card, 100);
        assert!(
            !rows.iter().any(|row| row.contains("always")),
            "{mode:?}: {rows:?}"
        );
    }
}

#[test]
fn session_spawn_offers_no_always_row() {
    let _g = lock(Some("dark"));
    hoocode_code_tui_keybindings::AppKeybindingsManager::default().install();
    let mut info = shell_info("$ ignored");
    info.tool = "SessionSpawn".to_string();
    info.detail = "Goal: refactor the parser".to_string();
    let (mut card, answered) = card(info);
    let rows = rows(&mut card, 100);
    assert!(
        !rows.iter().any(|row| row.contains("always")),
        "SessionSpawn must not offer always: {rows:?}"
    );
    // `a` has no row to highlight here, so Enter does not answer always.
    card.handle_input("a");
    card.handle_input("\r");
    assert_ne!(*answered.borrow(), Some(ClearanceChoice::Always));
}

#[test]
fn the_title_stays_bare_when_the_host_knows_the_step() {
    let _g = lock(Some("dark"));
    let mut info = shell_info("$ wc -l src/wrap.rs");
    info.step = Some(1);
    let (mut card, _) = card(info);
    let rows = rows(&mut card, 100);
    assert!(rows[0].contains("Allow Shell?"), "row 0: {:?}", rows[0]);
    assert!(!rows[0].contains("step"), "row 0: {:?}", rows[0]);
}

#[test]
fn number_keys_enter_and_esc_answer() {
    let _g = lock(Some("dark"));
    hoocode_code_tui_keybindings::AppKeybindingsManager::default().install();
    for (key, expected) in [
        ("1", ClearanceChoice::Once),
        ("2", ClearanceChoice::Always),
        ("3", ClearanceChoice::Deny { reason: None }),
        ("\x1b", ClearanceChoice::Deny { reason: None }),
        ("\r", ClearanceChoice::Once),
    ] {
        let (mut card, answered) = card(shell_info("$ echo hi"));
        card.handle_input(key);
        assert_eq!(*answered.borrow(), Some(expected), "key {key:?}");
    }
}

#[test]
fn y_and_n_answer_at_once_while_the_reply_is_empty() {
    // K-P4: y and n answer while no reply is typed.
    let _g = lock(Some("dark"));
    hoocode_code_tui_keybindings::AppKeybindingsManager::default().install();
    for (key, expected) in [
        ("y", ClearanceChoice::Once),
        ("n", ClearanceChoice::Deny { reason: None }),
    ] {
        let (mut card, answered) = card(shell_info("$ echo hi"));
        card.handle_input(key);
        assert_eq!(*answered.borrow(), Some(expected), "key {key:?}");
    }
}

#[test]
fn a_then_enter_answers_always() {
    let _g = lock(Some("dark"));
    hoocode_code_tui_keybindings::AppKeybindingsManager::default().install();
    let (mut card, answered) = card(shell_info("$ echo hi"));
    card.handle_input("a");
    assert_eq!(*answered.borrow(), None, "a only highlights the always row");
    card.handle_input("\r");
    assert_eq!(*answered.borrow(), Some(ClearanceChoice::Always));
}

#[test]
fn a_alone_only_moves_the_highlight_to_always() {
    let _g = lock(Some("dark"));
    hoocode_code_tui_keybindings::AppKeybindingsManager::default().install();
    let (mut card, answered) = card(shell_info("$ echo hi"));
    card.handle_input("a");
    assert_eq!(*answered.borrow(), None);
    let rows = rows(&mut card, 100);
    assert!(
        rows.iter().any(|row| row.contains("› 2  yes, always")),
        "{rows:?}"
    );
    assert!(!rows.iter().any(|row| row.contains("› a")), "{rows:?}");
}

#[test]
fn a_types_when_no_always_row_is_offered() {
    // A wrapper has no always row, so `a` is an ordinary letter in the reply.
    let _g = lock(Some("dark"));
    hoocode_code_tui_keybindings::AppKeybindingsManager::default().install();
    let (mut card, answered) = card(shell_info("$ sudo rm -rf /tmp/x"));
    card.handle_input("a");
    assert_eq!(*answered.borrow(), None);
    let rows = rows(&mut card, 100);
    assert!(rows.iter().any(|row| row == " a"), "{rows:?}");
}

#[test]
fn y_a_n_type_into_a_reply_that_has_text() {
    let _g = lock(Some("dark"));
    hoocode_code_tui_keybindings::AppKeybindingsManager::default().install();
    let (mut card, answered) = card(shell_info("$ echo hi"));
    for key in ["w", "a", "y", "n"] {
        card.handle_input(key);
    }
    assert_eq!(*answered.borrow(), None, "letters join the reply");
    let rows = rows(&mut card, 100);
    assert!(rows.iter().any(|row| row == " wayn"), "{rows:?}");
}

#[test]
fn arrows_move_the_pick_and_enter_answers_it() {
    let _g = lock(Some("dark"));
    let (mut card, answered) = card(shell_info("$ echo hi"));
    card.handle_input("\x1b[B");
    let rows = rows(&mut card, 100);
    assert!(
        rows.iter().any(|row| row.contains("› 2  yes, always")),
        "{rows:?}"
    );
    card.handle_input("\r");
    assert_eq!(*answered.borrow(), Some(ClearanceChoice::Always));
}

#[test]
fn typed_text_becomes_the_abort_reason() {
    let _g = lock(Some("dark"));
    let (mut card, answered) = card(shell_info("$ echo hi"));
    // Card keys stay quiet once text is there: even `y` types.
    for key in ["w", "h", "y"] {
        card.handle_input(key);
    }
    assert_eq!(*answered.borrow(), None);
    let rows = rows(&mut card, 100);
    assert!(rows.iter().any(|row| row == " why"), "{rows:?}");
    card.handle_input("\r");
    assert_eq!(
        *answered.borrow(),
        Some(ClearanceChoice::Deny {
            reason: Some("why".to_string())
        })
    );
}

#[test]
fn esc_clears_typed_text_before_denying() {
    let _g = lock(Some("dark"));
    let (mut card, answered) = card(shell_info("$ echo hi"));
    card.handle_input("x");
    card.handle_input("\x1b");
    assert_eq!(*answered.borrow(), None);
    let rows = rows(&mut card, 100);
    assert!(!rows.iter().any(|row| row.contains("› x")), "{rows:?}");
    card.handle_input("\x1b");
    assert_eq!(
        *answered.borrow(),
        Some(ClearanceChoice::Deny { reason: None })
    );
}

#[test]
fn e_edits_and_enter_revises() {
    let _g = lock(Some("dark"));
    hoocode_code_tui_keybindings::AppKeybindingsManager::default().install();
    let (mut card, answered) = card(shell_info("$ echo hi"));
    card.handle_input("e");
    let rows = rows(&mut card, 100);
    assert!(rows.iter().any(|row| row == " echo hi"), "{rows:?}");
    assert!(rows.iter().any(|row| row.contains("echo hi")), "{rows:?}");
    card.handle_input("!");
    card.handle_input("\r");
    assert_eq!(
        *answered.borrow(),
        Some(ClearanceChoice::Revise("echo hi!".to_string()))
    );
}

#[test]
fn esc_cancels_the_edit() {
    let _g = lock(Some("dark"));
    let (mut card, answered) = card(shell_info("$ echo hi"));
    card.handle_input("e");
    card.handle_input("\x1b");
    assert_eq!(*answered.borrow(), None);
    let rows = rows(&mut card, 100);
    assert!(rows.iter().any(|row| row.contains("e edit")), "{rows:?}");
}

/// The shell card in the light and monochrome looks.
#[test]
fn shell_card_light_golden() {
    let _g = lock(Some("dark"));
    look(Look::Light);
    let (mut card, _) = card(shell_info("$ printf 'alpha\\nbeta\\ngamma\\n' | sort -r"));
    assert_golden!("clearance_shell_light_100", render_golden(&mut card, 100));
}

#[test]
fn shell_card_mono_golden() {
    let _g = lock(Some("dark"));
    look(Look::Mono);
    let (mut card, _) = card(shell_info("$ printf 'alpha\\nbeta\\ngamma\\n' | sort -r"));
    assert_golden!("clearance_shell_mono_100", render_golden(&mut card, 100));
}

/// A parked call's card (SA-12 row 4b): yes, approve and detach, no. No always row and
/// no edit key, and `d` (or `2`) approves and detaches.
#[test]
fn a_parked_card_offers_yes_detach_and_no() {
    let _g = lock(Some("dark"));
    hoocode_code_tui_keybindings::AppKeybindingsManager::default().install();
    let answered: Answered = Rc::default();
    let detached = Rc::new(std::cell::Cell::new(false));
    let (sink, flag) = (answered.clone(), detached.clone());
    let mut card = ClearanceCard::parked(
        shell_info("$ ls"),
        Box::new(move |choice| *sink.borrow_mut() = Some(choice)),
        Box::new(move || flag.set(true)),
    );
    let rows = rows(&mut card, 100);
    assert!(rows[0].contains("Allow Shell?"), "row 0: {:?}", rows[0]);
    assert!(
        rows.iter()
            .any(|r| r.contains("parked call from a detached run")),
        "{rows:?}"
    );
    assert!(rows.iter().any(|r| r.contains("› 1  yes")), "{rows:?}");
    assert!(
        rows.iter().any(|r| r.contains("2  approve and detach")),
        "{rows:?}"
    );
    assert!(rows.iter().any(|r| r.contains("3  no")), "{rows:?}");
    assert!(!rows.iter().any(|r| r.contains("always")), "{rows:?}");

    card.handle_input("d");
    assert!(detached.get(), "d detaches");
    assert!(
        answered.borrow().is_none(),
        "detaching is not an answer to on_done"
    );
}

/// The answer a parked card gives for `key` typed at it.
fn parked_answer(key: &str) -> (Option<ClearanceChoice>, String) {
    hoocode_code_tui_keybindings::AppKeybindingsManager::default().install();
    let answered: Answered = Rc::default();
    let sink = answered.clone();
    let mut card = ClearanceCard::parked(
        shell_info("$ ls"),
        Box::new(move |choice| *sink.borrow_mut() = Some(choice)),
        Box::new(|| {}),
    );
    card.handle_input(key);
    let text = rows(&mut card, 100).join("\n");
    let answer = answered.borrow().clone();
    (answer, text)
}

#[test]
fn a_parked_card_answers_yes_and_no_and_has_no_edit_key() {
    let _g = lock(Some("dark"));
    assert_eq!(parked_answer("y").0, Some(ClearanceChoice::Once));
    assert_eq!(
        parked_answer("n").0,
        Some(ClearanceChoice::Deny { reason: None })
    );
    // `e` is not an edit key on a parked card: it types into the reply, so nothing is
    // answered and no edit row appears.
    let (answer, text) = parked_answer("e");
    assert_eq!(answer, None);
    assert!(!text.contains("edit"), "{text}");
}

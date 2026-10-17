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

use crate::support::{lock, plain};

type Answered = Rc<RefCell<Option<ClearanceChoice>>>;

fn shell_info(detail: &str) -> ClearanceInfo {
    let input = json!({"command": detail.trim_start_matches("$ ").trim_start()});
    ClearanceInfo {
        tool: "Shell".to_string(),
        detail: detail.to_string(),
        mode: "build".to_string(),
        step: None,
        reach: reach_for("Shell", &input),
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
fn shell_card_shows_title_command_reach_and_keys() {
    let _g = lock(Some("dark"));
    let (mut card, _) = card(shell_info("$ printf 'alpha\\nbeta\\ngamma\\n' | sort -r"));
    let rows = rows(&mut card, 100);
    assert_eq!(rows.len(), 5, "{rows:?}");
    assert!(
        rows[0].contains("◉ CLEARANCE NEEDED"),
        "row 0: {:?}",
        rows[0]
    );
    assert!(
        rows[0].contains("BUILD asks for Shell"),
        "row 0: {:?}",
        rows[0]
    );
    assert!(rows[1].contains("$ printf"), "row 1: {:?}", rows[1]);
    assert!(rows[2].contains("REACH"), "row 2: {:?}", rows[2]);
    assert!(
        rows[2].contains("■ pipe 2 programs"),
        "row 2: {:?}",
        rows[2]
    );
    assert!(rows[2].contains("□ read"), "row 2: {:?}", rows[2]);
    assert!(rows[3].contains("y go once"), "row 3: {:?}", rows[3]);
    assert!(
        rows[3].contains("a always printf in BUILD"),
        "row 3: {:?}",
        rows[3]
    );
    assert!(
        rows[3].contains("n abort + say why"),
        "row 3: {:?}",
        rows[3]
    );
    assert!(rows[3].contains("e edit first"), "row 3: {:?}", rows[3]);
    assert!(rows[4].contains("or type a reply"), "row 4: {:?}", rows[4]);
}

#[test]
fn shell_card_golden() {
    let _g = lock(Some("dark"));
    let (mut card, _) = card(shell_info("$ printf 'alpha\\nbeta\\ngamma\\n' | sort -r"));
    assert_golden!("clearance_shell_100", render_golden(&mut card, 100));
}

#[test]
fn read_path_comes_from_the_command() {
    let _g = lock(Some("dark"));
    let (mut card, _) = card(shell_info("$ wc -l src/wrap.rs"));
    let rows = rows(&mut card, 100);
    assert!(
        rows[2].contains("■ read src/wrap.rs"),
        "row 2: {:?}",
        rows[2]
    );
    assert!(rows[2].contains("□ write"), "row 2: {:?}", rows[2]);
    assert!(rows[2].contains("□ network"), "row 2: {:?}", rows[2]);
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
    });
    let rows = rows(&mut card, 100);
    assert!(
        rows[0].contains("BUILD asks for Edit"),
        "row 0: {:?}",
        rows[0]
    );
    assert!(
        rows[2].contains("■ read src/app.ts"),
        "row 2: {:?}",
        rows[2]
    );
    assert!(
        rows[2].contains("■ write src/app.ts"),
        "row 2: {:?}",
        rows[2]
    );
    assert!(
        rows[3].contains("a always Edit in BUILD"),
        "row 3: {:?}",
        rows[3]
    );
}

#[test]
fn wrapper_omits_always_and_a_runs_once() {
    let _g = lock(Some("dark"));
    let (mut card, answered) = card(shell_info("$ sudo rm -rf /tmp/x"));
    let rows = rows(&mut card, 100);
    assert!(
        !rows[3].contains("a always"),
        "row 3 must omit `a`: {:?}",
        rows[3]
    );
    card.handle_input("a");
    assert_eq!(*answered.borrow(), Some(ClearanceChoice::Once));
}

#[test]
fn step_number_shows_when_the_host_knows_it() {
    let _g = lock(Some("dark"));
    let mut info = shell_info("$ wc -l src/wrap.rs");
    info.step = Some(1);
    let (mut card, _) = card(info);
    let rows = rows(&mut card, 100);
    assert!(
        rows[0].contains("◉ CLEARANCE NEEDED · step 01"),
        "row 0: {:?}",
        rows[0]
    );
}

#[test]
fn y_a_enter_and_esc_answer() {
    let _g = lock(Some("dark"));
    for (key, expected) in [
        ("y", ClearanceChoice::Once),
        ("a", ClearanceChoice::Always),
        ("n", ClearanceChoice::Deny { reason: None }),
        ("\x1b", ClearanceChoice::Deny { reason: None }),
        ("\r", ClearanceChoice::Once),
    ] {
        let (mut card, answered) = card(shell_info("$ echo hi"));
        card.handle_input(key);
        assert_eq!(*answered.borrow(), Some(expected), "key {key:?}");
    }
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
    assert!(rows.iter().any(|row| row.contains("▸ why")), "{rows:?}");
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
    assert!(!rows.iter().any(|row| row.contains('▸')), "{rows:?}");
    card.handle_input("\x1b");
    assert_eq!(
        *answered.borrow(),
        Some(ClearanceChoice::Deny { reason: None })
    );
}

#[test]
fn e_edits_and_enter_revises() {
    let _g = lock(Some("dark"));
    let (mut card, answered) = card(shell_info("$ echo hi"));
    card.handle_input("e");
    let rows = rows(&mut card, 100);
    assert!(rows.iter().any(|row| row.contains("✎")), "{rows:?}");
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
    assert!(
        rows.iter().any(|row| row.contains("e edit first")),
        "{rows:?}"
    );
}

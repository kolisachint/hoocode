//! The end-of-shift cards (M6, spec §4 c8 and §6.2): the amber card with
//! the shift's debrief (`enter` ends, `esc` stays), and the satellites card
//! for quitting with jobs running (`k` brings them down, `l` leaves them).

use std::cell::RefCell;
use std::rc::Rc;

use hoocode_code_tui_app::end_shift::{
    EndShiftCard, EndShiftChoice, EndShiftJobsCard, EndShiftJobsChoice, ShiftDebrief, ShiftJob,
};
use hoocode_tui_render::assert_golden;
use hoocode_tui_render::golden::render_golden;
use hoocode_tui_render::Component;

use crate::support::{lock, plain};

fn debrief() -> ShiftDebrief {
    ShiftDebrief {
        missions: 2,
        steps: 5,
        input_tokens: 48_200,
        output_tokens: 9_750,
        cost: 1.234,
    }
}

fn jobs() -> Vec<ShiftJob> {
    vec![
        ShiftJob {
            id: "job-1".to_string(),
            command: "sleep 60".to_string(),
            running: true,
            elapsed_secs: Some(4),
            total_secs: Some(60),
            log_path: Some("jobs/job-1.log".to_string()),
        },
        ShiftJob {
            id: "job-2".to_string(),
            command: "sleep 120".to_string(),
            running: true,
            elapsed_secs: None,
            total_secs: None,
            log_path: None,
        },
    ]
}

/// A card's plain rows at `width` columns.
fn rows(card: &mut impl Component, width: u16) -> Vec<String> {
    plain(&render_golden(card, width))
        .lines()
        .take_while(|line| *line != "--- styles ---")
        .map(str::to_string)
        .collect()
}

#[test]
fn plain_card_shows_the_debrief_and_keys() {
    let _g = lock(Some("dark"));
    let answered = Rc::new(RefCell::new(None));
    let sink = answered.clone();
    let mut card = EndShiftCard::new(
        debrief(),
        Box::new(move |choice| *sink.borrow_mut() = Some(choice)),
    );
    let rows = rows(&mut card, 100);
    assert_eq!(rows.len(), 5, "{rows:?}");
    assert!(rows[0].contains("◉ END OF SHIFT"), "row 0: {:?}", rows[0]);
    assert!(rows[0].contains("esc stay"), "row 0: {:?}", rows[0]);
    assert!(rows[1].contains("2 missions"), "row 1: {:?}", rows[1]);
    assert!(rows[1].contains("5 steps"), "row 1: {:?}", rows[1]);
    assert!(rows[1].contains("CR $1.234"), "row 1: {:?}", rows[1]);
    assert!(
        rows[3].contains("enter end the shift"),
        "row 3: {:?}",
        rows[3]
    );
    assert!(rows[3].contains("esc stay"), "row 3: {:?}", rows[3]);
}

#[test]
fn plain_card_golden() {
    let _g = lock(Some("dark"));
    let mut card = EndShiftCard::new(debrief(), Box::new(|_| {}));
    assert_golden!("end_shift_plain_100", render_golden(&mut card, 100));
}

#[test]
fn plain_card_enter_ends_and_esc_stays() {
    let _g = lock(Some("dark"));
    let answered = Rc::new(RefCell::new(None));
    let sink = answered.clone();
    let mut card = EndShiftCard::new(
        debrief(),
        Box::new(move |choice| *sink.borrow_mut() = Some(choice)),
    );
    card.handle_input("\x1b");
    assert_eq!(*answered.borrow(), Some(EndShiftChoice::Stay));

    let answered = Rc::new(RefCell::new(None));
    let sink = answered.clone();
    let mut card = EndShiftCard::new(
        debrief(),
        Box::new(move |choice| *sink.borrow_mut() = Some(choice)),
    );
    card.handle_input("\r");
    assert_eq!(*answered.borrow(), Some(EndShiftChoice::End));
}

#[test]
fn jobs_card_lists_the_satellites() {
    let _g = lock(Some("dark"));
    let mut card = EndShiftJobsCard::new(jobs(), Box::new(|_| {}));
    let rows = rows(&mut card, 100);
    assert_eq!(rows.len(), 6, "{rows:?}");
    assert!(
        rows[0].contains("◎ END SHIFT WITH 2 SATELLITES UP?"),
        "row 0: {:?}",
        rows[0]
    );
    assert!(rows[1].contains("◉ job-1"), "row 1: {:?}", rows[1]);
    assert!(rows[1].contains("sleep 60"), "row 1: {:?}", rows[1]);
    assert!(
        rows[1].contains("in orbit 4s of ~60s"),
        "row 1: {:?}",
        rows[1]
    );
    assert!(rows[2].contains("◉ job-2"), "row 2: {:?}", rows[2]);
    assert!(
        rows[4].contains("k bring it down, then end shift"),
        "row 4: {:?}",
        rows[4]
    );
    assert!(
        rows[4].contains("l leave it in orbit"),
        "row 4: {:?}",
        rows[4]
    );
}

#[test]
fn jobs_card_golden() {
    let _g = lock(Some("dark"));
    let mut card = EndShiftJobsCard::new(jobs(), Box::new(|_| {}));
    assert_golden!("end_shift_jobs_100", render_golden(&mut card, 100));
}

#[test]
fn single_satellite_uses_the_singular() {
    let _g = lock(Some("dark"));
    let mut card = EndShiftJobsCard::new(jobs()[..1].to_vec(), Box::new(|_| {}));
    let rows = rows(&mut card, 100);
    assert!(
        rows[0].contains("◎ END SHIFT WITH 1 SATELLITE UP?"),
        "row 0: {:?}",
        rows[0]
    );
    assert!(
        rows[3].contains("log stays in jobs/job-1.log"),
        "foot names the log: {:?}",
        rows[3]
    );
}

#[test]
fn jobs_card_keys() {
    let _g = lock(Some("dark"));
    for (key, expected) in [
        ("k", EndShiftJobsChoice::KillAll),
        ("l", EndShiftJobsChoice::LeaveRunning),
        ("\r", EndShiftJobsChoice::KillAll),
        ("\x1b", EndShiftJobsChoice::Stay),
    ] {
        let answered = Rc::new(RefCell::new(None));
        let sink = answered.clone();
        let mut card = EndShiftJobsCard::new(
            jobs(),
            Box::new(move |choice| *sink.borrow_mut() = Some(choice)),
        );
        card.handle_input(key);
        assert_eq!(*answered.borrow(), Some(expected), "key {key:?}");
    }
}

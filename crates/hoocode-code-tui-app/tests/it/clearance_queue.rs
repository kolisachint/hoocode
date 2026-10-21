//! Clearance requests that arrive while a card is up wait their turn: the
//! second card shows when the first closes, nobody is auto-denied, and a
//! request withdrawn while waiting never shows.

use std::sync::mpsc;

use hoocode_code_permissions::{reach_for, ClearanceChoice, ClearanceInfo};
use hoocode_code_tui_app::clearance_queue::{
    ClearanceQueue, PendingClearance, Withdrawn, OWN_CALL,
};
use serde_json::json;

fn request(
    id: u64,
    command: &str,
    from: Option<&str>,
) -> (PendingClearance, mpsc::Receiver<ClearanceChoice>) {
    let input = json!({"command": command});
    let (reply, answer) = mpsc::channel();
    let info = ClearanceInfo {
        tool: "Shell".into(),
        detail: format!("$ {command}"),
        mode: "build".into(),
        step: None,
        reach: reach_for("Shell", &input),
        from: from.map(str::to_string),
    };
    (PendingClearance { id, info, reply }, answer)
}

fn detail(request: &Option<PendingClearance>) -> Option<&str> {
    request.as_ref().map(|r| r.info.detail.as_str())
}

#[test]
fn the_second_card_shows_after_the_first_closes() {
    let mut queue = ClearanceQueue::default();
    let (first, first_answer) = request(OWN_CALL, "ls", None);
    let (second, second_answer) = request(7, "rm x", Some("explore t1"));
    let (third, _third_answer) = request(8, "make", Some("review t2"));

    let shown = queue.arrive(first);
    assert_eq!(detail(&shown), Some("$ ls"), "the first shows at once");
    assert!(queue.arrive(second).is_none(), "the second waits");
    assert!(queue.arrive(third).is_none(), "the third waits");
    assert_eq!(queue.waiting(), 2);
    // Nobody was answered, least of all denied, by another arriving.
    assert!(first_answer.try_recv().is_err());
    assert!(second_answer.try_recv().is_err());

    // The first closes: the second is next, with its subagent's name.
    let next = queue.closed();
    assert_eq!(detail(&next), Some("$ rm x"));
    assert_eq!(
        next.as_ref().and_then(|r| r.info.from.as_deref()),
        Some("explore t1")
    );
    assert_eq!(queue.waiting(), 1);
    // And the third after it, then nothing.
    assert_eq!(detail(&queue.closed()), Some("$ make"));
    assert!(queue.closed().is_none());
    // With nothing up, the next arrival shows at once again.
    let (fourth, _answer) = request(9, "date", None);
    assert!(queue.arrive(fourth).is_some());
}

#[test]
fn a_withdrawn_request_never_shows() {
    let mut queue = ClearanceQueue::default();
    let (first, _a) = request(OWN_CALL, "ls", None);
    let (second, _b) = request(7, "rm x", Some("explore t1"));
    let (third, _c) = request(8, "make", Some("review t2"));
    queue.arrive(first);
    queue.arrive(second);
    queue.arrive(third);

    match queue.withdraw(7) {
        Withdrawn::Waiting(request) => assert_eq!(request.id, 7),
        _ => panic!("the waiting request should come out of the queue"),
    }
    assert!(matches!(queue.withdraw(7), Withdrawn::Unknown));
    assert_eq!(detail(&queue.closed()), Some("$ make"));
    // The card that is up is the caller's to close.
    assert!(matches!(queue.withdraw(8), Withdrawn::Open));
    // The main agent's own call is never withdrawn.
    assert!(matches!(queue.withdraw(OWN_CALL), Withdrawn::Unknown));
}

#[test]
fn draining_hands_back_everything_waiting() {
    let mut queue = ClearanceQueue::default();
    for (id, command) in [(OWN_CALL, "a"), (1, "b"), (2, "c")] {
        let (pending, _answer) = request(id, command, None);
        queue.arrive(pending);
    }
    let drained: Vec<u64> = queue.drain().iter().map(|r| r.id).collect();
    assert_eq!(drained, [1, 2]);
    assert!(queue.closed().is_none());
}

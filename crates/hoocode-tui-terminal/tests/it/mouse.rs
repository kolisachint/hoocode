//! Port of the pin's `test/mouse.test.ts`.

use hoocode_tui_terminal::{parse_mouse_report, MouseEvent, MouseEventKind as K};

fn event(data: &str) -> Option<MouseEvent> {
    parse_mouse_report(data).and_then(|(event, _)| event)
}

fn kind(data: &str) -> Option<K> {
    event(data).map(|e| e.kind)
}

/// Bytes the leading report occupies, 0 when there is none.
fn report_len(data: &str) -> usize {
    parse_mouse_report(data).map_or(0, |(_, len)| len)
}

#[test]
fn sgr_wheel_both_directions_and_no_button() {
    assert_eq!(kind("\x1b[<64;10;5M"), Some(K::WheelUp));
    assert_eq!(kind("\x1b[<65;10;5M"), Some(K::WheelDown));
    assert_eq!(event("\x1b[<64;1;1M").unwrap().button, -1);
    assert_eq!(kind("\x1b[<66;1;1M"), Some(K::WheelLeft));
    assert_eq!(kind("\x1b[<67;1;1M"), Some(K::WheelRight));
}

#[test]
fn sgr_coordinates_press_release_modifiers_buttons() {
    let e = event("\x1b[<0;42;7M").unwrap();
    assert_eq!((e.column, e.row), (42, 7));
    assert_eq!(event("\x1b[<0;400;9M").unwrap().column, 400);
    assert_eq!(kind("\x1b[<0;1;1M"), Some(K::Press));
    assert_eq!(kind("\x1b[<0;1;1m"), Some(K::Release));
    assert!(event("\x1b[<4;1;1M").unwrap().shift);
    assert!(event("\x1b[<8;1;1M").unwrap().alt);
    assert!(event("\x1b[<16;1;1M").unwrap().ctrl);
    let all = event("\x1b[<92;1;1M").unwrap();
    assert_eq!(
        (all.kind, all.shift, all.alt, all.ctrl),
        (K::WheelUp, true, true, true)
    );
    for b in 0..3 {
        assert_eq!(event(&format!("\x1b[<{b};1;1M")).unwrap().button, b);
    }
}

#[test]
fn x10_wheel_and_release() {
    let e = event(&format!(
        "\x1b[M\x60{}{}",
        char::from(32 + 10),
        char::from(32 + 5)
    ))
    .unwrap();
    assert_eq!((e.kind, e.column, e.row), (K::WheelUp, 10, 5));
    assert_eq!(
        kind(&format!("\x1b[M{}!!", char::from(32 + 3))),
        Some(K::Release)
    );
}

#[test]
fn rejects_non_reports() {
    for data in [
        "a",
        "\x1b[A",
        "\x1b[200~",
        "\x1b[<",
        "\x1b[M",
        "\x1b[Mab",
        "",
    ] {
        assert_eq!(parse_mouse_report(data), None, "{data:?}");
    }
}

#[test]
fn sequence_length() {
    assert_eq!(report_len("\x1b[<64;10;5M"), 11);
    assert_eq!(report_len("\x1b[<64;10;5Mx"), 11);
    assert_eq!(report_len("\x1b[Mabc"), 6);
    assert_eq!(report_len("x\x1b[<64;10;5M"), 0);
    assert_eq!(report_len("\x1b[A"), 0);
    let mut rest = "\x1b[<64;1;1M\x1b[<64;1;2M\x1b[<64;1;3M\x1b[A";
    let mut events = 0;
    while report_len(rest) > 0 {
        let n = report_len(rest);
        assert_eq!(kind(&rest[..n]), Some(K::WheelUp));
        rest = &rest[n..];
        events += 1;
    }
    assert_eq!((events, rest), (3, "\x1b[A"));
}

#[test]
fn reports_both_encodings_only() {
    assert!(parse_mouse_report("\x1b[<0;1;1M").is_some());
    assert!(parse_mouse_report("\x1b[Mabc").is_some());
    assert!(parse_mouse_report("\x1b[Ma").is_none());
    assert!(parse_mouse_report("\x1b[A").is_none());
}

/// An X10 report whose coordinates are below zero is still a report of six
/// bytes, with no event, so the caller skips it.
#[test]
fn malformed_x10_report_is_consumed_without_an_event() {
    assert_eq!(parse_mouse_report("\x1b[M\x01\x01\x01x"), Some((None, 6)));
}

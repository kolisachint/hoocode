//! `JsRegex::exec_at` with a start position past or inside a multi-byte character.
//! Regression for the `js_regex` fuzz crash (regex-automata `look.rs` index out of bounds).

use hoocode_tui_util::js_regex::JsRegex;

#[test]
fn multiline_dollar_exec_at_end_and_past_end_does_not_panic() {
    let subject = "a\u{FFFD}b\u{FFFD}";
    let re = JsRegex::new("$", "m");
    assert!(re.exec_at(subject, subject.len()).is_some());
    assert!(re.exec_at(subject, subject.len() + 1).is_none());
}

#[test]
fn exec_at_inside_multibyte_char_does_not_panic() {
    // Bytes 21..24 are one U+FFFD; start 23 is a continuation byte.
    let subject = "\u{1}\u{FFFD}\0\0\0\u{19}[\\\u{FFFD}\0z\u{19}\0\0\u{FFFD}\u{FFFD}";
    assert_eq!(subject.len(), 24);
    let re = JsRegex::new("$", "m");
    for pos in 0..=subject.len() + 1 {
        let _ = re.exec_at(subject, pos);
    }
    // The crash pattern from the fuzz corpus: pos 23 is a continuation byte of U+FFFD.
    if let Ok(crash) = JsRegex::try_new(r"$#\x06\x02\}", "m") {
        assert!(crash.exec_at(subject, 23).is_none());
    }
}

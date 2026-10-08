//! Target `js_regex`: the JavaScript RegExp emulation the markdown lexer runs on
//! (`hoocode-tui-util`, `js_regex`).
//!
//! The input is the pattern on its first line, the subject text after it, and the
//! first byte picks the flags. An invalid pattern must be an `Err`, never a panic, and
//! a valid one must match, search and replace any subject, including a start position
//! that falls inside a multi-byte character.

use hoocode_tui_util::js_regex::{translate, JsRegex};

const FLAG_SETS: [&str; 8] = ["", "g", "i", "gi", "m", "s", "y", "gm"];

pub fn run(data: &[u8]) {
    let flags = FLAG_SETS[usize::from(data.first().copied().unwrap_or(0)) % FLAG_SETS.len()];
    let text = String::from_utf8_lossy(data.get(1..).unwrap_or_default());
    let (pattern, subject) = text.split_once('\n').unwrap_or((&text, ""));

    let _ = translate(pattern, flags);
    let Ok(regex) = JsRegex::try_new(pattern, flags) else {
        return;
    };
    let _ = regex.test(subject);
    let _ = regex.search(subject);
    let _ = regex.exec(subject).map(|m| m.whole().len());
    let _ = regex.exec_at(subject, data.len() % (subject.len() + 1));
    let _ = regex.replace(subject, true, "[$&]");
}

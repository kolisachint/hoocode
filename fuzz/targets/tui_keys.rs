//! Target `tui_keys`: terminal input parsing (`hoocode-tui-keys`).
//!
//! Escape sequences arrive from the terminal in arbitrary chunks, so every entry point
//! must accept any string (legacy escapes, Kitty keyboard protocol, xterm
//! modifyOtherKeys) and return `None` rather than panic.

use hoocode_tui_keys::{
    decode_modify_other_keys_printable, decode_printable_key, is_key_release, parse_key,
    parse_kitty_sequence,
};

pub fn run(data: &[u8]) {
    let text = String::from_utf8_lossy(data);
    let _ = parse_key(&text);
    let _ = parse_kitty_sequence(&text);
    let _ = decode_modify_other_keys_printable(&text);
    let _ = decode_printable_key(&text);
    let _ = is_key_release(&text);
}

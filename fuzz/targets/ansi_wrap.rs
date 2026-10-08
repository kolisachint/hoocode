//! Target `ansi_wrap`: word wrapping and truncation of styled terminal text
//! (`hoocode-tui-util`, `text.rs` and `ansi.rs`).
//!
//! The first byte picks the width. Must not panic on any text, including escape
//! sequences cut short, wide characters and zero width.

use hoocode_tui_util::{
    strip_vt_control_characters, truncate_to_width, visible_width, wrap_text_with_ansi,
};

pub fn run(data: &[u8]) {
    let text = String::from_utf8_lossy(data);
    let width = usize::from(data.first().copied().unwrap_or(40)) % 120;

    for line in wrap_text_with_ansi(&text, width) {
        let _ = visible_width(&line);
    }
    let _ = truncate_to_width(&text, width, "...", true);
    let _ = strip_vt_control_characters(&text);
}

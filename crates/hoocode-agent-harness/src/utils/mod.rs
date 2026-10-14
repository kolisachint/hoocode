//! `harness/utils/`: output truncation, compression and shell capture.

pub mod output_compression;
pub mod shell_output;
pub mod truncate;

/// JS `\s` whitespace (`trim` / `trimEnd`); Rust's `char::is_whitespace` differs in U+FEFF and U+0085.
pub(crate) fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

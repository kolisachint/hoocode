//! Byte-index slicing of text that cannot panic.
//!
//! Input, model output and terminal output are not ASCII in general, so a byte
//! index can fall inside a multi-byte character. These helpers move an index
//! down to the nearest character boundary (and clamp it to the string length)
//! instead of panicking. `hoocode-code-tool-api::text_slice` is a copy for the
//! tool crates; the two must stay in step.

/// The largest char boundary at or below `i`, with `i` clamped to `s.len()`.
fn floor_boundary(s: &str, i: usize) -> usize {
    let mut i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// `s[..n]`, with `n` moved down to a char boundary.
pub fn prefix(s: &str, n: usize) -> &str {
    s.get(..floor_boundary(s, n)).unwrap_or_default()
}

/// `s[n..]`, with `n` moved down to a char boundary, so the result can start
/// a few bytes before `n`. An `n` past the end gives `""`.
pub fn suffix_from(s: &str, n: usize) -> &str {
    s.get(floor_boundary(s, n)..).unwrap_or_default()
}

/// `s[start..end]`, both ends moved down to char boundaries. When `start` ends
/// up after `end` the result is `""` instead of a panic.
pub fn range(s: &str, start: usize, end: usize) -> &str {
    let end = floor_boundary(s, end);
    let start = floor_boundary(s, start).min(end);
    s.get(start..end).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{prefix, range, suffix_from};

    #[test]
    fn prefix_never_splits_a_character() {
        // "é" is two bytes: index 1 falls inside it.
        assert_eq!(prefix("é!", 1), "");
        assert_eq!(prefix("é!", 2), "é");
        assert_eq!(prefix("abc", 10), "abc");
    }

    #[test]
    fn suffix_never_splits_a_character() {
        assert_eq!(suffix_from("é!", 1), "é!");
        assert_eq!(suffix_from("ab", 1), "b");
        assert_eq!(suffix_from("ab", 9), "");
    }

    #[test]
    fn range_handles_inverted_and_multibyte_bounds() {
        assert_eq!(range("héllo", 0, 2), "h");
        assert_eq!(range("héllo", 3, 1), "");
        assert_eq!(range("héllo", 1, 3), "é");
    }
}

//! ANSI-aware text wrapping, truncation, and column slicing.
//!
//! Ported from TypeScript `@kolisachint/hoocode-tui` → `utils.ts`
//! (`wrapTextWithAnsi`, `truncateToWidth`, `sliceByColumn`, `sliceWithWidth`,
//! `extractSegments`, `normalizeTerminalOutput`, `isWhitespaceChar`,
//! `isPunctuationChar`, `applyBackgroundToLine`).

use unicode_segmentation::UnicodeSegmentation;

use crate::ansi::{
    ansi_runs, extract_ansi_code, update_tracker_from_text, AnsiCodeTracker, AnsiRun,
};
use crate::width::{grapheme_width, visible_width};

// ---------------------------------------------------------------------------
// Character classification
// ---------------------------------------------------------------------------

/// Whether a (single-character) string is whitespace.
pub fn is_whitespace_char(c: &str) -> bool {
    c.chars().all(char::is_whitespace) && !c.is_empty()
}

const PUNCTUATION_CHARS: &str = "(){}[]<>.,;:'\"!?+-=*/\\|&%^$#@~`";

/// Whether a (single-character) string is one of the common punctuation marks.
pub fn is_punctuation_char(c: &str) -> bool {
    c.chars().count() == 1
        && c.chars()
            .next()
            .is_some_and(|ch| PUNCTUATION_CHARS.contains(ch))
}

/// Normalize precomposed Thai/Lao AM vowels to their compatibility
/// decomposition, avoiding stale-cell artifacts in some terminals'
/// differential repaint during streaming.
pub fn normalize_terminal_output(s: &str) -> String {
    if !s.contains('\u{0e33}') && !s.contains('\u{0eb3}') {
        return s.to_string();
    }
    s.chars()
        .flat_map(|c| match c {
            '\u{0e33}' => vec!['\u{0e4d}', '\u{0e32}'],
            '\u{0eb3}' => vec!['\u{0ecd}', '\u{0eb2}'],
            other => vec![other],
        })
        .collect()
}

/// Remove escape sequences (SGR, OSC 8, APC markers), keeping the visible text
/// (Node's `stripVTControlCharacters`).
pub fn strip_vt_control_characters(s: &str) -> String {
    if !s.contains('\x1b') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    for run in ansi_runs(s) {
        if let AnsiRun::Text(visible) = run {
            out.push_str(visible);
        }
    }
    out
}

/// Apply a background-color function to `line`, padding it to `width` first.
pub fn apply_background_to_line(
    line: &str,
    width: usize,
    bg_fn: impl Fn(&str) -> String,
) -> String {
    let opener = if line.contains("\x1b[") {
        band_opener(&bg_fn)
    } else {
        String::new()
    };
    apply_background_with_opener(line, width, &opener, bg_fn)
}

/// The background opener of a band: the sequence `bg_fn` puts before its text.
/// Compute it once per band and pass it to [`apply_background_with_opener`].
pub fn band_opener(bg_fn: &impl Fn(&str) -> String) -> String {
    let sentinel = "\u{0}";
    let painted = bg_fn(sentinel);
    painted.split(sentinel).next().unwrap_or("").to_string()
}

/// [`apply_background_to_line`] with the band's opener already computed by
/// [`band_opener`].
pub fn apply_background_with_opener(
    line: &str,
    width: usize,
    opener: &str,
    bg_fn: impl Fn(&str) -> String,
) -> String {
    let visible_len = visible_width(line);
    let padding = " ".repeat(width.saturating_sub(visible_len));
    let repaired = repair_nested_bg_resets(line, opener);
    bg_fn(&format!("{repaired}{padding}"))
}

/// Whether an SGR sequence's parameters put the background back to the
/// terminal's own: `49`, or `0` (also spelled `ESC[m` or an empty parameter),
/// alone or inside a compound sequence.
fn clears_background(params: &str) -> bool {
    if params.is_empty() {
        return true;
    }
    params
        .split(';')
        .any(|p| p.is_empty() || p.parse::<u64>().is_ok_and(|n| n == 0 || n == 49))
}

/// Re-open the band's background after anything inside the line closed it
/// (`repairNestedBgResets`). A child that paints and closes its own fill, or
/// ends with a full `ESC[0m`, would otherwise leave a hole from there to the
/// end of the row. The opener is recovered from `bg_fn` itself; only the
/// background is restored.
fn repair_nested_bg_resets(line: &str, opener: &str) -> String {
    if !line.contains("\x1b[") {
        return line.to_string();
    }
    if opener.is_empty() {
        return line.to_string();
    }
    // `/\x1b\[([0-9;]*)m/g`, scanning the original line only. This does not use
    // `ansi_runs`: it matches SGR-shaped text anywhere, including inside an OSC
    // payload or after a CSI that is not SGR, which the shared scanner skips.
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(pos) = rest.find("\x1b[") {
        out.push_str(crate::text_slice::prefix(rest, pos));
        let after = crate::text_slice::suffix_from(rest, pos + 2);
        let params_len = after
            .bytes()
            .take_while(|b| b.is_ascii_digit() || *b == b';')
            .count();
        if crate::text_slice::suffix_from(after, params_len).starts_with('m') {
            let params = crate::text_slice::prefix(after, params_len);
            out.push_str(crate::text_slice::range(
                rest,
                pos,
                pos + 2 + params_len + 1,
            ));
            if clears_background(params) {
                out.push_str(opener);
            }
            rest = crate::text_slice::suffix_from(after, params_len + 1);
        } else {
            out.push_str("\x1b[");
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

// ---------------------------------------------------------------------------
// Wrapping
// ---------------------------------------------------------------------------

fn split_into_tokens_with_ansi(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut pending_ansi = String::new();
    let mut in_whitespace = false;

    for run in ansi_runs(text) {
        let visible = match run {
            AnsiRun::Ansi(code) => {
                pending_ansi.push_str(code);
                continue;
            }
            AnsiRun::Text(visible) => visible,
        };
        for ch in visible.chars() {
            let char_is_space = ch == ' ';

            if char_is_space != in_whitespace && !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }

            if !pending_ansi.is_empty() {
                current.push_str(&pending_ansi);
                pending_ansi.clear();
            }

            in_whitespace = char_is_space;
            current.push(ch);
        }
    }

    if !pending_ansi.is_empty() {
        current.push_str(&pending_ansi);
    }
    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

fn break_long_word(word: &str, width: usize, tracker: &mut AnsiCodeTracker) -> Vec<String> {
    enum Seg<'a> {
        Ansi(&'a str),
        Grapheme(&'a str),
    }

    let mut segments = Vec::new();
    for run in ansi_runs(word) {
        match run {
            AnsiRun::Ansi(code) => segments.push(Seg::Ansi(code)),
            AnsiRun::Text(visible) => {
                segments.extend(visible.graphemes(true).map(Seg::Grapheme));
            }
        }
    }

    let mut lines = Vec::new();
    let mut current_line = tracker.active_codes();
    let mut current_width = 0usize;

    for seg in segments {
        match seg {
            Seg::Ansi(code) => {
                current_line.push_str(code);
                tracker.process(code);
            }
            Seg::Grapheme(g) => {
                if g.is_empty() {
                    continue;
                }
                let w = grapheme_width(g);
                if current_width + w > width {
                    let reset = tracker.line_end_reset();
                    current_line.push_str(&reset);
                    lines.push(std::mem::take(&mut current_line));
                    current_line = tracker.active_codes();
                    current_width = 0;
                }
                current_line.push_str(g);
                current_width += w;
            }
        }
    }

    if !current_line.is_empty() {
        lines.push(current_line);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn wrap_single_line(line: &str, width: usize) -> Vec<String> {
    if line.is_empty() {
        return vec![String::new()];
    }
    if visible_width(line) <= width {
        return vec![line.to_string()];
    }

    let mut wrapped = Vec::new();
    let mut tracker = AnsiCodeTracker::new();
    let tokens = split_into_tokens_with_ansi(line);

    let mut current_line = String::new();
    let mut current_visible_len = 0usize;

    for token in &tokens {
        let token_visible_len = visible_width(token);
        let is_whitespace = token.trim().is_empty();

        if token_visible_len > width && !is_whitespace {
            if !current_line.is_empty() {
                let reset = tracker.line_end_reset();
                if !reset.is_empty() {
                    current_line.push_str(&reset);
                }
                wrapped.push(std::mem::take(&mut current_line));
            }
            let mut broken = break_long_word(token, width, &mut tracker);
            let last = broken.pop().unwrap_or_default();
            wrapped.extend(broken);
            current_visible_len = visible_width(&last);
            current_line = last;
            continue;
        }

        let total_needed = current_visible_len + token_visible_len;
        if total_needed > width && current_visible_len > 0 {
            let mut line_to_wrap = current_line.trim_end().to_string();
            let reset = tracker.line_end_reset();
            if !reset.is_empty() {
                line_to_wrap.push_str(&reset);
            }
            wrapped.push(line_to_wrap);
            if is_whitespace {
                current_line = tracker.active_codes();
                current_visible_len = 0;
            } else {
                current_line = tracker.active_codes() + token;
                current_visible_len = token_visible_len;
            }
        } else {
            current_line.push_str(token);
            current_visible_len += token_visible_len;
        }

        update_tracker_from_text(token, &mut tracker);
    }

    if !current_line.is_empty() {
        wrapped.push(current_line);
    }

    if wrapped.is_empty() {
        vec![String::new()]
    } else {
        wrapped
            .into_iter()
            .map(|l| l.trim_end().to_string())
            .collect()
    }
}

/// Word-wrap `text` (which may contain ANSI codes and newlines) to `width`
/// visible columns per line. Only wraps — no padding, no background colors.
/// Active ANSI codes are carried across wrapped and literal newline breaks.
pub fn wrap_text_with_ansi(text: &str, width: usize) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }

    let input_lines: Vec<&str> = text.split('\n').collect();
    let mut result = Vec::new();
    let mut tracker = AnsiCodeTracker::new();

    for input_line in input_lines {
        let prefix = if !result.is_empty() {
            tracker.active_codes()
        } else {
            String::new()
        };
        result.extend(wrap_single_line(&format!("{prefix}{input_line}"), width));
        update_tracker_from_text(input_line, &mut tracker);
    }

    if result.is_empty() {
        vec![String::new()]
    } else {
        result
    }
}

// ---------------------------------------------------------------------------
// Truncation
// ---------------------------------------------------------------------------

/// One piece of a line for width-aware walking: an ANSI code, a tab, or a
/// run of text containing neither.
enum Piece<'a> {
    Ansi(&'a str),
    Tab,
    Run(&'a str),
}

fn pieces(text: &str) -> Vec<Piece<'_>> {
    let mut out = Vec::new();
    for run in ansi_runs(text) {
        match run {
            AnsiRun::Ansi(code) => out.push(Piece::Ansi(code)),
            AnsiRun::Text(visible) => {
                for (k, part) in visible.split('\t').enumerate() {
                    if k > 0 {
                        out.push(Piece::Tab);
                    }
                    if !part.is_empty() {
                        out.push(Piece::Run(part));
                    }
                }
            }
        }
    }
    out
}

fn truncate_fragment_to_width(text: &str, max_width: usize) -> (String, usize) {
    if max_width == 0 || text.is_empty() {
        return (String::new(), 0);
    }
    let mut result = String::new();
    let mut width = 0;
    let mut pending_ansi = String::new();
    for piece in pieces(text) {
        match piece {
            Piece::Ansi(code) => pending_ansi.push_str(code),
            Piece::Tab => {
                if width + 3 > max_width {
                    break;
                }
                result.push_str(&std::mem::take(&mut pending_ansi));
                result.push('\t');
                width += 3;
            }
            Piece::Run(run) => {
                for g in run.graphemes(true) {
                    let w = grapheme_width(g);
                    if width + w > max_width {
                        return (result, width);
                    }
                    result.push_str(&std::mem::take(&mut pending_ansi));
                    result.push_str(g);
                    width += w;
                }
            }
        }
    }
    (result, width)
}

fn finalize_truncated(
    prefix: &str,
    prefix_width: usize,
    ellipsis: &str,
    ellipsis_width: usize,
    max_width: usize,
    pad: bool,
) -> String {
    const RESET: &str = "\x1b[0m";
    let visible_width = prefix_width + ellipsis_width;
    let mut result = if ellipsis.is_empty() {
        format!("{prefix}{RESET}")
    } else {
        format!("{prefix}{RESET}{ellipsis}{RESET}")
    };
    if pad {
        result.push_str(&" ".repeat(max_width.saturating_sub(visible_width)));
    }
    result
}

/// Truncate `text` (which may contain ANSI codes) to at most `max_width`
/// visible columns, appending `ellipsis` when truncated. If `pad` is set,
/// the result is padded with spaces to exactly `max_width`.
pub fn truncate_to_width(text: &str, max_width: usize, ellipsis: &str, pad: bool) -> String {
    if max_width == 0 {
        return String::new();
    }
    if text.is_empty() {
        return if pad {
            " ".repeat(max_width)
        } else {
            String::new()
        };
    }

    let ellipsis_width = visible_width(ellipsis);
    if ellipsis_width >= max_width {
        let text_width = visible_width(text);
        if text_width <= max_width {
            return if pad {
                format!("{text}{}", " ".repeat(max_width - text_width))
            } else {
                text.to_string()
            };
        }
        let (clipped_text, clipped_width) = truncate_fragment_to_width(ellipsis, max_width);
        if clipped_width == 0 {
            return if pad {
                " ".repeat(max_width)
            } else {
                String::new()
            };
        }
        return finalize_truncated("", 0, &clipped_text, clipped_width, max_width, pad);
    }

    let target_width = max_width - ellipsis_width;
    let mut result = String::new();
    let mut pending_ansi = String::new();
    let mut visible_so_far = 0usize;
    let mut kept_width = 0usize;
    let mut keep_contiguous_prefix = true;
    let mut overflowed = false;

    'walk: for piece in pieces(text) {
        match piece {
            Piece::Ansi(code) => pending_ansi.push_str(code),
            Piece::Tab => {
                if keep_contiguous_prefix && kept_width + 3 <= target_width {
                    result.push_str(&std::mem::take(&mut pending_ansi));
                    result.push('\t');
                    kept_width += 3;
                } else {
                    keep_contiguous_prefix = false;
                    pending_ansi.clear();
                }
                visible_so_far += 3;
                if visible_so_far > max_width {
                    overflowed = true;
                    break 'walk;
                }
            }
            Piece::Run(run) => {
                for g in run.graphemes(true) {
                    let w = grapheme_width(g);
                    if keep_contiguous_prefix && kept_width + w <= target_width {
                        result.push_str(&std::mem::take(&mut pending_ansi));
                        result.push_str(g);
                        kept_width += w;
                    } else {
                        keep_contiguous_prefix = false;
                        pending_ansi.clear();
                    }
                    visible_so_far += w;
                    if visible_so_far > max_width {
                        overflowed = true;
                        break 'walk;
                    }
                }
            }
        }
    }

    if !overflowed {
        return if pad {
            format!(
                "{text}{}",
                " ".repeat(max_width.saturating_sub(visible_so_far))
            )
        } else {
            text.to_string()
        };
    }

    finalize_truncated(
        &result,
        kept_width,
        ellipsis,
        ellipsis_width,
        max_width,
        pad,
    )
}

/// Truncate `text` to at most `max_width` columns, ending with `…` when it is
/// cut. The cut falls between words when one fits; only a first word longer
/// than the width is cut inside. ANSI escapes take no columns and are never
/// split; a cut inside styled text closes the style with a reset.
pub fn truncate_at_word(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if visible_width(text) <= max_width {
        return text.to_string();
    }
    // Walk the visible characters that fit in one column less than the width.
    let budget = max_width - 1;
    let mut used = 0usize;
    // Byte offsets: just past the last visible character that fits, and the
    // last space that fits (a word break).
    let mut head_end = 0usize;
    let mut last_space: Option<usize> = None;
    let mut next_is_space = false;
    let mut i = 0usize;
    while i < text.len() {
        if let Some((_, len)) = extract_ansi_code(text, i) {
            i += len;
            continue;
        }
        let Some(ch) = crate::text_slice::suffix_from(text, i).chars().next() else {
            break;
        };
        let w = visible_width(ch.encode_utf8(&mut [0; 4]));
        if used + w > budget {
            next_is_space = ch == ' ';
            break;
        }
        used += w;
        if ch == ' ' {
            last_space = Some(i);
        }
        i += ch.len_utf8();
        head_end = i;
    }
    // The head is whole words when the next visible character is a space.
    let kept = if next_is_space {
        crate::text_slice::prefix(text, head_end)
    } else {
        match last_space {
            Some(space)
                if !strip_vt_control_characters(crate::text_slice::prefix(text, space))
                    .trim()
                    .is_empty() =>
            {
                crate::text_slice::prefix(text, space)
            }
            _ => crate::text_slice::prefix(text, head_end),
        }
    };
    let mut out = kept.trim_end().to_string();
    if out.contains('\x1b') {
        out.push_str("\x1b[0m");
    }
    out.push('…');
    out
}

// ---------------------------------------------------------------------------
// Column slicing
// ---------------------------------------------------------------------------

/// Extract a range of visible columns `[start_col, start_col+length)` from
/// `line`, along with the actual visible width extracted. If `strict`, a
/// wide grapheme that would straddle the range boundary is excluded rather
/// than clipped.
pub fn slice_with_width(
    line: &str,
    start_col: usize,
    length: usize,
    strict: bool,
) -> (String, usize) {
    if length == 0 {
        return (String::new(), 0);
    }
    let end_col = start_col + length;
    let mut result = String::new();
    let mut result_width = 0usize;
    let mut current_col = 0usize;
    let mut pending_ansi = String::new();

    for run in ansi_runs(line) {
        match run {
            AnsiRun::Ansi(code) => {
                if current_col >= start_col && current_col < end_col {
                    result.push_str(code);
                } else if current_col < start_col {
                    pending_ansi.push_str(code);
                }
            }
            AnsiRun::Text(visible) => {
                for g in visible.graphemes(true) {
                    let w = grapheme_width(g);
                    let in_range = current_col >= start_col && current_col < end_col;
                    let fits = !strict || current_col + w <= end_col;
                    if in_range && fits {
                        if !pending_ansi.is_empty() {
                            result.push_str(&pending_ansi);
                            pending_ansi.clear();
                        }
                        result.push_str(g);
                        result_width += w;
                    }
                    current_col += w;
                    if current_col >= end_col {
                        break;
                    }
                }
                if current_col >= end_col {
                    break;
                }
            }
        }
    }

    (result, result_width)
}

/// Like [`slice_with_width`] but returns only the extracted text.
pub fn slice_by_column(line: &str, start_col: usize, length: usize, strict: bool) -> String {
    slice_with_width(line, start_col, length, strict).0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_whitespace_char() {
        assert!(is_whitespace_char(" "));
        assert!(!is_whitespace_char("a"));
    }

    #[test]
    fn test_is_punctuation_char() {
        assert!(is_punctuation_char("."));
        assert!(is_punctuation_char("("));
        assert!(!is_punctuation_char("a"));
    }

    #[test]
    fn test_normalize_terminal_output_noop() {
        assert_eq!(normalize_terminal_output("hello"), "hello");
    }

    #[test]
    fn test_normalize_terminal_output_thai_am() {
        let out = normalize_terminal_output("\u{0e33}");
        assert_eq!(out, "\u{0e4d}\u{0e32}");
    }

    #[test]
    fn test_apply_background_to_line_pads() {
        let out = apply_background_to_line("hi", 5, |s| format!("[{s}]"));
        assert_eq!(out, "[hi   ]");
    }

    #[test]
    fn test_band_opener_matches_per_line_repair() {
        let band = |s: &str| format!("\x1b[44m{s}\x1b[49m");
        let opener = band_opener(&band);
        assert_eq!(opener, "\x1b[44m");
        // A nested full reset inside the line is repaired with the band's opener,
        // exactly as the per-line function does.
        for line in ["ab\x1b[0mcd", "a\x1b[49mb", "plain", "x\x1b[31my\x1b[39m"] {
            assert_eq!(
                apply_background_with_opener(line, 8, &opener, band),
                apply_background_to_line(line, 8, band),
                "line {line:?}"
            );
        }
        assert_eq!(
            apply_background_with_opener("ab\x1b[0mcd", 6, &opener, band),
            "\x1b[44mab\x1b[0m\x1b[44mcd  \x1b[49m"
        );
    }

    #[test]
    fn test_wrap_text_with_ansi_short_line_unchanged() {
        assert_eq!(wrap_text_with_ansi("hello", 20), vec!["hello".to_string()]);
    }

    #[test]
    fn test_wrap_text_with_ansi_wraps_on_word_boundary() {
        let wrapped = wrap_text_with_ansi("hello world foo", 5);
        assert_eq!(wrapped, vec!["hello", "world", "foo"]);
    }

    #[test]
    fn test_wrap_text_with_ansi_preserves_newlines() {
        let wrapped = wrap_text_with_ansi("a\nb", 20);
        assert_eq!(wrapped, vec!["a", "b"]);
    }

    #[test]
    fn test_wrap_text_with_ansi_breaks_long_word() {
        let wrapped = wrap_text_with_ansi("abcdefgh", 3);
        assert_eq!(wrapped, vec!["abc", "def", "gh"]);
    }

    #[test]
    fn test_wrap_text_with_ansi_carries_style_across_lines() {
        let wrapped = wrap_text_with_ansi("\x1b[1mhello world\x1b[0m", 5);
        assert!(
            wrapped[1].starts_with("\x1b[1m"),
            "continuation line should re-open bold: {:?}",
            wrapped[1]
        );
    }

    #[test]
    fn test_truncate_to_width_no_truncation_needed() {
        assert_eq!(truncate_to_width("hi", 10, "...", false), "hi");
    }

    #[test]
    fn test_truncate_to_width_basic() {
        assert_eq!(
            truncate_to_width("hello world", 8, "...", false),
            "hello\x1b[0m...\x1b[0m"
        );
    }

    #[test]
    fn test_truncate_to_width_pad() {
        let out = truncate_to_width("hi", 5, "", true);
        assert_eq!(out, "hi   ");
    }

    #[test]
    fn test_truncate_to_width_pad_when_truncated() {
        let out = truncate_to_width("hello world", 8, "", true);
        assert_eq!(out, "hello wo\x1b[0m");
    }

    #[test]
    fn test_truncate_to_width_zero_width() {
        assert_eq!(truncate_to_width("hello", 0, "...", false), "");
    }

    #[test]
    fn test_slice_by_column_basic() {
        assert_eq!(slice_by_column("hello world", 6, 5, false), "world");
    }

    #[test]
    fn test_slice_by_column_with_ansi() {
        let sliced = slice_by_column("\x1b[1mhello\x1b[0m world", 0, 5, false);
        assert!(sliced.starts_with("\x1b[1m"));
        assert!(sliced.contains("hello"));
    }

    #[test]
    fn test_slice_with_width_reports_width() {
        let (text, width) = slice_with_width("hello", 0, 3, false);
        assert_eq!(text, "hel");
        assert_eq!(width, 3);
    }
}

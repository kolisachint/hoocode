//! `components/diff.ts`: a unified-ish diff (`+12 line`, `-12 line`,
//! ` 12 line`): the sign coloured, the rest dim.

use hoocode_code_tui_theme::theme;
use hoocode_tui_util::js_regex::is_js_space;

use crate::render_utils::replace_tabs;

struct DiffLine<'a> {
    prefix: char,
    line_num: &'a str,
    content: &'a str,
}

/// `parseDiffLine`: `/^([+-\s])(\s*\d*)\s(.*)$/`.
fn parse_diff_line(line: &str) -> Option<DiffLine<'_>> {
    let mut chars = line.char_indices();
    let (_, prefix) = chars.next()?;
    if !(prefix == '+' || prefix == '-' || is_js_space(prefix)) {
        return None;
    }
    let rest = hoocode_tui_util::text_slice::suffix_from(line, prefix.len_utf8());
    // `(\s*\d*)\s(.*)`: the greedy groups backtrack until a whitespace follows.
    let spaces = rest.len() - rest.trim_start_matches(is_js_space).len();
    let digits = hoocode_tui_util::text_slice::suffix_from(rest, spaces)
        .bytes()
        .take_while(u8::is_ascii_digit)
        .count();
    let mut num_end = spaces + digits;
    loop {
        let after = hoocode_tui_util::text_slice::suffix_from(rest, num_end);
        if let Some(c) = after
            .chars()
            .next()
            .filter(|c| is_js_space(*c) && *c != '\n')
        {
            // `.` excludes line terminators; content must not contain them.
            let content = hoocode_tui_util::text_slice::suffix_from(after, c.len_utf8());
            if !content.contains(['\n', '\r', '\u{2028}', '\u{2029}']) {
                return Some(DiffLine {
                    prefix,
                    line_num: hoocode_tui_util::text_slice::prefix(rest, num_end),
                    content,
                });
            }
        }
        if num_end == 0 {
            return None;
        }
        // Give back one character of the number group and retry.
        let back = hoocode_tui_util::text_slice::prefix(rest, num_end)
            .chars()
            .next_back()
            .unwrap()
            .len_utf8();
        num_end -= back;
    }
}

/// One diff row: the sign in its own colour, the rest of the row dim.
fn diff_row(sign: char, token: &str, line_num: &str, content: &str) -> String {
    let t = theme();
    format!(
        "{}{}",
        t.fg(token, &sign.to_string()),
        t.fg("dim", &format!("{line_num} {}", replace_tabs(content)))
    )
}

/// `renderDiff`: a unified-ish diff (`+12 line`, `-12 line`, ` 12 line`). Only
/// the sign carries colour (`-` red, `+` green); the line number and the text
/// are dim, so a diff reads as quiet as any other tool body.
pub fn render_diff(diff_text: &str) -> String {
    let t = theme();
    diff_text
        .split('\n')
        .map(|line| match parse_diff_line(line) {
            None => t.fg("dim", line),
            Some(parsed) => match parsed.prefix {
                '-' => diff_row('-', "toolDiffRemoved", parsed.line_num, parsed.content),
                '+' => diff_row('+', "toolDiffAdded", parsed.line_num, parsed.content),
                _ => t.fg(
                    "dim",
                    &format!(" {} {}", parsed.line_num, replace_tabs(parsed.content)),
                ),
            },
        })
        .collect::<Vec<_>>()
        .join("\n")
}

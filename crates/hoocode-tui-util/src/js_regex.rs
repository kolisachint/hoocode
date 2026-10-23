//! JavaScript regular expressions on top of `fancy-regex`.
//!
//! `marked`'s rules are JavaScript regex sources; this module translates such
//! a source into the Rust dialect so the lexer can run the rules verbatim
//! (`rules.rs` holds the sources exactly as the pinned `marked` compiles
//! them). The translation keeps JavaScript semantics where the dialects
//! disagree:
//! - `\d`, `\w`, `\b` are ASCII-only in JavaScript (Unicode in Rust);
//! - `\s` is JavaScript's whitespace set (which differs from Unicode
//!   `White_Space` in U+FEFF and U+0085);
//! - `.` excludes `\r`, U+2028 and U+2029 besides `\n`;
//! - a `[` inside a character class is a literal (a nested class in Rust),
//!   and `&&`/`--`/`~~` are not set operators;
//! - a `{` that does not start a quantifier is a literal.
//!
//! Positions are byte offsets into the haystack; callers that mirror
//! JavaScript's UTF-16 arithmetic convert at the edges.

use fancy_regex::{Regex, RegexBuilder};

/// JavaScript's `\s` class body (without brackets).
const JS_SPACE: &str = r"\t\n\x0B\x0C\r \x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}";
const JS_WORD: &str = "0-9A-Za-z_";

/// Whether `c` is whitespace for JavaScript's `\s` / `String.prototype.trim`.
pub fn is_js_space(c: char) -> bool {
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

/// `String.prototype.trim`.
pub fn js_trim(s: &str) -> &str {
    s.trim_matches(is_js_space)
}

/// `String.prototype.trimEnd`.
pub fn js_trim_end(s: &str) -> &str {
    s.trim_end_matches(is_js_space)
}

/// [`is_js_space`] for a UTF-16 code unit (every code point in the set is a single unit).
pub fn is_js_space_unit(u: u16) -> bool {
    matches!(
        u,
        0x09..=0x0d
            | 0x20
            | 0xa0
            | 0x1680
            | 0x2000..=0x200a
            | 0x2028
            | 0x2029
            | 0x202f
            | 0x205f
            | 0x3000
            | 0xfeff
    )
}

/// `String.prototype.trim` over UTF-16 code units.
pub fn js_trim_units(units: &[u16]) -> &[u16] {
    let start = units.iter().take_while(|&&u| is_js_space_unit(u)).count();
    let end = units.len()
        - units[start..]
            .iter()
            .rev()
            .take_while(|&&u| is_js_space_unit(u))
            .count();
    &units[start..end]
}

/// The byte offset of UTF-16 index `idx` in `s` (clamped to the end; an index
/// inside a surrogate pair rounds up to the next character).
pub fn utf16_to_byte(s: &str, idx: usize) -> usize {
    let mut units = 0;
    for (b, c) in s.char_indices() {
        if units >= idx {
            return b;
        }
        units += c.len_utf16();
    }
    s.len()
}

/// The UTF-16 index of byte offset `byte` in `s`.
pub fn byte_to_utf16(s: &str, byte: usize) -> usize {
    crate::text_slice::prefix(s, byte)
        .chars()
        .map(char::len_utf16)
        .sum()
}

/// `s.slice(start)` with a UTF-16 start index.
pub fn js_slice_from(s: &str, start: usize) -> &str {
    crate::text_slice::suffix_from(s, utf16_to_byte(s, start))
}

/// Translate a JavaScript regex source (with its flags) to the Rust dialect.
pub fn translate(source: &str, flags: &str) -> String {
    let mut out = String::with_capacity(source.len() * 2);
    if flags.contains('i') {
        out.push_str("(?i)");
    }
    let multiline = flags.contains('m');
    if multiline {
        out.push_str("(?m)");
    }
    let chars: Vec<char> = source.chars().collect();
    let mut i = 0;
    let mut in_class = false;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() {
            let n = chars[i + 1];
            i += 2;
            match n {
                'd' => out.push_str(if in_class { "0-9" } else { "[0-9]" }),
                'D' => out.push_str("[^0-9]"),
                'w' => {
                    if in_class {
                        out.push_str(JS_WORD)
                    } else {
                        out.push_str("[0-9A-Za-z_]")
                    }
                }
                'W' => out.push_str("[^0-9A-Za-z_]"),
                's' => {
                    if in_class {
                        out.push_str(JS_SPACE);
                    } else {
                        out.push('[');
                        out.push_str(JS_SPACE);
                        out.push(']');
                    }
                }
                'S' => {
                    out.push_str("[^");
                    out.push_str(JS_SPACE);
                    out.push(']');
                }
                'b' if !in_class => out.push_str(
                    "(?:(?<=[0-9A-Za-z_])(?![0-9A-Za-z_])|(?<![0-9A-Za-z_])(?=[0-9A-Za-z_]))",
                ),
                'B' if !in_class => out.push_str(
                    "(?:(?<=[0-9A-Za-z_])(?=[0-9A-Za-z_])|(?<![0-9A-Za-z_])(?![0-9A-Za-z_]))",
                ),
                'b' => out.push_str(r"\x08"),
                'n' | 't' | 'r' | 'f' | 'v' => {
                    out.push('\\');
                    out.push(if n == 'v' { 'x' } else { n });
                    if n == 'v' {
                        out.push_str("0B");
                    }
                }
                // A malformed escape (too few characters left, no closing brace) is an
                // identity escape, as in JS Annex B. It must not index past the end: the
                // pattern comes from `try_new`, which reports bad input as an `Err`.
                'x' if i + 1 < chars.len() => {
                    out.push_str(r"\x");
                    out.push(chars[i]);
                    out.push(chars[i + 1]);
                    i += 2;
                }
                'u' if chars.get(i) == Some(&'{') && chars[i..].contains(&'}') => {
                    let end = chars[i..].iter().position(|&c| c == '}').unwrap() + i;
                    out.push_str(r"\x{");
                    out.extend(&chars[i + 1..end]);
                    out.push('}');
                    i = end + 1;
                }
                'u' if chars.get(i) != Some(&'{') && i + 4 <= chars.len() => {
                    out.push_str(r"\x{");
                    out.extend(&chars[i..i + 4]);
                    out.push('}');
                    i += 4;
                }
                'p' | 'P' if chars[i..].contains(&'}') => {
                    out.push('\\');
                    out.push(n);
                    let end = chars[i..].iter().position(|&c| c == '}').unwrap() + i;
                    out.extend(&chars[i..=end]);
                    i = end + 1;
                }
                '0' => out.push_str(r"\x00"),
                '1'..='9' if !in_class => {
                    out.push('\\');
                    out.push(n);
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        out.push(chars[i]);
                        i += 1;
                    }
                }
                _ => push_literal(&mut out, n, in_class),
            }
            continue;
        }
        i += 1;
        if in_class {
            match c {
                ']' => {
                    in_class = false;
                    out.push(']');
                }
                '-' => out.push('-'),
                _ => push_literal(&mut out, c, true),
            }
            continue;
        }
        match c {
            '[' => {
                in_class = true;
                out.push('[');
                if chars.get(i) == Some(&'^') {
                    out.push('^');
                    i += 1;
                }
                if chars.get(i) == Some(&']') {
                    // `[]` never matches, `[^]` matches anything.
                    let negated = out.ends_with('^');
                    out.truncate(out.len() - if negated { 2 } else { 1 });
                    out.push_str(if negated { r"[\s\S]" } else { r"[^\s\S]" });
                    in_class = false;
                    i += 1;
                }
            }
            '.' => out.push_str(r"[^\n\r\x{2028}\x{2029}]"),
            // JS line terminators include `\r`, U+2028 and U+2029.
            '^' if multiline => out.push_str(r"(?:^|(?<=[\r\x{2028}\x{2029}]))"),
            '$' if multiline => out.push_str(r"(?:$|(?=[\r\x{2028}\x{2029}]))"),
            '{' => {
                let rest: String = chars[i..].iter().take_while(|&&c| c != '}').collect();
                let closes = chars.get(i + rest.len()) == Some(&'}');
                let is_quant = closes && {
                    let mut parts = rest.splitn(2, ',');
                    let a = parts.next().unwrap_or("");
                    let b = parts.next();
                    !a.is_empty()
                        && a.chars().all(|c| c.is_ascii_digit())
                        && b.is_none_or(|b| b.chars().all(|c| c.is_ascii_digit()))
                };
                if is_quant {
                    out.push('{');
                } else {
                    out.push_str(r"\{");
                }
            }
            '}' => {
                // A `}` closing a quantifier follows digits opened by `{`.
                let before: String = out.chars().rev().take_while(|&c| c != '{').collect();
                let opened = out.len() > before.len()
                    && crate::text_slice::prefix(&out, out.len() - before.len()).ends_with('{')
                    && !crate::text_slice::prefix(&out, out.len() - before.len()).ends_with(r"\{")
                    && before.chars().all(|c| c.is_ascii_digit() || c == ',');
                if opened {
                    out.push('}');
                } else {
                    out.push_str(r"\}");
                }
            }
            _ => out.push(c),
        }
    }
    out
}

fn push_literal(out: &mut String, c: char, in_class: bool) {
    let meta = if in_class {
        matches!(c, '[' | ']' | '\\' | '^' | '-' | '&' | '~' | '|')
    } else {
        matches!(
            c,
            '\\' | '.'
                | '+'
                | '*'
                | '?'
                | '('
                | ')'
                | '|'
                | '['
                | ']'
                | '{'
                | '}'
                | '^'
                | '$'
                | '#'
                | '&'
                | '-'
                | '~'
        )
    };
    if meta {
        out.push('\\');
        out.push(c);
    } else if c == ' ' {
        out.push(' ');
    } else if c.is_ascii_alphanumeric() {
        // An identity escape of a letter (`\q`) means the letter itself.
        out.push(c);
    } else {
        // Punctuation that is not meta in either context is safe raw.
        out.push(c);
    }
}

/// `translated` with look-arounds removed and back-references made "anything":
/// a regex that accepts at least what `translated` accepts, built on the fast
/// engine. `None` when there is nothing to loosen (the pattern is not fancy) or
/// the pattern uses something this does not model.
fn loosen(translated: &str) -> Option<String> {
    let chars: Vec<char> = translated.chars().collect();
    let mut out = String::with_capacity(translated.len());
    let mut changed = false;
    let mut in_class = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' {
            let n = *chars.get(i + 1)?;
            if matches!(n, 'K' | 'G' | 'k') {
                return None;
            }
            if !in_class && n.is_ascii_digit() && n != '0' {
                i += 1;
                while chars.get(i).is_some_and(char::is_ascii_digit) {
                    i += 1;
                }
                out.push_str(r"[\s\S]*");
                changed = true;
                continue;
            }
            out.push(c);
            out.push(n);
            i += 2;
            continue;
        }
        if in_class {
            in_class = c != ']';
            out.push(c);
            i += 1;
            continue;
        }
        if c == '[' {
            in_class = true;
            out.push(c);
            i += 1;
            // A leading `^` and `]` are part of the class, not its end.
            if chars.get(i) == Some(&'^') {
                out.push('^');
                i += 1;
            }
            if chars.get(i) == Some(&']') {
                out.push(']');
                i += 1;
            }
            continue;
        }
        if c == '(' && chars.get(i + 1) == Some(&'?') {
            let rest: String = chars[i + 2..].iter().take(2).collect();
            let look =
                rest.starts_with('=') || rest.starts_with('!') || rest == "<=" || rest == "<!";
            if rest.starts_with('>') || rest.starts_with('(') || rest.starts_with("P=") {
                return None;
            }
            if look {
                i = skip_group(&chars, i)?;
                changed = true;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    changed.then_some(out)
}

/// The index just past the group that opens at `open` (a `(`).
fn skip_group(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut in_class = false;
    let mut i = open;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 1,
            ']' if in_class => in_class = false,
            '[' if !in_class => {
                in_class = true;
                if chars.get(i + 1) == Some(&'^') {
                    i += 1;
                }
                if chars.get(i + 1) == Some(&']') {
                    i += 1;
                }
            }
            '(' if !in_class => depth += 1,
            ')' if !in_class => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// A compiled JavaScript regex.
#[derive(Debug)]
pub struct JsRegex {
    re: Regex,
    /// A cheaper regex that matches wherever `re` does (and maybe more): `re` with
    /// its look-arounds and back-references loosened. Present only when `re` has
    /// any, which makes `fancy-regex` run its backtracking VM, whose failed search
    /// walks the whole haystack (a `^` rule tried on the rest of a document is
    /// then linear per call). A miss here is a miss for `re`.
    prefilter: Box<Prefilter>,
}

/// The translated source and, once a long haystack asks for it, the loosened regex.
#[derive(Debug)]
struct Prefilter {
    translated: String,
    regex: std::sync::OnceLock<Option<Regex>>,
}

/// Haystacks shorter than this are searched by the VM directly: the scan is
/// cheap, and building the prefilter is not.
const PREFILTER_MIN_HAYSTACK: usize = 2048;

static PREFILTER_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Turn the prefilter off or on for the whole process. It changes speed, never
/// results; tests switch it to compare the two.
pub fn set_prefilter(on: bool) {
    PREFILTER_ON.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// The captures of one match, as byte ranges into the haystack.
#[derive(Debug, Clone)]
pub struct Match<'h> {
    hay: &'h str,
    groups: Vec<Option<(usize, usize)>>,
}

impl<'h> Match<'h> {
    /// Byte offset of the match start.
    pub fn index(&self) -> usize {
        self.groups[0].unwrap().0
    }

    /// Byte offset just past the match.
    pub fn end(&self) -> usize {
        self.groups[0].unwrap().1
    }

    /// Group `i`, or `None` when it did not participate (JS `undefined`).
    pub fn get(&self, i: usize) -> Option<&'h str> {
        self.groups
            .get(i)
            .copied()
            .flatten()
            .map(|(a, b)| crate::text_slice::range(self.hay, a, b))
    }

    /// Group `i` with JavaScript truthiness: `Some` only for a non-empty capture.
    pub fn truthy(&self, i: usize) -> Option<&'h str> {
        self.get(i).filter(|s| !s.is_empty())
    }

    /// Number of groups, including group 0.
    pub fn len(&self) -> usize {
        self.groups.len()
    }

    /// Always false: a match has group 0.
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    /// Byte range of group `i`, if it participated.
    pub fn range(&self, i: usize) -> Option<(usize, usize)> {
        self.groups.get(i).copied().flatten()
    }

    /// The whole match.
    pub fn whole(&self) -> &'h str {
        self.get(0).unwrap()
    }
}

impl JsRegex {
    pub fn new(source: &str, flags: &str) -> Self {
        Self::try_new(source, flags).unwrap_or_else(|e| panic!("{e}"))
    }

    /// Compile, reporting a source the translation cannot express.
    pub fn try_new(source: &str, flags: &str) -> Result<Self, String> {
        let translated = translate(source, flags);
        let re = RegexBuilder::new(&translated)
            .backtrack_limit(50_000_000)
            .build()
            .map_err(|e| format!("bad regex {source:?} -> {translated:?}: {e}"))?;
        let prefilter = Box::new(Prefilter {
            translated,
            regex: std::sync::OnceLock::new(),
        });
        Ok(Self { re, prefilter })
    }

    /// `regex.exec(hay)` for a non-global regex.
    pub fn exec<'h>(&self, hay: &'h str) -> Option<Match<'h>> {
        self.exec_at(hay, 0)
    }

    /// `exec` of a global regex whose `lastIndex` is `pos` (a byte offset).
    pub fn exec_at<'h>(&self, hay: &'h str, pos: usize) -> Option<Match<'h>> {
        // fancy-regex steps by the lead byte's length from `pos`, so a `pos` inside a
        // character walks past the end (and panics in regex-automata). Round up to the
        // next boundary; past the end there is no match (JS `lastIndex > length`).
        let pos = (pos..=hay.len()).find(|&i| hay.is_char_boundary(i))?;
        if hay.len() - pos >= PREFILTER_MIN_HAYSTACK
            && PREFILTER_ON.load(std::sync::atomic::Ordering::Relaxed)
        {
            let filter = self.prefilter.regex.get_or_init(|| {
                loosen(&self.prefilter.translated).and_then(|loose| Regex::new(&loose).ok())
            });
            if let Some(filter) = filter {
                if matches!(filter.find_from_pos(hay, pos), Ok(None)) {
                    return None;
                }
            }
        }
        // A backtracking-limit error counts as no match, where JavaScript
        // would keep going: only pathological input gets here.
        let caps = self.re.captures_from_pos(hay, pos).ok().flatten()?;
        let groups = (0..caps.len())
            .map(|i| caps.get(i).map(|m| (m.start(), m.end())))
            .collect();
        Some(Match { hay, groups })
    }

    /// Number of capture groups (JavaScript's `exec('').length - 1` on an
    /// always-matching variant).
    pub fn captures_len(&self) -> usize {
        self.re.captures_len() - 1
    }

    /// `regex.test(hay)`.
    pub fn test(&self, hay: &str) -> bool {
        self.re.is_match(hay).unwrap_or(false)
    }

    /// `hay.search(regex)` as a UTF-16 index, `None` when absent.
    pub fn search(&self, hay: &str) -> Option<usize> {
        let m = self.re.find(hay).ok().flatten()?;
        Some(byte_to_utf16(hay, m.start()))
    }

    /// `hay.replace(regex, f)`; `global` replaces every match.
    pub fn replace_with(
        &self,
        hay: &str,
        global: bool,
        mut f: impl FnMut(&Match) -> String,
    ) -> String {
        let mut out = String::with_capacity(hay.len());
        let mut last = 0;
        let mut pos = 0;
        while pos <= hay.len() {
            let Some(m) = self.exec_at(hay, pos) else {
                break;
            };
            out.push_str(crate::text_slice::range(hay, last, m.index()));
            out.push_str(&f(&m));
            last = m.end();
            pos = if m.end() == m.index() {
                // Step over an empty match by one character.
                match crate::text_slice::suffix_from(hay, m.end()).chars().next() {
                    Some(c) => m.end() + c.len_utf8(),
                    None => hay.len() + 1,
                }
            } else {
                m.end()
            };
            if !global {
                break;
            }
        }
        if last <= hay.len() {
            out.push_str(crate::text_slice::suffix_from(hay, last));
        }
        out
    }

    /// `hay.replace(regex, replacement)` where `replacement` may use `$1`..`$9`.
    pub fn replace(&self, hay: &str, global: bool, replacement: &str) -> String {
        self.replace_with(hay, global, |m| expand(replacement, m))
    }
}

fn expand(replacement: &str, m: &Match) -> String {
    let mut out = String::new();
    let bytes: Vec<char> = replacement.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == '$' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit() {
            let g = bytes[i + 1].to_digit(10).unwrap() as usize;
            out.push_str(m.get(g).unwrap_or(""));
            i += 2;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn js_trim_follows_the_javascript_whitespace_set() {
        // BOM is JavaScript whitespace; U+0085 is Unicode whitespace but not JavaScript's.
        assert_eq!(js_trim("\u{feff} a \u{feff}"), "a");
        assert_eq!(js_trim("\u{85}a\u{85}"), "\u{85}a\u{85}");
    }

    #[test]
    fn unit_whitespace_matches_the_char_whitespace_set() {
        for u in 0..=0xffff_u16 {
            let by_char = char::from_u32(u32::from(u)).is_some_and(is_js_space);
            assert_eq!(is_js_space_unit(u), by_char, "U+{u:04X}");
        }
        let units: Vec<u16> = "\u{feff} a\u{3000}".encode_utf16().collect();
        assert_eq!(String::from_utf16_lossy(js_trim_units(&units)), "a");
    }

    use super::*;

    #[test]
    fn translates_js_only_constructs() {
        assert_eq!(translate(r"[^[\]]", ""), r"[^\[\]]");
        assert_eq!(translate(r"a{2,}", ""), "a{2,}");
        assert_eq!(translate(r"[{|}]", ""), r"[{\|}]");
        assert_eq!(translate(r"\/", ""), "/");
    }

    #[test]
    fn js_word_boundaries_are_ascii() {
        let re = JsRegex::new(r"\b_", "");
        assert!(re.test(" _"));
        assert!(!re.test("a_"));
        // `é` is not an ASCII word character, so JavaScript sees a boundary.
        assert!(re.test("é_"));
        assert!(JsRegex::new(r"\d", "").exec("٣").is_none());
    }

    #[test]
    fn replaces_like_js() {
        let re = JsRegex::new(r"\\([\[\]])", "g");
        assert_eq!(re.replace(r"a\[b\]", true, "$1"), "a[b]");
    }

    // Fuzzer finding (fuzz/targets/js_regex.rs): `\u` with fewer than four characters
    // left panicked with an out-of-range slice.
    #[test]
    fn truncated_escapes_are_identity_escapes_not_panics() {
        for pattern in [
            r"\x", r"\x1", r"\u", r"\u12", r"\u{41", r"\p", r"\pL", r"\P{",
        ] {
            let _ = JsRegex::try_new(pattern, "");
        }
        assert!(JsRegex::new(r"\u12", "").test("u12"));
        assert!(JsRegex::new(r"\x1", "").test("x1"));
        assert!(JsRegex::new(r"\pL", "").test("pL"));
    }
}

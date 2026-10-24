//! The regex prefilter (`hoocode_tui_util::js_regex`) only skips searches that cannot
//! match: lexing a long document gives the same tokens with it on and off, and
//! the long document lexes in linear time.

use hoocode_tui_components::markdown::lexer::lex;
use hoocode_tui_util::js_regex::set_prefilter;

const SNIPPETS: &[&str] = &[
    "# Heading one\n\nA paragraph with *emphasis*, `code` and a [link](https://example.com).\n\n",
    "```rust\nfn main() {}\n\n```\n\n",
    "~~~\nplain fence\n~~~\n\n",
    "> quoted line\n> second\n\n",
    "- item one\n- item two\n  - nested\n\n1. first\n2. second\n\n",
    "| a | b |\n|---|---|\n| 1 | 2 |\n\n",
    "Setext heading\n==============\n\n",
    "---\n\n",
    "[ref]: https://example.com/ref \"Title\"\n\nSee [ref].\n\n",
    "<div>\nhtml block\n</div>\n\n",
    "    indented code\n\n",
    "word lorem ipsum word lorem ipsum\n\n",
];

fn document(copies: usize) -> String {
    let mut text = String::new();
    for _ in 0..copies {
        for snippet in SNIPPETS {
            text.push_str(snippet);
        }
    }
    text
}

#[test]
fn a_long_document_lexes_the_same_with_the_prefilter_off() {
    let text = document(12);
    assert!(text.len() > 4096);
    // Cut at many places: ends inside fences, tables, quotes and lists.
    let mut cuts: Vec<usize> = (0..text.len()).step_by(211).collect();
    cuts.push(text.len());
    for cut in cuts {
        if !text.is_char_boundary(cut) {
            continue;
        }
        let part = &text[..cut];
        set_prefilter(true);
        let on = format!("{:?}", lex(part).tokens);
        set_prefilter(false);
        let off = format!("{:?}", lex(part).tokens);
        set_prefilter(true);
        assert_eq!(on, off, "tokens differ for the first {cut} bytes");
    }
}

#[test]
fn lexing_scales_linearly_with_the_prefilter() {
    set_prefilter(true);
    let small = document(20);
    let large = document(80);
    lex(&small);
    let time = |text: &str| {
        let started = std::time::Instant::now();
        lex(text);
        started.elapsed()
    };
    let small_time = time(&small).max(std::time::Duration::from_millis(1));
    let large_time = time(&large);
    // 4x the text; quadratic growth would be about 16x. Generous: debug builds, loaded machines.
    assert!(
        large_time < small_time * 12,
        "lex time grew from {small_time:?} to {large_time:?} for 4x the text"
    );
}

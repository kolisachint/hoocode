//! Markdown-to-terminal rendering component, ported from `components/markdown.ts`.
//!
//! Parsing is a literal port of the pinned `marked` lexer (`lexer.rs`, over
//! `marked`'s own regex sources); rendering follows markdown.ts token by
//! token (`render.rs`).

pub use hoocode_tui_util::js_regex;
pub mod lexer;
mod render;
mod rules;
mod rules_gen;

pub use js_regex::js_trim;
pub use render::{DefaultTextStyle, HeadingFn, HighlightCodeFn, MarkdownTheme};

use std::collections::VecDeque;
use std::rc::Rc;

use hoocode_tui_images::is_image_line;
use hoocode_tui_render::{fresh_stamp, Component};
use hoocode_tui_util::{
    apply_background_with_opener, band_opener, visible_width, wrap_text_with_ansi,
};

use lexer::Token;
use render::Renderer;

/// Rendered widths kept per text (markdown.ts `LINE_CACHE_WIDTHS`).
const LINE_CACHE_WIDTHS: usize = 3;

pub struct Markdown {
    text: String,
    padding_x: usize,
    padding_y: usize,
    default_text_style: Option<DefaultTextStyle>,
    theme: MarkdownTheme,
    /// Rendered lines per width, for the text version in `line_cache_rev`.
    line_cache: VecDeque<(usize, Rc<[String]>)>,
    line_cache_rev: Option<u64>,
    /// The tokens of the text version `text_rev`.
    cached_tokens: Option<(u64, Vec<Token>)>,
    /// New whenever the text changes: the lexed tokens are for one of these.
    text_rev: u64,
    /// New whenever the text changes or the cache is invalidated: the lines
    /// kept for one of these are exact, and it is the render stamp.
    rev: u64,
}

impl Markdown {
    pub fn new(
        text: impl Into<String>,
        padding_x: usize,
        padding_y: usize,
        theme: MarkdownTheme,
        default_text_style: Option<DefaultTextStyle>,
    ) -> Self {
        Self {
            text: text.into(),
            padding_x,
            padding_y,
            default_text_style,
            theme,
            line_cache: VecDeque::new(),
            line_cache_rev: None,
            cached_tokens: None,
            text_rev: fresh_stamp(),
            rev: fresh_stamp(),
        }
    }

    /// Replace the text. The same text again keeps the lines and tokens.
    pub fn set_text(&mut self, text: impl Into<String>) {
        let text = text.into();
        if text == self.text {
            return;
        }
        self.text = text;
        self.text_rev = fresh_stamp();
        self.invalidate_lines();
    }

    /// Drop rendered lines (they bake in theme and width) but keep the lexed
    /// tokens, which depend on the text alone.
    fn invalidate_lines(&mut self) {
        self.line_cache.clear();
        self.line_cache_rev = None;
        self.rev = fresh_stamp();
    }

    fn store_lines(&mut self, width: usize, lines: Vec<String>) -> Rc<[String]> {
        let lines: Rc<[String]> = lines.into();
        if self.line_cache.len() >= LINE_CACHE_WIDTHS {
            self.line_cache.pop_front();
        }
        self.line_cache.push_back((width, lines.clone()));
        lines
    }
}

impl Component for Markdown {
    fn invalidate(&mut self) {
        self.invalidate_lines();
    }

    /// The output is a function of the text, the padding, the theme and the
    /// width: the version below changes with the first two (a theme switch
    /// goes through `invalidate`, or the container's render epoch).
    fn render_stamp(&self) -> Option<u64> {
        Some(self.rev)
    }

    fn render(&mut self, width: u16) -> Vec<String> {
        self.render_shared(width).to_vec()
    }

    fn render_shared(&mut self, width: u16) -> Rc<[String]> {
        let width = width as usize;
        // A version compare, not a compare of the text: a hit costs nothing
        // and hands out the kept lines as they are.
        if self.line_cache_rev == Some(self.rev) {
            if let Some((_, lines)) = self.line_cache.iter().find(|(w, _)| *w == width) {
                return lines.clone();
            }
        } else {
            self.line_cache.clear();
            self.line_cache_rev = Some(self.rev);
        }

        let content_width = width.saturating_sub(self.padding_x * 2).max(1);

        if js_regex::js_trim(&self.text).is_empty() {
            return self.store_lines(width, Vec::new());
        }

        if self.cached_tokens.as_ref().map(|(rev, _)| rev) != Some(&self.text_rev) {
            let tokens = lexer::lex(
                &self
                    .text
                    .replace('\t', &" ".repeat(hoocode_tui_util::RENDER_TAB_WIDTH)),
            )
            .tokens;
            self.cached_tokens = Some((self.text_rev, tokens));
        }
        let tokens = &self.cached_tokens.as_ref().unwrap().1;

        let renderer = Renderer {
            theme: &self.theme,
            default_style: self.default_text_style.as_ref(),
        };
        let rendered_lines = renderer.render_tokens(tokens, content_width);

        let mut wrapped_lines = Vec::new();
        for line in &rendered_lines {
            if is_image_line(line) {
                wrapped_lines.push(line.clone());
            } else {
                wrapped_lines.extend(wrap_text_with_ansi(line, content_width));
            }
        }

        let left_margin = " ".repeat(self.padding_x);
        let right_margin = " ".repeat(self.padding_x);
        let bg_fn = self
            .default_text_style
            .as_ref()
            .and_then(|s| s.bg_color.as_ref());
        let mut content_lines = Vec::new();
        // One band: every line below shares this opener.
        let band = bg_fn.map(|bg_fn| (band_opener(&|s: &str| bg_fn(s)), bg_fn));

        for line in &wrapped_lines {
            if is_image_line(line) {
                content_lines.push(line.clone());
                continue;
            }
            let line_with_margins = format!("{left_margin}{line}{right_margin}");
            if let Some((opener, bg_fn)) = &band {
                content_lines.push(apply_background_with_opener(
                    &line_with_margins,
                    width,
                    opener,
                    |s| bg_fn(s),
                ));
            } else {
                let visible_len = visible_width(&line_with_margins);
                content_lines.push(format!(
                    "{line_with_margins}{}",
                    " ".repeat(width.saturating_sub(visible_len))
                ));
            }
        }

        let empty_line = " ".repeat(width);
        let mut empty_lines = Vec::with_capacity(self.padding_y);
        for _ in 0..self.padding_y {
            let line = match &band {
                Some((opener, bg_fn)) => {
                    apply_background_with_opener(&empty_line, width, opener, |s| bg_fn(s))
                }
                None => empty_line.clone(),
            };
            empty_lines.push(line);
        }

        let mut result = Vec::with_capacity(empty_lines.len() * 2 + content_lines.len());
        result.extend(empty_lines.iter().cloned());
        result.extend(content_lines);
        result.extend(empty_lines);

        let result = if result.is_empty() {
            vec![String::new()]
        } else {
            result
        };
        self.store_lines(width, result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> crate::ColorFn {
        Box::new(|s: &str| s.to_string())
    }

    fn theme() -> MarkdownTheme {
        MarkdownTheme {
            heading: Box::new(|s: &str, _| s.to_string()),
            heading_block: None,
            link: identity(),
            link_url: identity(),
            code: identity(),
            code_block: identity(),
            code_block_border: identity(),
            quote: identity(),
            quote_border: identity(),
            hr: identity(),
            list_bullet: identity(),
            bold: identity(),
            italic: identity(),
            strikethrough: identity(),
            underline: identity(),
            highlight_code: None,
            code_block_indent: None,
            code_block_bg: None,
        }
    }

    #[test]
    fn empty_text_renders_nothing() {
        let mut md = Markdown::new("", 0, 0, theme(), None);
        assert_eq!(md.render(40), Vec::<String>::new());
    }

    #[test]
    fn renders_heading_text() {
        let mut md = Markdown::new("# Title", 0, 0, theme(), None);
        let lines = md.render(40);
        assert!(lines[0].contains("Title"));
    }

    #[test]
    fn h3_heading_prints_no_hash_marks() {
        let mut md = Markdown::new("### Sub", 0, 0, theme(), None);
        let lines = md.render(40);
        assert!(lines[0].contains("Sub"));
        assert!(!lines[0].contains('#'));
    }

    #[test]
    fn renders_paragraph_with_bold_and_italic() {
        let mut md = Markdown::new("hello **bold** and *italic*", 0, 0, theme(), None);
        let lines = md.render(40);
        assert!(lines.join(" ").contains("bold"));
        assert!(lines.join(" ").contains("italic"));
    }

    #[test]
    fn code_block_prints_no_fence_rows() {
        let mut md = Markdown::new("```rust\nfn x() {}\n```", 0, 0, theme(), None);
        let lines = md.render(40);
        assert!(!lines.iter().any(|l| l.contains("```")));
        assert!(lines.iter().any(|l| l.contains("fn x()")));
    }

    #[test]
    fn renders_unordered_list() {
        let mut md = Markdown::new("- one\n- two\n", 0, 0, theme(), None);
        let lines = md.render(40);
        assert!(lines.iter().any(|l| l.contains("one")));
        assert!(lines.iter().any(|l| l.contains("two")));
    }

    #[test]
    fn renders_ordered_list_with_numbers() {
        let mut md = Markdown::new("1. first\n2. second\n", 0, 0, theme(), None);
        let lines = md.render(40);
        assert!(lines.iter().any(|l| l.contains("1.")));
        assert!(lines.iter().any(|l| l.contains("2.")));
    }

    #[test]
    fn renders_blockquote_with_border() {
        let mut md = Markdown::new("> quoted\n", 0, 0, theme(), None);
        let lines = md.render(40);
        assert!(lines
            .iter()
            .any(|l| l.starts_with("│ ") && l.contains("quoted")));
    }

    #[test]
    fn renders_table_with_borders() {
        let mut md = Markdown::new("| a | b |\n|---|---|\n| 1 | 2 |\n", 0, 0, theme(), None);
        let lines = md.render(40);
        assert!(lines.iter().any(|l| l.starts_with('┌')));
        assert!(lines.iter().any(|l| l.contains('a') && l.contains('b')));
        assert!(lines.iter().any(|l| l.starts_with('└')));
    }

    #[test]
    fn renders_horizontal_rule() {
        let mut md = Markdown::new("---\n", 0, 0, theme(), None);
        let lines = md.render(10);
        assert!(lines.iter().any(|l| l.contains('─')));
    }

    #[test]
    fn wraps_long_paragraphs_to_width() {
        let long_text = "word ".repeat(30);
        let mut md = Markdown::new(&long_text, 0, 0, theme(), None);
        let lines = md.render(20);
        for line in &lines {
            assert!(visible_width(line) <= 20);
        }
    }

    #[test]
    fn a_cache_hit_hands_out_the_kept_lines() {
        let mut md = Markdown::new("hello **world**", 0, 0, theme(), None);
        let first = md.render_shared(40);
        let second = md.render_shared(40);
        assert!(Rc::ptr_eq(&first, &second));
        assert_eq!(md.render(40), first.to_vec());
        // Another width is a separate entry; the first stays.
        let narrow = md.render_shared(10);
        assert!(!Rc::ptr_eq(&first, &narrow));
        assert!(Rc::ptr_eq(&first, &md.render_shared(40)));
    }

    #[test]
    fn set_text_changes_the_stamp_and_the_lines() {
        let mut md = Markdown::new("one", 0, 0, theme(), None);
        let stamp = md.render_stamp();
        let before = md.render_shared(20);
        md.set_text("two");
        assert_ne!(md.render_stamp(), stamp);
        let after = md.render_shared(20);
        assert!(after[0].contains("two") && before[0].contains("one"));
    }

    #[test]
    fn the_same_text_again_keeps_the_stamp_and_the_lines() {
        let mut md = Markdown::new("same", 0, 0, theme(), None);
        let stamp = md.render_stamp();
        let before = md.render_shared(20);
        md.set_text("same");
        assert_eq!(md.render_stamp(), stamp);
        assert!(Rc::ptr_eq(&before, &md.render_shared(20)));
    }

    #[test]
    fn invalidate_draws_again_under_a_new_stamp() {
        let mut md = Markdown::new("text", 0, 0, theme(), None);
        let stamp = md.render_stamp();
        let before = md.render_shared(20);
        md.invalidate();
        assert_ne!(md.render_stamp(), stamp);
        let after = md.render_shared(20);
        assert!(!Rc::ptr_eq(&before, &after));
        assert_eq!(before[..], after[..]);
    }

    #[test]
    fn padding_adds_margin_lines() {
        let mut md = Markdown::new("hi", 2, 1, theme(), None);
        let lines = md.render(20);
        assert_eq!(lines[0], " ".repeat(20));
        assert!(lines[1].starts_with("  hi"));
    }
}

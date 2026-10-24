//! Multi-line word-wrapped text component, ported from `components/text.ts`.
//!
//! The TypeScript original memoizes rendered lines by `(text, width)` for
//! reference-stable output; this port always recomputes on `render`
//! (content is identical either way — see `hoocode-tui-render`'s docs
//! on the same simplification).

use hoocode_tui_render::{fresh_stamp, Component};
use hoocode_tui_util::{
    apply_background_with_opener, band_opener, visible_width, wrap_text_with_ansi,
};

use crate::color::ColorFn;

pub struct Text {
    text: String,
    padding_x: usize,
    padding_y: usize,
    custom_bg_fn: Option<ColorFn>,
    rev: u64,
}

impl Text {
    pub fn new(text: impl Into<String>, padding_x: usize, padding_y: usize) -> Self {
        Self {
            text: text.into(),
            padding_x,
            padding_y,
            custom_bg_fn: None,
            rev: fresh_stamp(),
        }
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.rev = fresh_stamp();
    }

    pub fn set_custom_bg_fn(&mut self, bg_fn: Option<ColorFn>) {
        self.custom_bg_fn = bg_fn;
        self.rev = fresh_stamp();
    }

    pub fn text(&self) -> &str {
        &self.text
    }
}

impl Default for Text {
    fn default() -> Self {
        Self::new("", 1, 1)
    }
}

impl Component for Text {
    fn render_stamp(&self) -> Option<u64> {
        Some(self.rev)
    }

    fn render(&mut self, width: u16) -> Vec<String> {
        let width = width as usize;

        if self.text.trim().is_empty() {
            return Vec::new();
        }

        let normalized_text = self
            .text
            .replace('\t', &" ".repeat(hoocode_tui_util::RENDER_TAB_WIDTH));
        let content_width = (width.saturating_sub(self.padding_x * 2)).max(1);
        let wrapped_lines = wrap_text_with_ansi(&normalized_text, content_width);

        let left_margin = " ".repeat(self.padding_x);
        let right_margin = " ".repeat(self.padding_x);
        let mut content_lines = Vec::with_capacity(wrapped_lines.len());
        // One band: every line below shares this opener.
        let band = self
            .custom_bg_fn
            .as_ref()
            .map(|bg_fn| (band_opener(&|s: &str| bg_fn(s)), bg_fn));

        for line in &wrapped_lines {
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
                let padding_needed = width.saturating_sub(visible_len);
                content_lines.push(format!("{line_with_margins}{}", " ".repeat(padding_needed)));
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

        if result.is_empty() {
            vec![String::new()]
        } else {
            result
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_renders_nothing() {
        let mut t = Text::new("", 1, 1);
        assert_eq!(t.render(20), Vec::<String>::new());
    }

    #[test]
    fn whitespace_only_text_renders_nothing() {
        let mut t = Text::new("   ", 1, 1);
        assert_eq!(t.render(20), Vec::<String>::new());
    }

    #[test]
    fn wraps_and_pads_to_width() {
        let mut t = Text::new("hello", 1, 0);
        let lines = t.render(10);
        assert_eq!(lines.len(), 1);
        assert_eq!(visible_width(&lines[0]), 10);
        assert!(lines[0].contains("hello"));
    }

    #[test]
    fn adds_vertical_padding() {
        let mut t = Text::new("hi", 0, 1);
        let lines = t.render(5);
        // 1 empty line above, 1 content line, 1 empty line below.
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "     ");
        assert_eq!(lines[2], "     ");
    }

    #[test]
    fn tabs_are_replaced_with_three_spaces() {
        let mut t = Text::new("a\tb", 0, 0);
        let lines = t.render(20);
        assert!(lines[0].starts_with("a   b"));
    }

    #[test]
    fn set_text_updates_output() {
        let mut t = Text::new("a", 0, 0);
        t.set_text("b");
        let lines = t.render(5);
        assert!(lines[0].starts_with('b'));
    }
}

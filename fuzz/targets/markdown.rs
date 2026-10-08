//! Target `markdown`: the markdown lexer and terminal renderer (`hoocode-tui-components`).
//!
//! The lexer is a port of `marked`, with its list-item detection (`rules.rs`) and
//! inline rules built on the JS regex emulation. Must not panic on any text at any
//! render width.

use hoocode_tui_components::markdown::lexer::lex;
use hoocode_tui_components::{identity_color, Markdown, MarkdownTheme};
use hoocode_tui_render::Component;

pub fn run(data: &[u8]) {
    let text = String::from_utf8_lossy(data).into_owned();
    let _ = lex(&text);

    let width = 1 + u16::from(data.first().copied().unwrap_or(60)) % 120;
    let mut markdown = Markdown::new(text, 1, 0, theme(), None);
    let _ = markdown.render(width);
}

fn theme() -> MarkdownTheme {
    MarkdownTheme {
        heading: Box::new(|s: &str, _| s.to_string()),
        heading_block: None,
        link: identity_color(),
        link_url: identity_color(),
        code: identity_color(),
        code_block: identity_color(),
        code_block_border: identity_color(),
        quote: identity_color(),
        quote_border: identity_color(),
        hr: identity_color(),
        list_bullet: identity_color(),
        bold: identity_color(),
        italic: identity_color(),
        strikethrough: identity_color(),
        underline: identity_color(),
        highlight_code: None,
        code_block_indent: None,
    }
}

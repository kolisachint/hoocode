//! `core/tools/render-utils.ts`: helpers the tool renderers share.

use hoocode_ai_types::Content;
use hoocode_code_tool_bash::shell::{sanitize_binary_output, strip_ansi};
use hoocode_code_tui_theme::theme;
use hoocode_tui_images::{get_capabilities, get_image_dimensions, image_fallback};
use serde_json::Value;

/// `str(value)`: a string as itself, `null`/missing as `""`, anything else
/// as `None` (an invalid argument).
pub fn str_arg(value: Option<&Value>) -> Option<String> {
    match value {
        None | Some(Value::Null) => Some(String::new()),
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => None,
    }
}

/// `args?.a ?? args?.b`: the first key present and not `null`.
pub fn arg_or<'a>(args: &'a Value, first: &str, second: &str) -> Option<&'a Value> {
    match args.get(first) {
        Some(Value::Null) | None => args.get(second),
        some => some,
    }
}

/// `replaceTabs`: tabs become the render tab width, as the other render
/// surfaces expand them.
pub fn replace_tabs(text: &str) -> String {
    text.replace('\t', &" ".repeat(hoocode_tui_util::RENDER_TAB_WIDTH))
}

/// `normalizeDisplayText`.
pub fn normalize_display_text(text: &str) -> String {
    text.replace('\r', "")
}

/// `getTextOutput`: the text blocks (ANSI stripped, sanitized), plus a
/// placeholder per image when images are not drawn.
pub fn get_text_output(content: Option<&[Content]>, show_images: bool) -> String {
    let Some(content) = content else {
        return String::new();
    };
    let mut texts = Vec::new();
    let mut images = Vec::new();
    for c in content {
        match c {
            Content::Text(t) => texts.push(t),
            Content::Image(i) => images.push(i),
            _ => {}
        }
    }
    let mut output = texts
        .iter()
        .map(|t| sanitize_binary_output(&strip_ansi(&t.text)).replace('\r', ""))
        .collect::<Vec<_>>()
        .join("\n");
    let caps = get_capabilities();
    if !images.is_empty() && (caps.images.is_none() || !show_images) {
        let indicators = images
            .iter()
            .map(|img| {
                let mime = if img.media_type.is_empty() {
                    "image/unknown"
                } else {
                    img.media_type.as_str()
                };
                let dims = if img.data.is_empty() || img.media_type.is_empty() {
                    None
                } else {
                    get_image_dimensions(&img.data, &img.media_type)
                };
                image_fallback(mime, dims, None)
            })
            .collect::<Vec<_>>()
            .join("\n");
        output = if output.is_empty() {
            indicators
        } else {
            format!("{output}\n{indicators}")
        };
    }
    output
}

/// `invalidArgText`.
pub fn invalid_arg_text() -> String {
    theme().fg("error", "[invalid arg]")
}

#[cfg(test)]
mod tests {
    use hoocode_tui_util::{visible_width, RENDER_TAB_WIDTH};

    use super::replace_tabs;

    #[test]
    fn replace_tabs_expands_to_the_render_tab_width() {
        let spaces = " ".repeat(RENDER_TAB_WIDTH);
        assert_eq!(replace_tabs("a\tb"), format!("a{spaces}b"));
        assert_eq!(visible_width(&replace_tabs("\t")), RENDER_TAB_WIDTH);
    }
}

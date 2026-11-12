//! The tool-result types the tool blocks share: the result text, memoized
//! per image mode, and the call's subject for the chain line.

use std::cell::OnceCell;
use std::path::Path;

use hoocode_ai_types::Content;
use hoocode_tui_components::markdown::js_trim;
use hoocode_tui_util::js_regex::is_js_space;
use serde_json::Value;

use crate::render_utils::get_text_output;

/// Argument names that carry a call's subject, most specific first.
const SUBJECT_KEYS: [&str; 12] = [
    "command",
    "file_path",
    // Before `path`: for a search the pattern identifies the call.
    "pattern",
    "query",
    "url",
    "path",
    "prompt",
    "description",
    "task_id",
    "subagent_type",
    "name",
    "id",
];

fn non_blank_string(value: &Value) -> Option<&str> {
    value.as_str().filter(|s| !js_trim(s).is_empty())
}

fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// The most identifying argument as written, before whitespace is collapsed.
fn subject_source(args: &Value, cwd: &str) -> String {
    let Some(record) = args.as_object() else {
        return String::new();
    };
    let subject = SUBJECT_KEYS
        .iter()
        .find_map(|key| record.get(*key).and_then(non_blank_string))
        .or_else(|| record.values().find_map(non_blank_string));
    let Some(subject) = subject else {
        return String::new();
    };
    // URLs are not paths: relativizing one against the cwd mangles it
    // (the empty `//` component collapses, so `https://` loses a slash).
    if subject.contains("://") {
        return subject.to_string();
    }
    // Paths are the most common subject and the least readable in absolute form.
    let relative = hoocode_code_paths::cwd_relative_path(Path::new(subject), Path::new(cwd))
        .map(|p| p.to_string_lossy().into_owned());
    match relative {
        Some(r) if utf16_len(&r) < utf16_len(subject) => r,
        _ => subject.to_string(),
    }
}

/// The subject a chain line names a call by. An Agent call is named by its
/// description; its prompt is the whole brief and would fill the row.
pub fn chain_subject(tool: &str, args: &Value, cwd: &str) -> String {
    if tool == "Agent" {
        if let Some(description) = args.get("description").and_then(Value::as_str) {
            if !description.trim().is_empty() {
                let mut record = serde_json::Map::new();
                record.insert("description".into(), Value::String(description.into()));
                return tool_subject(&Value::Object(record), cwd);
            }
        }
    }
    tool_subject(args, cwd)
}

/// `toolSubject`: the single most identifying argument, on one line.
pub fn tool_subject(args: &Value, cwd: &str) -> String {
    let display = subject_source(args, cwd);
    // Multi-line commands collapse to one line.
    let mut out = String::new();
    let mut in_space = false;
    for c in display.chars() {
        if is_js_space(c) {
            if !in_space {
                out.push(' ');
            }
            in_space = true;
        } else {
            out.push(c);
            in_space = false;
        }
    }
    js_trim(&out).to_string()
}

/// The text a result's renderers show, computed on first use and kept with
/// the result: one cell per image mode, since the image placeholders depend
/// on it. The memo is derived from the content it sits beside, so it is not
/// part of a result's equality, and a copy starts empty.
#[derive(Debug, Default)]
pub struct ResultText {
    with_images: OnceCell<String>,
    without_images: OnceCell<String>,
}

impl ResultText {
    /// `get_text_output` of `content`, computed by the first call for this mode.
    pub fn get(&self, content: &[Content], show_images: bool) -> &str {
        let cell = if show_images {
            &self.with_images
        } else {
            &self.without_images
        };
        cell.get_or_init(|| get_text_output(Some(content), show_images))
    }
}

impl Clone for ResultText {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl PartialEq for ResultText {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

/// A finished (or partial) tool result, as the tool blocks see it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ToolResult {
    pub content: Vec<Content>,
    pub details: Value,
    pub is_error: bool,
}

/// A tool result as a block holds it: shared through an `Rc` by the block and
/// its renderers, with the text they show memoized beside it. Reads go to the
/// result itself (`Deref`). The result is private, so the memo cannot go
/// stale: a changed result is a new holder (`ToolExecutionComponent::update_result`).
#[derive(Debug, Default)]
pub struct ResultHolder {
    result: ToolResult,
    pub(crate) memo: ResultText,
}

impl ResultHolder {
    pub fn new(result: ToolResult) -> Self {
        Self {
            result,
            memo: ResultText::default(),
        }
    }

    pub fn result(&self) -> &ToolResult {
        &self.result
    }

    /// The result's text (ANSI stripped, sanitized), computed once per image
    /// mode. Renderers rebuild on every frame, so they read this, not the content.
    pub fn text(&self, show_images: bool) -> &str {
        self.memo.get(&self.result.content, show_images)
    }
}

impl std::ops::Deref for ResultHolder {
    type Target = ToolResult;

    fn deref(&self) -> &ToolResult {
        &self.result
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use super::*;

    fn holder(text: &str) -> Rc<ResultHolder> {
        Rc::new(ResultHolder::new(ToolResult {
            content: vec![Content::text(text)],
            ..Default::default()
        }))
    }

    #[test]
    fn the_text_is_computed_once_and_shared_by_every_reader() {
        let shared = holder(
            "line one
line two
",
        );
        let first = shared.text(false).as_ptr();
        let again = Rc::clone(&shared).text(false).as_ptr();
        assert_eq!(first, again, "a second read hands back the memoized text");
        assert_eq!(
            shared.text(false),
            "line one
line two
"
        );
    }

    #[test]
    fn each_image_mode_has_its_own_memo() {
        let shared = holder("plain");
        let with = shared.text(true).as_ptr();
        let without = shared.text(false).as_ptr();
        assert_ne!(with, without, "the two modes are memoized apart");
        assert_eq!(shared.text(true).as_ptr(), with);
        assert_eq!(shared.text(false).as_ptr(), without);
    }

    #[test]
    fn the_image_mode_memoizes_its_own_text() {
        use hoocode_ai_types::ImageContent;
        use hoocode_tui_images::{
            reset_capabilities_cache, set_capabilities, ImageProtocol, TerminalCapabilities,
        };
        set_capabilities(TerminalCapabilities {
            images: Some(ImageProtocol::Kitty),
            true_color: true,
            hyperlinks: true,
        });
        let shared = Rc::new(ResultHolder::new(ToolResult {
            content: vec![
                Content::text("before"),
                Content::Image(ImageContent {
                    data: "AAAA".into(),
                    media_type: "image/png".into(),
                }),
            ],
            ..Default::default()
        }));
        // The image is drawn in one mode and a placeholder in the other, so
        // a shared memo would hand one mode's text to the other.
        let drawn = shared.text(true).to_string();
        let placeholder = shared.text(false).to_string();
        reset_capabilities_cache();
        assert!(!drawn.contains("[Image: image/png]"), "{drawn:?}");
        assert!(
            placeholder.contains("[Image: image/png]"),
            "{placeholder:?}"
        );
        assert_eq!(shared.text(true), drawn);
        assert_eq!(shared.text(false), placeholder);
    }

    #[test]
    fn an_update_is_a_new_holder_with_its_own_text() {
        assert_eq!(holder("old").text(false), "old");
        assert_eq!(holder("new").text(false), "new");
    }
}

//! `core/tools/write.ts` renderers: the call line with a preview of the
//! content (highlighted when the path has a language), and errors.

use std::rc::Rc;

use hoocode_code_tui_theme::{get_language_from_path, highlight_code, theme};
use hoocode_tui_render::Container;
use serde_json::{json, Value};

use super::{text, trim_trailing_empty_lines};
use crate::fold::more_lines_hint;
use crate::read_output::render_read_output;
use crate::render_utils::{
    arg_or, invalid_arg_text, normalize_display_text, replace_tabs, str_arg,
};
use crate::tool_execution::ToolRenderDefinition;
use crate::tool_output_view::{line_budget, peek_block};
use hoocode_home::shorten_path;

/// Lines re-highlighted as one block while the content streams in.
const WRITE_PARTIAL_FULL_HIGHLIGHT_LINES: usize = 50;

/// `WriteHighlightCache`, kept in the block's renderer state.
#[derive(Debug, Clone, Default)]
struct HighlightCache {
    raw_path: Option<String>,
    lang: String,
    raw_content: String,
    normalized_lines: Vec<String>,
    highlighted_lines: Vec<String>,
}

const CACHE_KEY: &str = "writeHighlightCache";

impl HighlightCache {
    fn load(value: Option<&Value>) -> Option<Self> {
        let v = value?;
        let strings = |key: &str| -> Vec<String> {
            v.get(key)
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default()
        };
        Some(Self {
            raw_path: v.get("rawPath").and_then(Value::as_str).map(String::from),
            lang: v.get("lang")?.as_str()?.to_string(),
            raw_content: v.get("rawContent")?.as_str()?.to_string(),
            normalized_lines: strings("normalizedLines"),
            highlighted_lines: strings("highlightedLines"),
        })
    }

    fn store(&self) -> Value {
        json!({
            "rawPath": self.raw_path,
            "lang": self.lang,
            "rawContent": self.raw_content,
            "normalizedLines": self.normalized_lines,
            "highlightedLines": self.highlighted_lines,
        })
    }
}

fn highlight_single_line(line: &str, lang: &str) -> String {
    highlight_code(line, Some(lang))
        .into_iter()
        .next()
        .unwrap_or_default()
}

fn refresh_prefix(cache: &mut HighlightCache) {
    let count = WRITE_PARTIAL_FULL_HIGHLIGHT_LINES.min(cache.normalized_lines.len());
    if count == 0 {
        return;
    }
    let source = cache.normalized_lines[..count].join("\n");
    let highlighted = highlight_code(&source, Some(&cache.lang));
    for i in 0..count {
        cache.highlighted_lines[i] = highlighted
            .get(i)
            .cloned()
            .unwrap_or_else(|| highlight_single_line(&cache.normalized_lines[i], &cache.lang));
    }
}

fn rebuild_full(raw_path: Option<&str>, content: &str) -> Option<HighlightCache> {
    let lang = raw_path.and_then(get_language_from_path)?;
    let normalized = replace_tabs(&normalize_display_text(content));
    Some(HighlightCache {
        raw_path: raw_path.map(String::from),
        lang: lang.to_string(),
        raw_content: content.to_string(),
        normalized_lines: normalized.split('\n').map(String::from).collect(),
        highlighted_lines: highlight_code(&normalized, Some(lang)),
    })
}

fn update_incremental(
    cache: Option<HighlightCache>,
    raw_path: Option<&str>,
    content: &str,
) -> Option<HighlightCache> {
    let lang = raw_path.and_then(get_language_from_path)?;
    let Some(mut cache) = cache else {
        return rebuild_full(raw_path, content);
    };
    if cache.lang != lang || cache.raw_path.as_deref() != raw_path {
        return rebuild_full(raw_path, content);
    }
    if !content.starts_with(&cache.raw_content) {
        return rebuild_full(raw_path, content);
    }
    if content.len() == cache.raw_content.len() {
        return Some(cache);
    }
    let delta = replace_tabs(&normalize_display_text(
        hoocode_tui_util::text_slice::suffix_from(content, cache.raw_content.len()),
    ));
    cache.raw_content = content.to_string();
    if cache.normalized_lines.is_empty() {
        cache.normalized_lines.push(String::new());
        cache.highlighted_lines.push(String::new());
    }
    let segments: Vec<&str> = delta.split('\n').collect();
    let last = cache.normalized_lines.len() - 1;
    cache.normalized_lines[last].push_str(segments[0]);
    cache.highlighted_lines[last] =
        highlight_single_line(&cache.normalized_lines[last], &cache.lang);
    for segment in &segments[1..] {
        cache.normalized_lines.push(segment.to_string());
        cache
            .highlighted_lines
            .push(highlight_single_line(segment, &cache.lang));
    }
    refresh_prefix(&mut cache);
    Some(cache)
}

/// The success preview: the first lines of the written content, numbered like
/// Read, with the shared `… N more lines` hint. `None` for empty content.
fn format_write_preview(raw_path: Option<&str>, content: &str, expanded: bool) -> Option<String> {
    let normalized = replace_tabs(&normalize_display_text(content));
    let rendered: Vec<String> = match raw_path.and_then(get_language_from_path) {
        Some(lang) => highlight_code(&normalized, Some(lang)),
        None => normalized.split('\n').map(String::from).collect(),
    };
    let lines = trim_trailing_empty_lines(rendered);
    if lines.is_empty() {
        return None;
    }
    Some(peek_block(expanded, &lines, |shown| {
        render_read_output(&shown.join("\n"), 1)
    }))
}

/// The call line. `show_body` holds while there is no result: the content
/// previews under the header as it streams, and stays while the call runs
/// or if it was cut short. Once a result exists it is the one copy of the
/// content (numbered, or the error), so the call is a header only.
fn format_write_call(
    args: &Value,
    cache: Option<&HighlightCache>,
    show_body: bool,
    expanded: bool,
) -> String {
    let t = theme();
    let raw_path = str_arg(arg_or(args, "file_path", "path"));
    let path = raw_path.as_deref().map(shorten_path);
    let path_display = match path {
        None => invalid_arg_text(),
        Some(p) if !p.is_empty() => t.fg("accent", &p),
        Some(_) => t.fg("toolOutput", "…"),
    };
    let mut text = format!("{} {path_display}", t.fg("toolTitle", &t.bold("Write")));
    match args.get("content") {
        None | Some(Value::Null) => {}
        Some(Value::String(content)) if show_body && !content.is_empty() => {
            let content = content.as_str();
            let lang = raw_path.as_deref().and_then(get_language_from_path);
            let rendered: Vec<String> = match (lang, cache) {
                (Some(_), Some(cache)) => cache.highlighted_lines.clone(),
                (Some(lang), None) => {
                    highlight_code(&replace_tabs(&normalize_display_text(content)), Some(lang))
                }
                (None, _) => normalize_display_text(content)
                    .split('\n')
                    .map(String::from)
                    .collect(),
            };
            let lines = trim_trailing_empty_lines(rendered);
            let total = lines.len();
            let max = line_budget(expanded);
            let shown: Vec<String> = lines
                .iter()
                .take(max)
                .map(|l| {
                    if lang.is_some() {
                        l.clone()
                    } else {
                        t.fg("toolOutput", &replace_tabs(l))
                    }
                })
                .collect();
            text.push_str(&format!("\n\n{}", shown.join("\n")));
            if total > max {
                text.push_str(&format!("\n{}", more_lines_hint(total - max)));
            }
        }
        Some(Value::String(_)) => {}
        Some(_) => text.push_str(&format!(
            "\n\n{}",
            t.fg("error", "[invalid content arg - expected string]")
        )),
    }
    text
}

pub fn definition() -> ToolRenderDefinition {
    ToolRenderDefinition {
        render_call: Some(Rc::new(|args, ctx| {
            let raw_path = str_arg(arg_or(args, "file_path", "path"));
            // The preview is built only while no result has replaced it.
            let show_body = !ctx.has_result;
            let cache = match args.get("content").and_then(Value::as_str) {
                Some(content) if show_body => {
                    let previous = HighlightCache::load(ctx.state.get(CACHE_KEY));
                    update_incremental(previous, raw_path.as_deref(), content)
                }
                _ => None,
            };
            match &cache {
                Some(c) => {
                    ctx.state.insert(CACHE_KEY.into(), c.store());
                }
                None => {
                    ctx.state.remove(CACHE_KEY);
                }
            }
            Ok(text(format_write_call(
                args,
                cache.as_ref(),
                show_body,
                ctx.expanded,
            )))
        })),
        render_result: Some(Rc::new(|result, options, ctx| {
            if ctx.is_error {
                let output = result
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        hoocode_ai_types::Content::Text(t) => Some(t.text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                if !output.is_empty() {
                    return Ok(text(theme().fg("error", &output)));
                }
            } else if let Some(content) = str_arg(ctx.args.get("content")) {
                let raw_path = str_arg(arg_or(ctx.args, "file_path", "path"));
                if let Some(preview) =
                    format_write_preview(raw_path.as_deref(), &content, options.expanded)
                {
                    return Ok(text(preview));
                }
            }
            Ok(std::rc::Rc::new(std::cell::RefCell::new(Container::new())))
        })),
    }
}

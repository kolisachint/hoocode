//! `core/tools/read.ts` renderers: the call line (compact for skills,
//! resources and the app's own docs) and the numbered result, trimmed to the
//! peek budget.

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use hoocode_code_tool_api::{format_size, resolve_read_path, DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES};
use hoocode_code_tui_theme::{get_language_from_path, highlight_code, message_label, theme, Theme};
use serde_json::Value;

use super::search::js_number;
use super::text;
use crate::read_output::render_read_output;
use crate::render_utils::{arg_or, invalid_arg_text, replace_tabs, str_arg};
use crate::tool_execution::{ToolRenderDefinition, ToolResultView};
use crate::tool_output_view::{peek_window, PEEK_LINES};
use hoocode_home::shorten_path;

#[derive(Debug, Clone, PartialEq, Eq)]
enum CompactKind {
    Docs,
    Resource,
    Skill,
}

const COMPACT_RESOURCE_FILE_NAMES: [&str; 4] = ["AGENTS.md", "AGENTS.MD", "CLAUDE.md", "CLAUDE.MD"];

fn raw_path(args: &Value) -> Option<String> {
    str_arg(arg_or(args, "file_path", "path"))
}

fn format_line_range(args: &Value) -> String {
    // `undefined` checks: an explicit `null` counts as given, as in the pin.
    let (offset, limit) = (args.get("offset"), args.get("limit"));
    if offset.is_none() && limit.is_none() {
        return String::new();
    }
    let start = match offset {
        Some(Value::Null) | None => serde_json::json!(1),
        Some(v) => v.clone(),
    };
    let end = limit.map(|l| {
        let n = start.as_f64().unwrap_or(f64::NAN) + l.as_f64().unwrap_or(0.0) - 1.0;
        if n == 0.0 || n.is_nan() {
            String::new()
        } else {
            js_number(&serde_json::json!(n))
        }
    });
    let end = end.filter(|e| !e.is_empty());
    theme().fg(
        "warning",
        &format!(
            ":{}{}",
            js_number(&start),
            end.map(|e| format!("-{e}")).unwrap_or_default()
        ),
    )
}

fn format_read_call(args: &Value) -> String {
    let t = theme();
    let path = raw_path(args).map(|p| shorten_path(&p));
    let display = match path {
        None => invalid_arg_text(),
        Some(p) if !p.is_empty() => t.fg("accent", &p),
        Some(_) => t.fg("toolOutput", "..."),
    };
    format!(
        "{} {display}{}",
        t.fg("toolTitle", &t.bold("Read")),
        format_line_range(args)
    )
}

/// `path.relative` for two absolute, normalized paths.
fn relative(from: &Path, to: &Path) -> PathBuf {
    let from: Vec<_> = from.components().collect();
    let to: Vec<_> = to.components().collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut out = PathBuf::new();
    for _ in common..from.len() {
        out.push("..");
    }
    for c in &to[common..] {
        out.push(c.as_os_str());
    }
    out
}

/// `getReadmePath`: the package's README.
pub fn readme_path() -> PathBuf {
    hoocode_code_paths::package_dir().join("README.md")
}

fn docs_classification(absolute: &Path) -> Option<(CompactKind, String)> {
    let package_root = std::path::absolute(readme_path())
        .ok()?
        .parent()?
        .to_path_buf();
    let absolute = std::path::absolute(absolute).ok()?;
    let rel = relative(&package_root, &absolute);
    let rel_str = rel
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/");
    if rel_str.is_empty() || rel_str == ".." || rel_str.starts_with("../") || rel.is_absolute() {
        return None;
    }
    if rel_str == "README.md" || rel_str.starts_with("docs/") || rel_str.starts_with("examples/") {
        return Some((CompactKind::Docs, rel_str));
    }
    None
}

fn compact_classification(args: &Value, cwd: &str) -> Option<(CompactKind, String)> {
    let raw = raw_path(args).filter(|p| !p.is_empty())?;
    let absolute = resolve_read_path(&raw, Path::new(cwd));
    let file_name = absolute
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if file_name == "SKILL.md" {
        let dir = absolute
            .parent()
            .and_then(Path::file_name)
            .map(|n| n.to_string_lossy().into_owned())
            .filter(|n| !n.is_empty())
            .unwrap_or(file_name);
        return Some((CompactKind::Skill, dir));
    }
    if let Some(docs) = docs_classification(&absolute) {
        return Some(docs);
    }
    if COMPACT_RESOURCE_FILE_NAMES.contains(&file_name.as_str()) {
        return Some((
            CompactKind::Resource,
            hoocode_code_paths::format_path_relative_to_cwd_or_absolute(&absolute, Path::new(cwd)),
        ));
    }
    None
}

fn format_compact_read_call(kind: &CompactKind, label: &str, args: &Value) -> String {
    let t = theme();
    if *kind == CompactKind::Skill {
        return format!(
            "{} {}{}",
            message_label("skill"),
            t.fg("customMessageText", label),
            format_line_range(args)
        );
    }
    let kind = match kind {
        CompactKind::Docs => "docs",
        _ => "resource",
    };
    format!(
        "{} {}{}",
        t.fg("toolTitle", &t.bold(&format!("Read {kind}"))),
        t.fg("accent", label),
        format_line_range(args)
    )
}

/// The peek window of a Read result: the first lines it shows, highlighted,
/// and how many lines the result has in all. Only the window is highlighted;
/// the count is taken from the raw text, because the highlighter appends a
/// closing colour line when a token is open at the end of its input.
#[derive(Debug, Clone)]
struct ReadPreview {
    lang: Option<&'static str>,
    /// The theme the highlighting used: a theme switch re-highlights.
    theme: Arc<Theme>,
    output: String,
    total: usize,
    shown: Vec<String>,
}

/// Renderer state for the Read result, kept in the block's objects.
const PREVIEW_KEY: &str = "read.preview";

fn preview_state(objects: &mut HashMap<String, Rc<dyn Any>>) -> Rc<RefCell<Option<ReadPreview>>> {
    if let Some(existing) = objects.get(PREVIEW_KEY) {
        if let Ok(state) = existing.clone().downcast::<RefCell<Option<ReadPreview>>>() {
            return state;
        }
    }
    let state = Rc::new(RefCell::new(None));
    objects.insert(PREVIEW_KEY.into(), state.clone());
    state
}

/// Raw lines the highlighter reads past the peek window. Highlighting is
/// stateful: whether a token open at a line's end is kept depends on the text
/// after it. The margin keeps the visible lines identical to a full highlight
/// on real files (tests); it is not a proof for every input.
const HIGHLIGHT_LOOKAHEAD: usize = 16;

fn build_preview(output: &str, lang: Option<&'static str>) -> ReadPreview {
    let text = match lang {
        Some(_) => replace_tabs(output),
        None => output.to_string(),
    };
    let lines: Vec<&str> = text.split('\n').collect();
    // `trim_trailing_empty_lines` on the raw lines.
    let total = lines
        .iter()
        .rposition(|l| !l.is_empty())
        .map_or(0, |i| i + 1);
    let shown: Vec<String> = match lang {
        Some(lang) if total > 0 => {
            let window = lines.len().min(PEEK_LINES + HIGHLIGHT_LOOKAHEAD);
            let mut code = lines[..window].join("\n");
            if window < lines.len() {
                code.push('\n');
            }
            let mut rendered = highlight_code(&code, Some(lang));
            rendered.truncate(total.min(PEEK_LINES));
            rendered
        }
        _ => lines[..total.min(PEEK_LINES)]
            .iter()
            .map(|l| l.to_string())
            .collect(),
    };
    ReadPreview {
        lang,
        theme: theme(),
        output: output.to_string(),
        total,
        shown,
    }
}

/// The memoized preview, rebuilt only when the output, the language or the
/// theme changed (the highlight colours come from the theme).
fn ensure_preview<'a>(
    memo: &'a mut Option<ReadPreview>,
    output: &str,
    lang: Option<&'static str>,
) -> &'a ReadPreview {
    let current = theme();
    let fresh = memo
        .as_ref()
        .is_some_and(|p| p.lang == lang && p.output == output && Arc::ptr_eq(&p.theme, &current));
    if !fresh {
        *memo = Some(build_preview(output, lang));
    }
    memo.as_ref().expect("set above when stale")
}

fn format_read_result(
    args: &Value,
    result: &ToolResultView<'_>,
    show_images: bool,
    memo: &RefCell<Option<ReadPreview>>,
) -> String {
    let t = theme();
    let raw = raw_path(args);
    let output = result.text(show_images);
    let lang = raw.as_deref().and_then(get_language_from_path);
    let mut memo = memo.borrow_mut();
    let preview = ensure_preview(&mut memo, output, lang);
    let start_line = args
        .get("offset")
        .and_then(Value::as_f64)
        .map(|n| n as i64)
        .unwrap_or(1);
    let body = peek_window(preview.total, &preview.shown, |shown| {
        render_read_output(&shown.join("\n"), start_line)
    });
    let mut text = body;

    let truncation = result.details.get("truncation");
    if let Some(tr) = truncation.filter(|tr| tr.get("truncated") == Some(&Value::Bool(true))) {
        let num = |key: &str, default: usize| {
            tr.get(key)
                .and_then(Value::as_u64)
                .map(|n| n as usize)
                .unwrap_or(default)
        };
        let max_bytes = num("maxBytes", DEFAULT_MAX_BYTES);
        if tr.get("firstLineExceedsLimit") == Some(&Value::Bool(true)) {
            text.push_str(&format!(
                "\n{}",
                t.fg(
                    "warning",
                    &format!("[First line exceeds {} limit]", format_size(max_bytes))
                )
            ));
        } else if tr.get("truncatedBy").and_then(Value::as_str) == Some("lines") {
            text.push_str(&format!(
                "\n{}",
                t.fg(
                    "warning",
                    &format!(
                        "[Truncated: showing {} of {} lines ({} line limit)]",
                        num("outputLines", 0),
                        num("totalLines", 0),
                        num("maxLines", DEFAULT_MAX_LINES)
                    )
                )
            ));
        } else {
            text.push_str(&format!(
                "\n{}",
                t.fg(
                    "warning",
                    &format!(
                        "[Truncated: {} lines shown ({} limit)]",
                        num("outputLines", 0),
                        format_size(max_bytes)
                    )
                )
            ));
        }
    }
    text
}

pub fn definition() -> ToolRenderDefinition {
    ToolRenderDefinition {
        render_call: Some(Rc::new(|args, ctx| {
            let compact = compact_classification(args, ctx.cwd);
            Ok(text(match compact {
                Some((kind, label)) => format_compact_read_call(&kind, &label, args),
                None => format_read_call(args),
            }))
        })),
        render_result: Some(Rc::new(|result, _, ctx| {
            let memo = preview_state(ctx.objects);
            Ok(text(format_read_result(
                ctx.args,
                result,
                ctx.show_images,
                &memo,
            )))
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_counts_every_line_and_shows_the_first_few() {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        let output = "a\nb\nc\nd\ne\nf\ng\n\n";
        let preview = build_preview(output, None);
        assert_eq!(preview.total, 7, "trailing empty lines are not counted");
        assert_eq!(preview.shown, ["a", "b", "c", "d", "e"]);
        let block = super::peek_window(preview.total, &preview.shown, |s| s.to_vec());
        assert!(block.contains("... (2 more lines)"), "{block}");
    }

    #[test]
    fn preview_is_reused_for_the_same_output_and_rebuilt_on_change() {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        let mut memo = None;
        ensure_preview(&mut memo, "let a = 1;\n", Some("rust"));
        if let Some(p) = memo.as_mut() {
            p.shown = vec!["SENTINEL".into()];
        }
        ensure_preview(&mut memo, "let a = 1;\n", Some("rust"));
        assert_eq!(
            memo.as_ref().unwrap().shown,
            ["SENTINEL"],
            "same output hits the memo"
        );
        ensure_preview(&mut memo, "let a = 2;\n", Some("rust"));
        assert_ne!(
            memo.as_ref().unwrap().shown,
            ["SENTINEL"],
            "new output rebuilds"
        );
        ensure_preview(&mut memo, "let a = 2;\n", None);
        assert_eq!(memo.as_ref().unwrap().lang, None, "a new language rebuilds");
    }

    #[test]
    fn preview_is_rebuilt_when_the_theme_changes() {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        let mut memo = None;
        ensure_preview(&mut memo, "let a = 1;\n", Some("rust"));
        // The same colours under another theme instance: the highlight is stale.
        memo.as_mut().unwrap().theme = Arc::new((*theme()).clone());
        memo.as_mut().unwrap().shown = vec!["SENTINEL".into()];
        let shown = ensure_preview(&mut memo, "let a = 1;\n", Some("rust"))
            .shown
            .clone();
        assert_ne!(shown, ["SENTINEL"], "a theme switch re-highlights");
        assert!(Arc::ptr_eq(&memo.as_ref().unwrap().theme, &theme()));
    }

    /// The path before the window: highlight the whole output, then trim and
    /// cut. Kept here as the reference the window must reproduce.
    fn full_highlight_preview(output: &str, lang: &'static str) -> (usize, Vec<String>) {
        let rendered = highlight_code(&replace_tabs(output), Some(lang));
        let mut lines = super::super::trim_trailing_empty_lines(rendered);
        let total = lines.len();
        lines.truncate(PEEK_LINES);
        (total, lines)
    }

    fn assert_same_as_full_highlight(output: &str, lang: &'static str) {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        let (old_total, old_shown) = full_highlight_preview(output, lang);
        let new = build_preview(output, Some(lang));
        assert_eq!(new.shown, old_shown, "visible lines differ for {lang}");
        assert_eq!(new.total, old_total, "line count differs for {lang}");
    }

    #[test]
    fn a_large_read_output_renders_as_the_full_highlight_did() {
        // Two real source files, several hundred lines each.
        assert_same_as_full_highlight(include_str!("read.rs"), "rust");
        assert_same_as_full_highlight(
            include_str!("../../../hoocode-code-tui-theme/src/tui.rs"),
            "rust",
        );
    }

    #[test]
    fn a_multi_line_token_across_the_window_renders_as_before() {
        let mut output = String::from("fn a() {}\n/* a comment that\n");
        for i in 0..200 {
            output.push_str(&format!("still inside line {i}\n"));
        }
        output.push_str("*/\nlet x = 1;\n");
        assert_same_as_full_highlight(&output, "rust");
        // A string still open at the end: the count is the file's lines (201),
        // where the old count also took the highlighter's colour-only line.
        let mut string = String::from("let s = \"open\n");
        for i in 0..200 {
            string.push_str(&format!("line {i}\n"));
        }
        let (_, old_shown) = full_highlight_preview(&string, "rust");
        let new = build_preview(&string, Some("rust"));
        assert_eq!(new.shown, old_shown, "visible lines differ");
        assert_eq!(new.total, 201);
    }

    #[test]
    fn the_line_count_comes_from_the_raw_text() {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        // A string left open at the end: the highlighter adds a colour-only
        // line, which is not a line of the file.
        let preview = build_preview("let s = \"open\nmore\n\n", Some("rust"));
        assert_eq!(preview.total, 2);
        assert!(preview.shown.len() <= 2, "{:?}", preview.shown);
    }

    #[test]
    fn only_the_window_is_highlighted() {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        let output: String = (0..5000).map(|i| format!("let v{i} = {i};\n")).collect();
        let preview = build_preview(&output, Some("rust"));
        assert_eq!(preview.total, 5000);
        assert_eq!(preview.shown.len(), PEEK_LINES);
        let head: String = (0..10).map(|i| format!("let v{i} = {i};\n")).collect();
        let (_, full) = full_highlight_preview(&head, "rust");
        assert_eq!(preview.shown, full, "the first lines are the same");
    }
}

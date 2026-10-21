//! `core/tools/search.ts` renderers (`CodeSearch`).

use std::rc::Rc;

use hoocode_code_tui_theme::theme;
use hoocode_tui_components::markdown::js_trim;
use serde_json::Value;

use super::text;
use crate::fold::more_lines_hint;
use crate::render_utils::{invalid_arg_text, str_arg};
use crate::tool_execution::{ToolRenderDefinition, ToolResultView};
use crate::tool_output_view::line_budget;

/// A JS number as `String(n)` prints it.
pub(crate) fn js_number(value: &Value) -> String {
    match value {
        Value::Number(n) => match n.as_f64() {
            Some(f) if f.fract() == 0.0 && f.abs() < 1e21 => format!("{}", f as i64),
            Some(f) => format!("{f}"),
            None => n.to_string(),
        },
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".into(),
        other => other.to_string(),
    }
}

fn format_search_call(args: &Value) -> String {
    let t = theme();
    let query_display = match str_arg(args.get("query")) {
        None => invalid_arg_text(),
        Some(q) if !q.is_empty() => t.fg("accent", &q),
        Some(_) => t.fg("toolOutput", "…"),
    };
    let mut text = format!(
        "{}{query_display}",
        t.fg("toolTitle", &t.bold("CodeSearch "))
    );
    let mut extras = Vec::new();
    if let Some(mode) = args.get("mode").and_then(Value::as_str) {
        if !mode.is_empty() && mode != "auto" {
            extras.push(mode.to_string());
        }
    }
    if let Some(limit) = args.get("limit").filter(|v| !v.is_null()) {
        extras.push(js_number(limit));
    }
    if !extras.is_empty() {
        text.push_str(&t.fg("muted", &format!(" ({})", extras.join(", "))));
    }
    text
}

/// One parsed snippet row: its file section and 1-indexed line number.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SearchHit {
    file: String,
    lineno: usize,
    snippet: String,
}

/// `path:start-end [sources]`, the header the assembler prints per file
/// section (`crates/hoocode-code-tool-search/src/assembler.rs`).
fn parse_section_header(line: &str) -> Option<String> {
    let line = line.strip_suffix(']').filter(|_| line.ends_with(']'))?;
    let (head, _) = line.rsplit_once(" [")?;
    let (path, _) = head.rsplit_once(':')?;
    (!path.is_empty()).then(|| path.to_string())
}

/// `  12: snippet`, an indented snippet row under a section header.
fn parse_snippet_row(line: &str) -> Option<(usize, String)> {
    let rest = line.strip_prefix("  ")?.trim_start();
    let (num, snippet) = rest.split_once(':')?;
    Some((
        num.trim().parse().ok()?,
        snippet.strip_prefix(' ').unwrap_or(snippet).to_string(),
    ))
}

/// The result body: one faint-`lineno` + dim-`snippet` row per hit, grouped
/// under a dim file row only when hits span files (a single file's header
/// would repeat the header's own path), then the shared `… N more lines` fold.
fn format_search_result(result: &ToolResultView<'_>, show_images: bool, expanded: bool) -> String {
    let t = theme();
    let output = js_trim(result.text(show_images));
    if output.is_empty() {
        return String::new();
    }
    let mut files: Vec<String> = Vec::new();
    let mut hits: Vec<SearchHit> = Vec::new();
    let mut plain: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    for line in output.split('\n') {
        if line.trim().is_empty() {
            continue;
        }
        if let Some((lineno, snippet)) = parse_snippet_row(line) {
            let file = current.clone().unwrap_or_default();
            if !files.contains(&file) {
                files.push(file.clone());
            }
            hits.push(SearchHit {
                file,
                lineno,
                snippet,
            });
        } else if let Some(path) = parse_section_header(line) {
            current = Some(path);
        } else {
            // `No results for ...`, `[notice]`, or a header-only section.
            plain.push(t.fg("dim", line));
        }
    }
    if hits.is_empty() {
        return plain.join("\n");
    }
    let multi_file = files.len() > 1;
    let digits = hits
        .iter()
        .map(|h| h.lineno.to_string().len())
        .max()
        .unwrap_or(1)
        .max(2);
    let mut rows: Vec<String> = Vec::new();
    let mut last_file: Option<&str> = None;
    for hit in &hits {
        if multi_file && last_file != Some(hit.file.as_str()) {
            rows.push(t.fg("dim", &hit.file));
            last_file = Some(hit.file.as_str());
        }
        let num = format!("{:>digits$}", hit.lineno, digits = digits);
        rows.push(format!(
            "{} {}",
            t.fg("faint", &num),
            t.fg("dim", &hit.snippet)
        ));
    }
    rows.extend(plain);
    let budget = line_budget(expanded);
    if rows.len() <= budget {
        return rows.join("\n");
    }
    let hidden = rows.len() - budget;
    rows.truncate(budget);
    rows.push(more_lines_hint(hidden));
    rows.join("\n")
}

pub fn definition() -> ToolRenderDefinition {
    ToolRenderDefinition {
        render_call: Some(Rc::new(|args, _ctx| Ok(text(format_search_call(args))))),
        render_result: Some(Rc::new(|result, _, ctx| {
            Ok(text(format_search_result(
                result,
                ctx.show_images,
                ctx.expanded,
            )))
        })),
    }
}

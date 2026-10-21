//! `core/tools/bash.ts` renderers: `$ command` and the output preview (the
//! last peek-budget visual lines), truncation notes and the `Took`/`Elapsed`
//! line.

use std::cell::RefCell;
use std::rc::Rc;

use hoocode_code_tool_api::{format_size, DEFAULT_MAX_BYTES};
use hoocode_code_tui_theme::theme;
use hoocode_tui_components::markdown::js_trim;
use hoocode_tui_components::Text;
use hoocode_tui_render::Component;
use serde_json::{json, Value};

use super::search::js_number;
use super::text;
use crate::fold::earlier_lines_hint;
use crate::render_utils::{invalid_arg_text, str_arg};
use crate::tool_execution::{
    body_layout, BodyLayout, ToolRenderDefinition, ToolRenderResultOptions, ToolResultView,
};
use crate::tool_output_view::line_budget;
use crate::visual_truncate::truncate_to_visual_lines;

/// State key: the block needs re-rendering every second (a live `Elapsed`).
pub const TICKING_KEY: &str = "interval";

fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0)
}

/// `formatDuration`: seconds with one decimal.
pub(crate) fn format_duration(ms: f64) -> String {
    hoocode_code_agent_session::format::js_to_fixed(ms / 1000.0, 1) + "s"
}

pub fn format_bash_call(args: &Value) -> String {
    let t = theme();
    let command = str_arg(args.get("command"));
    let timeout = args.get("timeout").filter(|v| match v {
        Value::Null => false,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::Bool(b) => *b,
        Value::String(s) => !s.is_empty(),
        _ => true,
    });
    let suffix = timeout
        .map(|v| t.fg("muted", &format!(" (timeout {}s)", js_number(v))))
        .unwrap_or_default();
    let display = match command {
        None => invalid_arg_text(),
        Some(c) if !c.is_empty() => c,
        Some(_) => t.fg("toolOutput", "…"),
    };
    format!("{}  {display}{suffix}", t.fg("toolTitle", &t.bold("Shell")))
}

/// The result body (`BashResultRenderComponent`), laid out at render width.
pub struct BashResult {
    styled_output: Option<String>,
    /// Tells the step how many rows at the end of the body are not output.
    layout: Rc<BodyLayout>,
    /// The full stop: every line of the output, wrapped, nothing folded.
    expanded: bool,
    warning: Option<String>,
    timing: Option<String>,
}

impl Component for BashResult {
    fn render(&mut self, width: u16) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(output) = &self.styled_output {
            let preview = truncate_to_visual_lines(output, line_budget(self.expanded), width, 0);
            if preview.skipped_count > 0 {
                lines.push(earlier_lines_hint(preview.skipped_count));
            }
            lines.extend(preview.visual_lines);
        }
        let output_rows = lines.len();
        for extra in [&self.warning, &self.timing].into_iter().flatten() {
            lines.extend(Text::new(extra.clone(), 0, 0).render(width));
        }
        self.layout.footer_rows.set(lines.len() - output_rows);
        lines
    }
}

fn build_result(
    result: &ToolResultView<'_>,
    options: ToolRenderResultOptions,
    show_images: bool,
    started_at: Option<f64>,
    ended_at: Option<f64>,
    // A failed run shows no `Took`: the header's `HIT` chip carries the
    // exit code and duration instead.
    is_error: bool,
    layout: Rc<BodyLayout>,
) -> BashResult {
    let t = theme();
    let output = js_trim(result.text(show_images));
    let styled_output = (!output.is_empty()).then(|| {
        output
            .split('\n')
            .map(|l| t.fg("toolOutput", l))
            .collect::<Vec<_>>()
            .join("\n")
    });

    let truncation = result.details.get("truncation").filter(|v| !v.is_null());
    let truncated = truncation.is_some_and(|tr| tr.get("truncated") == Some(&Value::Bool(true)));
    let full_output_path = result
        .details
        .get("fullOutputPath")
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty());
    let warning = (truncated || full_output_path.is_some()).then(|| {
        let mut warnings = Vec::new();
        if let Some(path) = full_output_path {
            warnings.push(format!("Full output: {path}"));
        }
        if let Some(tr) = truncation.filter(|_| truncated) {
            let num = |k: &str| {
                tr.get(k)
                    .map(js_number)
                    .unwrap_or_else(|| "undefined".into())
            };
            if tr.get("truncatedBy").and_then(Value::as_str) == Some("lines") {
                warnings.push(format!(
                    "Truncated: showing {} of {} lines",
                    num("outputLines"),
                    num("totalLines")
                ));
            } else {
                let max_bytes = tr
                    .get("maxBytes")
                    .and_then(Value::as_u64)
                    .map(|n| n as usize)
                    .unwrap_or(DEFAULT_MAX_BYTES);
                warnings.push(format!(
                    "Truncated: {} lines shown ({} limit)",
                    num("outputLines"),
                    format_size(max_bytes)
                ));
            }
        }
        format!(
            "\n{}",
            t.fg("warning", &format!("[{}]", warnings.join(". ")))
        )
    });

    // Live: the wall clock from the call's start. A rebuilt transcript has no
    // clock; the duration the loop stored on the result stands in for it.
    let elapsed_ms = match started_at {
        Some(start) => Some(ended_at.unwrap_or_else(now_ms) - start),
        None if !options.is_partial => result.details.get("durationMs").and_then(Value::as_f64),
        None => None,
    };
    let timing = (!is_error).then_some(elapsed_ms).flatten().map(|ms| {
        let label = if options.is_partial {
            "Elapsed"
        } else {
            "Took"
        };
        format!(
            "\n{}",
            t.fg("muted", &format!("{label} {}", format_duration(ms)))
        )
    });

    BashResult {
        styled_output,
        layout,
        expanded: options.expanded,
        warning,
        timing,
    }
}

pub fn definition() -> ToolRenderDefinition {
    ToolRenderDefinition {
        render_call: Some(Rc::new(|args, ctx| {
            if ctx.execution_started && ctx.state.get("startedAt").is_none() {
                ctx.state.insert("startedAt".into(), json!(now_ms()));
                ctx.state.remove("endedAt");
            }
            Ok(text(format_bash_call(args)))
        })),
        render_result: Some(Rc::new(|result, options, ctx| {
            let started = ctx.state.get("startedAt").and_then(Value::as_f64);
            if started.is_some() && options.is_partial && !ctx.state.contains_key(TICKING_KEY) {
                ctx.state.insert(TICKING_KEY.into(), json!(true));
            }
            if !options.is_partial || ctx.is_error {
                if ctx.state.get("endedAt").is_none() {
                    ctx.state.insert("endedAt".into(), json!(now_ms()));
                }
                ctx.state.remove(TICKING_KEY);
            }
            let ended = ctx.state.get("endedAt").and_then(Value::as_f64);
            let body = build_result(
                result,
                options,
                ctx.show_images,
                started,
                ended,
                ctx.is_error,
                body_layout(ctx.objects, true),
            );
            Ok(Rc::new(RefCell::new(body)))
        })),
    }
}

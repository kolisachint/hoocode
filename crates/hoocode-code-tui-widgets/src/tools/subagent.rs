//! `core/tools/subagent.ts` renderers: the `Agent` call line and `AgentOutput`
//! (its call, and the result body or roster). The legacy `Task`/`AgentOutput`
//! names render through the same functions.

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use hoocode_ai_types::Content;
use hoocode_code_agent_session::format::format_duration_secs;
use hoocode_code_subagents::inbox::subagent_inbox;
use hoocode_code_tui_theme::{agent_color_for, theme};
use hoocode_tui_components::markdown::js_trim;
use hoocode_tui_components::Text;
use hoocode_tui_render::{Component, ComponentHandle};
use hoocode_tui_util::wrap_text_with_ansi;
use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;

use super::text;
use crate::tool_chain_summary::{TaskOutcome, TaskState};
use crate::tool_execution::{ToolRenderDefinition, ToolResultView};
use crate::tool_output_view::peek_block;
use crate::tool_signal::ToolResult;

/// A JS value as `String(value ?? "")` prints it.
fn js_string(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => super::search::js_number(other),
    }
}

fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Some(_) => true,
    }
}

/// The color of a `type#n` label's agent, or `fallback` for a raw id.
pub(crate) fn label_color(label: &str, fallback: &'static str) -> &'static str {
    match label.find('#') {
        Some(i) if i > 0 => agent_color_for(hoocode_tui_util::text_slice::prefix(label, i)),
        _ => fallback,
    }
}

pub fn format_task_call(args: &Value) -> String {
    let t = theme();
    let agent_type = match args.get("subagent_type") {
        None | Some(Value::Null) => "agent".to_string(),
        Some(v) => js_string(Some(v)),
    };
    // The line says the tool's own name: `Agent explore`, `Agent resume
    // explore`. The pin's `Agent [explore]` was right about the word while the
    // tool was still called `Task`; now the two agree.
    let title = if truthy(args.get("resume_task_id")) {
        "Agent resume "
    } else {
        "Agent "
    };
    let mut tail = Vec::new();
    if let Some(summary) = task_summary(args) {
        tail.push(t.fg("dim", &format!("· {summary}")));
    }
    if args.get("background").and_then(Value::as_bool) == Some(true) {
        tail.push(t.fg("dim", "· background"));
    }
    format!(
        "{}{} {}",
        t.fg("toolTitle", &t.bold(title)),
        t.fg(agent_color_for(&agent_type), &agent_type),
        tail.join(" ")
    )
}

/// What the Agent call line says the task is: `description` when given, else
/// the first line of `prompt`. The first line only, so the call stays one row.
fn task_summary(args: &Value) -> Option<String> {
    ["description", "prompt"]
        .iter()
        .filter_map(|key| args.get(*key).and_then(Value::as_str))
        .find_map(|s| {
            let line = js_trim(s.lines().next().unwrap_or(""));
            (!line.is_empty()).then(|| line.to_string())
        })
}

pub fn format_task_output_call(args: &Value) -> String {
    let t = theme();
    let target = if truthy(args.get("list")) {
        "list".to_string()
    } else {
        js_string(args.get("task_id"))
    };
    let styled = match target.find('#') {
        Some(i) if i > 0 => t.fg(
            agent_color_for(hoocode_tui_util::text_slice::prefix(&target, i)),
            &target,
        ),
        _ => t.fg("dim", &target),
    };
    format!(
        "{}{styled}{}",
        t.fg("toolTitle", &t.bold("AgentOutput ")),
        if truthy(args.get("wait")) {
            t.fg("dim", " (wait)")
        } else {
            String::new()
        }
    )
}

/// One roster line, its label, status and rest coloured. Other lines pass through.
fn roster_line(line: &str) -> String {
    let t = theme();
    match ROSTER_LINE.captures(line) {
        None => t.fg("toolOutput", line),
        Some(caps) => {
            let label = &caps[1];
            let status = &caps[2];
            let rest = &caps[3];
            let status_color = if status == "running" {
                "warning"
            } else if status.starts_with("done") {
                "success"
            } else if status == "collected" || status == "cancelled" {
                "muted"
            } else {
                "error"
            };
            format!(
                "- {}  {}{}",
                t.fg(label_color(label, "accent"), label),
                t.fg(status_color, status),
                t.fg("dim", rest)
            )
        }
    }
}

static ROSTER_LINE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^- (\S+)\s{2}(running|done \(uncollected\)|collected|failed|stalled|timeout|cancelled)(.*)$")
        .expect("roster line")
});

fn now_ms() -> u64 {
    hoocode_code_task_store::now_ms()
}

/// The text blocks of a result, joined by newlines.
fn result_text(content: &[Content]) -> String {
    content
        .iter()
        .filter_map(|c| match c {
            Content::Text(tc) if !tc.text.is_empty() => Some(tc.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The status word of a single task's result (not the roster, not an empty list).
fn card_status(result: &ToolResultView<'_>) -> Option<String> {
    let has_task = result
        .details
        .get("task_id")
        .and_then(Value::as_str)
        .is_some_and(|id| !id.is_empty());
    if !has_task {
        return None;
    }
    result
        .details
        .get("status")
        .and_then(Value::as_str)
        .filter(|s| *s != "list" && *s != "empty")
        .map(String::from)
}

fn status_tone(status: &str) -> &'static str {
    match status {
        "done" => "success",
        "running" => "warning",
        "collected" => "muted",
        "cancelled" | "unknown" => "dim",
        _ => "error",
    }
}

/// How long a task has run, in the panel's format, once the inbox knows it.
pub(crate) fn elapsed_text(label: &str) -> Option<String> {
    let record = subagent_inbox().get(label)?;
    let ms = record
        .ended_at
        .unwrap_or_else(now_ms)
        .saturating_sub(record.started_at);
    (ms > 0).then(|| format_duration_secs(ms as f64 / 1000.0))
}

/// The roster of background subagents (`AgentOutput list`), one row per task,
/// trimmed to the peek budget. A single task's body is not built here: its
/// peek is `peek_body`.
pub fn format_task_output_result(result: &ToolResultView<'_>) -> String {
    let text = result_text(result.content);
    if text.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = text.split('\n').map(str::to_string).collect();
    peek_block(&lines, |shown| {
        shown.iter().map(|line| roster_line(line)).collect()
    })
}

/// The rows a result shows at peek.
const SUMMARY_ROWS: usize = 3;
/// The gutter before every peek row: `  │ `, four columns.
const GUTTER_WIDTH: usize = 4;

/// The gutter before every peek row: `  │ `, as the summary rows draw it.
pub(crate) fn peek_gutter() -> String {
    format!("  {} ", theme().fg("borderMuted", "│"))
}

/// One line of markdown as plain words: no emphasis, heading or bullet marks.
pub(crate) fn strip_markdown(line: &str) -> String {
    let line = line.trim().trim_start_matches('#').trim_start();
    let line = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .unwrap_or(line);
    line.replace("**", "")
        .replace("__", "")
        .replace('`', "")
        .trim()
        .to_string()
}

/// The peek rows of a body: the first `SUMMARY_ROWS` rows of its plain prose,
/// wrapped to `width` behind a gutter, then how many non-blank lines are left.
fn summary_rows(body: &str, width: usize) -> Vec<String> {
    let t = theme();
    let gutter = peek_gutter();
    let inner = width.saturating_sub(GUTTER_WIDTH).max(1);
    let prose: Vec<String> = body
        .split('\n')
        .map(strip_markdown)
        .filter(|line| !line.is_empty())
        .collect();
    let mut rows: Vec<String> = Vec::new();
    // Source lines with at least one row on screen.
    let mut started = 0;
    for line in &prose {
        let room = SUMMARY_ROWS - rows.len();
        if room == 0 {
            break;
        }
        started += 1;
        rows.extend(wrap_text_with_ansi(line, inner).into_iter().take(room));
    }
    let mut out: Vec<String> = rows
        .iter()
        .map(|row| format!("{gutter}{}", t.fg("toolOutput", row)))
        .collect();
    let more = prose.len() - started;
    if more > 0 {
        let noun = if more == 1 { "line" } else { "lines" };
        out.push(format!(
            "{gutter}{}",
            t.fg("muted", &format!("… {more} more {noun}"))
        ));
    }
    out
}

/// A result body at peek: three rows of plain prose, no box.
struct PeekBody {
    text: String,
}

impl Component for PeekBody {
    fn render(&mut self, width: u16) -> Vec<String> {
        summary_rows(&self.text, width as usize)
    }
}

/// A result's body at peek, as the three-row summary.
fn peek_body(text: String) -> ComponentHandle {
    Rc::new(RefCell::new(PeekBody { text }))
}

pub fn task_definition() -> ToolRenderDefinition {
    ToolRenderDefinition {
        render_call: Some(Rc::new(|args, _| Ok(text(format_task_call(args))))),
        render_result: Some(Rc::new(|result, _, _| {
            Ok(peek_body(result_text(result.content)))
        })),
    }
}

/// What a subagent call said about its task, once its result is in. `None`
/// while the call is a background placeholder (its word comes from the
/// `AgentOutput` that collects it) and for a roster, which names no task.
pub fn task_outcome(tool: &str, args: &Value, result: Option<&ToolResult>) -> Option<TaskOutcome> {
    match tool {
        "Agent" => {
            let background = truthy(args.get("background"))
                || result.is_some_and(|r| truthy(r.details.get("background")));
            if background {
                return None;
            }
            let state = result.map_or(TaskState::Running, settled_state);
            Some(TaskOutcome { state, task: None })
        }
        "AgentOutput" => {
            if truthy(args.get("list")) {
                return None;
            }
            let task = js_string(args.get("task_id"));
            let task = task.trim();
            if task.is_empty() {
                return None;
            }
            let state = match result {
                None => TaskState::Running,
                Some(r)
                    if matches!(
                        r.details.get("status").and_then(Value::as_str),
                        Some("list" | "empty")
                    ) =>
                {
                    return None
                }
                Some(r) => settled_state(r),
            };
            Some(TaskOutcome {
                state,
                task: Some(task.to_string()),
            })
        }
        _ => None,
    }
}

/// A finished call's word on its task, from its error text or its details.
fn settled_state(result: &ToolResult) -> TaskState {
    if result.is_error {
        return if result_text(&result.content).contains("cancelled by user") {
            TaskState::Stopped
        } else {
            TaskState::Failed
        };
    }
    match result.details.get("status").and_then(Value::as_str) {
        Some("done" | "collected") => TaskState::Done,
        Some("running") => TaskState::Running,
        Some("cancelled") => TaskState::Stopped,
        Some(_) => TaskState::Failed,
        None => match result.details.get("ok").and_then(Value::as_bool) {
            Some(false) => TaskState::Failed,
            _ => TaskState::Done,
        },
    }
}

/// What an `AgentOutput` call's header learns from its result.
#[derive(Default)]
struct OutputCallState {
    /// The task's status word: `done`, `running`, `failed`, ...
    status: Option<String>,
    /// How long the task has run, e.g. `6m05s`.
    elapsed: Option<String>,
}

const OUTPUT_CALL_KEY: &str = "subagent.outputCall";

fn output_call_state(objects: &mut HashMap<String, Rc<dyn Any>>) -> Rc<RefCell<OutputCallState>> {
    if let Some(existing) = objects.get(OUTPUT_CALL_KEY) {
        if let Ok(state) = existing.clone().downcast::<RefCell<OutputCallState>>() {
            return state;
        }
    }
    let state = Rc::new(RefCell::new(OutputCallState::default()));
    objects.insert(OUTPUT_CALL_KEY.into(), state.clone());
    state
}

/// The `AgentOutput` call line, read at render time so the status a result
/// settles reaches the header without re-running the call slot. It reads as
/// `AgentOutput <handle> · <status> · <elapsed>`.
struct OutputCall {
    args: Value,
    state: Rc<RefCell<OutputCallState>>,
}

impl Component for OutputCall {
    fn render(&mut self, width: u16) -> Vec<String> {
        let t = theme();
        let mut line = format_task_output_call(&self.args);
        let state = self.state.borrow();
        if let Some(status) = &state.status {
            line.push_str(&t.fg("dim", " · "));
            line.push_str(&t.fg(status_tone(status), status));
            if let Some(elapsed) = &state.elapsed {
                line.push_str(&t.fg("dim", &format!(" · {elapsed}")));
            }
        }
        Text::new(line, 0, 0).render(width)
    }
}

pub fn task_output_definition() -> ToolRenderDefinition {
    ToolRenderDefinition {
        render_call: Some(Rc::new(|args, ctx| {
            let state = output_call_state(ctx.objects);
            Ok(Rc::new(RefCell::new(OutputCall {
                args: args.clone(),
                state,
            })))
        })),
        render_result: Some(Rc::new(|result, _, ctx| {
            let Some(status) = card_status(result) else {
                // The roster keeps the peek budget of five lines.
                return Ok(text(format_task_output_result(result)));
            };
            let state = output_call_state(ctx.objects);
            {
                let mut s = state.borrow_mut();
                s.status = Some(status);
                s.elapsed = result
                    .details
                    .get("task_id")
                    .and_then(Value::as_str)
                    .and_then(elapsed_text);
            }
            Ok(peek_body(result_text(result.content)))
        })),
    }
}

//! Live activity of a subagent run, for the UI (Phase T3).
//!
//! Three pieces, all bounded:
//! - [`DeltaCoalescer`]: assistant text deltas are held and sent as one
//!   message at most every [`DELTA_THROTTLE`]. The child uses it on its stdout,
//!   the parent again on the forwarded events, so no text is lost, only grouped.
//! - [`LiveForwarder`]: the parent's pool listener turns pool events into the
//!   `(task id, event)` pairs the UI receives. A run's text is coalesced, and
//!   a run's entries are dropped when it settles.
//! - The tail store ([`apply`], [`tail`], [`is_running`]): the last
//!   [`TAIL_LINES`] rows of a run's activity. It is written on the UI thread
//!   only, and holds a few lines per running task.
//!
//! [`run_for_tool_call`] maps an Agent tool call to its pool run, so a tool
//! block can find its run even when the call returned a background placeholder.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::events::SUBAGENT_STDOUT_EVENT_TYPES;
use crate::pool::PoolEvent;

/// The minimum gap between two text deltas of one run. It is the TUI's
/// streaming render rate (`STREAM_RENDER_THROTTLE` in `hoocode-code-tui-app`).
pub const DELTA_THROTTLE: Duration = Duration::from_millis(100);

/// Rows of activity a run keeps, and rows a tool block shows.
pub const TAIL_LINES: usize = 5;

/// Most characters of one line kept (the rest of a long line is cut).
const LINE_CHARS: usize = 240;

/// Most Agent tool calls remembered for [`run_for_tool_call`].
const CALL_RUNS_KEPT: usize = 256;

/// Holds assistant text deltas and releases them as one, at most once per
/// throttle window. Nothing is dropped: the held text goes out with the next
/// release, or with [`DeltaCoalescer::flush`].
#[derive(Debug, Default)]
pub struct DeltaCoalescer {
    last: Option<Instant>,
    pending: String,
}

impl DeltaCoalescer {
    /// Adds `delta`. Returns the text to send now, if the window has passed.
    pub fn push(&mut self, delta: &str, now: Instant, throttle: Duration) -> Option<String> {
        self.pending.push_str(delta);
        if self
            .last
            .is_some_and(|last| now.saturating_duration_since(last) < throttle)
        {
            return None;
        }
        self.last = Some(now);
        Some(std::mem::take(&mut self.pending))
    }

    /// The held text, if any (the message ended, or the run did).
    pub fn flush(&mut self) -> Option<String> {
        (!self.pending.is_empty()).then(|| std::mem::take(&mut self.pending))
    }

    /// The held text of a message that ended, and a fresh window for the next
    /// message: its first delta is released at once. Without the reset, how a
    /// message splits would depend on how long the work between messages took.
    pub fn end_message(&mut self) -> Option<String> {
        self.last = None;
        self.flush()
    }
}

/// The UI's side of the live events: where they go, and the throttle.
#[derive(Clone)]
pub struct LiveSink {
    pub throttle: Duration,
    pub send: Arc<dyn Fn(String, Value) + Send + Sync>,
}

/// Turns pool events into the `(task id, event)` pairs the UI receives.
#[derive(Default)]
pub struct LiveForwarder {
    coalescers: HashMap<String, DeltaCoalescer>,
}

impl LiveForwarder {
    /// The pairs to send for one pool event, in order. Text deltas may be held
    /// back; a run's settle releases what it held.
    pub fn forward(
        &mut self,
        event: &PoolEvent,
        now: Instant,
        throttle: Duration,
    ) -> Vec<(String, Value)> {
        let Some(task_id) = event.data.get("task_id").and_then(Value::as_str) else {
            return Vec::new();
        };
        let task_id = task_id.to_string();
        if event.name == "task_progress" {
            let Some(inner) = event.data.get("event") else {
                return Vec::new();
            };
            if inner.get("type").and_then(Value::as_str) == Some("message_update") {
                let delta = inner
                    .get("assistantMessageEvent")
                    .and_then(|e| e.get("delta"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let coalescer = self.coalescers.entry(task_id.clone()).or_default();
                return match coalescer.push(delta, now, throttle) {
                    Some(text) => vec![(task_id, text_delta_event(&text))],
                    None => Vec::new(),
                };
            }
            // Anything else ends the held text first, so the order holds. A
            // message's end also opens a fresh window for the next message.
            let mut out = if inner.get("type").and_then(Value::as_str) == Some("message_end") {
                self.end_task_message(&task_id)
            } else {
                self.flush_task(&task_id)
            };
            out.push((task_id, inner.clone()));
            return out;
        }
        if event.name.starts_with("task_") {
            // A lifecycle event: started, or a settle. The run's held text goes first.
            let mut out = self.flush_task(&task_id);
            if event.name != "task_started" {
                self.coalescers.remove(&task_id);
            }
            out.push((task_id, json!({"type": event.name})));
            return out;
        }
        Vec::new()
    }

    fn end_task_message(&mut self, task_id: &str) -> Vec<(String, Value)> {
        match self
            .coalescers
            .get_mut(task_id)
            .and_then(DeltaCoalescer::end_message)
        {
            Some(text) => vec![(task_id.to_string(), text_delta_event(&text))],
            None => Vec::new(),
        }
    }

    fn flush_task(&mut self, task_id: &str) -> Vec<(String, Value)> {
        match self
            .coalescers
            .get_mut(task_id)
            .and_then(DeltaCoalescer::flush)
        {
            Some(text) => vec![(task_id.to_string(), text_delta_event(&text))],
            None => Vec::new(),
        }
    }
}

/// A `message_update` carrying one text delta, as the UI reads it.
pub fn text_delta_event(delta: &str) -> Value {
    json!({
        "type": "message_update",
        "assistantMessageEvent": {"type": "text_delta", "delta": delta},
    })
}

/// What a subagent child writes to its stdout for one session event. Only the
/// types in [`SUBAGENT_STDOUT_EVENT_TYPES`] pass. A `message_update` is kept
/// only as a text delta, coalesced; the rest of its payload (the partial
/// message, repeated on every delta) is dropped. Held text is released before
/// any other event that passes: a non-text update, `message_end` or any other
/// type, so the child's stream keeps its order.
pub fn child_stdout_events(
    event: Value,
    coalescer: &mut DeltaCoalescer,
    now: Instant,
) -> Vec<Value> {
    let kind = event.get("type").and_then(Value::as_str).unwrap_or("");
    if !SUBAGENT_STDOUT_EVENT_TYPES.contains(&kind) {
        return Vec::new();
    }
    if kind == "message_update" {
        let delta = event
            .get("assistantMessageEvent")
            .filter(|e| e.get("type").and_then(Value::as_str) == Some("text_delta"))
            .and_then(|e| e.get("delta"))
            .and_then(Value::as_str);
        if let Some(delta) = delta {
            return coalescer
                .push(delta, now, DELTA_THROTTLE)
                .map(|text| vec![text_delta_event(&text)])
                .unwrap_or_default();
        }
        // A non-text update (thinking, tool-call arguments) ends the held text.
        return flushed(coalescer);
    }
    // A message's end also starts a fresh throttle window for the next message.
    let mut out = if kind == "message_end" {
        coalescer
            .end_message()
            .map(|text| vec![text_delta_event(&text)])
            .unwrap_or_default()
    } else {
        flushed(coalescer)
    };
    out.push(event);
    out
}

/// The held text as one event, if any.
fn flushed(coalescer: &mut DeltaCoalescer) -> Vec<Value> {
    coalescer
        .flush()
        .map(|text| vec![text_delta_event(&text)])
        .unwrap_or_default()
}

/// Most sub-steps a run keeps: its running calls and its failed ones.
const SUB_STEPS_KEPT: usize = 16;

/// One of a subagent's own tool calls, as the radar flight log shows it:
/// a running call, or a failed one that stays until it is read. A call that
/// settles clean is dropped.
#[derive(Debug, Clone, PartialEq)]
pub struct SubStep {
    /// The child's tool call id.
    pub id: String,
    pub tool: String,
    /// The call's subject (a path, a command), as the tool's args name it.
    pub subject: String,
    /// When the call started.
    pub started: Instant,
    /// The first line of the failure's output, once the call failed.
    pub failure: Option<String>,
}

/// One run's activity: its last rows, the line being streamed, and its
/// own tool calls still running or failed.
#[derive(Default)]
struct RunTail {
    rows: VecDeque<String>,
    partial: String,
    steps: Vec<SubStep>,
}

static TAILS: LazyLock<Mutex<HashMap<String, RunTail>>> = LazyLock::new(Mutex::default);

/// The Agent tool calls remembered for [`run_for_tool_call`]: a map for the
/// lookup (the UI makes one per tool block on every event), and the call ids
/// oldest first for eviction.
#[derive(Default)]
struct CallRuns {
    by_call: HashMap<String, String>,
    order: VecDeque<String>,
}

static CALL_RUNS: LazyLock<Mutex<CallRuns>> = LazyLock::new(Mutex::default);

fn tails() -> MutexGuard<'static, HashMap<String, RunTail>> {
    TAILS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Records one forwarded event of a run on the UI thread. `task_started`
/// opens the run's tail; any other `task_*` event closes it.
pub fn apply(task_id: &str, event: &Value) {
    let kind = event.get("type").and_then(Value::as_str).unwrap_or("");
    if kind == "task_started" {
        tails().insert(task_id.to_string(), RunTail::default());
        return;
    }
    if kind.starts_with("task_") {
        tails().remove(task_id);
        return;
    }
    let mut guard = tails();
    let Some(run) = guard.get_mut(task_id) else {
        return;
    };
    match kind {
        "tool_execution_start" => {
            flush_partial(run);
            let tool = event
                .get("toolName")
                .and_then(Value::as_str)
                .unwrap_or("tool");
            let subject = tool_subject(event.get("args"));
            push_row(run, &format!("→ {tool}{subject}"));
            run.steps.push(SubStep {
                id: str_of(event, "toolCallId"),
                tool: tool.to_string(),
                subject: subject.trim().to_string(),
                started: Instant::now(),
                failure: None,
            });
            while run.steps.len() > SUB_STEPS_KEPT {
                run.steps.remove(0);
            }
        }
        "tool_execution_end" => {
            let id = str_of(event, "toolCallId");
            let Some(at) = run.steps.iter().position(|step| step.id == id) else {
                return;
            };
            if event.get("isError").and_then(Value::as_bool) == Some(true) {
                run.steps[at].failure = Some(end_snippet(event));
            } else {
                run.steps.remove(at);
            }
        }
        "message_update" => {
            let delta = event
                .get("assistantMessageEvent")
                .and_then(|e| e.get("delta"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let delta = printable(delta);
            for (i, part) in delta.split('\n').enumerate() {
                if i > 0 {
                    flush_partial(run);
                }
                run.partial.push_str(part);
            }
            clip_partial(&mut run.partial);
        }
        "message_end" | "turn_end" => flush_partial(run),
        _ => {}
    }
}

fn str_of(event: &Value, key: &str) -> String {
    event
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// The first non-blank line of a finished call's text output.
fn end_snippet(event: &Value) -> String {
    event
        .pointer("/result/content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .flat_map(str::lines)
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(clip_line)
        .unwrap_or_default()
}

/// The run's own tool calls still running or failed, oldest first. Empty
/// once the run settles.
pub fn sub_steps(task_id: &str) -> Vec<SubStep> {
    tails()
        .get(task_id)
        .map(|run| run.steps.clone())
        .unwrap_or_default()
}

/// The run's last [`TAIL_LINES`] rows: finished rows, then the line being
/// streamed. Empty before the first activity.
pub fn tail(task_id: &str) -> Vec<String> {
    let guard = tails();
    let Some(run) = guard.get(task_id) else {
        return Vec::new();
    };
    let mut rows: Vec<String> = run.rows.iter().cloned().collect();
    if !run.partial.is_empty() {
        rows.push(run.partial.clone());
    }
    let start = rows.len().saturating_sub(TAIL_LINES);
    rows.split_off(start)
}

/// Whether the run has started and not yet settled.
pub fn is_running(task_id: &str) -> bool {
    tails().contains_key(task_id)
}

fn push_row(run: &mut RunTail, row: &str) {
    run.rows.push_back(clip_line(&printable(row)));
    while run.rows.len() > TAIL_LINES {
        run.rows.pop_front();
    }
}

fn flush_partial(run: &mut RunTail) {
    let row = std::mem::take(&mut run.partial);
    if !row.trim().is_empty() {
        push_row(run, &row);
    }
}

/// Keeps the newest characters of a streamed line.
fn clip_partial(partial: &mut String) {
    let count = partial.chars().count();
    if count > LINE_CHARS {
        *partial = partial.chars().skip(count - LINE_CHARS).collect();
    }
}

/// The text with its control characters removed (a tab becomes a space), so a
/// child's escape sequences reach the screen as plain text. Newlines are kept.
fn printable(text: &str) -> String {
    text.chars()
        .filter_map(|c| match c {
            '\t' => Some(' '),
            '\n' => Some('\n'),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect()
}

fn clip_line(row: &str) -> String {
    row.chars().take(LINE_CHARS).collect()
}

/// The argument a tool line names: its path, command or pattern.
fn tool_subject(args: Option<&Value>) -> String {
    let Some(args) = args else {
        return String::new();
    };
    ["file_path", "path", "command", "pattern", "url", "query"]
        .iter()
        .find_map(|key| args.get(*key).and_then(Value::as_str))
        .and_then(|s| s.lines().next())
        .map(|s| format!(" {}", s.trim()))
        .unwrap_or_default()
}

/// Remembers that the Agent tool call `tool_call_id` dispatched pool run `task_id`.
pub fn remember_run(tool_call_id: &str, task_id: &str) {
    let mut runs = CALL_RUNS.lock().unwrap_or_else(|e| e.into_inner());
    let CallRuns { by_call, order } = &mut *runs;
    if by_call
        .insert(tool_call_id.to_string(), task_id.to_string())
        .is_none()
    {
        order.push_back(tool_call_id.to_string());
    }
    while order.len() > CALL_RUNS_KEPT {
        if let Some(old) = order.pop_front() {
            by_call.remove(&old);
        }
    }
}

/// The pool run an Agent tool call dispatched, if it still remembers it.
pub fn run_for_tool_call(tool_call_id: &str) -> Option<String> {
    let runs = CALL_RUNS.lock().unwrap_or_else(|e| e.into_inner());
    runs.by_call.get(tool_call_id).cloned()
}

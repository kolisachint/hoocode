//! `core/subagent-result.ts`: a subagent's `result.json` audit file.
//!
//! A child run with `--task-id <id>` writes
//! `.hoocode/dispatch/<task_id>/result.json`, which the parent pool verifies
//! against a fixed schema ([`OutputVerifier`](crate::output_verifier::OutputVerifier)).
//! The file is derived from the finished session, so subagents never write it
//! themselves.

use std::collections::HashMap;
use std::path::Path;

use hoocode_agent_types::AgentMessage;
use hoocode_ai_types::{Content, StopReason};
use hoocode_code_agent_session::stats::{
    last_assistant_text_with_origin, FROM_EARLIER_TURN_PREFIX,
};
use hoocode_code_task_store::{Task, TaskSource, TaskStatus, TaskUsage};
use serde_json::{json, Map, Value};

/// `SubagentUsage`.
pub type SubagentUsage = TaskUsage;

/// `SubagentTaskNode`: one node of a subagent's task subtree, propagated to
/// the parent so nested delegation is visible above the process boundary.
#[derive(Debug, Clone, PartialEq)]
pub struct SubagentTaskNode {
    pub id: u64,
    pub title: String,
    pub status: TaskStatus,
    /// Origin of the task (unset: the subagent's own TodoWrite plan).
    pub source: Option<TaskSource>,
    pub subagent_mode: Option<String>,
    pub usage: Option<SubagentUsage>,
    pub children: Vec<SubagentTaskNode>,
}

/// `status` of a result file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultStatus {
    Complete,
    Partial,
    Failed,
}

impl ResultStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Partial => "partial",
            Self::Failed => "failed",
        }
    }
}

/// `SubagentResultFile`.
#[derive(Debug, Clone, PartialEq)]
pub struct SubagentResultFile {
    pub summary: String,
    pub files_changed: Vec<String>,
    pub confidence: f64,
    pub status: ResultStatus,
    /// Token and cost usage (extra field; ignored by the verifier).
    pub usage: Option<SubagentUsage>,
    /// The child's own task subtree (extra field; ignored by the verifier).
    pub task_tree: Option<Vec<SubagentTaskNode>>,
}

/// A JSON number as JS holds it: integral values print without `.0`.
pub(crate) fn js_number(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 9.007_199_254_740_992e15 {
        Value::from(n as i64)
    } else {
        serde_json::Number::from_f64(n).map_or(Value::Null, Value::Number)
    }
}

fn usage_json(usage: &SubagentUsage) -> Value {
    json!({
        "input": js_number(usage.input),
        "output": js_number(usage.output),
        "cacheRead": js_number(usage.cache_read),
        "cacheWrite": js_number(usage.cache_write),
        "cost": js_number(usage.cost),
    })
}

fn source_str(source: TaskSource) -> &'static str {
    match source {
        TaskSource::Subagent => "subagent",
        TaskSource::Mcp => "mcp",
        TaskSource::Schedule => "schedule",
        TaskSource::Shell => "shell",
    }
}

impl SubagentTaskNode {
    /// The node in `JSON.stringify` shape (unset fields omitted).
    pub fn to_json(&self) -> Value {
        let mut map = Map::new();
        map.insert("id".into(), Value::from(self.id));
        map.insert("title".into(), Value::from(self.title.clone()));
        map.insert("status".into(), Value::from(self.status.as_str()));
        if let Some(source) = self.source {
            map.insert("source".into(), Value::from(source_str(source)));
        }
        if let Some(mode) = &self.subagent_mode {
            map.insert("subagentMode".into(), Value::from(mode.clone()));
        }
        if let Some(usage) = &self.usage {
            map.insert("usage".into(), usage_json(usage));
        }
        map.insert(
            "children".into(),
            Value::Array(self.children.iter().map(Self::to_json).collect()),
        );
        Value::Object(map)
    }
}

impl SubagentResultFile {
    /// The file's JSON (field order as hoocode writes it).
    pub fn to_json(&self) -> Value {
        let mut map = Map::new();
        map.insert("summary".into(), Value::from(self.summary.clone()));
        map.insert("files_changed".into(), json!(self.files_changed));
        map.insert("confidence".into(), js_number(self.confidence));
        map.insert("status".into(), Value::from(self.status.as_str()));
        if let Some(usage) = &self.usage {
            map.insert("usage".into(), usage_json(usage));
        }
        if let Some(tree) = &self.task_tree {
            map.insert(
                "task_tree".into(),
                Value::Array(tree.iter().map(SubagentTaskNode::to_json).collect()),
            );
        }
        Value::Object(map)
    }
}

/// `buildTaskForest`: link a flat task list into a forest via
/// `parent_task_id`, keeping store order among siblings.
pub fn build_task_forest(tasks: &[Task]) -> Vec<SubagentTaskNode> {
    let mut children_by_parent: HashMap<u64, Vec<&Task>> = HashMap::new();
    let mut roots = Vec::new();
    for task in tasks {
        match task.parent_task_id {
            None => roots.push(task),
            Some(parent) => children_by_parent.entry(parent).or_default().push(task),
        }
    }
    fn to_node(task: &Task, children: &HashMap<u64, Vec<&Task>>) -> SubagentTaskNode {
        SubagentTaskNode {
            id: task.id,
            title: task.title.clone(),
            status: task.status,
            source: task.source,
            subagent_mode: task.subagent_mode.clone(),
            usage: task.usage,
            children: children
                .get(&task.id)
                .map(|kids| kids.iter().map(|k| to_node(k, children)).collect())
                .unwrap_or_default(),
        }
    }
    roots
        .into_iter()
        .map(|t| to_node(t, &children_by_parent))
        .collect()
}

/// Tools that mutate files; their `path`/`file_path` argument is a changed file.
const MUTATING_TOOLS: &[&str] = &["Edit", "Write"];

/// Distinct file paths touched by edit/write tool calls, in first-seen order.
fn collect_changed_files(messages: &[AgentMessage]) -> Vec<String> {
    let mut files: Vec<String> = Vec::new();
    for message in messages {
        let AgentMessage::Assistant(assistant) = message else {
            continue;
        };
        for content in &assistant.content {
            let Content::ToolCall(call) = content else {
                continue;
            };
            if !MUTATING_TOOLS.contains(&call.name.as_str()) {
                continue;
            }
            let path = call
                .arguments
                .get("path")
                .and_then(Value::as_str)
                .or_else(|| call.arguments.get("file_path").and_then(Value::as_str));
            if let Some(path) = path.filter(|p| !p.is_empty()) {
                if !files.iter().any(|f| f == path) {
                    files.push(path.to_string());
                }
            }
        }
    }
    files
}

/// The last non-empty assistant text, trimmed. When tool calls followed that
/// text, it is prefixed so the parent knows it may be stale narration.
fn derive_summary(messages: &[AgentMessage]) -> String {
    match last_assistant_text_with_origin(messages) {
        Some((text, true)) => format!("{FROM_EARLIER_TURN_PREFIX}{text}"),
        Some((text, false)) => text,
        None => String::new(),
    }
}

fn last_assistant(messages: &[AgentMessage]) -> Option<&hoocode_ai_types::AssistantMessage> {
    match messages.last() {
        Some(AgentMessage::Assistant(assistant)) => Some(assistant),
        _ => None,
    }
}

/// Failed when the final assistant message errored or was aborted.
fn derive_failed(messages: &[AgentMessage]) -> bool {
    last_assistant(messages)
        .is_some_and(|a| matches!(a.stop_reason, StopReason::Error | StopReason::Aborted))
}

/// The final assistant message's error text when the run ended in an error.
fn derive_error_message(messages: &[AgentMessage]) -> Option<String> {
    let assistant = last_assistant(messages)?;
    if assistant.stop_reason != StopReason::Error {
        return None;
    }
    let trimmed = assistant.error_message.as_deref()?.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// `BuildSubagentResultOptions`.
#[derive(Debug, Clone, Copy, Default)]
pub struct BuildSubagentResultOptions {
    /// The run stopped at its turn cap: report a usable partial result.
    pub reached_max_turns: bool,
    /// The run was asked to wrap up because it was nearing its wall-clock
    /// deadline. Same treatment: the work is usable, but it was cut short.
    pub reached_deadline: bool,
    /// The supervisor stopped the run for lack of progress or over the memory
    /// limit. Not a deadline: the run did not use up its time.
    pub stalled: bool,
}

impl BuildSubagentResultOptions {
    /// Whether the run was cut short rather than finished on its own terms.
    fn cut_short(&self) -> bool {
        self.reached_max_turns || self.reached_deadline || self.stalled
    }

    /// The stop reason named in the summary when the run has no final text.
    fn stop_reason(&self) -> Option<&'static str> {
        if self.stalled {
            Some("stalled")
        } else if self.reached_deadline {
            Some("deadline")
        } else if self.reached_max_turns {
            Some("max-turns")
        } else {
            None
        }
    }
}

fn or_else(text: String, fallback: &str) -> String {
    if text.is_empty() {
        fallback.to_string()
    } else {
        text
    }
}

/// The summary for a run with no final text anywhere in its transcript: the
/// stop reason, the turn count, and the last tool it called. `stop` is the
/// supervisor's reason when it cut the run short; otherwise the final
/// assistant message's own stop reason is named.
fn no_text_summary(messages: &[AgentMessage], stop: Option<&str>) -> String {
    let stop = stop.map(str::to_string).unwrap_or_else(|| {
        last_assistant(messages)
            .map(|a| {
                match a.stop_reason {
                    StopReason::Stop => "stop",
                    StopReason::Length => "length",
                    StopReason::ToolUse => "tool-use",
                    StopReason::Error => "error",
                    StopReason::Aborted => "aborted",
                }
                .to_string()
            })
            .unwrap_or_else(|| "unknown".to_string())
    });
    let turns = messages
        .iter()
        .filter(|m| matches!(m, AgentMessage::Assistant(_)))
        .count();
    let last_tool = messages
        .iter()
        .rev()
        .find_map(|m| match m {
            AgentMessage::Assistant(a) => a.content.iter().rev().find_map(|c| match c {
                Content::ToolCall(call) => Some(call.name.clone()),
                _ => None,
            }),
            _ => None,
        })
        .unwrap_or_else(|| "none".to_string());
    format!("No final text. Stop reason: {stop}; turns: {turns}; last tool: {last_tool}")
}

/// `buildSubagentResult`: the `result.json` payload for a finished session.
/// The verifier needs a non-empty summary and confidence >= 0.5, so every
/// branch yields both.
pub fn build_subagent_result(
    messages: &[AgentMessage],
    usage: Option<SubagentUsage>,
    options: BuildSubagentResultOptions,
) -> SubagentResultFile {
    let files_changed = collect_changed_files(messages);
    if options.cut_short() {
        return SubagentResultFile {
            summary: or_else(
                derive_summary(messages),
                &no_text_summary(messages, options.stop_reason()),
            ),
            files_changed,
            confidence: 0.6,
            status: ResultStatus::Partial,
            usage,
            task_tree: None,
        };
    }
    if derive_failed(messages) {
        // Prefer the provider/model error over partial assistant text.
        let summary = match derive_error_message(messages) {
            Some(error) => format!("Task failed: {error}"),
            None => or_else(derive_summary(messages), "Task failed."),
        };
        return SubagentResultFile {
            summary,
            files_changed,
            confidence: 0.5,
            status: ResultStatus::Failed,
            usage,
            task_tree: None,
        };
    }
    SubagentResultFile {
        summary: or_else(derive_summary(messages), &no_text_summary(messages, None)),
        files_changed,
        confidence: 0.9,
        status: ResultStatus::Complete,
        usage,
        task_tree: None,
    }
}

/// `writeFileAtomicSync`: temp file in the same directory, then rename.
pub(crate) fn write_file_atomic(path: &Path, data: &str) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let mut tmp = tempfile::Builder::new()
        .prefix(".")
        .suffix(".tmp")
        .tempfile_in(dir)?;
    std::io::Write::write_all(&mut tmp, data.as_bytes())?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// `writeSubagentResult`: write `result.json` for a task.
///
/// Returns whether it landed. It used to be `let _ = write_file_atomic(...)`,
/// which meant a full disk or a read-only dispatch dir produced a child that
/// exited cleanly with no result — and the parent reported a failure with no
/// cause. The caller now turns a failed write into a non-zero exit and a log
/// line.
pub fn write_subagent_result(cwd: &Path, task_id: &str, result: &SubagentResultFile) -> bool {
    let path = hoocode_code_paths::dispatch_task_dir(cwd, task_id).join("result.json");
    // Atomic: the parent may read the file the moment it appears, or kill the
    // child mid-write; a torn file would turn a success into a failure.
    let text = serde_json::to_string_pretty(&result.to_json()).unwrap_or_default();
    match write_file_atomic(&path, &text) {
        Ok(()) => true,
        Err(error) => {
            crate::agent_log::agent_log(&format!(
                "[DISPATCH] result.json for {task_id} could not be written: {error}"
            ));
            false
        }
    }
}

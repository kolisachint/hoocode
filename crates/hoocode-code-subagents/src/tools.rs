//! `core/tools/subagent.ts`: the `Task` tool (delegate a focused task to a
//! specialized subagent) and the `AgentOutput` tool (check on and collect
//! background subagents).
//!
//! The parent agent decides when to delegate and picks the agent with
//! `subagent_type`; the chosen agent runs as a separate child process with its own context (the
//! cold pool, or a warm RPC worker when enabled) and only its final answer
//! comes back. Background dispatches leave their body in the inbox and return
//! a compact notification; `AgentOutput` pulls it. Interactive rendering
//! (`renderCall`/`renderResult`) arrives with the TUI (phase 11).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hoocode_agent_types::{AgentToolCall, AgentToolResult, AgentToolUpdateCallback};
use hoocode_ai_types::{AbortSignal, Content, Model};
use hoocode_code_agent_session::provider_health::get_provider_exhaustion;
use hoocode_code_resources::{
    load_agent_registry, LoadAgentRegistryOptions, TASK_OUTPUT_TOOL_NAME, TASK_TOOL_NAME,
};
use hoocode_code_session::SessionManager;
use hoocode_code_task_store::{
    task_store, AgentStats, CreateTaskOptions, TaskAgentKind, TaskAgentPatch, TaskAgentState,
    TaskPatch, TaskSource, TaskStatus, TaskUsage,
};
use hoocode_code_tool_api::{ToolContext, ToolDefinition, ToolError};
use serde_json::{json, Map, Value};

use crate::agent_log::agent_log;
use crate::depth::{delegate_allow_list, is_delegate_allowed, ProcessEnv};
use crate::inbox::{named, subagent_inbox, CancelAsk, InboxRecord, TaskLifecycle};
use crate::instance::get_subagent_pool;
use crate::limits::DEFAULT_WAIT_TIMEOUT_MS;
use crate::model_categories::{ModelRequest, ModelSelection};
use crate::pool::TaskStatus as PoolTaskStatus;
use crate::pool::{
    run_transcript, DispatchOptions, ResultStatus, SubagentPool, SubagentResult, TaskResult,
};
use crate::warm::{
    get_warm_subagent_pool, warm_subagents_enabled, WarmDispatchOptions, WarmProgressCallback,
    WarmRunResult,
};

pub use hoocode_code_resources::summarize_agent_description;

const TASK_MAIN_PROMPT: &str = include_str!("../templates/prompts/task-main.md");

/// Slack added to the pool's longest run budget (its resolved deadline plus the
/// load headroom, `SubagentPool::max_run_budget_ms`) before `AgentOutput` stops
/// believing a `running` record that the pool says nothing about.
const RECONCILE_SLACK_MS: u64 = 60 * 1000;

fn now_ms() -> u64 {
    hoocode_code_task_store::now_ms()
}

fn text_result(text: impl Into<String>, details: Value) -> AgentToolResult {
    AgentToolResult {
        content: vec![Content::text(text)],
        details,
        terminate: false,
    }
}

/// Names of agents whose definition opts out of the background default
/// (`background: false`). Every other agent runs in the background (card 9, D1).
fn collect_foreground_agent_names(cwd: &Path) -> HashSet<String> {
    load_agent_registry(&LoadAgentRegistryOptions::new(cwd.to_string_lossy()))
        .list()
        .iter()
        .filter(|a| a.background == Some(false))
        .map(|a| a.name.clone())
        .collect()
}

/// Whether a dispatch runs in the background. The call's `background` wins;
/// else background, unless the agent's definition says `background: false`.
fn runs_in_background(params: &Value, subagent_type: &str, foreground: &HashSet<String>) -> bool {
    lenient_bool(params.get("background")).unwrap_or_else(|| !foreground.contains(subagent_type))
}

/// `buildTaskMainPrompt`: the main-session delegation instructions appended
/// to the system prompt. One template for every project: every agent runs in
/// the background unless its definition or the call says otherwise.
pub fn build_task_main_prompt() -> String {
    TASK_MAIN_PROMPT.trim().to_string()
}

/// JS `.length` (UTF-16 units) and `slice(0, n)`.
fn js_truncate(text: &str, max: usize) -> String {
    let units: Vec<u16> = text.encode_utf16().collect();
    if units.len() > max {
        format!("{}\u{2026}", String::from_utf16_lossy(&units[..max - 1]))
    } else {
        text.to_string()
    }
}

/// A glanceable task name: the first line, at most 8 words and 60 chars.
fn summarize(task: &str) -> String {
    let first = task.trim().split('\n').next().unwrap_or("").trim();
    if first.is_empty() {
        return "(task)".into();
    }
    let words: Vec<&str> = first.split_whitespace().collect();
    let name = if words.len() > 8 {
        format!("{}\u{2026}", words[..8].join(" "))
    } else {
        first.to_string()
    };
    js_truncate(&name, 60)
}

/// What a call is for: the first line of its `description` (at most 60
/// characters), else a summary of its prompt. Shown in the task panel and next
/// to the run's label wherever the model sees it, inside double quotes, so a
/// `"` in it becomes `'`.
fn call_summary(params: &Value) -> String {
    let description = str_param(params, "description")
        .map(|d| d.trim().lines().next().unwrap_or("").trim())
        .unwrap_or("");
    let summary = if description.is_empty() {
        summarize(str_param(params, "prompt").unwrap_or(""))
    } else {
        js_truncate(description, 60)
    };
    summary.replace('"', "'")
}

/// The session a call belongs to: the dispatching session's file. Runs,
/// labels and listings are per session (see [`crate::inbox`]).
fn session_key(ctx: Option<&ToolContext>) -> Option<String> {
    ctx.and_then(|c| c.session_file.as_ref())
        .map(|f| f.to_string_lossy().into_owned())
}

/// The placeholder result of a background Agent call: the label its run will
/// carry and what it is for. The label is reserved here (the loop makes
/// placeholders one by one in call order) and taken back by the call's
/// execute, so the handle the model is told is the handle the run gets.
///
/// Without it every background call was answered with the same
/// `Started "Agent" in the background` line: a model that dispatched two runs
/// of one agent type could not tell which `explore#N` was which, took one for
/// a duplicate, cancelled the wrong one and dispatched it again.
///
/// The run is listed (`starting`) from this moment, so a roster, status or
/// cancel in the model's next turn finds it even if its execute has not
/// started yet. `None` (the loop's generic line) when the call id is already
/// reserved in this message.
fn background_placeholder_text(call: &AgentToolCall, ctx: Option<&ToolContext>) -> Option<String> {
    let subagent_type = str_param(&call.arguments, "subagent_type")?;
    if subagent_type.trim().is_empty() {
        return None;
    }
    let session = session_key(ctx);
    let label = subagent_inbox().reserve_label(
        &call.id,
        subagent_type,
        &call_summary(&call.arguments),
        session.as_deref(),
    )?;
    let resumes = str_param(&call.arguments, "resume_task_id")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|id| format!(", continuing {id}"))
        .unwrap_or_default();
    Some(format!(
        "{} started in the background{resumes}. Its handle is {label}: AgentOutput(\"{label}\") shows its status, AgentOutput(\"{label}\", cancel: true) stops it. Its result arrives as a follow-up message; keep working.",
        named(&label, Some(&call_summary(&call.arguments)))
    ))
}

/// `formatDurationSecs` (hoocode `core/format-duration.ts`).
pub use hoocode_code_agent_session::format::format_duration_secs;

/// A pool task id for a dispatch (the pool's own format).
fn new_dispatch_task_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
        ^ COUNTER
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let suffix: String = (0..6)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            DIGITS[(seed % 36) as usize] as char
        })
        .collect();
    format!("dispatch-{}-{suffix}", now_ms())
}

/// The single in_progress main-agent TodoWrite item, when unambiguous.
fn linked_todo_id() -> Option<u64> {
    let in_progress: Vec<u64> = task_store()
        .list()
        .into_iter()
        .filter(|t| {
            t.source.is_none()
                && t.agent.is_none()
                && t.parent_task_id.is_none()
                && t.status == TaskStatus::InProgress
        })
        .map(|t| t.id)
        .collect();
    (in_progress.len() == 1).then(|| in_progress[0])
}

/// A roster row per dispatch, keyed by its run (pool task) id.
fn register_subagent_dispatch(run_id: &str, label: &str) {
    task_store().upsert_agent(
        run_id,
        label,
        TaskAgentKind::Subagent,
        TaskAgentPatch {
            role: Some("subagent".into()),
            state: Some(TaskAgentState::Running),
            ..Default::default()
        },
    );
}

fn patch_agent_state(run_id: &str, state: TaskAgentState) {
    task_store().patch_agent(
        run_id,
        TaskAgentPatch {
            state: Some(state),
            activity: Some(String::new()),
            ..Default::default()
        },
    );
}

fn set_status(task_id: u64, status: TaskStatus) {
    task_store().update(
        task_id,
        TaskPatch {
            status: Some(status),
            ..Default::default()
        },
    );
}

/// Mark the dispatch failed unless it already settled (a cancel stays one).
fn mark_dispatch_failed(task_id: u64, run_id: &str) {
    let current = task_store().list().into_iter().find(|t| t.id == task_id);
    if current.is_some_and(|t| !matches!(t.status, TaskStatus::Pending | TaskStatus::InProgress)) {
        return;
    }
    set_status(task_id, TaskStatus::Failed);
    patch_agent_state(run_id, TaskAgentState::Failed);
}

fn create_dispatch_task(summary: &str, subagent_type: &str, run_id: &str) -> u64 {
    task_store()
        .create(
            summary,
            CreateTaskOptions {
                source: Some(TaskSource::Subagent),
                subagent_mode: Some(subagent_type.to_string()),
                agent: Some(run_id.to_string()),
                linked_task_id: linked_todo_id(),
                ..Default::default()
            },
        )
        .id
}

/// For a `fork: true` agent, fork the parent's session so the subagent
/// inherits its conversation; `None` falls back to a fresh session.
pub fn resolve_fork_session_file(
    fork: Option<bool>,
    parent_session: Option<&Path>,
    cwd: &Path,
) -> Option<PathBuf> {
    if fork != Some(true) {
        return None;
    }
    let parent = parent_session?;
    SessionManager::fork_from(parent, cwd.to_string_lossy(), None)
        .ok()?
        .session_file()
        .map(Path::to_path_buf)
}

fn usage_of(value: Option<&Value>) -> Option<TaskUsage> {
    let usage = value?.as_object()?;
    let num = |k: &str| usage.get(k).and_then(Value::as_f64).unwrap_or(0.0);
    Some(TaskUsage {
        input: num("input"),
        output: num("output"),
        cache_read: num("cacheRead"),
        cache_write: num("cacheWrite"),
        cost: num("cost"),
    })
}

fn parse_status(status: &str) -> Option<TaskStatus> {
    Some(match status {
        "pending" => TaskStatus::Pending,
        "in_progress" => TaskStatus::InProgress,
        "done" => TaskStatus::Done,
        "failed" => TaskStatus::Failed,
        "cancelled" => TaskStatus::Cancelled,
        _ => return None,
    })
}

/// Merge a child's task subtree under the dispatching task.
fn merge_child_task_tree(nodes: Option<&Value>, parent_task_id: u64) {
    let Some(nodes) = nodes.and_then(Value::as_array) else {
        return;
    };
    let store = task_store();
    store.batch(|store| {
        for node in nodes {
            let created = store.create(
                node.get("title").and_then(Value::as_str).unwrap_or(""),
                CreateTaskOptions {
                    source: match node.get("source").and_then(Value::as_str) {
                        Some("subagent") => Some(TaskSource::Subagent),
                        Some("mcp") => Some(TaskSource::Mcp),
                        _ => None,
                    },
                    subagent_mode: node
                        .get("subagentMode")
                        .and_then(Value::as_str)
                        .map(String::from),
                    parent_task_id: Some(parent_task_id),
                    ..Default::default()
                },
            );
            store.update(
                created.id,
                TaskPatch {
                    status: node
                        .get("status")
                        .and_then(Value::as_str)
                        .and_then(parse_status),
                    usage: usage_of(node.get("usage")),
                    ..Default::default()
                },
            );
            merge_child_task_tree(node.get("children"), created.id);
        }
    });
}

/// A warm run as the TaskResult the shared finish path consumes.
fn warm_result_to_task_result(warm: &WarmRunResult, agent_type: &str, task_id: u64) -> TaskResult {
    let mut data = Map::new();
    data.insert(
        "summary".into(),
        Value::from(if warm.summary.is_empty() {
            "(subagent returned no output)".to_string()
        } else {
            warm.summary.clone()
        }),
    );
    data.insert("files_changed".into(), json!([]));
    data.insert("confidence".into(), json!(if warm.ok { 1 } else { 0 }));
    data.insert("status".into(), Value::from(warm.status));
    if let Some(usage) = &warm.usage {
        data.insert(
            "usage".into(),
            json!({
                "input": usage.input, "output": usage.output, "cacheRead": usage.cache_read,
                "cacheWrite": usage.cache_write, "cost": usage.cost,
            }),
        );
    }
    TaskResult {
        handled_inline: false,
        agent_type: Some(agent_type.to_string()),
        result: Some(SubagentResult {
            task_id: task_id.to_string(),
            ok: warm.ok,
            exit_code: Some(if warm.ok { 0 } else { 1 }),
            status: Some(if warm.ok {
                ResultStatus::Complete
            } else {
                ResultStatus::Failed
            }),
            error: warm.error.clone(),
            result_data: Some(data),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// A background dispatch's notification handle.
struct Background {
    task_id: String,
    label: String,
    /// The call's description, shown next to the label.
    description: String,
    /// The dispatching session (its runs are counted in the notification).
    session: Option<String>,
}

impl Background {
    fn named(&self) -> String {
        named(&self.label, Some(&self.description))
    }
}

/// The tool result's `details`. `poolTaskId` is this run's own pool task (the
/// id the task panel keys its agent state by). A resumed run gets a new one:
/// its transcript lives under the original session's task id, which the
/// `resume_task_id` label resolves to.
fn details(
    subagent_type: &str,
    ok: bool,
    error: Option<&str>,
    task_id: u64,
    pool_task_id: Option<&str>,
    background: bool,
) -> Value {
    let mut d = Map::new();
    d.insert("subagent_type".into(), Value::from(subagent_type));
    d.insert("ok".into(), Value::from(ok));
    if let Some(error) = error {
        d.insert("error".into(), Value::from(error));
    }
    d.insert("taskId".into(), Value::from(task_id));
    if let Some(id) = pool_task_id {
        d.insert("poolTaskId".into(), Value::from(id));
    }
    if background {
        d.insert("background".into(), Value::from(true));
    }
    Value::Object(d)
}

/// The line that tells the main model which model a dispatch ran on.
fn model_line(selection: &ModelSelection) -> String {
    let model = match (&selection.model, &selection.alias) {
        (Some(model), Some(alias)) => format!("{alias} ({model})"),
        (Some(model), None) => model.clone(),
        (None, _) => "the default model".into(),
    };
    let mut line = format!("[model: {model}");
    if let Some(effort) = &selection.effort {
        line.push_str(&format!(", effort {}", effort.as_str()));
    }
    if let Some(note) = &selection.note {
        line.push_str(&format!("; {note}"));
    }
    line.push(']');
    line
}

/// The partial update an Agent call emits at dispatch: its pool run's id, so
/// the tool block knows which run is its own (T3.2). A background call has no
/// update channel (its placeholder is its result), so the run is also kept by
/// call id (`live::remember_run`), which the block reads too.
fn announce_run(
    call_id: &str,
    pool_task_id: &str,
    subagent_type: &str,
    on_update: &Option<AgentToolUpdateCallback>,
) {
    crate::live::remember_run(call_id, pool_task_id);
    if let Some(update) = on_update {
        update(AgentToolResult {
            content: Vec::new(),
            details: json!({
                "poolTaskId": pool_task_id,
                "subagent_type": subagent_type,
                "status": "running",
            }),
            terminate: false,
        });
    }
}

/// Update the task panel from a finished dispatch and shape the tool result.
/// A foreground failure is an error; a background one is a notification.
fn finalize_dispatch_result(
    model_line: &str,
    dispatch: &TaskResult,
    subagent_type: &str,
    run_id: &str,
    task_id: u64,
    resume_label: Option<&str>,
    background: Option<&Background>,
) -> Result<AgentToolResult, ToolError> {
    let result = dispatch.result.as_ref();
    // Every outcome names the label to resume by, when the run left a session.
    let resume_line = resume_label
        .map(|label| format!("\nresume_task_id: {label}"))
        .unwrap_or_default();
    // A retry on the session's model says so; the first attempt's model is
    // otherwise what the line reports.
    let line = if result.and_then(|r| r.used_inherited_model_fallback) == Some(true) {
        format!("{model_line} [retried on the session's model]")
    } else {
        model_line.to_string()
    };
    let data = result.and_then(|r| r.result_data.as_ref());
    let usage = usage_of(data.and_then(|d| d.get("usage")));
    merge_child_task_tree(data.and_then(|d| d.get("task_tree")), task_id);
    if let Some(usage) = &usage {
        task_store().add_agent_stats(
            run_id,
            AgentStats {
                input: usage.input,
                output: usage.output,
                cost: usage.cost,
            },
        );
    }

    let Some(result) = result.filter(|r| r.ok) else {
        let cancelled = result.and_then(|r| r.status) == Some(ResultStatus::Cancelled);
        let fallback = result.and_then(|r| r.used_inherited_model_fallback) == Some(true);
        task_store().update(
            task_id,
            TaskPatch {
                status: Some(if cancelled {
                    TaskStatus::Cancelled
                } else {
                    TaskStatus::Failed
                }),
                usage,
                note: Some(fallback.then(|| "inherited-model retry failed".to_string())),
                ..Default::default()
            },
        );
        patch_agent_state(
            run_id,
            if cancelled {
                TaskAgentState::Cancelled
            } else {
                TaskAgentState::Failed
            },
        );
        let reason = result.and_then(|r| r.error.clone()).unwrap_or_else(|| {
            match result.and_then(|r| r.status) {
                Some(status) => format!("subagent {}", status.as_str()),
                None => "unknown error".into(),
            }
        });
        if let Some(background) = background {
            let verdict = if cancelled {
                "cancelled ⊘"
            } else {
                "failed ✗"
            };
            return Ok(text_result(
                format!(
                    "{} {verdict} — {reason}\n{line}{resume_line}",
                    background.named()
                ),
                details(
                    subagent_type,
                    false,
                    Some(&reason),
                    task_id,
                    Some(&background.task_id),
                    true,
                ),
            ));
        }
        if cancelled {
            return Err(format!("Subagent ({subagent_type}) cancelled by user.").into());
        }
        let stderr = result.map(|r| r.stderr.trim()).unwrap_or("");
        let stderr = if stderr.is_empty() {
            String::new()
        } else {
            let units: Vec<u16> = stderr.encode_utf16().collect();
            format!(
                "\nstderr: {}",
                String::from_utf16_lossy(&units[units.len().saturating_sub(500)..])
            )
        };
        return Err(format!(
            "Subagent ({subagent_type}) failed: {reason}{stderr}\n{line}{resume_line}"
        )
        .into());
    };

    let fallback_note = (result.used_inherited_model_fallback == Some(true))
        .then(|| "ran on inherited model".to_string());
    task_store().update(
        task_id,
        TaskPatch {
            status: Some(TaskStatus::Done),
            usage,
            note: Some(fallback_note),
            ..Default::default()
        },
    );
    patch_agent_state(run_id, TaskAgentState::Done);
    let mut answer = data
        .and_then(|d| d.get("summary"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("(subagent returned no output)")
        .to_string();
    // A result.json with status "partial" settles as ResultStatus::Partial
    // (pool.rs settle); its session stays resumable.
    let partial = result.status == Some(ResultStatus::Partial);
    if partial {
        if let Some(label) = resume_label {
            answer.push_str(&format!(
                "\n\n[Partial result. To continue this subagent, call Agent again with resume_task_id=\"{label}\".]"
            ));
        }
    }

    if let Some(background) = background {
        let partial_note = if partial {
            " (partial — resume to continue)"
        } else {
            ""
        };
        let outstanding = subagent_inbox()
            .outstanding_in(background.session.as_deref())
            .len();
        let tail = if outstanding > 0 {
            format!(" {outstanding} still running.")
        } else {
            String::new()
        };
        let text = format!(
            "{} finished ✓{partial_note} — {}.{tail}\nRead the full result with AgentOutput(\"{}\").\n{line}{resume_line}",
            background.named(),
            summarize(&answer),
            background.label
        );
        return Ok(text_result(
            text,
            details(
                subagent_type,
                true,
                None,
                task_id,
                Some(&background.task_id),
                true,
            ),
        ));
    }
    Ok(text_result(
        format!("{line}\n\n{answer}{resume_line}"),
        details(subagent_type, true, None, task_id, Some(run_id), false),
    ))
}

/// Bind a background run's inbox record (reserved at its placeholder, or new).
/// `Err` with the call's result when the run was cancelled while starting: it
/// is not dispatched, and the task panel shows it cancelled.
fn start_background(
    session: Option<&str>,
    run_id: &str,
    label: &str,
    subagent_type: &str,
    summary: &str,
    task_id: u64,
) -> Result<(), AgentToolResult> {
    let rec = subagent_inbox().start_in(session, run_id, label, subagent_type, summary);
    if rec.lifecycle == TaskLifecycle::Running {
        return Ok(());
    }
    set_status(task_id, TaskStatus::Cancelled);
    patch_agent_state(run_id, TaskAgentState::Cancelled);
    let reason = rec
        .error
        .clone()
        .unwrap_or_else(|| "cancelled before it started".into());
    Err(text_result(
        format!("{} cancelled ⊘ — {reason}", rec.named()),
        details(
            subagent_type,
            false,
            Some(&reason),
            task_id,
            Some(run_id),
            true,
        ),
    ))
}

/// How often a run asked to stop is offered to the pool again until the pool
/// holds it (the moments between `start_in` and the dispatch registering it).
const CANCEL_RETRY: Duration = Duration::from_millis(25);

/// Run a dispatch, cancelling its run (process tree) when the call aborts, or
/// when `AgentOutput(cancel: true)` asks for it in the inbox. That ask can
/// land before the pool holds the run, where `pool.cancel` misses; it is
/// offered again until the pool takes it or the run ends.
pub async fn cancel_on_abort<F>(
    pool: &SubagentPool,
    run_id: &str,
    signal: Option<AbortSignal>,
    run: F,
) -> F::Output
where
    F: std::future::Future,
{
    tokio::pin!(run);
    let inbox = subagent_inbox();
    let mut aborted = false;
    let mut cancelled = false;
    loop {
        let asked = inbox.cancel_notified();
        tokio::pin!(asked);
        asked.as_mut().enable();
        let pending = !cancelled && inbox.cancel_requested(run_id);
        if pending && pool.cancel(run_id) {
            cancelled = true;
        }
        let retry = pending && !cancelled;
        let abort = async {
            match &signal {
                Some(signal) => signal.cancelled().await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            output = &mut run => return output,
            _ = abort, if !aborted => {
                aborted = true;
                pool.cancel(run_id);
            }
            _ = &mut asked, if !cancelled && !retry => {}
            _ = tokio::time::sleep(CANCEL_RETRY), if retry => {}
        }
    }
}

/// Block on a future from a tool's (blocking) execute.
fn block_on<F: std::future::Future>(future: F) -> F::Output {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| handle.block_on(future)),
        Err(_) => hoocode_runtime::block_on_current_thread(future),
    }
}

fn str_param<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params.get(key).and_then(Value::as_str)
}

/// A flag as the model sent it: a JSON bool, or the string "true"/"false"
/// (models often quote booleans). Anything else is unset.
fn lenient_bool(value: Option<&Value>) -> Option<bool> {
    match value? {
        Value::Bool(flag) => Some(*flag),
        Value::String(text) => match text.trim().to_ascii_lowercase().as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

/// The handle a run is resumed by: its label, once the run left a transcript
/// (see [`run_transcript`]). `None` otherwise, so no `resume_task_id` line is
/// offered for a session that does not exist.
fn resumable_label(
    cwd: &Path,
    parent_session: Option<&Path>,
    session_id: &str,
    label: &str,
) -> Option<String> {
    run_transcript(cwd, session_id, parent_session).map(|_| label.to_string())
}

fn task_parameters() -> Value {
    json!({
        "type": "object",
        "required": ["description", "prompt", "subagent_type"],
        "properties": {
            "description": {"type": "string", "description": "A short (3-5 word) description of the task, shown in the task panel."},
            "prompt": {"type": "string", "description": "The full, self-contained task for the subagent. It cannot see this conversation, so include all needed context, files, and constraints."},
            "subagent_type": {"type": "string", "description": "The name of the specialized agent to delegate to. Must be one of the available agents."},
            "model": {"type": "string", "description": "Which model the subagent runs on: a category (cheap, fast, standard, capable), or the alias or id of a scoped model (listed in the system prompt when scoped models are set). Omit to use the agent's default model. An explicit model overrides the agent's own `model:`. When scoped models are set, a subagent can only use one of them."},
            "effort": {"type": "string", "enum": ["off", "minimal", "low", "medium", "high", "xhigh"], "description": "Thinking level for the subagent. Overrides the effort of the chosen scoped model; omit to use that model's effort."},
            "background": {"type": "boolean", "description": "Set true to run non-blocking: you get a short notification when it finishes and pull the full result with AgentOutput; set false to wait and get the answer inline. Defaults to background, unless the agent's definition sets background: false."},
            "resume_task_id": {"type": "string", "description": "Optional. To continue a previous subagent run, pass the label from its result's `resume_task_id:` line (e.g. \"explore#1\"). A raw task_id from an earlier Agent or AgentOutput call also works. The subagent resumes with its full prior transcript and `prompt` is your follow-up instruction."}
        }
    })
}

/// `createTaskToolDefinition`.
pub fn create_task_tool_definition(cwd: &Path) -> ToolDefinition {
    let foreground_agents = Arc::new(collect_foreground_agent_names(cwd));
    let predicate_agents = foreground_agents.clone();
    let default_cwd = cwd.to_path_buf();
    ToolDefinition { ordered_start: false,
        name: TASK_TOOL_NAME.into(),
        label: TASK_TOOL_NAME.into(),
        description: "Delegate a focused task to a specialized subagent that runs as a separate process with its own, fresh context (it cannot see this conversation). Choose one of the available agents (listed in the system prompt) via `subagent_type` and pass everything it needs via `prompt`; the subagent returns only its final answer. Every call starts its own run with its own handle (e.g. \"explore#2\"). A background call's result names that handle next to its description at once; a foreground call names it on its `resume_task_id:` line. Two runs of one agent type are different runs, not duplicates. Before dispatching a task again, check AgentOutput(list: true): it shows every background run with its handle, description and status.".into(),
        prompt_snippet: Some(
            "delegate a self-contained task to a specialized subagent (choose via subagent_type)"
                .into(),
        ),
        prompt_guidelines: Vec::new(),
        parameters: task_parameters(),
        prepare_arguments: None,
        execution_mode: None,
        background: false,
        // A per-call `background` wins; else background, unless the agent opts out.
        background_when: Some(Arc::new(move |call: &AgentToolCall| {
            let subagent_type = call
                .arguments
                .get("subagent_type")
                .and_then(Value::as_str)
                .unwrap_or("");
            runs_in_background(&call.arguments, subagent_type, &predicate_agents)
        })),
        background_placeholder: Some(Arc::new(background_placeholder_text)),
        execute: Arc::new(move |call_id, params, signal, on_update, ctx| {
            let cwd = ctx
                .and_then(|c| c.cwd.clone())
                .unwrap_or_else(|| default_cwd.clone());
            block_on(execute_task(
                params,
                signal,
                ctx.cloned(),
                cwd,
                foreground_agents.clone(),
                call_id,
                on_update,
            ))
        }),
    }
}

/// The Agent call. Takes the label the call's placeholder gave the model (a
/// background call), else a new one. A reserved run that the call never
/// started (refused: unknown agent, delegation, model, effort, provider
/// exhaustion) is settled `failed` with the reason, and the reason names the
/// run, so `AgentOutput("explore#1")` and the follow-up both say what happened.
async fn execute_task(
    params: Value,
    signal: Option<AbortSignal>,
    ctx: Option<ToolContext>,
    cwd: PathBuf,
    foreground_agents: Arc<HashSet<String>>,
    call_id: String,
    on_update: Option<AgentToolUpdateCallback>,
) -> Result<AgentToolResult, ToolError> {
    let subagent_type = str_param(&params, "subagent_type")
        .unwrap_or("")
        .to_string();
    let summary = call_summary(&params);
    let session = session_key(ctx.as_ref());
    let label = subagent_inbox().take_label(&call_id, &subagent_type, &summary, session.as_deref());
    let outcome = execute_task_inner(
        params,
        signal,
        ctx,
        cwd,
        foreground_agents,
        call_id,
        on_update,
        label.clone(),
        session.clone(),
    )
    .await;
    if subagent_inbox()
        .reserved(session.as_deref(), &label)
        .is_none()
    {
        return outcome;
    }
    // Still reserved: the call returned before it started its run.
    let name = named(&label, Some(&summary));
    match outcome {
        Err(error) => {
            let reason = error.to_string();
            subagent_inbox().fail_reserved(session.as_deref(), &label, &reason);
            Err(format!("{name} failed ✗ — {reason}").into())
        }
        Ok(mut result) => {
            let reason = result
                .content
                .iter()
                .find_map(|c| match c {
                    Content::Text(t) => Some(t.text.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| "not dispatched".into());
            subagent_inbox().fail_reserved(session.as_deref(), &label, &reason);
            result.content = vec![Content::text(format!("{name} failed ✗ — {reason}"))];
            Ok(result)
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn execute_task_inner(
    params: Value,
    signal: Option<AbortSignal>,
    ctx: Option<ToolContext>,
    cwd: PathBuf,
    foreground_agents: Arc<HashSet<String>>,
    call_id: String,
    on_update: Option<AgentToolUpdateCallback>,
    label: String,
    session: Option<String>,
) -> Result<AgentToolResult, ToolError> {
    let subagent_type = str_param(&params, "subagent_type")
        .unwrap_or("")
        .to_string();
    let prompt = str_param(&params, "prompt").unwrap_or("").to_string();
    let summary = call_summary(&params);
    let available: Vec<Model> = ctx
        .as_ref()
        .map(|c| c.available_models.clone())
        .unwrap_or_default();
    let model = ctx.as_ref().and_then(|c| c.model.clone());
    // The dispatching session: its subagents' transcripts go under it.
    let parent_session = ctx.as_ref().and_then(|c| c.session_file.clone());
    let pool = get_subagent_pool(&cwd, &available);
    let ask = str_param(&params, "model")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);
    let effort = str_param(&params, "effort")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);
    if let Some(raw) = &effort {
        if hoocode_code_models::parse_thinking_level(raw).is_none() {
            return Err(format!(
                "Unknown effort \"{raw}\". Use one of: off, minimal, low, medium, high, xhigh."
            )
            .into());
        }
    }
    // Pre-flight: the inherited provider just exhausted its quota; a subagent
    // on the same provider would fail too.
    if let Some(provider) = model.as_ref().map(|m| m.provider.clone()) {
        if let Some(exhaustion) = get_provider_exhaustion(&provider) {
            let run_id = new_dispatch_task_id();
            let task_id = create_dispatch_task(&summary, &subagent_type, &run_id);
            register_subagent_dispatch(&run_id, &label);
            task_store().update(
                task_id,
                TaskPatch {
                    status: Some(TaskStatus::Failed),
                    note: Some(Some(format!("{provider} exhausted"))),
                    ..Default::default()
                },
            );
            task_store().patch_agent(
                &run_id,
                TaskAgentPatch {
                    state: Some(TaskAgentState::Failed),
                    ..Default::default()
                },
            );
            return Ok(text_result(
                format!(
                    "Did not dispatch subagent \"{subagent_type}\": the \"{provider}\" provider appears exhausted or rate-limited (this session just failed with: {}). Subagents run on the same provider, so dispatching would fail too. Wait for the quota to reset or switch model/provider, then retry — or complete the work directly in this session.",
                    exhaustion.message
                ),
                details(&subagent_type, false, None, task_id, None, false),
            ));
        }
    }

    // Scoped delegation (`delegate: <types>`); the root is unrestricted.
    if !is_delegate_allowed(&subagent_type, &ProcessEnv) {
        let allowed = delegate_allow_list(&ProcessEnv)
            .map(|l| l.join(", "))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "(none)".into());
        return Err(format!(
            "This agent may not delegate to \"{subagent_type}\". Allowed subagent types: {allowed}."
        )
        .into());
    }

    // `provider/id`: the scope and the category match on that form.
    let model_ref = model.as_ref().map(|m| format!("{}/{}", m.provider, m.id));
    let provider = model.as_ref().map(|m| m.provider.clone());
    // Which model the dispatch runs on, decided once here so the result can
    // say so. An ask that cannot be used is refused before anything is created.
    let selection = pool.select_model(
        &subagent_type,
        &ModelRequest {
            ask: ask.clone(),
            effort: effort.clone(),
            pin: None,
            inherited: model_ref.clone(),
        },
    );
    if let Some(error) = &selection.error {
        return Err(error.clone().into());
    }
    let model_line = model_line(&selection);

    // Resume: continue a previous subagent with its persisted transcript.
    if let Some(resume_id) = str_param(&params, "resume_task_id")
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        // The model names a label (`explore#4`); the session lives under the
        // original pool task id. A raw task id is accepted too.
        let bound = subagent_inbox().session_id_for_in(session.as_deref(), resume_id);
        if bound.is_none() && subagent_inbox().is_earlier_run(session.as_deref(), resume_id) {
            return Err(format!(
                "{resume_id} ran in an earlier process; its handle cannot be resumed from this one. Dispatch a new run with the context it needs."
            )
            .into());
        }
        let session_id = bound.unwrap_or_else(|| resume_id.to_string());
        let run_id = new_dispatch_task_id();
        // This run continues the same session, so its label resumes that session too.
        subagent_inbox().bind_label_in(session.as_deref(), &label, &session_id);
        let task_id = create_dispatch_task(&summary, &subagent_type, &run_id);
        register_subagent_dispatch(&run_id, &label);
        set_status(task_id, TaskStatus::InProgress);
        announce_run(&call_id, &run_id, &subagent_type, &on_update);
        // A background resume is pulled with AgentOutput, so it needs an inbox record.
        let is_background = runs_in_background(&params, &subagent_type, &foreground_agents);
        if is_background {
            subagent_inbox().observe(&pool);
            if let Err(cancelled) = start_background(
                session.as_deref(),
                &run_id,
                &label,
                &subagent_type,
                &summary,
                task_id,
            ) {
                return Ok(cancelled);
            }
        }
        let outcome = cancel_on_abort(
            &pool,
            &run_id,
            signal,
            pool.resume(
                &session_id,
                &prompt,
                DispatchOptions {
                    model: ask.clone(),
                    inherited_model: model_ref.clone(),
                    effort: effort.clone(),
                    provider,
                    task_id: Some(run_id.clone()),
                    parent_session_file: parent_session.clone(),
                    ..Default::default()
                },
            ),
        )
        .await;
        return match outcome {
            Ok(dispatch) => {
                subagent_inbox().finish(&run_id, &dispatch);
                finalize_dispatch_result(
                    &model_line,
                    &dispatch,
                    &subagent_type,
                    &run_id,
                    task_id,
                    resumable_label(&cwd, parent_session.as_deref(), &session_id, &label)
                        .as_deref(),
                    None,
                )
            }
            Err(error) => {
                mark_dispatch_failed(task_id, &run_id);
                subagent_inbox().fail(&run_id, &error.0, TaskLifecycle::Failed);
                Err(error.0.into())
            }
        };
    }

    // The model already chose the agent: validate it against the registry.
    let registry = load_agent_registry(&LoadAgentRegistryOptions::new(cwd.to_string_lossy()));
    let Some(def) = registry.get(&subagent_type).cloned() else {
        let available = registry
            .list()
            .iter()
            .map(|a| a.name.clone())
            .collect::<Vec<_>>()
            .join(", ");
        let available = if available.is_empty() {
            "(none)".into()
        } else {
            available
        };
        return Err(format!(
            "Unknown subagent_type: \"{subagent_type}\". Available agents: {available}."
        )
        .into());
    };

    let pool_task_id = new_dispatch_task_id();
    // The label is what the model sees; it resumes this run's session.
    subagent_inbox().bind_label_in(session.as_deref(), &label, &pool_task_id);
    let task_id = create_dispatch_task(&summary, &subagent_type, &pool_task_id);
    register_subagent_dispatch(&pool_task_id, &label);
    set_status(task_id, TaskStatus::InProgress);
    announce_run(&call_id, &pool_task_id, &subagent_type, &on_update);
    // Fork agents inherit the parent's conversation via a forked session.
    let fork_session_file = if def.fork == Some(true) {
        resolve_fork_session_file(
            def.fork,
            ctx.as_ref().and_then(|c| c.session_file.as_deref()),
            &cwd,
        )
    } else {
        None
    };
    let is_background = runs_in_background(&params, &subagent_type, &foreground_agents);
    let warm_options = WarmDispatchOptions {
        agent_type: subagent_type.clone(),
        cwd: cwd.clone(),
        model: ask.clone(),
        provider: provider.clone(),
        inherited_model: model_ref.clone(),
        effort: effort.clone(),
    };
    let progress: WarmProgressCallback = {
        let run_id = pool_task_id.clone();
        Arc::new(move |activity: &str| {
            task_store().patch_agent(
                &run_id,
                TaskAgentPatch {
                    activity: Some(activity.to_string()),
                    ..Default::default()
                },
            );
        })
    };
    let dispatch_options = DispatchOptions {
        force_agent: Some(subagent_type.clone()),
        model: ask.clone(),
        // The parent's own model: the default when nothing is asked, and where
        // the inherited-model fallback runs.
        inherited_model: model_ref.clone(),
        effort: effort.clone(),
        provider: provider.clone(),
        session_file: fork_session_file.clone(),
        parent_session_file: parent_session.clone(),
        task_id: Some(pool_task_id.clone()),
        background: Some(is_background),
    };
    let use_warm = warm_subagents_enabled(&ProcessEnv) && fork_session_file.is_none();

    if is_background {
        // Notify-and-pull: the body stays in the inbox.
        subagent_inbox().observe(&pool);
        if let Err(cancelled) = start_background(
            session.as_deref(),
            &pool_task_id,
            &label,
            &subagent_type,
            &summary,
            task_id,
        ) {
            return Ok(cancelled);
        }
        let background = Background {
            task_id: pool_task_id.clone(),
            label: label.clone(),
            description: summary.clone(),
            session: session.clone(),
        };
        if use_warm {
            let warm = get_warm_subagent_pool(&cwd, &available);
            if warm.is_poolable(&subagent_type) {
                match warm
                    .dispatch(&prompt, &warm_options, Some(progress.clone()))
                    .await
                {
                    Ok(warm_result) => {
                        let dispatch =
                            warm_result_to_task_result(&warm_result, &subagent_type, task_id);
                        subagent_inbox().finish(&pool_task_id, &dispatch);
                        return finalize_dispatch_result(
                            &model_line,
                            &dispatch,
                            &subagent_type,
                            &pool_task_id,
                            task_id,
                            resumable_label(&cwd, parent_session.as_deref(), &pool_task_id, &label)
                                .as_deref(),
                            Some(&background),
                        );
                    }
                    Err(error) => agent_log(&format!(
                        "[WARM] {subagent_type} fell back to cold spawn: {error}"
                    )),
                }
            }
        }
        let outcome = cancel_on_abort(
            &pool,
            &pool_task_id,
            signal,
            pool.dispatch(&prompt, dispatch_options),
        )
        .await;
        return match outcome {
            Ok(dispatch) => {
                subagent_inbox().finish(&pool_task_id, &dispatch);
                finalize_dispatch_result(
                    &model_line,
                    &dispatch,
                    &subagent_type,
                    &pool_task_id,
                    task_id,
                    resumable_label(&cwd, parent_session.as_deref(), &pool_task_id, &label)
                        .as_deref(),
                    Some(&background),
                )
            }
            Err(error) => {
                let reason = error.0;
                mark_dispatch_failed(task_id, &pool_task_id);
                subagent_inbox().fail(&pool_task_id, &reason, TaskLifecycle::Failed);
                // Already answered by a placeholder: a notification, not an error.
                Ok(text_result(
                    format!("{} failed ✗ — {reason}", background.named()),
                    details(
                        &subagent_type,
                        false,
                        Some(&reason),
                        task_id,
                        Some(&pool_task_id),
                        true,
                    ),
                ))
            }
        };
    }

    // Warm path for a foreground dispatch; infra failures fall back to cold.
    if use_warm {
        let warm = get_warm_subagent_pool(&cwd, &available);
        if warm.is_poolable(&subagent_type) {
            match warm.dispatch(&prompt, &warm_options, Some(progress)).await {
                Ok(warm_result) => {
                    let dispatch =
                        warm_result_to_task_result(&warm_result, &subagent_type, task_id);
                    return finalize_dispatch_result(
                        &model_line,
                        &dispatch,
                        &subagent_type,
                        &pool_task_id,
                        task_id,
                        resumable_label(&cwd, parent_session.as_deref(), &pool_task_id, &label)
                            .as_deref(),
                        None,
                    );
                }
                Err(error) => agent_log(&format!(
                    "[WARM] {subagent_type} fell back to cold spawn: {error}"
                )),
            }
        }
    }

    // Foreground: block and return the full answer inline.
    let outcome = cancel_on_abort(
        &pool,
        &pool_task_id,
        signal,
        pool.dispatch(&prompt, dispatch_options),
    )
    .await;
    match outcome {
        Ok(dispatch) => finalize_dispatch_result(
            &model_line,
            &dispatch,
            &subagent_type,
            &pool_task_id,
            task_id,
            resumable_label(&cwd, parent_session.as_deref(), &pool_task_id, &label).as_deref(),
            None,
        ),
        Err(error) => {
            mark_dispatch_failed(task_id, &pool_task_id);
            Err(error.0.into())
        }
    }
}

// ---------------------------------------------------------------------------
// AgentOutput
// ---------------------------------------------------------------------------

/// How long a record has run (so far, or until it settled).
fn record_elapsed(rec: &InboxRecord) -> String {
    let end = rec.ended_at.unwrap_or_else(now_ms);
    format_duration_secs(end.saturating_sub(rec.started_at) as f64 / 1000.0)
}

fn output_details(
    task_id: Option<&str>,
    status: &str,
    ok: bool,
    outstanding: Option<usize>,
) -> Value {
    let mut d = Map::new();
    if let Some(id) = task_id {
        d.insert("task_id".into(), Value::from(id));
    }
    d.insert("status".into(), Value::from(status));
    d.insert("ok".into(), Value::from(ok));
    if let Some(n) = outstanding {
        d.insert("outstanding".into(), Value::from(n));
    }
    Value::Object(d)
}

/// Every known background subagent: status and activity, no bodies.
fn format_task_roster(session: Option<&str>) -> AgentToolResult {
    let inbox = subagent_inbox();
    let all = inbox.list_in(session);
    let outstanding = inbox.outstanding_in(session).len();
    if all.is_empty() {
        return text_result(
            "No background subagents have been dispatched.",
            output_details(None, "empty", true, Some(0)),
        );
    }
    let lines: Vec<String> = all
        .iter()
        .map(|r| {
            let when = record_elapsed(r);
            let summary = r.summary_line.as_deref().unwrap_or("");
            let name = r.named();
            match r.lifecycle {
                TaskLifecycle::Running => format!(
                    "- {name}  running  {when}{}",
                    r.last_activity
                        .as_deref()
                        .filter(|a| !a.is_empty())
                        .map(|a| format!("  · {a}"))
                        .unwrap_or_default()
                ),
                TaskLifecycle::Done => {
                    format!("- {name}  done (uncollected)  {when} — {summary}")
                }
                TaskLifecycle::Collected => format!("- {name}  collected  {when} — {summary}"),
                TaskLifecycle::Cancelled => format!(
                    "- {name}  cancelled ⊘  — {}",
                    r.error.as_deref().unwrap_or("cancelled by user")
                ),
                other => format!(
                    "- {name}  {} ✗  — {}",
                    other.as_str(),
                    r.error.as_deref().unwrap_or("unknown error")
                ),
            }
        })
        .collect();
    let header = format!(
        "{} background subagent{} ({outstanding} running):",
        all.len(),
        if all.len() == 1 { "" } else { "s" }
    );
    let hint = if all.iter().any(|r| r.lifecycle == TaskLifecycle::Done) {
        "\nRead a finished one with AgentOutput(\"<label>\")."
    } else {
        ""
    };
    text_result(
        format!("{header}\n{}{hint}", lines.join("\n")),
        output_details(None, "list", true, Some(outstanding)),
    )
}

fn task_output_parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "task_id": {"type": "string", "description": "Handle of a background subagent — its task_id or friendly label (e.g. \"explore#1\") from an Agent notification. Omit (or set list:true) to see every background task."},
            "list": {"type": "boolean", "description": "List all background subagents with their status (running/done/failed/cancelled) and current activity. No result bodies are returned."},
            "wait": {"type": "boolean", "description": "Block until the named task finishes — or, with no task_id, until all outstanding subagents finish (a swarm barrier) — before returning. Bounded by timeout_ms."},
            "timeout_ms": {"type": "number", "description": "Maximum time to block in wait mode, in milliseconds (default 300000, or the subagentWaitTimeoutSeconds setting / HOOCODE_SUBAGENT_WAIT_TIMEOUT_MS)."},
            "cancel": {"type": "boolean", "description": "Stop the named running subagent (kills its process); requires task_id. Use when its result is no longer needed."}
        }
    })
}

/// `createTaskOutputToolDefinition`.
pub fn create_task_output_tool_definition() -> ToolDefinition {
    ToolDefinition { ordered_start: false,
        name: TASK_OUTPUT_TOOL_NAME.into(),
        label: TASK_OUTPUT_TOOL_NAME.into(),
        description: [
            "Check on background subagents dispatched via Agent, and pull their results.",
            "Pass a task_id/label (e.g. \"explore#1\") to read a finished subagent's full result, or to see its status while it runs.",
            "Set list:true (or omit task_id) to list every background subagent with its handle, description, status and current activity. Check it before dispatching a task again or cancelling: two runs of one agent type with different descriptions are not duplicates.",
            "Set wait:true to block until that task finishes — or, with no task_id, until all outstanding subagents finish (a swarm barrier).",
            "A long run can take up to the subagent deadline (4 hours by default): poll it by task_id, and set cancel:true with a task_id to stop one whose result you no longer need. A cancel stops exactly the run its handle names and says which one it stopped.",
            "It never errors on a valid handle: a running task reports status, a finished one returns its result, an already-read one says so.",
        ]
        .join("\n"),
        prompt_snippet: Some(
            "check status / list / collect the results of background subagents".into(),
        ),
        prompt_guidelines: Vec::new(),
        parameters: task_output_parameters(),
        prepare_arguments: None,
        execution_mode: None,
        background: false,
        background_when: None,
        background_placeholder: None,
        execute: Arc::new(|_id, params, _signal, _on_update, ctx| {
            let ctx = ctx.cloned();
            block_on(execute_task_output(params, ctx))
        }),
    }
}

async fn execute_task_output(
    params: Value,
    ctx: Option<ToolContext>,
) -> Result<AgentToolResult, ToolError> {
    let handle = str_param(&params, "task_id")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);
    if params.get("cancel").and_then(Value::as_bool) == Some(true) {
        return cancel_background_task(handle.as_deref(), &params, ctx.as_ref());
    }
    Ok(read_background_task(params, ctx, handle).await)
}

/// The answer for a handle the inbox does not hold: a run of an earlier
/// process (a resumed session), or an unknown handle.
fn unknown_handle(session: Option<&str>, handle: &str) -> String {
    if subagent_inbox().is_earlier_run(session, handle) {
        format!(
            "{handle} ran in an earlier process and is not running now. Its result, if it finished, is earlier in this conversation; it cannot be cancelled or read from here."
        )
    } else {
        format!(
            "No background task \"{handle}\". Call AgentOutput with list:true to see active tasks."
        )
    }
}

/// A cancelled run that never got a process again: `pool.cancel` missed it,
/// and it is younger than this. The execute awaiting it stops it as soon as
/// the pool holds it (see [`cancel_on_abort`]).
const CANCEL_PENDING_MAX_AGE_MS: u64 = 60_000;

/// `AgentOutput(task_id, cancel: true)`: stop a background subagent, by the
/// exact handle (task id or label) of this session's run. A run still
/// starting never starts; a running one has its process tree killed and
/// settles `cancelled` when its dispatch returns. The answer names the run it
/// stopped. Errors on a missing or unknown handle, and when combined with
/// `wait` or `list`.
fn cancel_background_task(
    handle: Option<&str>,
    params: &Value,
    ctx: Option<&ToolContext>,
) -> Result<AgentToolResult, ToolError> {
    if params.get("wait").and_then(Value::as_bool) == Some(true)
        || params.get("list").and_then(Value::as_bool) == Some(true)
    {
        return Err("AgentOutput with cancel:true cannot be combined with wait or list.".into());
    }
    let Some(handle) = handle else {
        return Err(
            "AgentOutput with cancel:true needs a task_id, e.g. AgentOutput(\"explore#1\", cancel: true).".into(),
        );
    };
    let session = session_key(ctx);
    let cwd = ctx.and_then(|c| c.cwd.clone());
    let Some(ask) = subagent_inbox().request_cancel(session.as_deref(), handle) else {
        return Err(unknown_handle(session.as_deref(), handle).into());
    };
    let rec = match ask {
        CancelAsk::Settled(rec) => {
            return Ok(text_result(
                format!(
                    "{} already finished ({}); nothing was cancelled.",
                    rec.named(),
                    rec.lifecycle.as_str()
                ),
                output_details(Some(handle), rec.lifecycle.as_str(), true, None),
            ))
        }
        CancelAsk::BeforeStart(rec) => {
            return Ok(text_result(
                format!(
                    "Cancelled {} before it started. Only this run was stopped.",
                    rec.named()
                ),
                output_details(Some(handle), "cancelled", true, None),
            ))
        }
        CancelAsk::Running(rec) => rec,
    };
    let cwd = cwd.ok_or("AgentOutput with cancel:true needs a working directory.")?;
    let models = ctx.map(|c| c.available_models.clone()).unwrap_or_default();
    let pool = get_subagent_pool(&cwd, &models);
    let cancelled = format!("Cancelled {}. Only this run was stopped.", rec.named());
    if pool.cancel(&rec.task_id) {
        return Ok(text_result(
            cancelled,
            output_details(Some(handle), "cancelled", true, None),
        ));
    }
    // The pool does not hold it. Either it has not registered the run yet (the
    // moments after its execute bound it), and the execute stops it once it
    // does, or the pool settled it before the inbox heard. A task the pool has
    // forgotten without a settle is reported `failed`, as the reconcile would.
    let status = match pool.get_status(&rec.task_id) {
        PoolTaskStatus::Unknown
            if now_ms().saturating_sub(rec.started_at) < CANCEL_PENDING_MAX_AGE_MS =>
        {
            return Ok(text_result(
                cancelled,
                output_details(Some(handle), "cancelled", true, None),
            ));
        }
        PoolTaskStatus::Unknown | PoolTaskStatus::Running | PoolTaskStatus::Queued => "failed",
        other => other.as_str(),
    };
    Ok(text_result(
        format!(
            "{} already finished ({status}); nothing was cancelled.",
            rec.named()
        ),
        output_details(Some(handle), status, true, None),
    ))
}

async fn read_background_task(
    params: Value,
    ctx: Option<ToolContext>,
    handle: Option<String>,
) -> AgentToolResult {
    // Wire the inbox to the pool's progress stream for live activity.
    if let Some(cwd) = ctx.as_ref().and_then(|c| c.cwd.clone()) {
        let models = ctx
            .as_ref()
            .map(|c| c.available_models.clone())
            .unwrap_or_default();
        subagent_inbox().observe(&get_subagent_pool(&cwd, &models));
    }
    let inbox = subagent_inbox();
    let session = session_key(ctx.as_ref());
    let scope = session.as_deref();
    // Before answering anything about a handle: a record the pool has already
    // forgotten is reconciled here, so a lost settle event cannot leave the
    // model (or the task panel) looking at a run that finished or died long ago.
    if let Some(cwd) = ctx.as_ref().and_then(|c| c.cwd.clone()) {
        let models = ctx
            .as_ref()
            .map(|c| c.available_models.clone())
            .unwrap_or_default();
        let pool = get_subagent_pool(&cwd, &models);
        inbox.reconcile(
            |task_id| {
                matches!(
                    pool.get_status(task_id),
                    PoolTaskStatus::Running | PoolTaskStatus::Queued
                )
            },
            pool.max_run_budget_ms().saturating_add(RECONCILE_SLACK_MS),
        );
    }
    if params.get("wait").and_then(Value::as_bool) == Some(true) {
        // The pool's resolved wait default (`subagentWaitTimeoutSeconds` or
        // its env), else the crate default when there is no working directory.
        let default_wait_ms = ctx
            .as_ref()
            .and_then(|c| c.cwd.clone())
            .map(|cwd| {
                let models = ctx
                    .as_ref()
                    .map(|c| c.available_models.clone())
                    .unwrap_or_default();
                get_subagent_pool(&cwd, &models).wait_timeout_ms()
            })
            .unwrap_or(DEFAULT_WAIT_TIMEOUT_MS);
        let timeout = Duration::from_millis(
            params
                .get("timeout_ms")
                .and_then(Value::as_f64)
                .map(|n| n.max(0.0) as u64)
                .unwrap_or(default_wait_ms),
        );
        match &handle {
            Some(handle) => {
                inbox.wait_for_in(Some(scope), handle, timeout).await;
            }
            None => inbox.wait_for_all_in(Some(scope), timeout).await,
        }
    }

    let Some(handle) = handle.filter(|_| params.get("list").and_then(Value::as_bool) != Some(true))
    else {
        return format_task_roster(scope);
    };

    let Some(rec) = inbox.get_in(scope, &handle) else {
        return text_result(
            unknown_handle(scope, &handle),
            output_details(Some(&handle), "unknown", false, None),
        );
    };
    match rec.lifecycle {
        TaskLifecycle::Running => {
            let activity = rec
                .last_activity
                .as_deref()
                .filter(|a| !a.is_empty())
                .map(|a| format!(" (currently: {a})"))
                .unwrap_or_default();
            text_result(
                format!(
                    "{} is still running — {} elapsed{activity}. Call AgentOutput again, or with wait:true to block until it finishes.",
                    rec.named(),
                    record_elapsed(&rec)
                ),
                output_details(Some(&handle), "running", true, None),
            )
        }
        TaskLifecycle::Done => {
            let body = inbox
                .collect_in(scope, &handle)
                .map(|(_, body)| body)
                .or(rec.summary_line.clone())
                .unwrap_or_else(|| "(subagent returned no output)".into());
            text_result(body, output_details(Some(&handle), "done", true, None))
        }
        TaskLifecycle::Collected => text_result(
            format!(
                "{} was already delivered — {}.",
                rec.named(),
                rec.summary_line.as_deref().unwrap_or("(no summary kept)")
            ),
            output_details(Some(&handle), "collected", true, None),
        ),
        other => {
            let glyph = if other == TaskLifecycle::Cancelled {
                "⊘"
            } else {
                "✗"
            };
            text_result(
                format!(
                    "{} {} {glyph} — {}.",
                    rec.named(),
                    other.as_str(),
                    rec.error.as_deref().unwrap_or("unknown error")
                ),
                output_details(Some(&handle), other.as_str(), false, None),
            )
        }
    }
}

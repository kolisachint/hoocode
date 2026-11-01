//! The `ShellOutput` and `ShellKill` tools: read and stop the background jobs
//! that `Shell` starts with `run_in_background` (docs/design/tui-activity.md,
//! T5.2). They are opt-in: the session registers them only with
//! `enableBackgroundShell`, so they cost no schema otherwise.

use std::sync::Arc;
use std::time::Duration;

use hoocode_agent_types::AgentToolResult;
use hoocode_ai_types::{AbortSignal, Content, TextContent};
use hoocode_code_tool_api::{ToolDefinition, ToolError};
use serde_json::{json, Value};

use crate::jobs::{JobRegistry, WaitEnd};

/// `ShellOutput`.
pub const SHELL_OUTPUT_TOOL_NAME: &str = "ShellOutput";
/// `ShellKill`.
pub const SHELL_KILL_TOOL_NAME: &str = "ShellKill";

/// How long `ShellOutput` with `wait` blocks when `timeout_ms` is not given.
pub const WAIT_DEFAULT_MS: u64 = 300_000;
/// The most `timeout_ms` may ask for.
pub const WAIT_MAX_MS: u64 = 1_800_000;

fn text_result(text: String) -> AgentToolResult {
    AgentToolResult {
        content: vec![Content::Text(TextContent {
            text_signature: None,
            text,
        })],
        details: Value::Null,
        terminate: false,
    }
}

fn job_id(args: &Value) -> Result<&str, ToolError> {
    let id = args
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| {
            ToolError::from("The \"id\" argument must be a job id such as \"job-1\".")
        })?;
    // A subagent handle (`explore#1`) belongs to AgentOutput; say so rather than
    // "no such job", which sends the model looking for a job it never started.
    if id.contains('#') {
        return Err(format!(
            "{id} is a subagent handle, not a shell job. Use AgentOutput for subagents; ShellOutput reads job-N ids from Shell."
        )
        .into());
    }
    Ok(id)
}

/// A flag as the model sent it: a JSON bool, or the string "true" (models often
/// quote booleans).
fn flag(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(flag)) => *flag,
        Some(Value::String(text)) => text.trim().eq_ignore_ascii_case("true"),
        _ => false,
    }
}

/// The `timeout_ms` of a wait: the default when absent, at most [`WAIT_MAX_MS`].
fn wait_timeout(args: &Value) -> Result<Duration, ToolError> {
    let ms = match args.get("timeout_ms") {
        None | Some(Value::Null) => WAIT_DEFAULT_MS,
        Some(value) => value
            .as_f64()
            .filter(|n| n.is_finite())
            .map(|n| n.max(0.0) as u64)
            .ok_or_else(|| {
                ToolError::from("The \"timeout_ms\" argument must be a number of milliseconds.")
            })?,
    };
    Ok(Duration::from_millis(ms.min(WAIT_MAX_MS)))
}

fn aborted(signal: &Option<AbortSignal>) -> bool {
    signal.as_ref().is_some_and(AbortSignal::aborted)
}

fn definition(
    name: &str,
    description: &str,
    prompt_snippet: &str,
    parameters: Value,
    execute: hoocode_code_tool_api::DefinitionExecuteFn,
) -> ToolDefinition {
    ToolDefinition {
        ordered_start: false,
        background_when: None,
        background_placeholder: None,
        name: name.into(),
        label: name.into(),
        description: description.into(),
        prompt_snippet: Some(prompt_snippet.into()),
        prompt_guidelines: Vec::new(),
        parameters,
        prepare_arguments: None,
        execution_mode: None,
        background: false,
        execute,
    }
}

/// `ShellOutput(id, since?, wait?, timeout_ms?)`: the job's output from byte
/// offset `since`, with the offset to pass next time. Without `since`, the last
/// part of the log. With `wait`, it first blocks until the job ends.
pub fn create_shell_output_tool_definition(jobs: JobRegistry) -> ToolDefinition {
    let parameters = json!({
        "type": "object",
        "required": ["id"],
        "properties": {
            "id": {"type": "string", "description": "The job id that Shell returned, e.g. job-1 (not a subagent handle)"},
            "since": {"type": "number", "description": "Byte offset to read from: the next offset from the previous ShellOutput call (optional; without it the last part of the output is returned)"},
            "wait": {"type": "boolean", "description": "Block until the job has ended before reading (optional). Bounded by timeout_ms. Use this instead of sleeping in a foreground Shell to wait for a job."},
            "timeout_ms": {"type": "number", "description": "Maximum time to block in wait mode, in milliseconds (default 300000, at most 1800000)."}
        }
    });
    definition(
        SHELL_OUTPUT_TOOL_NAME,
        "Read the output of a background shell job started with Shell and run_in_background (ids like job-1; subagents use AgentOutput). Returns the output from the given byte offset, the job's state, and the offset to pass as `since` next time. Set wait:true to block until the job has ended (up to timeout_ms) instead of polling; if it is still running then, the output so far is returned.",
        "read a background Shell job's output (job-N ids); wait: true blocks until it ends",
        parameters,
        Arc::new(move |_id, args, signal, _on_update, _ctx| {
            let id = job_id(&args)?;
            let waited = if flag(args.get("wait")) {
                let timeout = wait_timeout(&args)?;
                let end = jobs.wait_for_exit(id, timeout, || aborted(&signal))?;
                Some((end, timeout))
            } else {
                None
            };
            let since = match args.get("since") {
                None | Some(Value::Null) => None,
                Some(value) => Some(
                    value
                        .as_u64()
                        .ok_or_else(|| ToolError::from("The \"since\" argument must be a whole number of bytes."))?,
                ),
            };
            let read = jobs.read(id, since)?;
            let status = format!(
                "[{id} {}; bytes {}-{} of {}; pass since {} to continue]",
                read.state.describe(),
                read.from,
                read.next,
                read.size,
                read.next
            );
            let body = if read.text.is_empty() {
                "(no new output)".to_owned()
            } else {
                read.text
            };
            let note = match &waited {
                Some((WaitEnd::TimedOut, timeout)) => format!(
                    "\n[still running after {}s; call again with wait: true or keep working]",
                    timeout.as_millis().div_ceil(1000)
                ),
                Some((WaitEnd::Aborted, _)) => {
                    "\n[wait cancelled; the job is still running]".to_owned()
                }
                _ => String::new(),
            };
            // The model waited for this job and the answer shows how it ended, so
            // no completion notice repeats it.
            if matches!(waited, Some((WaitEnd::Exited, _))) {
                jobs.acknowledge(id);
            }
            Ok(text_result(format!("{body}\n\n{status}{note}")))
        }),
    )
}

/// `ShellKill(id)`: stops a running background job (its process group).
pub fn create_shell_kill_tool_definition(jobs: JobRegistry) -> ToolDefinition {
    let parameters = json!({
        "type": "object",
        "required": ["id"],
        "properties": {
            "id": {"type": "string", "description": "The job id that Shell returned, e.g. job-1 (not a subagent handle)"}
        }
    });
    definition(
        SHELL_KILL_TOOL_NAME,
        "Stop a background shell job started with Shell and run_in_background (ids like job-1; subagents are stopped with AgentOutput cancel). Kills the job's whole process group. A job that already finished is reported as it is, and the processes it left running are stopped.",
        "stop a background Shell job (job-N ids)",
        parameters,
        Arc::new(move |_id, args, _signal, _on_update, _ctx| {
            let id = job_id(&args)?;
            let info = jobs.kill(id)?;
            let text = if info.state.is_running() {
                format!("Kill sent to {id}; it has not exited yet.")
            } else {
                format!(
                    "{id} is {}. Any processes it left running were stopped too.",
                    info.state.describe()
                )
            };
            Ok(text_result(text))
        }),
    )
}

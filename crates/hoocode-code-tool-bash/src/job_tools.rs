//! The `ShellOutput` and `ShellKill` tools: read and stop the background jobs
//! that `Shell` starts with `run_in_background` (docs/design/tui-activity.md,
//! T5.2). They are opt-in: the session registers them only with
//! `enableBackgroundShell`, so they cost no schema otherwise.

use std::sync::Arc;

use hoocode_agent_types::AgentToolResult;
use hoocode_ai_types::{Content, TextContent};
use hoocode_code_tool_api::{ToolDefinition, ToolError};
use serde_json::{json, Value};

use crate::jobs::JobRegistry;

/// `ShellOutput`.
pub const SHELL_OUTPUT_TOOL_NAME: &str = "ShellOutput";
/// `ShellKill`.
pub const SHELL_KILL_TOOL_NAME: &str = "ShellKill";

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
    args.get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| ToolError::from("The \"id\" argument must be a job id such as \"job-1\"."))
}

fn definition(
    name: &str,
    description: &str,
    parameters: Value,
    execute: hoocode_code_tool_api::DefinitionExecuteFn,
) -> ToolDefinition {
    ToolDefinition {
        ordered_start: false,
        background_when: None,
        name: name.into(),
        label: name.into(),
        description: description.into(),
        prompt_snippet: Some(format!("{name}: read or stop a background shell job")),
        prompt_guidelines: Vec::new(),
        parameters,
        prepare_arguments: None,
        execution_mode: None,
        background: false,
        execute,
    }
}

/// `ShellOutput(id, since?)`: the job's output from byte offset `since`, with
/// the offset to pass next time. Without `since`, the last part of the log.
pub fn create_shell_output_tool_definition(jobs: JobRegistry) -> ToolDefinition {
    let parameters = json!({
        "type": "object",
        "required": ["id"],
        "properties": {
            "id": {"type": "string", "description": "The job id that Shell returned, e.g. job-1"},
            "since": {"type": "number", "description": "Byte offset to read from: the next offset from the previous ShellOutput call (optional; without it the last part of the output is returned)"}
        }
    });
    definition(
        SHELL_OUTPUT_TOOL_NAME,
        "Read the output of a background shell job started with Shell and run_in_background. Returns the output from the given byte offset, the job's state, and the offset to pass as `since` next time.",
        parameters,
        Arc::new(move |_id, args, _signal, _on_update, _ctx| {
            let id = job_id(&args)?;
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
            Ok(text_result(format!("{body}\n\n{status}")))
        }),
    )
}

/// `ShellKill(id)`: stops a running background job (its process group).
pub fn create_shell_kill_tool_definition(jobs: JobRegistry) -> ToolDefinition {
    let parameters = json!({
        "type": "object",
        "required": ["id"],
        "properties": {
            "id": {"type": "string", "description": "The job id that Shell returned, e.g. job-1"}
        }
    });
    definition(
        SHELL_KILL_TOOL_NAME,
        "Stop a background shell job started with Shell and run_in_background. Kills the job's whole process group. A job that already finished is reported as it is, and the processes it left running are stopped.",
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

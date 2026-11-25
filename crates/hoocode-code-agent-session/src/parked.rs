//! Resolving the call a detached run parked (SA-12 row 4b).
//!
//! A detached run that meets a gated call writes a `parked_call` record, flushes and exits
//! (see `hoocode-code-session::detached`). The assistant message that holds the call is in
//! the file with no result after it. Opening that session again, the user decides: the call
//! runs once (approve) or is answered with a denial, and the calls of the same message that
//! never ran are answered `Not run`. The run then continues from those results.

use std::path::{Path, PathBuf};

use hoocode_agent_types::{AgentEvent, AgentMessage, AgentToolResult};
use hoocode_ai_types::{Content, ToolCallContent, ToolResultMessage};
use hoocode_ai_util::SchemaOrigin;
use hoocode_code_session::detached::PARKED_CALL_ENTRY;
use hoocode_code_session::{FileEntry, SESSION_FLUSH_DEADLINE};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::session::{AgentSession, AgentSessionError};

/// The result text of a call the user denied.
pub const DENIED_TEXT: &str = "Denied by the user.";

/// The result text of a call of a parked message that never ran.
pub const NOT_RUN_TEXT: &str = "Not run: the session parked before this call ran.";

/// The result text of a parked call whose target file changed after it was parked.
fn target_changed_text(path: &Path) -> String {
    format!(
        "Not run: {} changed after this call was parked. Read it again before deciding what to do.",
        path.display()
    )
}

/// A call a detached run parked and no result answers yet.
#[derive(Debug, Clone, PartialEq)]
pub struct ParkedCall {
    pub tool_call_id: String,
    pub tool_name: String,
    /// The call's arguments, as the assistant message holds them.
    pub input: Value,
    /// Why the call needed approval.
    pub reason: String,
    /// The file the call touches, with its sha256 when it existed (`None` when the
    /// input named no path, or the file did not exist when the call was parked).
    pub target: Option<(PathBuf, Option<String>)>,
}

/// What the user decided for a parked call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParkedDecision {
    Approve,
    Deny,
}

/// How a parked call was answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParkedOutcome {
    /// The tool ran (its own result may still be an error).
    Ran,
    /// The user denied it; the result says so.
    Denied,
    /// The target file changed since parking; the call did not run.
    TargetChanged,
}

/// The lowercase hex sha256 of the file at `path`; `None` when it cannot be read.
pub fn file_sha256(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let digest = Sha256::digest(&bytes);
    Some(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// The tool calls of the last assistant message on `branch` that holds any, minus those a
/// result after it answers. Empty when there is none.
fn open_calls_on_branch(branch: &[&FileEntry]) -> Vec<ToolCallContent> {
    let Some(index) = branch.iter().rposition(|entry| {
        matches!(
            entry,
            FileEntry::Message {
                message: AgentMessage::Assistant(assistant),
                ..
            } if assistant.content.iter().any(|c| matches!(c, Content::ToolCall(_)))
        )
    }) else {
        return Vec::new();
    };
    let FileEntry::Message {
        message: AgentMessage::Assistant(assistant),
        ..
    } = branch[index]
    else {
        return Vec::new();
    };
    let answered: Vec<&str> = branch[index + 1..]
        .iter()
        .filter_map(|entry| match entry {
            FileEntry::Message {
                message: AgentMessage::ToolResult(result),
                ..
            } => Some(result.tool_call_id.as_str()),
            _ => None,
        })
        .collect();
    assistant
        .content
        .iter()
        .filter_map(|content| match content {
            Content::ToolCall(call) if !answered.contains(&call.id.as_str()) => Some(call.clone()),
            _ => None,
        })
        .collect()
}

/// The `target` of a `parked_call` record: `(path, sha256)`, or `None` for `null`.
fn target_of(data: &Value) -> Option<(PathBuf, Option<String>)> {
    let target = data.get("target").filter(|t| !t.is_null())?;
    let path = PathBuf::from(target.get("path")?.as_str()?);
    let sha = target
        .get("sha256")
        .and_then(Value::as_str)
        .map(str::to_string);
    Some((path, sha))
}

impl AgentSession {
    /// The parked call this session waits on: the latest `parked_call` record on the
    /// current branch that no later tool result answers, with the arguments of its call.
    pub fn open_parked_call(&self) -> Option<ParkedCall> {
        let manager = self.session_manager();
        let branch = manager.branch(None);
        let open = open_calls_on_branch(&branch);
        branch.iter().enumerate().rev().find_map(|(index, entry)| {
            let FileEntry::Custom {
                custom_type, data, ..
            } = entry
            else {
                return None;
            };
            if custom_type != PARKED_CALL_ENTRY {
                return None;
            }
            let data = data.as_ref()?;
            let tool_call_id = data.get("toolCallId")?.as_str()?;
            let answered_later = branch[index + 1..].iter().any(|later| {
                matches!(
                    later,
                    FileEntry::Message {
                        message: AgentMessage::ToolResult(result),
                        ..
                    } if result.tool_call_id == tool_call_id
                )
            });
            if answered_later {
                return None;
            }
            let call = open.iter().find(|c| c.id == tool_call_id)?;
            Some(ParkedCall {
                tool_call_id: call.id.clone(),
                tool_name: call.name.clone(),
                input: call.arguments.clone(),
                reason: data
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                target: target_of(data),
            })
        })
    }

    /// Whether the file the parked call touches changed (or appeared or vanished) since
    /// it was parked. A call with no target never changed.
    pub fn parked_call_target_changed(&self, call: &ParkedCall) -> bool {
        match &call.target {
            None => false,
            Some((path, stored)) => file_sha256(path) != *stored,
        }
    }

    /// Answers `call` by `decision` and the other open calls of its message with
    /// `Not run`, then flushes the session file. The target is checked again right before
    /// the call runs: a changed file gives [`ParkedOutcome::TargetChanged`] and nothing runs.
    /// `call` comes from [`AgentSession::open_parked_call`].
    pub async fn resolve_parked_call(
        &self,
        call: &ParkedCall,
        decision: ParkedDecision,
    ) -> ParkedOutcome {
        let outcome = match call
            .target
            .as_ref()
            .filter(|_| self.parked_call_target_changed(call))
        {
            Some((path, _)) => {
                self.record_result(
                    &call.tool_call_id,
                    &call.tool_name,
                    &call.input,
                    vec![Content::text(target_changed_text(path))],
                    None,
                    true,
                );
                ParkedOutcome::TargetChanged
            }
            None => match decision {
                ParkedDecision::Approve => {
                    let (content, details, is_error) = self.run_parked_tool(call).await;
                    self.record_result(
                        &call.tool_call_id,
                        &call.tool_name,
                        &call.input,
                        content,
                        details,
                        is_error,
                    );
                    ParkedOutcome::Ran
                }
                ParkedDecision::Deny => {
                    self.record_result(
                        &call.tool_call_id,
                        &call.tool_name,
                        &call.input,
                        vec![Content::text(DENIED_TEXT)],
                        None,
                        true,
                    );
                    ParkedOutcome::Denied
                }
            },
        };
        self.answer_open_calls(NOT_RUN_TEXT);
        let _ = self.session_manager().flush(SESSION_FLUSH_DEADLINE);
        outcome
    }

    /// Continues the run from the trailing tool results (`Agent::continue`), as a prompt
    /// does: the model answers them.
    pub async fn continue_run(&self) -> Result<(), AgentSessionError> {
        self.agent()
            .r#continue()
            .await
            .map_err(|e| AgentSessionError(e.0))?;
        self.wait_for_retry().await;
        Ok(())
    }

    /// Runs the parked call's tool once, as the loop would after approval, without the
    /// permission gate. Returns the content, the details and whether it failed.
    async fn run_parked_tool(&self, call: &ParkedCall) -> (Vec<Content>, Option<Value>, bool) {
        let tool = self
            .agent()
            .with_state(|s| s.tools.find(&call.tool_name).cloned());
        let Some(tool) = tool else {
            return (
                vec![Content::text(format!("Tool {} not found", call.tool_name))],
                None,
                true,
            );
        };
        let prepared = match &tool.prepare_arguments {
            Some(prepare) => prepare(call.input.clone()),
            None => call.input.clone(),
        };
        let origin = if tool.plain_json_schema {
            SchemaOrigin::PlainJson
        } else {
            SchemaOrigin::TypeBox
        };
        let args = match hoocode_ai_util::validate_tool_arguments(
            &tool.name,
            &tool.parameters,
            &prepared,
            origin,
        ) {
            Ok(args) => args,
            Err(message) => return (vec![Content::text(message)], None, true),
        };
        let execute = tool.execute.clone();
        let id = call.tool_call_id.clone();
        let outcome = tokio::task::spawn_blocking(move || execute(id, args, None, None)).await;
        match outcome {
            Ok(Ok(AgentToolResult {
                content, details, ..
            })) => (content, (!details.is_null()).then_some(details), false),
            Ok(Err(e)) => (vec![Content::text(e.to_string())], None, true),
            Err(e) => (
                vec![Content::text(format!("tool task failed: {e}"))],
                None,
                true,
            ),
        }
    }

    /// Answers every call of the last assistant message that has no result yet with `text`.
    fn answer_open_calls(&self, text: &str) {
        let open: Vec<ToolCallContent> = {
            let manager = self.session_manager();
            open_calls_on_branch(&manager.branch(None))
        };
        for call in open {
            self.record_result(
                &call.id,
                &call.name,
                &call.arguments,
                vec![Content::text(text)],
                None,
                true,
            );
        }
    }

    /// Emits and persists one tool result as the loop does, and adds it to the agent's
    /// messages so the next run sees it.
    fn record_result(
        &self,
        tool_call_id: &str,
        tool_name: &str,
        args: &Value,
        content: Vec<Content>,
        details: Option<Value>,
        is_error: bool,
    ) {
        self.process_agent_event(&AgentEvent::ToolExecutionStart {
            tool_call_id: tool_call_id.to_string(),
            tool_name: tool_name.to_string(),
            args: args.clone(),
            origin: None,
            render: None,
        });
        self.process_agent_event(&AgentEvent::ToolExecutionEnd {
            tool_call_id: tool_call_id.to_string(),
            tool_name: tool_name.to_string(),
            result: AgentToolResult {
                content: content.clone(),
                details: details.clone().unwrap_or(Value::Null),
                terminate: false,
            },
            is_error,
        });
        let message = AgentMessage::ToolResult(ToolResultMessage {
            duration_ms: None,
            tool_call_id: tool_call_id.to_string(),
            tool_name: tool_name.to_string(),
            content,
            details,
            is_error,
            timestamp: hoocode_ai_types::now_ms(),
        });
        self.process_agent_event(&AgentEvent::MessageStart {
            message: message.clone(),
        });
        self.process_agent_event(&AgentEvent::MessageEnd {
            message: message.clone(),
        });
        self.agent().append_message(message);
    }
}

//! `core/agent-session-stats.ts`: statistics, context usage, forkable user
//! messages, transcripts and JSONL export derived from session state.

use std::path::{Path, PathBuf};

use hoocode_agent_types::AgentMessage;
use hoocode_ai_types::{Content, Model, StopReason, UserContent};
use hoocode_code_session::{FileEntry, SessionManager, CURRENT_SESSION_VERSION};
use serde::Serialize;

/// `ContextUsage`: `tokens`/`percent` are `None` right after a compaction,
/// until the next response reports real usage.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextUsage {
    pub tokens: Option<u64>,
    pub context_window: u64,
    #[serde(serialize_with = "js_f64_opt")]
    pub percent: Option<f64>,
}

fn js_f64_opt<S: serde::Serializer>(n: &Option<f64>, serializer: S) -> Result<S::Ok, S::Error> {
    match n {
        Some(n) => hoocode_ai_types::js_f64(n, serializer),
        None => serializer.serialize_none(),
    }
}

/// Token totals in [`SessionStats`].
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenStats {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub total: u64,
}

/// `SessionStats` (for `/session`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStats {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_file: Option<String>,
    pub session_id: String,
    pub user_messages: usize,
    pub assistant_messages: usize,
    pub tool_calls: usize,
    pub tool_results: usize,
    pub total_messages: usize,
    pub tokens: TokenStats,
    #[serde(serialize_with = "hoocode_ai_types::js_f64")]
    pub cost: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_usage: Option<ContextUsage>,
}

/// `AssistantUsageTotals`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AssistantUsageTotals {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cost: f64,
}

/// `sumAssistantUsage`: token and cost totals over the assistant message entries.
pub fn sum_assistant_usage<'a>(
    entries: impl IntoIterator<Item = &'a FileEntry>,
) -> AssistantUsageTotals {
    let mut totals = AssistantUsageTotals::default();
    for entry in entries {
        if let FileEntry::Message {
            message: AgentMessage::Assistant(message),
            ..
        } = entry
        {
            totals.input += message.usage.input;
            totals.output += message.usage.output;
            totals.cache_read += message.usage.cache_read;
            totals.cache_write += message.usage.cache_write;
            totals.cost += message.usage.cost.total;
        }
    }
    totals
}

/// `extractUserMessageText`: the text blocks, concatenated.
pub fn extract_user_message_text(content: &UserContent) -> String {
    match content {
        UserContent::Text(text) => text.clone(),
        UserContent::Blocks(blocks) => text_of(blocks),
    }
}

fn text_of(blocks: &[Content]) -> String {
    blocks
        .iter()
        .filter_map(|block| match block {
            Content::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect()
}

/// `computeSessionStats`.
pub fn compute_session_stats(
    messages: &[AgentMessage],
    session_file: Option<String>,
    session_id: String,
    context_usage: Option<ContextUsage>,
) -> SessionStats {
    let count = |role: &str| messages.iter().filter(|m| m.role() == role).count();
    let mut tool_calls = 0;
    let mut tokens = TokenStats::default();
    let mut cost = 0.0;
    for message in messages {
        if let AgentMessage::Assistant(message) = message {
            tool_calls += message
                .content
                .iter()
                .filter(|c| matches!(c, Content::ToolCall(_)))
                .count();
            tokens.input += message.usage.input;
            tokens.output += message.usage.output;
            tokens.cache_read += message.usage.cache_read;
            tokens.cache_write += message.usage.cache_write;
            cost += message.usage.cost.total;
        }
    }
    tokens.total = tokens.input + tokens.output + tokens.cache_read + tokens.cache_write;
    SessionStats {
        session_file,
        session_id,
        user_messages: count("user"),
        assistant_messages: count("assistant"),
        tool_calls,
        tool_results: count("toolResult"),
        total_messages: messages.len(),
        tokens,
        cost,
        context_usage,
    }
}

/// `computeContextUsage`: estimated context-window usage. After a compaction
/// the usage is unknown until an assistant responds past the boundary.
pub fn compute_context_usage(
    model: Option<&Model>,
    session_manager: &SessionManager,
    messages: &[AgentMessage],
) -> Option<ContextUsage> {
    let context_window = model?.context_window;
    if context_window == 0 {
        return None;
    }
    let branch = session_manager.branch(None);
    let latest_compaction = branch
        .iter()
        .rposition(|entry| matches!(entry, FileEntry::Compaction { .. }));
    if let Some(compaction_index) = latest_compaction {
        let mut has_post_compaction_usage = false;
        for entry in branch[compaction_index + 1..].iter().rev() {
            if let FileEntry::Message {
                message: AgentMessage::Assistant(assistant),
                ..
            } = entry
            {
                if !matches!(
                    assistant.stop_reason,
                    StopReason::Aborted | StopReason::Error
                ) {
                    if hoocode_agent_compaction::calculate_context_tokens(&assistant.usage) > 0 {
                        has_post_compaction_usage = true;
                    }
                    break;
                }
            }
        }
        if !has_post_compaction_usage {
            return Some(ContextUsage {
                tokens: None,
                context_window,
                percent: None,
            });
        }
    }
    let tokens = hoocode_agent_compaction::estimate_context_tokens(messages).tokens;
    Some(ContextUsage {
        tokens: Some(tokens),
        context_window,
        percent: Some(tokens as f64 / context_window as f64 * 100.0),
    })
}

/// A user message the fork selector offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkableMessage {
    pub entry_id: String,
    pub text: String,
}

/// `collectUserMessagesForForking`: every user message entry with text.
pub fn collect_user_messages_for_forking(session_manager: &SessionManager) -> Vec<ForkableMessage> {
    session_manager
        .entries()
        .iter()
        .filter_map(|entry| match entry {
            FileEntry::Message {
                id,
                message: AgentMessage::User(user),
                ..
            } => {
                let text = extract_user_message_text(&user.content);
                (!text.is_empty()).then(|| ForkableMessage {
                    entry_id: id.clone(),
                    text,
                })
            }
            _ => None,
        })
        .collect()
}

/// Prefix for a subagent summary whose text is stale: the run made tool calls
/// after the assistant message the text came from, so it may be narration.
pub const FROM_EARLIER_TURN_PREFIX: &str = "(from an earlier turn) ";

/// `getLastAssistantText`: the text of the last assistant message (skipping
/// empty aborted ones), trimmed; `None` when empty.
pub fn get_last_assistant_text(messages: &[AgentMessage]) -> Option<String> {
    let last = messages.iter().rev().find_map(|m| match m {
        AgentMessage::Assistant(a)
            if !(a.stop_reason == StopReason::Aborted && a.content.is_empty()) =>
        {
            Some(a)
        }
        _ => None,
    })?;
    let text = text_of(&last.content);
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// The text of the last assistant message that has any text (skipping empty
/// aborted ones), plus whether it is stale: `true` when that message or any
/// assistant message after it contains a tool call, so the run kept working
/// after that text (a run cut mid-tool leaves its own tool call unanswered).
/// Empty or aborted messages after it do not count. `None` when no assistant
/// message has text.
pub fn last_assistant_text_with_origin(messages: &[AgentMessage]) -> Option<(String, bool)> {
    let (index, text) = messages
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, m)| match m {
            AgentMessage::Assistant(a)
                if !(a.stop_reason == StopReason::Aborted && a.content.is_empty()) =>
            {
                let text = text_of(&a.content);
                let text = text.trim();
                (!text.is_empty()).then(|| (index, text.to_string()))
            }
            _ => None,
        })?;
    let stale = messages[index..].iter().any(|m| match m {
        AgentMessage::Assistant(a) => a.content.iter().any(|c| matches!(c, Content::ToolCall(_))),
        _ => false,
    });
    Some((text, stale))
}

/// `TranscriptSelection`: how much of a session a copy takes.
#[derive(Debug, Clone, Default)]
pub struct TranscriptSelection {
    /// Exchanges counted back from the newest; `None` takes them all.
    pub turns: Option<usize>,
    pub user_label: Option<String>,
    pub agent_label: Option<String>,
}

/// `sessionToMarkdown`: the conversation (no tool calls) as markdown.
pub fn session_to_markdown(messages: &[AgentMessage], selection: &TranscriptSelection) -> String {
    let user_label = selection.user_label.as_deref().unwrap_or("You");
    let agent_label = selection.agent_label.as_deref().unwrap_or("Agent");
    let mut blocks: Vec<(&str, String, bool)> = Vec::new();
    for message in messages {
        match message {
            AgentMessage::User(user) => {
                let text = extract_user_message_text(&user.content).trim().to_string();
                if !text.is_empty() {
                    blocks.push((user_label, text, true));
                }
            }
            AgentMessage::Assistant(assistant) => {
                let text = text_of(&assistant.content).trim().to_string();
                if !text.is_empty() {
                    blocks.push((agent_label, text, false));
                }
            }
            _ => {}
        }
    }
    if let Some(turns) = selection.turns.filter(|&t| t > 0) {
        let starts: Vec<usize> = blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| b.2)
            .map(|(i, _)| i)
            .collect();
        if let Some(&from) = starts.get(starts.len().saturating_sub(turns)) {
            blocks.drain(..from);
        }
    }
    blocks
        .iter()
        .map(|(label, text, _)| format!("## {label}\n\n{text}"))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn iso_now() -> String {
    chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

/// `exportSessionBranchToJsonl`: the header, then the current branch with its
/// parent ids re-chained into a line. Returns the absolute output path.
pub fn export_session_branch_to_jsonl(
    session_manager: &SessionManager,
    output_path: Option<&Path>,
) -> std::io::Result<PathBuf> {
    let path = match output_path {
        Some(path) => path.to_path_buf(),
        None => PathBuf::from(format!(
            "session-{}.jsonl",
            iso_now().replace([':', '.'], "-")
        )),
    };
    let path = std::path::absolute(path)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let header = serde_json::json!({
        "type": "session",
        "version": CURRENT_SESSION_VERSION,
        "id": session_manager.session_id(),
        "timestamp": iso_now(),
        "cwd": session_manager.cwd(),
    });
    let mut lines = vec![header.to_string()];
    let mut prev_id: Option<String> = None;
    for entry in session_manager.branch(None) {
        let mut value = serde_json::to_value(entry).map_err(std::io::Error::other)?;
        if let Some(object) = value.as_object_mut() {
            object.insert(
                "parentId".into(),
                prev_id.clone().map_or(serde_json::Value::Null, Into::into),
            );
        }
        lines.push(value.to_string());
        prev_id = entry.id().map(str::to_string);
    }
    std::fs::write(&path, format!("{}\n", lines.join("\n")))?;
    Ok(path)
}

#[cfg(test)]
mod last_assistant_text_tests {
    use super::*;
    use hoocode_ai_types::{AssistantMessage, ToolCallContent};

    fn assistant(content: Vec<Content>, stop_reason: StopReason) -> AgentMessage {
        AgentMessage::Assistant(AssistantMessage {
            content,
            stop_reason,
            ..Default::default()
        })
    }

    fn tool_call_only() -> AgentMessage {
        assistant(
            vec![Content::ToolCall(ToolCallContent {
                id: "t".into(),
                name: "Bash".into(),
                arguments: serde_json::json!({"command": "ls"}),
                thought_signature: None,
            })],
            StopReason::ToolUse,
        )
    }

    fn narration(text: &str) -> AgentMessage {
        assistant(vec![Content::text(text)], StopReason::ToolUse)
    }

    #[test]
    fn strict_getter_stops_at_the_final_assistant_message() {
        // Existing callers (/copy, the RPC command) keep the original
        // behavior: a tool-only final message means no text.
        let messages = [narration("Found it at pool.rs:176."), tool_call_only()];
        assert_eq!(get_last_assistant_text(&messages), None);
        let messages = [narration("Done."), tool_call_only()];
        assert_eq!(get_last_assistant_text(&messages), None);
    }

    #[test]
    fn strict_getter_returns_the_final_text() {
        let messages = [narration("first"), narration("  second  ")];
        assert_eq!(
            get_last_assistant_text(&messages).as_deref(),
            Some("second")
        );
    }

    #[test]
    fn origin_walks_back_past_a_tool_only_final_message() {
        let messages = [narration("Found it at pool.rs:176."), tool_call_only()];
        assert_eq!(
            last_assistant_text_with_origin(&messages),
            Some(("Found it at pool.rs:176.".to_string(), true))
        );
    }

    #[test]
    fn origin_is_false_when_the_final_message_has_the_text() {
        let messages = [tool_call_only(), narration("  Answer.  ")];
        assert_eq!(
            last_assistant_text_with_origin(&messages),
            Some(("Answer.".to_string(), false))
        );
    }

    #[test]
    fn origin_skips_an_empty_aborted_final_message_without_marking_it_stale() {
        // A deadline wrap-up that aborts with no content: the earlier text is
        // the answer, so there is no prefix.
        let aborted = assistant(Vec::new(), StopReason::Aborted);
        let messages = [narration("Partial plan."), aborted];
        assert_eq!(
            last_assistant_text_with_origin(&messages),
            Some(("Partial plan.".to_string(), false))
        );
    }

    #[test]
    fn origin_is_stale_when_a_tool_call_follows_the_text() {
        // Tool calls after the text, then an empty aborted message: the run
        // kept working, so the text may be stale.
        let aborted = assistant(Vec::new(), StopReason::Aborted);
        let messages = [
            narration("Found it at pool.rs:176."),
            tool_call_only(),
            aborted,
        ];
        assert_eq!(
            last_assistant_text_with_origin(&messages),
            Some(("Found it at pool.rs:176.".to_string(), true))
        );
    }

    #[test]
    fn origin_is_stale_when_the_text_message_itself_has_a_tool_call() {
        // "Checking pool.rs next" + Read, then the run was cut mid-tool: the
        // message's own tool call counts, so the text is not the answer.
        let text_and_call = assistant(
            vec![
                Content::text("Checking pool.rs next"),
                Content::ToolCall(ToolCallContent {
                    id: "r".into(),
                    name: "Read".into(),
                    arguments: serde_json::json!({"path": "pool.rs"}),
                    thought_signature: None,
                }),
            ],
            StopReason::ToolUse,
        );
        assert_eq!(
            last_assistant_text_with_origin(&[text_and_call]),
            Some(("Checking pool.rs next".to_string(), true))
        );
    }

    #[test]
    fn origin_ignores_user_and_tool_result_messages_after_the_text() {
        // Only assistant tool calls make the text stale; a plain wrap-up user
        // message after the text does not.
        let user = AgentMessage::User(hoocode_ai_types::UserMessage {
            content: hoocode_ai_types::UserContent::Text("Stop and summarize.".into()),
            ..Default::default()
        });
        let messages = [narration("Summary of findings."), user];
        assert_eq!(
            last_assistant_text_with_origin(&messages),
            Some(("Summary of findings.".to_string(), false))
        );
    }

    #[test]
    fn origin_is_none_when_no_assistant_message_has_text() {
        assert_eq!(last_assistant_text_with_origin(&[tool_call_only()]), None);
        assert_eq!(last_assistant_text_with_origin(&[]), None);
    }

    #[test]
    fn prefix_constant_is_the_documented_wording() {
        assert_eq!(FROM_EARLIER_TURN_PREFIX, "(from an earlier turn) ");
    }
}

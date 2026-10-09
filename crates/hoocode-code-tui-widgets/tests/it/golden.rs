//! Component goldens (T0.2): the task panel, the Shell and Agent tool blocks,
//! the session chip, and the user and assistant messages, at 80 columns (and
//! 120 for the key cases). Output is pinned to the dark theme and carries no
//! times: tool blocks get no `startedAt` (no `Took`/`Elapsed` line) and the
//! task panel shows only plan items, never delegated runs.

use hoocode_ai_types::{AssistantMessage, Content, ThinkingContent};
use hoocode_code_task_store::{task_store, CreateTaskOptions, TaskPatch, TaskStatus};
use hoocode_code_tui_widgets::session_chip::render_session_chip;
use hoocode_code_tui_widgets::task_panel::{TaskPanelComponent, TaskPanelDensity};
use hoocode_code_tui_widgets::tool_execution::{ToolExecutionComponent, ToolExecutionOptions};
use hoocode_code_tui_widgets::tool_output_view::ToolOutputView;
use hoocode_code_tui_widgets::tool_signal::ToolResult;
use hoocode_code_tui_widgets::tools::{builtin_tool_definition, registered_tool_definition};
use hoocode_code_tui_widgets::{AssistantMessageComponent, ThinkingDisplay, UserMessageComponent};
use hoocode_tui_components::Text;
use hoocode_tui_render::assert_golden;
use hoocode_tui_render::golden::render_golden;
use serde_json::{json, Value};

use crate::support::lock;

/// A fixed working directory: the tool renderers print paths relative to it.
const CWD: &str = "/work/project";

fn tool(name: &str, args: Value, view: ToolOutputView) -> ToolExecutionComponent {
    // The registry as the app builds it: built-in renderers first, then the
    // registered ones (Agent, AgentOutput).
    let definition =
        builtin_tool_definition(name).unwrap_or_else(|| registered_tool_definition(name));
    ToolExecutionComponent::new(
        name,
        "call-1",
        args,
        ToolExecutionOptions {
            view,
            ..Default::default()
        },
        Some(definition),
        CWD,
    )
}

fn result(text: &str, is_error: bool, details: Value) -> ToolResult {
    ToolResult {
        content: vec![Content::text(text)],
        details,
        is_error,
    }
}

fn shell_args() -> Value {
    json!({ "command": "cargo test -p hoocode-code-tui-widgets" })
}

/// Lines of test output, `n` of them, numbered so truncation is visible.
fn output_lines(n: usize) -> String {
    (1..=n)
        .map(|i| format!("test case_{i:02} ... ok"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn add_task(title: &str, status: TaskStatus) {
    let id = task_store().create(title, CreateTaskOptions::default()).id;
    if status != TaskStatus::Pending {
        task_store().update(
            id,
            TaskPatch {
                status: Some(status),
                ..Default::default()
            },
        );
    }
}

fn mixed_tasks() {
    add_task("Read the migration plan", TaskStatus::Done);
    add_task("Port the task panel renderer", TaskStatus::InProgress);
    add_task("Add golden snapshots", TaskStatus::Pending);
    add_task("Run the parity check", TaskStatus::Failed);
    add_task("Drop the old lens tabs", TaskStatus::Cancelled);
}

const LONG_TITLE: &str = "Refactor the authentication middleware so that session tokens are rotated on every privileged request and the audit log records each rotation with the caller's address";

fn assistant(content: Vec<Content>) -> AssistantMessage {
    AssistantMessage {
        content,
        api: "openai-responses".into(),
        provider: "openai".into(),
        model: "gpt-4o-mini".into(),
        ..Default::default()
    }
}

fn thinking(text: &str) -> Content {
    Content::Thinking(ThinkingContent {
        thinking: text.into(),
        signature: Some(String::new()),
        ..Default::default()
    })
}

const MARKDOWN: &str = "## Plan\n\nThe fix has two parts:\n\n1. Rotate the token in `middleware.rs`.\n2. Log the rotation.\n\n```rust\nfn rotate(session: &mut Session) {\n    session.token = new_token();\n}\n```\n\nRun `cargo test` afterwards.";

// ---- task panel ----

#[test]
fn task_panel_empty_80() {
    let _g = lock();
    task_store().clear();
    let mut panel = TaskPanelComponent::new();
    assert_golden!("task_panel_empty_80", render_golden(&mut panel, 80));
}

#[test]
fn task_panel_mixed_80() {
    let _g = lock();
    task_store().clear();
    mixed_tasks();
    let mut panel = TaskPanelComponent::new();
    assert_golden!("task_panel_mixed_80", render_golden(&mut panel, 80));
}

#[test]
fn task_panel_mixed_120() {
    let _g = lock();
    task_store().clear();
    mixed_tasks();
    let mut panel = TaskPanelComponent::new();
    assert_golden!("task_panel_mixed_120", render_golden(&mut panel, 120));
}

#[test]
fn task_panel_long_text_80() {
    let _g = lock();
    task_store().clear();
    add_task(LONG_TITLE, TaskStatus::InProgress);
    add_task("Short follow-up", TaskStatus::Pending);
    let mut panel = TaskPanelComponent::new();
    assert_golden!("task_panel_long_text_80", render_golden(&mut panel, 80));
}

#[test]
fn task_panel_long_text_120() {
    let _g = lock();
    task_store().clear();
    add_task(LONG_TITLE, TaskStatus::InProgress);
    add_task("Short follow-up", TaskStatus::Pending);
    let mut panel = TaskPanelComponent::new();
    assert_golden!("task_panel_long_text_120", render_golden(&mut panel, 120));
}

#[test]
fn task_panel_summary_80() {
    let _g = lock();
    task_store().clear();
    mixed_tasks();
    let mut panel = TaskPanelComponent::new();
    panel.set_density(TaskPanelDensity::Summary);
    assert_golden!("task_panel_summary_80", render_golden(&mut panel, 80));
}

// ---- Shell (bash) tool block ----

#[test]
fn shell_running_80() {
    let _g = lock();
    let mut block = tool("Shell", shell_args(), ToolOutputView::Peek);
    block.update_result(result(&output_lines(2), false, json!({})), true);
    assert_golden!("shell_running_80", render_golden(&mut block, 80));
}

#[test]
fn shell_success_80() {
    let _g = lock();
    let mut block = tool("Shell", shell_args(), ToolOutputView::Peek);
    block.update_result(
        result("test result: ok. 12 passed; 0 failed", false, json!({})),
        false,
    );
    assert_golden!("shell_success_80", render_golden(&mut block, 80));
}

#[test]
fn shell_success_120() {
    let _g = lock();
    let mut block = tool("Shell", shell_args(), ToolOutputView::Peek);
    block.update_result(
        result("test result: ok. 12 passed; 0 failed", false, json!({})),
        false,
    );
    assert_golden!("shell_success_120", render_golden(&mut block, 120));
}

#[test]
fn shell_error_80() {
    let _g = lock();
    let mut block = tool("Shell", shell_args(), ToolOutputView::Peek);
    block.update_result(
        result(
            "error[E0432]: unresolved import `crate::missing`\nexit code 101",
            true,
            json!({}),
        ),
        false,
    );
    assert_golden!("shell_error_80", render_golden(&mut block, 80));
}

#[test]
fn shell_output_collapsed_80() {
    let _g = lock();
    let mut block = tool("Shell", shell_args(), ToolOutputView::Radar);
    block.update_result(result(&output_lines(12), false, json!({})), false);
    assert_golden!("shell_output_collapsed_80", render_golden(&mut block, 80));
}

#[test]
fn shell_output_peek_80() {
    let _g = lock();
    let mut block = tool("Shell", shell_args(), ToolOutputView::Peek);
    block.update_result(result(&output_lines(12), false, json!({})), false);
    assert_golden!("shell_output_peek_80", render_golden(&mut block, 80));
}

#[test]
fn shell_output_expanded_80() {
    let _g = lock();
    let mut block = tool("Shell", shell_args(), ToolOutputView::Full);
    block.update_result(result(&output_lines(12), false, json!({})), false);
    assert_golden!("shell_output_expanded_80", render_golden(&mut block, 80));
}

// ---- Agent tool block ----

fn agent_args() -> Value {
    json!({
        "subagent_type": "explore",
        "description": "Find the footer code",
        "prompt": "Locate the footer renderer and list its inputs."
    })
}

#[test]
fn agent_call_80() {
    let _g = lock();
    let mut block = tool("Agent", agent_args(), ToolOutputView::Peek);
    assert_golden!("agent_call_80", render_golden(&mut block, 80));
}

#[test]
fn agent_background_call_80() {
    let _g = lock();
    let mut args = agent_args();
    args["background"] = json!(true);
    let mut block = tool("Agent", args, ToolOutputView::Peek);
    assert_golden!("agent_background_call_80", render_golden(&mut block, 80));
}

#[test]
fn agent_result_80() {
    let _g = lock();
    let mut block = tool("Agent", agent_args(), ToolOutputView::Peek);
    block.update_result(
        result("Found the footer in footer.rs.", false, json!({})),
        false,
    );
    assert_golden!("agent_result_80", render_golden(&mut block, 80));
}

#[test]
fn agent_result_120() {
    let _g = lock();
    let mut block = tool("Agent", agent_args(), ToolOutputView::Peek);
    block.update_result(
        result("Found the footer in footer.rs.", false, json!({})),
        false,
    );
    assert_golden!("agent_result_120", render_golden(&mut block, 120));
}

// ---- AgentOutput result card (subagent status) ----

fn agent_output(text: &str, status: &str) -> ToolResult {
    result(
        text,
        false,
        json!({ "task_id": "explore#1", "status": status }),
    )
}

#[test]
fn agent_output_done_80() {
    let _g = lock();
    let mut block = tool(
        "AgentOutput",
        json!({ "task_id": "explore#1" }),
        ToolOutputView::Full,
    );
    block.update_result(
        agent_output("Footer is in footer.rs, 340 lines.", "done"),
        false,
    );
    assert_golden!("agent_output_done_80", render_golden(&mut block, 80));
}

#[test]
fn agent_output_running_80() {
    let _g = lock();
    let mut block = tool(
        "AgentOutput",
        json!({ "task_id": "explore#1" }),
        ToolOutputView::Full,
    );
    block.update_result(agent_output("Still reading files.", "running"), true);
    assert_golden!("agent_output_running_80", render_golden(&mut block, 80));
}

#[test]
fn agent_output_failed_80() {
    let _g = lock();
    let mut block = tool(
        "AgentOutput",
        json!({ "task_id": "explore#1" }),
        ToolOutputView::Full,
    );
    block.update_result(
        agent_output("Stopped: tool limit reached.", "failed"),
        false,
    );
    assert_golden!("agent_output_failed_80", render_golden(&mut block, 80));
}

// ---- session chip ----

#[test]
fn session_chip_80() {
    let _g = lock();
    let chip = render_session_chip("refactor-auth", 1).expect("a chip for a name");
    let mut line = Text::new(chip.styled, 0, 0);
    assert_golden!("session_chip_80", render_golden(&mut line, 80));
}

#[test]
fn session_chip_truncated_80() {
    let _g = lock();
    let chip = render_session_chip("an-extremely-long-session-name-nobody-can-scan", 3)
        .expect("a chip for a name");
    let mut line = Text::new(chip.styled, 0, 0);
    assert_golden!("session_chip_truncated_80", render_golden(&mut line, 80));
}

#[test]
fn session_chip_120() {
    let _g = lock();
    let chip = render_session_chip("amber-harbor", 4).expect("a chip for a name");
    let mut line = Text::new(chip.styled, 0, 0);
    assert_golden!("session_chip_120", render_golden(&mut line, 120));
}

// ---- user message ----

#[test]
fn user_message_plain_80() {
    let _g = lock();
    let mut msg =
        UserMessageComponent::new("Why does the footer hide the model name on narrow terminals?");
    assert_golden!("user_message_plain_80", render_golden(&mut msg, 80));
}

#[test]
fn user_message_markdown_80() {
    let _g = lock();
    let mut msg = UserMessageComponent::new(
        "Please fix this:\n\n```rust\nfn main() {\n    println!(\"hi\");\n}\n```\n\nThanks, see `main.rs`.",
    );
    assert_golden!("user_message_markdown_80", render_golden(&mut msg, 80));
}

#[test]
fn user_message_plain_120() {
    let _g = lock();
    let mut msg =
        UserMessageComponent::new("Why does the footer hide the model name on narrow terminals?");
    assert_golden!("user_message_plain_120", render_golden(&mut msg, 120));
}

// ---- assistant message ----

#[test]
fn assistant_message_plain_80() {
    let _g = lock();
    let mut msg = AssistantMessageComponent::new(
        Some(&assistant(vec![Content::text(
            "The model name is dropped when the line is too long.",
        )])),
        ThinkingDisplay::Full,
    );
    assert_golden!("assistant_message_plain_80", render_golden(&mut msg, 80));
}

#[test]
fn assistant_message_markdown_80() {
    let _g = lock();
    let mut msg = AssistantMessageComponent::new(
        Some(&assistant(vec![Content::text(MARKDOWN)])),
        ThinkingDisplay::Full,
    );
    assert_golden!("assistant_message_markdown_80", render_golden(&mut msg, 80));
}

#[test]
fn assistant_message_markdown_120() {
    let _g = lock();
    let mut msg = AssistantMessageComponent::new(
        Some(&assistant(vec![Content::text(MARKDOWN)])),
        ThinkingDisplay::Full,
    );
    assert_golden!(
        "assistant_message_markdown_120",
        render_golden(&mut msg, 120)
    );
}

#[test]
fn assistant_message_thinking_full_80() {
    let _g = lock();
    let mut msg = AssistantMessageComponent::new(
        Some(&assistant(vec![
            thinking("The footer drops the name first, then the branch."),
            Content::text("Narrow footers drop the session name first."),
        ])),
        ThinkingDisplay::Full,
    );
    assert_golden!(
        "assistant_message_thinking_full_80",
        render_golden(&mut msg, 80)
    );
}

#[test]
fn assistant_message_thinking_label_80() {
    let _g = lock();
    let mut msg = AssistantMessageComponent::new(
        Some(&assistant(vec![
            thinking("The footer drops the name first, then the branch."),
            Content::text("Narrow footers drop the session name first."),
        ])),
        ThinkingDisplay::Label,
    );
    assert_golden!(
        "assistant_message_thinking_label_80",
        render_golden(&mut msg, 80)
    );
}

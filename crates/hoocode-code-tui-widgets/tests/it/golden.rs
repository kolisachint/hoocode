//! Component goldens (T0.2): the task panel, the Shell and Agent tool blocks,
//! the session chip, and the user and assistant messages, at 80 columns (and
//! 120 for the key cases). Output is pinned to the dark theme and carries no
//! times: tool blocks get no `startedAt` (no `Took`/`Elapsed` line) and the
//! task panel shows only plan items, never delegated runs.

use std::cell::RefCell;
use std::rc::Rc;

use hoocode_ai_types::{AssistantMessage, Content, ThinkingContent};
use hoocode_code_task_store::{task_store, CreateTaskOptions, TaskPatch, TaskSource, TaskStatus};
use hoocode_code_tui_keybindings::AppKeybindingsManager;
use hoocode_code_tui_widgets::outline::{FlightMark, FlightStats};
use hoocode_code_tui_widgets::task_panel::{TaskPanelComponent, TaskPanelView};
use hoocode_code_tui_widgets::tool_chain::ToolChainComponent;
use hoocode_code_tui_widgets::tool_execution::{ToolExecutionComponent, ToolExecutionOptions};
use hoocode_code_tui_widgets::tool_signal::ToolResult;
use hoocode_code_tui_widgets::tools::{builtin_tool_definition, registered_tool_definition};
use hoocode_code_tui_widgets::trajectory::{LandingLine, MissionHeader};
use hoocode_code_tui_widgets::{AssistantMessageComponent, ThinkingDisplay, UserMessageComponent};
use hoocode_tui_render::golden::render_golden;
use hoocode_tui_render::{assert_golden, Component};
use serde_json::{json, Value};

use crate::support::lock;

/// A fixed working directory: the tool renderers print paths relative to it.
const CWD: &str = "/work/project";

fn tool(name: &str, args: Value) -> ToolExecutionComponent {
    // The registry as the app builds it: built-in renderers first, then the
    // registered ones (Agent, AgentOutput).
    let definition =
        builtin_tool_definition(name).unwrap_or_else(|| registered_tool_definition(name));
    ToolExecutionComponent::new(
        name,
        "call-1",
        args,
        ToolExecutionOptions::default(),
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

// ---- task panel tabs (T4.1) ----

/// One row of each kind, with no timed rows: a plan item in progress, a Done
/// subagent run and a Done MCP call (a running one would show its clock).
fn three_tabs() {
    add_task("Read the migration plan", TaskStatus::Done);
    add_task("Port the task panel renderer", TaskStatus::InProgress);
    add_task("Add golden snapshots", TaskStatus::Pending);
    let run = task_store()
        .create(
            "Trace the auth flow",
            CreateTaskOptions {
                source: Some(TaskSource::Subagent),
                subagent_mode: Some("explore".into()),
                agent: Some("run-1".into()),
                ..Default::default()
            },
        )
        .id;
    set_done(run);
    let call = task_store()
        .create(
            "Fetch the spec",
            CreateTaskOptions {
                source: Some(TaskSource::Mcp),
                subagent_mode: Some("web".into()),
                ..Default::default()
            },
        )
        .id;
    set_done(call);
}

fn set_done(id: u64) {
    task_store().update(
        id,
        TaskPatch {
            status: Some(TaskStatus::Done),
            ..Default::default()
        },
    );
}

#[test]
fn task_panel_tabs_unfocused_80() {
    let _g = lock();
    task_store().clear();
    three_tabs();
    let mut panel = TaskPanelComponent::new();
    assert_golden!(
        "task_panel_tabs_unfocused_80",
        render_golden(&mut panel, 80)
    );
}

#[test]
fn flight_plan_80() {
    let _g = lock();
    AppKeybindingsManager::default().install();
    task_store().clear();
    three_tabs();
    let mut panel = TaskPanelComponent::new();
    panel.set_focused(true);
    assert_golden!("flight_plan_80", render_golden(&mut panel, 80));
}

#[test]
fn flight_plan_100() {
    let _g = lock();
    AppKeybindingsManager::default().install();
    task_store().clear();
    three_tabs();
    let mut panel = TaskPanelComponent::new();
    panel.set_focused(true);
    assert_golden!("flight_plan_100", render_golden(&mut panel, 100));
}

#[test]
fn flight_plan_agents_80() {
    let _g = lock();
    AppKeybindingsManager::default().install();
    task_store().clear();
    three_tabs();
    let mut panel = TaskPanelComponent::new();
    panel.set_view(TaskPanelView::Subagents);
    panel.set_focused(true);
    assert_golden!("flight_plan_agents_80", render_golden(&mut panel, 80));
}

// ---- task panel: Shell tab (T5.3) ----

/// Three background jobs with no timed state: one Done, one Failed with its
/// exit status, one killed.
fn shell_jobs() {
    let shell = |key: &str| CreateTaskOptions {
        source: Some(TaskSource::Shell),
        key: Some(key.into()),
        ..Default::default()
    };
    let done = task_store()
        .create("cargo build --release", shell("shell:job-1"))
        .id;
    set_done(done);
    let failed = task_store()
        .create("cargo test -p nope", shell("shell:job-2"))
        .id;
    task_store().update(
        failed,
        TaskPatch {
            status: Some(TaskStatus::Failed),
            note: Some(Some("exit 101".into())),
            ..Default::default()
        },
    );
    let killed = task_store().create("sleep 600", shell("shell:job-3")).id;
    task_store().update(
        killed,
        TaskPatch {
            status: Some(TaskStatus::Cancelled),
            ..Default::default()
        },
    );
}

#[test]
fn flight_plan_shell_80() {
    let _g = lock();
    AppKeybindingsManager::default().install();
    task_store().clear();
    shell_jobs();
    let mut panel = TaskPanelComponent::new();
    panel.set_view(TaskPanelView::Shell);
    panel.set_focused(true);
    assert_golden!("flight_plan_shell_80", render_golden(&mut panel, 80));
}

#[test]
fn task_panel_tabs_with_shell_unfocused_80() {
    let _g = lock();
    AppKeybindingsManager::default().install();
    task_store().clear();
    three_tabs();
    shell_jobs();
    let mut panel = TaskPanelComponent::new();
    assert_golden!(
        "task_panel_tabs_with_shell_unfocused_80",
        render_golden(&mut panel, 80)
    );
}

#[test]
fn task_panel_empty_tabs_hidden_80() {
    let _g = lock();
    task_store().clear();
    add_task("Read the migration plan", TaskStatus::Done);
    let call = task_store()
        .create(
            "Fetch the spec",
            CreateTaskOptions {
                source: Some(TaskSource::Mcp),
                subagent_mode: Some("web".into()),
                ..Default::default()
            },
        )
        .id;
    set_done(call);
    // No subagent run: the Agents tab is not in the strip.
    let mut panel = TaskPanelComponent::new();
    assert_golden!(
        "task_panel_empty_tabs_hidden_80",
        render_golden(&mut panel, 80)
    );
}

// ---- Shell (bash) tool block ----

#[test]
fn shell_running_80() {
    let _g = lock();
    let mut block = tool("Shell", shell_args());
    block.update_result(result(&output_lines(2), false, json!({})), true);
    assert_golden!("shell_running_80", render_golden(&mut block, 80));
}

#[test]
fn shell_success_80() {
    let _g = lock();
    let mut block = tool("Shell", shell_args());
    block.update_result(
        result("test result: ok. 12 passed; 0 failed", false, json!({})),
        false,
    );
    assert_golden!("shell_success_80", render_golden(&mut block, 80));
}

#[test]
fn shell_success_120() {
    let _g = lock();
    let mut block = tool("Shell", shell_args());
    block.update_result(
        result("test result: ok. 12 passed; 0 failed", false, json!({})),
        false,
    );
    assert_golden!("shell_success_120", render_golden(&mut block, 120));
}

#[test]
fn shell_error_80() {
    let _g = lock();
    let mut block = tool("Shell", shell_args());
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
    let mut block = tool("Shell", shell_args());
    block.update_result(result(&output_lines(12), false, json!({})), false);
    assert_golden!("shell_output_collapsed_80", render_golden(&mut block, 80));
}

#[test]
fn shell_output_peek_80() {
    let _g = lock();
    let mut block = tool("Shell", shell_args());
    block.update_result(result(&output_lines(12), false, json!({})), false);
    assert_golden!("shell_output_peek_80", render_golden(&mut block, 80));
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
    let mut block = tool("Agent", agent_args());
    assert_golden!("agent_call_80", render_golden(&mut block, 80));
}

#[test]
fn agent_background_call_80() {
    let _g = lock();
    let mut args = agent_args();
    args["background"] = json!(true);
    let mut block = tool("Agent", args);
    assert_golden!("agent_background_call_80", render_golden(&mut block, 80));
}

#[test]
fn agent_result_80() {
    let _g = lock();
    let mut block = tool("Agent", agent_args());
    block.update_result(
        result("Found the footer in footer.rs.", false, json!({})),
        false,
    );
    assert_golden!("agent_result_80", render_golden(&mut block, 80));
}

#[test]
fn agent_result_120() {
    let _g = lock();
    let mut block = tool("Agent", agent_args());
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
    let mut block = tool("AgentOutput", json!({ "task_id": "explore#1" }));
    block.update_result(
        agent_output("Footer is in footer.rs, 340 lines.", "done"),
        false,
    );
    assert_golden!("agent_output_done_80", render_golden(&mut block, 80));
}

#[test]
fn agent_output_running_80() {
    let _g = lock();
    let mut block = tool("AgentOutput", json!({ "task_id": "explore#1" }));
    block.update_result(agent_output("Still reading files.", "running"), true);
    assert_golden!("agent_output_running_80", render_golden(&mut block, 80));
}

#[test]
fn agent_output_failed_80() {
    let _g = lock();
    let mut block = tool("AgentOutput", json!({ "task_id": "explore#1" }));
    block.update_result(
        agent_output("Stopped: tool limit reached.", "failed"),
        false,
    );
    assert_golden!("agent_output_failed_80", render_golden(&mut block, 80));
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

// ---- The turn: user message, failed call (peek) ----

#[test]
fn user_message_band_80() {
    let _g = lock();
    let mut header = MissionHeader::new("how does wrapping work?");
    assert_golden!("user_message_band_80", render_golden(&mut header, 80));
}

#[test]
fn user_message_band_wrapped_40() {
    // Wrapped and later lines start under the text, at column 3.
    let _g = lock();
    let mut header = MissionHeader::new(
        "check wrap.rs for the overflow in the second branch\nthen run the tests",
    );
    assert_golden!(
        "user_message_band_wrapped_40",
        render_golden(&mut header, 40)
    );
}

#[test]
fn peek_failed_call_80() {
    // Single view: tool rows with no step number and no rail; the failed
    // Shell reads red `⊗` and its error lines.
    let _g = lock();
    let mut chain = ToolChainComponent::new();
    let mut read = tool("Read", json!({ "path": "src/wrap.rs" }));
    read.update_result(result("line one\nline two", false, json!({})), false);
    chain.add(Rc::new(RefCell::new(read)));
    let mut shell = tool("Shell", shell_args());
    shell.update_result(
        result(
            "boom\nCommand exited with code 3",
            true,
            json!({ "durationMs": 100.0 }),
        ),
        false,
    );
    chain.add(Rc::new(RefCell::new(shell)));
    assert_golden!("peek_failed_call_80", render_golden(&mut chain, 80));
}

// ---- Assistant message outside a mission ----

#[test]
fn outline_hides_thinking_outside_a_mission() {
    // Outline draws no thinking, in or out of a mission (2026-10-31).
    let _g = lock();
    let mut msg = AssistantMessageComponent::new(None, ThinkingDisplay::Omit);
    msg.update_content(
        &assistant(vec![
            thinking("weighing the options"),
            Content::text("here is the answer"),
        ]),
        true,
    );
    msg.set_outline_prose_shown(true);
    let drawn = hoocode_tui_render::Component::render(&mut msg, 80).join("\n");
    assert!(
        !drawn.contains('✻') && !drawn.contains("weighing"),
        "{drawn}"
    );
    assert!(drawn.contains("here is the answer"), "{drawn}");
}
#[test]
fn turn_end_failed_80() {
    // Single view: a failed turn's end line; tokens and cost are the
    // status line's.
    let _g = lock();
    let stats = Rc::new(RefCell::new(FlightStats {
        mark: FlightMark::DidNotLand,
        steps: 5,
        hits: 1,
        landed_time: Some("4.2s".to_string()),
        tokens: Some("1.2k tokens".to_string()),
        cost: Some("$0.012".to_string()),
        failure: Some("connection reset by peer".to_string()),
        ..Default::default()
    }));
    let mut landing = LandingLine::new(stats);
    assert_golden!("turn_end_failed_80", render_golden(&mut landing, 80));
}

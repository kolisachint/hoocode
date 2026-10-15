//! T3.2: an Agent tool block shows the last five rows of its run's activity
//! while the run is going, and its result once the run settles. The run is the
//! one the call's partial names, or the one the call dispatched (a background
//! call's placeholder names none).

use hoocode_ai_types::Content;
use hoocode_code_subagents::live::{apply, remember_run};
use hoocode_code_tui_widgets::tool_execution::{ToolExecutionComponent, ToolExecutionOptions};
use hoocode_code_tui_widgets::tool_output_view::ToolOutputView;
use hoocode_code_tui_widgets::tool_signal::ToolResult;
use hoocode_code_tui_widgets::tools::registered_tool_definition;
use hoocode_tui_render::golden::render_golden;
use hoocode_tui_render::{assert_golden, Component};
use serde_json::{json, Value};

use crate::support::{lock, strip_all};

const CWD: &str = "/work/project";
const RUN: &str = "widget-live-run-1";
const CALL: &str = "call-live-tail";

fn agent_block() -> ToolExecutionComponent {
    let args = json!({
        "subagent_type": "explore",
        "description": "Find the footer code",
        "prompt": "Locate the footer renderer and list its inputs.",
    });
    ToolExecutionComponent::new(
        "Agent",
        CALL,
        args,
        ToolExecutionOptions {
            view: ToolOutputView::Peek,
            ..Default::default()
        },
        Some(registered_tool_definition("Agent")),
        CWD,
    )
}

fn event(value: Value) {
    apply(RUN, &value);
}

fn tool_start(tool: &str, key: &str, subject: &str) {
    event(json!({
        "type": "tool_execution_start",
        "toolName": tool,
        "args": {key: subject},
    }));
}

fn text(delta: &str) {
    event(json!({
        "type": "message_update",
        "assistantMessageEvent": {"type": "text_delta", "delta": delta},
    }));
}

/// A run that has started and done some work: more rows than the tail shows.
fn start_run() {
    event(json!({"type": "task_started"}));
    tool_start(
        "Read",
        "file_path",
        "crates/hoocode-code-tui-widgets/src/footer.rs",
    );
    tool_start("Grep", "pattern", "fn render");
    tool_start(
        "Read",
        "file_path",
        "crates/hoocode-code-tui-widgets/src/status.rs",
    );
    text("The footer is built in footer.rs.\nIt draws the model");
    tool_start(
        "Read",
        "file_path",
        "crates/hoocode-code-tui-widgets/src/chip.rs",
    );
}

/// The block's result slot while the run goes: a partial naming the run.
fn running(block: &mut ToolExecutionComponent) {
    block.update_result(
        ToolResult {
            content: Vec::new(),
            details: json!({"poolTaskId": RUN, "subagent_type": "explore", "status": "running"}),
            is_error: false,
        },
        true,
    );
}

fn lines(block: &mut ToolExecutionComponent) -> String {
    strip_all(&block.render(80))
}

#[test]
fn a_running_agent_block_shows_the_last_five_rows_of_its_run() {
    let _g = lock();
    start_run();
    let mut block = agent_block();
    running(&mut block);
    let shown = lines(&mut block);
    assert!(
        shown.contains("→ Read crates/hoocode-code-tui-widgets/src/status.rs"),
        "{shown}"
    );
    assert!(shown.contains("It draws the model"), "{shown}");
    assert!(
        !shown.contains("→ Read crates/hoocode-code-tui-widgets/src/footer.rs"),
        "{shown}"
    );
    assert_golden!("agent_live_tail_80", render_golden(&mut block, 80));
    event(json!({"type": "task_done"}));
}

#[test]
fn a_settled_run_shows_its_result_in_place_of_the_tail() {
    let _g = lock();
    start_run();
    let mut block = agent_block();
    running(&mut block);
    event(json!({"type": "task_done"}));
    block.refresh();
    block.update_result(
        ToolResult {
            content: vec![Content::text("Found the footer in footer.rs.")],
            details: json!({"poolTaskId": RUN, "status": "done"}),
            is_error: false,
        },
        false,
    );
    let shown = lines(&mut block);
    assert!(shown.contains("Found the footer in footer.rs."), "{shown}");
    assert!(!shown.contains("→ Grep"), "{shown}");
}

#[test]
fn a_block_of_a_live_run_is_not_frozen_until_the_run_settles() {
    let _g = lock();
    start_run();
    let mut block = agent_block();
    // A background placeholder (not partial) that names its run through the call.
    remember_run(CALL, RUN);
    block.update_result(
        ToolResult {
            content: vec![Content::text("Started in the background.")],
            details: json!({"background": true, "status": "running"}),
            is_error: false,
        },
        false,
    );
    block.refresh();
    assert!(!block.is_freezable(), "a live run keeps its block unfrozen");
    event(json!({"type": "task_done"}));
    block.refresh();
    assert!(block.is_freezable(), "a settled run's block may freeze");
}

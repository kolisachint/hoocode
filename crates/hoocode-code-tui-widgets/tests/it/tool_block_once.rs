//! A Write block prints its content once, and a TodoWrite block is one header
//! row: the plan itself shows in the task panel.

use crate::support::{lock, strip};
use hoocode_ai_types::Content;
use hoocode_code_tui_widgets::tool_execution::{ToolExecutionComponent, ToolExecutionOptions};
use hoocode_code_tui_widgets::tool_output_view::ToolOutputView;
use hoocode_code_tui_widgets::tool_signal::ToolResult;
use hoocode_code_tui_widgets::tools::{builtin_tool_definition, registered_tool_definition};
use hoocode_tui_render::Component;
use serde_json::{json, Value};

fn block(name: &str, args: Value) -> ToolExecutionComponent {
    let definition =
        builtin_tool_definition(name).unwrap_or_else(|| registered_tool_definition(name));
    ToolExecutionComponent::new(
        name,
        "call-1",
        args,
        ToolExecutionOptions {
            view: ToolOutputView::Peek,
            ..Default::default()
        },
        Some(definition),
        "/work/project",
    )
}

fn result(text: &str, is_error: bool) -> ToolResult {
    ToolResult {
        content: vec![Content::text(text)],
        details: json!({}),
        is_error,
    }
}

/// The rendered block with blank rows dropped (the block leads with a gap).
fn rows(b: &mut ToolExecutionComponent) -> Vec<String> {
    b.render(80)
        .iter()
        .map(|l| strip(l))
        .filter(|l| !l.trim().is_empty())
        .collect()
}

#[test]
fn write_previews_the_content_while_the_args_stream() {
    let _g = lock();
    let mut b = block(
        "Write",
        json!({"path": "notes.txt", "content": "alpha\nbeta\n"}),
    );
    let text = rows(&mut b).join("\n");
    assert_eq!(text.matches("alpha").count(), 1, "{text}");
}

#[test]
fn write_prints_its_content_once_after_the_result() {
    let _g = lock();
    let mut b = block(
        "Write",
        json!({"path": "notes.txt", "content": "alpha\nbeta\n"}),
    );
    b.set_args_complete();
    b.update_result(result("Successfully wrote 11 bytes", false), false);
    let text = rows(&mut b).join("\n");
    assert_eq!(text.matches("alpha").count(), 1, "{text}");
    assert_eq!(text.matches("beta").count(), 1, "{text}");
}

#[test]
fn write_is_a_header_alone_once_its_args_are_complete() {
    let _g = lock();
    let mut b = block(
        "Write",
        json!({"path": "notes.txt", "content": "alpha\nbeta\n"}),
    );
    b.set_args_complete();
    let lines = rows(&mut b);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("Write"), "{lines:?}");
    assert!(!lines[0].contains("alpha"), "{lines:?}");
}

#[test]
fn todo_write_is_one_header_row_without_a_list_body() {
    let _g = lock();
    let mut b = block(
        "TodoWrite",
        json!({"todos": [
            {"content": "Read the parser", "status": "completed"},
            {"content": "Add tests", "status": "in_progress", "activeForm": "Adding tests"},
            {"content": "Ship it", "status": "pending"},
        ]}),
    );
    b.set_args_complete();
    b.update_result(
        result(
            "Todos updated (1 in progress, 1 pending, 1 completed):\n[x] Read the parser\n[~] Adding tests\n[ ] Ship it",
            false,
        ),
        false,
    );
    let lines = rows(&mut b);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("TodoWrite"), "{lines:?}");
    assert!(lines[0].contains("3 items · 1 done"), "{lines:?}");
    assert!(!lines.iter().any(|l| l.contains("Ship it")), "{lines:?}");
}

#[test]
fn todo_write_failure_shows_its_reason() {
    let _g = lock();
    let mut b = block("TodoWrite", json!({"todos": []}));
    b.set_args_complete();
    b.update_result(result("todo status must be pending", true), false);
    let lines = rows(&mut b);
    assert!(
        lines
            .iter()
            .any(|l| l.contains("todo status must be pending")),
        "{lines:?}"
    );
}

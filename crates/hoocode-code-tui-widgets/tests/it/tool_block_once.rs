//! A Write block prints its content once, and a TodoWrite block is one header
//! row: the plan itself shows in the task panel.

use crate::support::{lock, strip};
use hoocode_ai_types::Content;
use hoocode_code_tui_widgets::tool_execution::{ToolExecutionComponent, ToolExecutionOptions};
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
        ToolExecutionOptions::default(),
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
    // Single view: the content opens on the toggle.
    b.set_expanded(true);
    let text = rows(&mut b).join("\n");
    assert_eq!(text.matches("alpha").count(), 1, "{text}");
    assert_eq!(text.matches("beta").count(), 1, "{text}");
}

#[test]
fn write_keeps_its_body_while_no_result_replaces_it() {
    // A call cut short (quit or crash mid-tool, a replayed session) has no
    // result: its content is the only copy there is.
    let _g = lock();
    let mut b = block(
        "Write",
        json!({"path": "notes.txt", "content": "alpha\nbeta\n"}),
    );
    b.set_args_complete();
    let text = rows(&mut b).join("\n");
    assert_eq!(text.matches("alpha").count(), 1, "{text}");
    assert_eq!(text.matches("beta").count(), 1, "{text}");
}

#[test]
fn write_with_an_error_result_shows_the_error_not_the_body() {
    // Live abort or error: the error is the single copy once it exists.
    let _g = lock();
    let mut b = block(
        "Write",
        json!({"path": "notes.txt", "content": "alpha\nbeta\n"}),
    );
    b.update_result(result("Operation aborted", true), false);
    b.set_expanded(true);
    let text = rows(&mut b).join("\n");
    assert_eq!(text.matches("Operation aborted").count(), 1, "{text}");
    assert!(!text.contains("alpha"), "{text}");
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
    b.set_expanded(true);
    let lines = rows(&mut b);
    assert!(
        lines
            .iter()
            .any(|l| l.contains("todo status must be pending")),
        "{lines:?}"
    );
}

#[test]
fn todo_write_failure_shows_no_counts_from_its_args() {
    let _g = lock();
    let mut b = block(
        "TodoWrite",
        json!({"todos": [
            {"content": "Read the parser", "status": "completed"},
            {"content": "Add tests", "status": "pending"},
        ]}),
    );
    b.set_args_complete();
    b.update_result(result("todo status must be pending", true), false);
    let lines = rows(&mut b);
    assert!(lines[0].contains("TodoWrite"), "{lines:?}");
    assert!(!lines.iter().any(|l| l.contains("items")), "{lines:?}");
}

#[test]
fn pending_write_preview_matches_the_finished_one_when_open() {
    // A 12-line file: open, the pending preview and the finished body show
    // the same whole content. No blank row ahead of the content.
    let _g = lock();
    let content = "import sys\n\n\ndef main(args: list[str]) -> int:\n    \"\"\"Print the arguments.\"\"\"\n    for a in args:\n        print(f\"arg: {a}\")  # echo\n    return 0\n\n\nif __name__ == \"__main__\":\n    sys.exit(main(sys.argv[1:]))\n";
    let args = json!({"path": "tools/util.py", "content": content});

    let mut pending = block("Write", args.clone());
    pending.set_args_complete();
    pending.set_expanded(true);
    let pending_rows = rows(&mut pending);

    let mut finished = block("Write", args);
    finished.set_args_complete();
    finished.update_result(result("Successfully wrote 12 lines", false), false);
    finished.set_expanded(true);
    let finished_rows = rows(&mut finished);

    assert_eq!(pending_rows, finished_rows, "preview matches the body");
    assert!(
        strip(&pending_rows[1]).contains("import sys"),
        "{pending_rows:?}"
    );
}

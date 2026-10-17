//! A rebuilt transcript has no clock, so the `Took` row of a finished Shell
//! call reads the duration stored on its result (`details.durationMs`).

use crate::support::{lock, strip};
use hoocode_code_tui_widgets::tool_execution::{
    ToolRenderContext, ToolRenderResultOptions, ToolResultView,
};
use hoocode_code_tui_widgets::tool_signal::ResultText;
use hoocode_code_tui_widgets::tools::builtin_tool_definition;
use serde_json::{json, Value};

fn took_line(details: &Value, is_partial: bool) -> Option<String> {
    let _g = lock();
    let def = builtin_tool_definition("Shell").expect("bash renderer");
    let args = json!({"command": "cargo test"});
    let mut state = serde_json::Map::new();
    let mut objects = std::collections::HashMap::new();
    let mut ctx = ToolRenderContext {
        args: &args,
        tool_call_id: "t",
        last_component: None,
        state: &mut state,
        objects: &mut objects,
        cwd: "/work",
        execution_started: true,
        args_complete: true,
        is_partial,
        expanded: false,
        show_images: false,
        is_error: false,
        has_result: true,
    };
    let content = vec![hoocode_ai_types::Content::text("ok")];
    let text = ResultText::default();
    let view = ToolResultView {
        content: &content,
        details,
        text: &text,
    };
    let options = ToolRenderResultOptions {
        expanded: false,
        is_partial,
    };
    let render = def.render_result.as_ref().expect("bash result slot");
    let body = render(&view, options, &mut ctx).unwrap();
    let lines = body.borrow_mut().render(80);
    lines
        .iter()
        .map(|l| strip(l))
        .find(|l| l.trim_start().starts_with("Took ") || l.trim_start().starts_with("Elapsed "))
        .map(|l| l.trim_end().to_string())
}

#[test]
fn rebuilt_shell_result_shows_the_stored_duration() {
    let took = took_line(&json!({"durationMs": 1234}), false);
    assert_eq!(took.as_deref(), Some("Took 1.2s"));
}

#[test]
fn result_without_a_stored_duration_shows_no_took_row() {
    assert_eq!(took_line(&Value::Null, false), None);
}

#[test]
fn a_running_call_still_ticks_on_the_clock_not_the_stored_value() {
    // Partial output has no `Took` from stored data: the live clock owns it.
    assert_eq!(took_line(&json!({"durationMs": 1234}), true), None);
}

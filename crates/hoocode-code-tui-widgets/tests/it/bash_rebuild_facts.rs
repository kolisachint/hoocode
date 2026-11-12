//! A rebuilt transcript has no clock, so the facts of a finished Shell row
//! (`exit 0 · 1 line · 1.2s`) read the duration stored on its result
//! (`details.durationMs`) plus the output size. The body carries no `Took` line.

use std::cell::RefCell;
use std::rc::Rc;

use crate::support::{lock, strip};
use hoocode_ai_types::Content;
use hoocode_code_tui_widgets::tool_chain::{ChainState, ToolChainComponent};
use hoocode_code_tui_widgets::tool_execution::{ToolExecutionComponent, ToolExecutionOptions};
use hoocode_code_tui_widgets::tool_signal::ToolResult;
use hoocode_tui_render::Component;
use serde_json::{json, Value};

/// The plain rows of one settled Shell call in a peek chain.
fn rows(details: Value, is_partial: bool, is_error: bool, output: &str) -> Vec<String> {
    let mut block = ToolExecutionComponent::new(
        "Shell",
        "t",
        json!({"command": "cargo test"}),
        ToolExecutionOptions::default(),
        None,
        "/work",
    );
    block.update_result(
        ToolResult {
            content: vec![Content::text(output)],
            details,
            is_error,
        },
        is_partial,
    );
    let mut chain = ToolChainComponent::new();
    let handle = Rc::new(RefCell::new(block));
    // Single view: bodies open on the toggle; the running row stays shut.
    if !is_partial {
        handle.borrow_mut().set_expanded(true);
    }
    chain.add(handle);
    chain.close(ChainState::Done);
    chain
        .render(80)
        .iter()
        .map(|l| strip(l).trim_end().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

#[test]
fn rebuilt_shell_row_shows_the_stored_duration_in_its_facts() {
    let _g = lock();
    let rows = rows(json!({"durationMs": 1234}), false, false, "ok");
    assert_eq!(
        rows[0], " ● Shell cargo test  exit 0 · 1 line · 1.2s ▾",
        "{rows:?}"
    );
    assert_eq!(rows[1], "    ok", "{rows:?}");
    assert!(!rows.iter().any(|r| r.contains("Took")), "{rows:?}");
}

#[test]
fn result_without_a_stored_duration_shows_the_exit_alone() {
    let _g = lock();
    let rows = rows(Value::Null, false, false, "ok");
    assert_eq!(
        rows[0], " ● Shell cargo test  exit 0 · 1 line ▾",
        "{rows:?}"
    );
}

#[test]
fn a_running_call_shows_no_facts_and_no_elapsed_row() {
    let _g = lock();
    // Partial output: the live clock lives in the status line, not the row.
    let rows = rows(json!({"durationMs": 1234}), true, false, "ok");
    assert_eq!(rows[0], " ◉ Shell cargo test ▸", "{rows:?}");
    assert!(!rows.iter().any(|r| r.contains("Elapsed")), "{rows:?}");
}

#[test]
fn a_failed_call_names_its_exit_code_and_leaves_the_exit_line_out_of_the_body() {
    let _g = lock();
    let rows = rows(
        json!({"durationMs": 3100}),
        false,
        true,
        "boom\n\nCommand exited with code 3",
    );
    assert_eq!(
        rows[0], " ⊗ Shell cargo test  exit 3 · boom · 3 lines · 3.1s ▾",
        "{rows:?}"
    );
    assert_eq!(rows[1], "    boom", "{rows:?}");
    assert_eq!(rows.len(), 2, "{rows:?}");
}

#[test]
fn a_failed_call_without_an_exit_code_shows_the_duration_alone() {
    let _g = lock();
    let rows = rows(
        json!({"durationMs": 1000}),
        false,
        true,
        "Command timed out after 1 seconds",
    );
    assert_eq!(
        rows[0], " ⊗ Shell cargo test  Command timed out after 1 seconds · 1 line · 1.0s ▾",
        "{rows:?}"
    );
}

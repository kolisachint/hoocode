//! Radar's delegated-work group row and AgentOutput's folded peek.

use std::cell::RefCell;
use std::rc::Rc;

use crate::support::{lock, strip};
use hoocode_ai_types::Content;
use hoocode_code_tui_widgets::tool_chain::{ToolBlock, ToolChainComponent};
use hoocode_code_tui_widgets::tool_chain_summary::ChainState;
use hoocode_code_tui_widgets::tool_execution::{ToolExecutionComponent, ToolExecutionOptions};
use hoocode_code_tui_widgets::tool_output_view::ToolOutputView;
use hoocode_code_tui_widgets::tool_signal::ToolResult;
use hoocode_code_tui_widgets::tools::registered_tool_definition;
use hoocode_tui_render::Component;
use serde_json::{json, Value};

use ToolOutputView::{Peek, Radar};

fn cwd() -> String {
    std::env::current_dir()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

/// One delegated call, as the transcript sees it once its result is in.
struct Sub {
    tool: &'static str,
    args: Value,
    text: &'static str,
    details: Value,
    is_error: bool,
}

fn agent_done() -> Sub {
    Sub {
        tool: "Agent",
        args: json!({"subagent_type": "explore"}),
        text: "Mapped the module.",
        details: json!({"ok": true}),
        is_error: false,
    }
}

fn agent_stopped() -> Sub {
    Sub {
        tool: "Agent",
        args: json!({"subagent_type": "explore"}),
        text: "Subagent (explore) cancelled by user.",
        details: json!({"ok": false}),
        is_error: true,
    }
}

/// A background dispatch: its placeholder names no task and no outcome.
fn background_placeholder() -> Sub {
    Sub {
        tool: "Agent",
        args: json!({"subagent_type": "explore", "background": true}),
        text: "Started explore in the background.",
        details: json!({"background": true, "status": "running"}),
        is_error: false,
    }
}

/// An `AgentOutput` poll of one task, settled with `status`.
fn poll(task: &'static str, status: &'static str) -> Sub {
    Sub {
        tool: "AgentOutput",
        args: json!({"task_id": task}),
        text: "Mapped the module.",
        details: json!({"task_id": task, "status": status}),
        is_error: false,
    }
}

fn block(view: ToolOutputView, index: usize, sub: Sub) -> ToolBlock {
    let mut component = ToolExecutionComponent::new(
        sub.tool,
        &format!("sub-{index}"),
        sub.args,
        ToolExecutionOptions {
            view,
            ..Default::default()
        },
        Some(registered_tool_definition(sub.tool)),
        &cwd(),
    );
    component.update_result(
        ToolResult {
            content: vec![Content::text(sub.text)],
            details: sub.details,
            is_error: sub.is_error,
        },
        false,
    );
    Rc::new(RefCell::new(component))
}

fn group(view: ToolOutputView, subs: Vec<Sub>, state: ChainState) -> ToolChainComponent {
    let mut chain = ToolChainComponent::new(view);
    for (i, sub) in subs.into_iter().enumerate() {
        chain.add(block(view, i, sub));
    }
    chain.close(state);
    chain
}

fn plain(lines: Vec<String>) -> Vec<String> {
    lines.iter().map(|l| strip(l)).collect()
}

/// The group row's line: the first non-blank row of a collapsed chain.
fn group_row(chain: &mut ToolChainComponent) -> String {
    plain(chain.render(120))
        .into_iter()
        .find(|l| !l.trim().is_empty())
        .unwrap_or_default()
}

// --- radar: the group row's outcome counts -------------------------------------

#[test]
fn radar_group_row_counts_outcomes_and_omits_zero_counts() {
    let _g = lock();
    let mut subs: Vec<Sub> = (0..10).map(|_| agent_done()).collect();
    subs.push(agent_stopped());
    let mut chain = group(Radar, subs, ChainState::Done);
    let row = group_row(&mut chain);
    assert!(row.contains("Delegated 11 tasks"), "{row:?}");
    assert!(row.contains("10 done · 1 stopped"), "{row:?}");
    assert!(!row.contains("failed"), "{row:?}");
    assert!(!row.contains("running"), "{row:?}");
}

#[test]
fn radar_group_row_shows_only_the_outcomes_that_happened() {
    let _g = lock();
    let mut chain = group(Radar, vec![agent_done(), agent_done()], ChainState::Done);
    let row = group_row(&mut chain);
    assert!(row.ends_with("2 done"), "{row:?}");
}

#[test]
fn radar_group_row_counts_each_task_once_however_often_it_was_polled() {
    let _g = lock();
    let subs = vec![
        poll("explore#1", "running"),
        poll("explore#1", "running"),
        poll("explore#1", "done"),
        poll("explore#2", "done"),
    ];
    let mut chain = group(Radar, subs, ChainState::Done);
    let row = group_row(&mut chain);
    assert!(row.contains("Delegated 2 tasks"), "{row:?}");
    assert!(row.ends_with("2 done"), "{row:?}");
}

#[test]
fn radar_group_row_falls_back_to_a_call_count_without_outcomes() {
    let _g = lock();
    let subs = vec![background_placeholder(), background_placeholder()];
    let mut chain = group(Radar, subs, ChainState::Done);
    let row = group_row(&mut chain);
    assert!(row.contains("Delegated"), "{row:?}");
    assert!(row.ends_with("2 calls"), "{row:?}");
}

// --- peek: AgentOutput's folded body ---------------------------------------------

/// One `AgentOutput` call, settled, in peek at `width`.
fn output_peek(text: &'static str, width: u16) -> Vec<String> {
    let sub = Sub {
        tool: "AgentOutput",
        args: json!({"task_id": "explore#1"}),
        text,
        details: json!({"task_id": "explore#1", "status": "done"}),
        is_error: false,
    };
    let component = block(Peek, 0, sub);
    let lines = component.borrow_mut().render(width);
    plain(lines)
}

/// The peek rows that carry the gutter. The content box indents every row
/// one column and pads it to the width, so both ends are trimmed.
fn gutter_rows(lines: &[String]) -> Vec<&str> {
    lines
        .iter()
        .map(|l| l.trim())
        .filter(|l| l.starts_with("│ "))
        .collect()
}

#[test]
fn agent_output_peek_shows_three_summary_rows_then_how_many_are_left() {
    let _g = lock();
    let body = "one\ntwo\nthree\nfour\nfive\nsix";
    let lines = output_peek(body, 60);
    assert!(
        lines
            .iter()
            .any(|l| l.contains("AgentOutput explore#1 · done")),
        "{lines:?}"
    );
    let rows = gutter_rows(&lines);
    assert_eq!(
        rows.len(),
        4,
        "three rows and the more-lines row: {lines:?}"
    );
    assert_eq!(rows[0], "│ one");
    assert_eq!(rows[2], "│ three");
    assert_eq!(rows[3], "│ … 3 more lines");
}

#[test]
fn agent_output_peek_guards_every_wrapped_row_with_the_gutter() {
    let _g = lock();
    let long = "a wrapped summary line that is long enough to need a second peek row\nnext";
    let lines = output_peek(long, 40);
    let rows = gutter_rows(&lines);
    // Three peek rows (two of them the wrapped line), then the more-lines row.
    assert_eq!(rows.len(), 4, "{lines:?}");
    assert_eq!(rows[0], "│ a wrapped summary line that is");
    assert_eq!(
        rows[1], "│ long enough to need a second peek",
        "the continuation row keeps the gutter: {lines:?}"
    );
    assert_eq!(rows[3], "│ … 1 more line", "{lines:?}");
}

#[test]
fn agent_output_peek_strips_markdown_from_the_summary() {
    let _g = lock();
    let lines = output_peek("# Findings\n- **Mapped** the `module`\nlast", 60);
    let joined = lines.join("\n");
    assert!(joined.contains("│ Findings"), "{lines:?}");
    assert!(joined.contains("│ Mapped the module"), "{lines:?}");
    assert!(!joined.contains("**"), "{lines:?}");
    assert!(!joined.contains('`'), "{lines:?}");
    assert!(!joined.contains("# "), "{lines:?}");
    assert!(!joined.contains("- "), "{lines:?}");
}

#[test]
fn agent_output_peek_is_unboxed() {
    let _g = lock();
    let lines = output_peek("one\ntwo", 60);
    let joined = lines.join("\n");
    assert!(!joined.contains('╭'), "{lines:?}");
    assert!(!joined.contains('╰'), "{lines:?}");
}

//! Outline's delegated-work rows (their hits) and AgentOutput's folded peek.

use std::cell::RefCell;
use std::rc::Rc;

use crate::support::{lock, strip};
use hoocode_ai_types::Content;
use hoocode_code_tui_widgets::outline::{turn_rows, FlightItem, FlightMark, FlightStats};
use hoocode_code_tui_widgets::tool_chain::{ToolBlock, ToolChainComponent};
use hoocode_code_tui_widgets::tool_chain_summary::ChainState;
use hoocode_code_tui_widgets::tool_execution::{ToolExecutionComponent, ToolExecutionOptions};
use hoocode_code_tui_widgets::tool_output_view::ToolOutputView;
use hoocode_code_tui_widgets::tool_signal::ToolResult;
use hoocode_code_tui_widgets::tools::registered_tool_definition;
use hoocode_tui_ratatui::Palette;
use hoocode_tui_render::Component;
use serde_json::{json, Value};

use ToolOutputView::{Outline, Peek};

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

// --- outline: a settled subagent folds into its recap; only a failed one keeps
// --- its own row -------------------------------------------------------------

/// The outline rows of one turn that holds the chain's calls as one batch,
/// keeping only the failed calls' rows.
fn outline_hits(chain: &ToolChainComponent) -> Vec<String> {
    let stats = FlightStats {
        mark: FlightMark::Landed,
        landed_time: Some("1s".to_string()),
        items: vec![FlightItem::Batch(chain.tool_blocks().to_vec())],
        ..Default::default()
    };
    plain(turn_rows("delegate", &stats, &Palette::default(), 120))
        .into_iter()
        .filter(|l| l.trim_start().starts_with('⊗'))
        .collect()
}

#[test]
fn outline_keeps_only_the_failed_subagent_as_its_own_row() {
    let _g = lock();
    let mut subs: Vec<Sub> = (0..10).map(|_| agent_done()).collect();
    subs.push(agent_stopped());
    let chain = group(Outline, subs, ChainState::Done);
    // Ten settled subagents fold into the recap; the stopped one keeps its own row.
    let hits = outline_hits(&chain);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(hits[0].starts_with(" ⊗ Agent"), "{hits:?}");
}

#[test]
fn outline_shows_no_failed_row_when_the_subagents_outcomes_all_happened() {
    let _g = lock();
    let chain = group(Outline, vec![agent_done(), agent_done()], ChainState::Done);
    assert!(outline_hits(&chain).is_empty());
}

#[test]
fn outline_counts_each_task_once_however_often_it_was_polled() {
    let _g = lock();
    let subs = vec![
        poll("explore#1", "running"),
        poll("explore#1", "running"),
        poll("explore#1", "done"),
        poll("explore#2", "done"),
    ];
    let chain = group(Outline, subs, ChainState::Done);
    assert!(outline_hits(&chain).is_empty());
}

#[test]
fn outline_has_no_failed_row_for_a_background_dispatch_without_an_outcome() {
    let _g = lock();
    let subs = vec![background_placeholder(), background_placeholder()];
    let chain = group(Outline, subs, ChainState::Done);
    assert!(outline_hits(&chain).is_empty());
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
    // Single view: the body opens on the toggle.
    component.borrow_mut().set_expanded(true);
    let lines = component.borrow_mut().render(width);
    plain(lines)
}

/// The body rows: dim text at column 4, no gutter. Rows are padded to the
/// width, so the right end is trimmed.
fn body_rows(lines: &[String]) -> Vec<&str> {
    lines
        .iter()
        .filter(|l| l.starts_with("    "))
        .map(|l| l.trim())
        .collect()
}

#[test]
fn agent_output_open_shows_every_summary_row_whole() {
    let _g = lock();
    let body = "one\ntwo\nthree\nfour\nfive\nsix";
    let lines = output_peek(body, 60);
    assert!(
        lines
            .iter()
            .any(|l| l.contains("AgentOutput explore#1 · done")),
        "{lines:?}"
    );
    let rows = body_rows(&lines);
    assert_eq!(rows.len(), 6, "every row, no fold: {lines:?}");
    assert_eq!(rows[0], "one");
    assert_eq!(rows[5], "six");
}

#[test]
fn agent_output_peek_keeps_every_wrapped_row_at_the_body_column() {
    let _g = lock();
    let long = "a wrapped summary line that is long enough to need a second peek row\nnext";
    let lines = output_peek(long, 40);
    let rows = body_rows(&lines);
    // The wrapped line takes two rows, then the next line: three rows in all.
    assert_eq!(rows.len(), 3, "{lines:?}");
    assert_eq!(rows[0], "a wrapped summary line that is long");
    assert_eq!(
        rows[1], "enough to need a second peek row",
        "the continuation row keeps the column: {lines:?}"
    );
    assert_eq!(rows[2], "next", "{lines:?}");
}

#[test]
fn agent_output_peek_strips_markdown_from_the_summary() {
    let _g = lock();
    let lines = output_peek("# Findings\n- **Mapped** the `module`\nlast", 60);
    let joined = lines.join("\n");
    assert!(joined.contains("    Findings"), "{lines:?}");
    assert!(joined.contains("    Mapped the module"), "{lines:?}");
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

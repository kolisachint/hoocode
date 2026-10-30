//! Radar's delegated-work rows (their hits) and AgentOutput's folded peek.

use std::cell::RefCell;
use std::rc::Rc;

use crate::support::{lock, strip};
use hoocode_ai_types::Content;
use hoocode_code_tui_widgets::radar::{mission_rows, FlightItem, FlightMark, FlightStats};
use hoocode_code_tui_widgets::tool_chain::{ToolBlock, ToolChainComponent};
use hoocode_code_tui_widgets::tool_chain_summary::ChainState;
use hoocode_code_tui_widgets::tool_execution::{ToolExecutionComponent, ToolExecutionOptions};
use hoocode_code_tui_widgets::tool_output_view::ToolOutputView;
use hoocode_code_tui_widgets::tool_signal::ToolResult;
use hoocode_code_tui_widgets::tools::registered_tool_definition;
use hoocode_tui_ratatui::Palette;
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

// --- radar: a settled subagent folds into its batch; only a failed one keeps
// --- its own hit row ---------------------------------------------------------

/// The radar rows of one mission that holds the chain's calls as one batch,
/// keeping only the rows that carry a hit.
fn radar_hits(chain: &ToolChainComponent) -> Vec<String> {
    let stats = FlightStats {
        mark: FlightMark::Landed,
        landed_time: Some("1s".to_string()),
        items: vec![FlightItem::Batch(chain.tool_blocks().to_vec())],
        ..Default::default()
    };
    plain(mission_rows(
        1,
        "delegate",
        &stats,
        &Palette::default(),
        120,
    ))
    .into_iter()
    .filter(|l| l.contains("HIT"))
    .collect()
}

#[test]
fn radar_keeps_only_the_failed_subagent_as_a_hit_row() {
    let _g = lock();
    let mut subs: Vec<Sub> = (0..10).map(|_| agent_done()).collect();
    subs.push(agent_stopped());
    let chain = group(Radar, subs, ChainState::Done);
    // Ten settled subagents fold into the batch line; the stopped one keeps a hit row.
    let hits = radar_hits(&chain);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(hits[0].starts_with("   ┊   └ 11 Agent"), "{hits:?}");
    assert!(hits[0].trim_end().ends_with("HIT"), "{hits:?}");
}

#[test]
fn radar_shows_no_hit_when_the_subagents_outcomes_all_happened() {
    let _g = lock();
    let chain = group(Radar, vec![agent_done(), agent_done()], ChainState::Done);
    assert!(radar_hits(&chain).is_empty());
}

#[test]
fn radar_counts_each_task_once_however_often_it_was_polled() {
    let _g = lock();
    let subs = vec![
        poll("explore#1", "running"),
        poll("explore#1", "running"),
        poll("explore#1", "done"),
        poll("explore#2", "done"),
    ];
    let chain = group(Radar, subs, ChainState::Done);
    assert!(radar_hits(&chain).is_empty());
}

#[test]
fn radar_has_no_hit_for_a_background_dispatch_without_an_outcome() {
    let _g = lock();
    let subs = vec![background_placeholder(), background_placeholder()];
    let chain = group(Radar, subs, ChainState::Done);
    assert!(radar_hits(&chain).is_empty());
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
    // Three peek rows (all from the wrapped line), then the more-lines row.
    assert_eq!(rows.len(), 4, "{lines:?}");
    assert_eq!(rows[0], "│ a wrapped summary line that is");
    assert_eq!(
        rows[1], "│ long enough to need a second",
        "the continuation row keeps the gutter: {lines:?}"
    );
    assert_eq!(
        rows[2], "│ peek row",
        "the second continuation row keeps the gutter: {lines:?}"
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

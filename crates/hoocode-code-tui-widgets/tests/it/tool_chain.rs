//! Port of the pin's `test/tool-chain.test.ts`.

use std::cell::RefCell;
use std::rc::Rc;

use crate::support::{lock, strip};
use hoocode_ai_types::Content;
use hoocode_code_tui_widgets::tool_chain::ToolChainComponent;
use hoocode_code_tui_widgets::tool_chain_summary::ChainState;
use hoocode_code_tui_widgets::tool_execution::{ToolExecutionComponent, ToolExecutionOptions};
use hoocode_code_tui_widgets::tool_output_view::ToolOutputView;
use hoocode_code_tui_widgets::tool_signal::ToolResult;
use hoocode_tui_render::Component;
use serde_json::{json, Value};

use ToolOutputView::{Peek, Radar};

struct Call {
    tool: &'static str,
    args: Value,
    out: &'static str,
    is_error: bool,
    pending: bool,
}

fn call(tool: &'static str, args: Value, out: &'static str) -> Call {
    Call {
        tool,
        args,
        out,
        is_error: false,
        pending: false,
    }
}

fn cwd() -> String {
    std::env::current_dir()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

fn chain_of(view: ToolOutputView, calls: Vec<Call>, id: &str) -> ToolChainComponent {
    let mut chain = ToolChainComponent::new(view);
    for (i, c) in calls.into_iter().enumerate() {
        // `createAllToolDefinitions(cwd)[tool]`: the built-in table.
        let mut block = ToolExecutionComponent::new(
            c.tool,
            &format!("{id}-{i}"),
            c.args,
            ToolExecutionOptions {
                view,
                ..Default::default()
            },
            None,
            &cwd(),
        );
        if !c.pending {
            block.update_result(
                ToolResult {
                    content: vec![Content::text(c.out)],
                    details: json!({}),
                    is_error: c.is_error,
                },
                false,
            );
        }
        chain.add(Rc::new(RefCell::new(block)));
    }
    chain
}

fn render(chain: &mut ToolChainComponent) -> String {
    strip(&chain.render(100).join("\n"))
}

fn run() -> Vec<Call> {
    let cwd = cwd();
    vec![
        call(
            "CodeSearch",
            json!({"query": "toolOutputView", "glob": "packages/**"}),
            "a\nb",
        ),
        call(
            "Read",
            json!({"file_path": format!("{cwd}/src/keys.ts")}),
            "x\ny\nz",
        ),
        call(
            "Edit",
            json!({"path": format!("{cwd}/src/keys.ts"), "edits": []}),
            "",
        ),
    ]
}

#[test]
fn collapses_a_run_of_calls_to_one_line_in_radar() {
    let _g = lock();
    let mut chain = chain_of(Radar, run(), "c");
    let out = render(&mut chain);
    let lines: Vec<&str> = out.split('\n').filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("CodeSearch › Read › Edit"));
}

#[test]
fn shows_the_shape_while_running_and_what_it_amounted_to_once_done() {
    let _g = lock();
    let mut calls = run();
    calls.push(Call {
        pending: true,
        ..call("Shell", json!({"command": "x"}), "")
    });
    let mut running = chain_of(Radar, calls, "r");
    let out = render(&mut running);
    assert!(out.contains("CodeSearch › Read › Edit › Shell"));
    assert!(out.contains("running"));

    let mut done = chain_of(Radar, run(), "d");
    done.close(ChainState::Done);
    let settled = render(&mut done);
    assert!(settled.contains("Edited"));
    assert!(!settled.contains("CodeSearch › Read"));
}

#[test]
fn an_interrupted_chain_keeps_the_shape_and_says_so_claiming_nothing() {
    let _g = lock();
    let mut chain = chain_of(Radar, run(), "i");
    chain.close(ChainState::Interrupted);
    let out = render(&mut chain);
    assert!(out.contains("CodeSearch › Read › Edit"));
    assert!(out.contains("interrupted"));
    assert!(!out.contains("Edited src"));
}

#[test]
fn a_collapsed_chain_still_shows_why_a_call_failed() {
    let _g = lock();
    let first = run().remove(0);
    let mut chain = chain_of(
        Radar,
        vec![
            first,
            Call {
                is_error: true,
                ..call("Shell", json!({"command": "bun test"}), "ASSERTION FAILED")
            },
        ],
        "e",
    );
    chain.close(ChainState::Done);
    let out = render(&mut chain);
    assert!(out.contains("1 failed"));
    assert!(out.contains("ASSERTION FAILED"));
}

#[test]
fn the_dial_not_a_per_chain_toggle_is_what_turns_the_line_back_into_calls() {
    let _g = lock();
    let mut chain = chain_of(Radar, run(), "o");
    chain.close(ChainState::Done);
    assert!(!render(&mut chain).contains("CodeSearch"));
    chain.set_view(Peek);
    let opened = render(&mut chain);
    assert!(opened.contains("CodeSearch"));
    assert!(opened.contains("Read"));
    assert!(opened.contains("Edit"));
}

#[test]
fn is_a_plain_pass_through_in_peek() {
    let _g = lock();
    let mut chain = chain_of(Peek, run(), "p");
    chain.close(ChainState::Done);
    let out = render(&mut chain);
    assert!(!out.contains('›'));
    assert!(out.contains("CodeSearch"));
}

#[test]
fn follows_the_dial_when_the_view_changes_under_it() {
    let _g = lock();
    let mut chain = chain_of(Peek, run(), "v");
    chain.close(ChainState::Done);
    assert!(!render(&mut chain).contains('›'));
    chain.set_view(Radar);
    assert!(render(&mut chain).contains("Edited"));
    chain.set_view(Peek);
    assert!(!render(&mut chain).contains('›'));
}

#[test]
fn every_tools_call_line_starts_in_the_same_column() {
    let _g = lock();
    let cwd = cwd();
    let mut chain = chain_of(
        Peek,
        vec![
            call("CodeSearch", json!({"query": "docs"}), "a"),
            call("Read", json!({"file_path": format!("{cwd}/a.ts")}), "a"),
            call(
                "Edit",
                json!({"path": format!("{cwd}/a.ts"), "edits": []}),
                "",
            ),
        ],
        "align",
    );
    chain.close(ChainState::Done);
    let out = render(&mut chain);
    let columns: std::collections::BTreeSet<usize> = out
        .split('\n')
        .filter(|l| l.contains('●'))
        .map(|l| l.chars().position(|c| c == '●').unwrap())
        .collect();
    assert_eq!(columns.len(), 1, "{out}");
}

#[test]
fn a_failures_reason_hangs_off_its_radar_row_rather_than_starting_a_new_column() {
    let _g = lock();
    let mut chain = chain_of(
        Radar,
        vec![Call {
            is_error: true,
            ..call("Shell", json!({"command": "bun test"}), "ASSERTION FAILED")
        }],
        "indent",
    );
    chain.close(ChainState::Done);
    let out = render(&mut chain);
    let lines: Vec<&str> = out.split('\n').collect();
    let row = lines.iter().position(|l| l.contains('●')).unwrap();
    let body = lines
        .iter()
        .position(|l| l.contains("ASSERTION FAILED"))
        .unwrap();
    assert!(body > row);
    let col = |s: &str, pat: &str| s.find(pat).map(|b| s[..b].chars().count()).unwrap();
    assert!(col(lines[body], "ASSERTION") > col(lines[row], "●"));
}

#[test]
fn a_chain_of_one_reads_like_a_run_in_radar() {
    let _g = lock();
    let first = run().remove(0);
    let mut chain = chain_of(Radar, vec![first], "one");
    chain.close(ChainState::Done);
    let out = render(&mut chain);
    assert!(out.contains("Searched toolOutputView"));
    assert!(out.contains("1 call · 2 lines"));
    assert!(!out.contains("CodeSearch"));
    assert!(chain.is_summarised());
}

#[test]
fn a_single_call_in_radar_uses_the_group_summary_row() {
    let _g = lock();
    let mut chain = chain_of(
        Radar,
        vec![call(
            "Shell",
            json!({"command": "npm run check"}),
            "l1\nl2\nl3",
        )],
        "single",
    );
    chain.close(ChainState::Done);
    let lines = chain.render(100);
    assert_eq!(lines[0], "");
    let row = strip(&lines[1]);
    assert!(row.starts_with(" ● Ran npm run check"), "{row:?}");
    assert!(row.ends_with("1 call · 3 lines"), "{row:?}");
    assert!(!row.contains("Shell"));
}

#[test]
fn a_running_single_call_in_radar_still_says_running() {
    let _g = lock();
    let mut chain = chain_of(
        Radar,
        vec![Call {
            pending: true,
            ..call("Shell", json!({"command": "npm run check"}), "")
        }],
        "running1",
    );
    let out = render(&mut chain);
    assert!(out.contains("Shell…"), "{out}");
    assert!(out.contains("running"), "{out}");
}

#[test]
fn a_failed_single_call_in_radar_still_shows_its_reason() {
    let _g = lock();
    let mut chain = chain_of(
        Radar,
        vec![Call {
            is_error: true,
            ..call("Shell", json!({"command": "bun test"}), "ASSERTION FAILED")
        }],
        "failed1",
    );
    chain.close(ChainState::Done);
    let out = render(&mut chain);
    assert!(out.contains("Ran bun test"), "{out}");
    assert!(out.contains("1 call · 1 failed"), "{out}");
    assert!(out.contains("ASSERTION FAILED"), "{out}");
}

#[test]
fn a_single_call_in_peek_is_still_its_own_block() {
    let _g = lock();
    let mut chain = chain_of(
        Peek,
        vec![call("Shell", json!({"command": "npm run check"}), "l1")],
        "peek1",
    );
    chain.close(ChainState::Done);
    let out = render(&mut chain);
    // Peek draws the call's own block: a Shell call reads as `$ command`.
    assert!(out.contains("$ npm run check"), "{out}");
    assert!(!out.contains("1 call"), "{out}");
    assert!(!chain.is_summarised());
}

#[test]
fn two_calls_are_still_worth_folding() {
    let _g = lock();
    let mut calls = run();
    calls.truncate(2);
    let mut chain = chain_of(Radar, calls, "two");
    assert!(chain.is_summarised());
    assert!(render(&mut chain).contains("CodeSearch › Read"));
}

#[test]
fn holds_a_radar_run_off_whatever_came_before_it() {
    let _g = lock();
    let mut chain = chain_of(Radar, run(), "lead");
    chain.close(ChainState::Done);
    let lines = chain.render(100);
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0], "");
    assert!(strip(&lines[1]).contains("Edited"));
}

#[test]
fn a_radar_run_of_one_gets_the_same_lead_in() {
    let _g = lock();
    let first = run().remove(0);
    let mut chain = chain_of(Radar, vec![first], "lead-one");
    chain.close(ChainState::Done);
    let lines = chain.render(100);
    assert_eq!(lines[0], "");
    assert!(strip(&lines[1]).contains("Searched toolOutputView"));
}

#[test]
fn does_not_double_the_gap_where_the_blocks_already_draw_one() {
    let _g = lock();
    let mut chain = chain_of(Peek, run(), "nolead");
    chain.close(ChainState::Done);
    let lines = chain.render(100);
    assert_eq!(lines.iter().take(2).filter(|l| l.is_empty()).count(), 1);
}

#[test]
fn reports_whether_it_is_still_collecting_calls() {
    let _g = lock();
    let mut chain = chain_of(Radar, run(), "s");
    assert!(chain.is_open());
    chain.close(ChainState::Done);
    assert!(!chain.is_open());
}

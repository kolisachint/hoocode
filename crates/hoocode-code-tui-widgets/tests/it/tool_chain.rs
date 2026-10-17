//! Port of the pin's `test/tool-chain.test.ts`.

use std::cell::RefCell;
use std::rc::Rc;

use crate::support::{lock, strip};
use hoocode_ai_types::{Content, ThinkingContent};
use hoocode_code_task_store::{task_store, CreateTaskOptions, TaskPatch, TaskSource, TaskStatus};
use hoocode_code_tui_widgets::tool_chain::{speaking_ends_chain, ToolChainComponent};
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
    // The last call still runs, so the line shows the run's shape.
    let mut calls = run();
    calls[2].pending = true;
    let mut chain = chain_of(Radar, calls, "c");
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
    // Mission Control peek: the step is numbered and sits on the `┊` rail —
    // ` 01 ● Shell  command`, then `   ┊  l1` — with no summary row.
    let rows: Vec<&str> = out.lines().map(str::trim_end).collect();
    assert!(
        rows.windows(2)
            .any(|w| w == [" 01 ● Shell  npm run check", "   ┊    l1"]),
        "{out}"
    );
    assert!(!out.contains("1 call"), "{out}");
    assert!(!chain.is_summarised());
}

#[test]
fn two_calls_are_still_worth_folding() {
    let _g = lock();
    let mut calls = run();
    calls.truncate(2);
    calls[1].pending = true;
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

fn background_shell() -> ToolExecutionComponent {
    let mut block = ToolExecutionComponent::new(
        "Shell",
        "bg-0",
        json!({"command": "npm run dev", "run_in_background": true}),
        ToolExecutionOptions {
            view: Radar,
            ..Default::default()
        },
        None,
        &cwd(),
    );
    block.update_result(
        ToolResult {
            content: vec![Content::text("Started background job j1")],
            details: json!({"jobId": "j1"}),
            is_error: false,
        },
        false,
    );
    block
}

#[test]
fn an_interrupted_chain_with_no_failure_is_cancelled_never_green() {
    let _g = lock();
    let mut chain = chain_of(Radar, run(), "cancel");
    chain.close(ChainState::Interrupted);
    let out = render(&mut chain);
    assert!(out.contains("⊘"), "{out}");
    assert!(!out.contains('●'), "{out}");
}

/// The job's row in the task store, as the Shell tool keeps it: `shell:<id>`,
/// in progress while the job runs. `None` removes it (a replayed history).
fn set_job_row(job: &str, status: Option<TaskStatus>) {
    let key = hoocode_code_tool_bash::jobs::row_key(job);
    task_store().remove_keyed(&key);
    if let Some(status) = status {
        task_store().upsert_keyed(
            &key,
            "npm run dev",
            CreateTaskOptions {
                source: Some(TaskSource::Shell),
                key: Some(key.clone()),
                ..Default::default()
            },
            TaskPatch {
                status: Some(status),
                ..Default::default()
            },
        );
    }
}

fn chain_with_job(state: ChainState) -> ToolChainComponent {
    let mut chain = ToolChainComponent::new(Radar);
    chain.add(Rc::new(RefCell::new(background_shell())));
    chain.close(state);
    chain
}

#[test]
fn a_background_call_reads_running_while_its_job_runs() {
    let _g = lock();
    set_job_row("j1", Some(TaskStatus::InProgress));
    let out = render(&mut chain_with_job(ChainState::Done));
    set_job_row("j1", None);
    assert!(out.contains("◐"), "{out}");
    assert!(!out.contains('●'), "{out}");
    assert!(out.contains("running"), "{out}");
}

#[test]
fn a_replayed_background_call_with_no_live_job_reads_done() {
    let _g = lock();
    set_job_row("j1", None);
    let out = render(&mut chain_with_job(ChainState::Done));
    assert!(out.contains("●"), "{out}");
    assert!(!out.contains('◐'), "{out}");
    assert!(!out.contains("running"), "{out}");
}

#[test]
fn an_aborted_chain_keeps_a_live_background_job_running() {
    let _g = lock();
    // Aborting the turn does not stop a job: the chain still reads running.
    set_job_row("j1", Some(TaskStatus::InProgress));
    let out = render(&mut chain_with_job(ChainState::Interrupted));
    set_job_row("j1", None);
    assert!(out.contains("◐"), "{out}");
    assert!(!out.contains('⊘'), "{out}");
}

#[test]
fn an_aborted_chain_whose_job_has_ended_reads_interrupted() {
    let _g = lock();
    set_job_row("j1", Some(TaskStatus::Done));
    let out = render(&mut chain_with_job(ChainState::Interrupted));
    set_job_row("j1", None);
    assert!(out.contains("⊘"), "{out}");
    assert!(!out.contains('◐'), "{out}");
}

#[test]
fn a_finished_shell_reads_done_as_soon_as_its_result_arrives() {
    let _g = lock();
    // The chain is still open: the model has not answered yet.
    let mut chain = chain_of(
        Radar,
        vec![call("Shell", json!({"command": "npm test"}), "ok")],
        "fin",
    );
    let out = render(&mut chain);
    assert!(out.contains("● Ran npm test"), "{out}");
    assert!(!out.contains("running"), "{out}");
    assert!(!out.contains('◐'), "{out}");
}

#[test]
fn a_failed_call_shows_the_last_two_lines_of_its_reason() {
    let _g = lock();
    let mut chain = chain_of(
        Radar,
        vec![Call {
            is_error: true,
            ..call("Shell", json!({"command": "make"}), "l1\nl2\nl3\nl4\nl5")
        }],
        "cap",
    );
    chain.close(ChainState::Done);
    let out = render(&mut chain);
    assert!(out.contains("l4"), "{out}");
    assert!(out.contains("l5"), "{out}");
    assert!(!out.contains("l3"), "{out}");
    assert!(out.contains("… 3 more lines · ctrl+o"), "{out}");
    let body = out.lines().find(|l| l.contains("l4")).unwrap();
    assert!(body.starts_with("      l4"), "{body:?}");
}

#[test]
fn a_failed_call_with_two_lines_or_fewer_shows_no_fold() {
    let _g = lock();
    let mut chain = chain_of(
        Radar,
        vec![Call {
            is_error: true,
            ..call("Shell", json!({"command": "make"}), "l1\nl2")
        }],
        "short",
    );
    chain.close(ChainState::Done);
    let out = render(&mut chain);
    assert!(out.contains("l1") && out.contains("l2"), "{out}");
    assert!(!out.contains("more line"), "{out}");
}

/// The stats are whole. A label that does not fit beside them is dropped;
/// the stats are cut only when they do not fit alone.
#[test]
fn a_label_that_does_not_fit_beside_the_stats_is_dropped() {
    let _g = lock();
    let long = format!("npm run check -- {}", "--flag-with-a-long-name ".repeat(6));
    let mut chain = chain_of(
        Radar,
        vec![call("Shell", json!({ "command": long }), "l1\nl2\nl3")],
        "drop",
    );
    chain.close(ChainState::Done);
    let row = strip(&chain.render(30)[1]);
    assert!(row.ends_with("1 call · 3 lines"), "{row:?}");
    assert!(!row.contains("npm"), "{row:?}");
    assert!(!row.contains('…'), "{row:?}");
    assert_eq!(hoocode_tui_util::visible_width(&row), 30, "{row:?}");
}

#[test]
fn stats_that_do_not_fit_alone_are_cut_last() {
    let _g = lock();
    let mut chain = chain_of(
        Radar,
        vec![call(
            "Shell",
            json!({ "command": "npm test" }),
            "l1\nl2\nl3",
        )],
        "cut",
    );
    chain.close(ChainState::Done);
    let lines = chain.render(12);
    let row = strip(&lines[1]);
    assert!(hoocode_tui_util::visible_width(&lines[1]) <= 12, "{row:?}");
    assert!(row.contains('…'), "{row:?}");
    assert!(!row.contains("npm"), "{row:?}");
}

#[test]
fn the_stats_stay_at_eighty_columns_and_the_label_gives_way() {
    let _g = lock();
    let long = format!("npm run check -- {}", "--flag-with-a-long-name ".repeat(6));
    let mut chain = chain_of(
        Radar,
        vec![call("Shell", json!({ "command": long }), "l1\nl2\nl3")],
        "wide",
    );
    chain.close(ChainState::Done);
    let lines = chain.render(80);
    let row = strip(&lines[1]);
    assert!(hoocode_tui_util::visible_width(&lines[1]) <= 80, "{row:?}");
    assert!(row.ends_with("1 call · 3 lines"), "{row:?}");
    assert!(!row.contains("npm"), "{row:?}");
}

#[test]
fn an_agent_failure_names_the_description_not_the_whole_prompt() {
    let _g = lock();
    let mut chain = chain_of(
        Radar,
        vec![Call {
            is_error: true,
            ..call(
                "Agent",
                json!({
                    "description": "Survey repo",
                    "prompt": "Read every file in the repository and summarise the architecture"
                }),
                "boom",
            )
        }],
        "agent",
    );
    chain.close(ChainState::Done);
    let out = render(&mut chain);
    assert!(out.contains("Survey repo"), "{out}");
    assert!(!out.contains("Read every file"), "{out}");
}

#[test]
fn thinking_never_ends_a_chain_but_speech_does() {
    let thought = Content::Thinking(ThinkingContent {
        thinking: "weighing the options".into(),
        ..Default::default()
    });
    assert!(!speaking_ends_chain(std::slice::from_ref(&thought)));
    assert!(speaking_ends_chain(&[thought, Content::text("Done.")]));
    assert!(!speaking_ends_chain(&[Content::text("   ")]));
}

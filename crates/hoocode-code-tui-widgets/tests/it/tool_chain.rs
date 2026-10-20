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
fn radar_shows_a_live_step_for_the_call_still_running() {
    let _g = lock();
    // Settled clean calls get no row: the mission row above counts them.
    // The call still running reads as its live step.
    let mut calls = run();
    calls[2].pending = true;
    let mut chain = chain_of(Radar, calls, "c");
    let out = render(&mut chain);
    let lines: Vec<&str> = out.split('\n').filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 1, "{out}");
    assert!(lines[0].contains("◐"), "{out}");
    assert!(lines[0].contains("03"), "{out}");
    assert!(lines[0].contains("Edit"), "{out}");
    assert!(!out.contains("›"), "{out}");
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
    assert!(out.contains("◐"), "{out}");
    assert!(out.contains("04"), "{out}");
    assert!(out.contains("Shell"), "{out}");

    // Settled clean: nothing left for the chain to say.
    let mut done = chain_of(Radar, run(), "d");
    done.close(ChainState::Done);
    assert!(render(&mut done).trim().is_empty());
}

#[test]
fn an_interrupted_chain_with_no_hits_leaves_radar_to_its_mission_row() {
    let _g = lock();
    let mut chain = chain_of(Radar, run(), "i");
    chain.close(ChainState::Interrupted);
    assert!(render(&mut chain).trim().is_empty());
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
    assert!(out.contains("⊗"), "{out}");
    assert!(out.contains("02"), "{out}");
    assert!(out.contains("HIT"), "{out}");
    assert!(out.contains("bun test"), "{out}");
    assert!(out.contains("ASSERTION FAILED"), "{out}");
    assert!(!out.contains("1 failed"), "{out}");
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
    assert!(render(&mut chain).trim().is_empty());
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
fn a_hit_row_carries_tool_subject_snippet_and_chip_on_one_line() {
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
    let rows: Vec<&str> = out.split('\n').filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(rows.len(), 1, "{out}");
    assert!(rows[0].contains("⊗"), "{out}");
    assert!(rows[0].contains("Shell"), "{out}");
    assert!(rows[0].contains("bun test"), "{out}");
    assert!(rows[0].contains("ASSERTION FAILED"), "{out}");
    assert!(rows[0].trim_end().ends_with("HIT"), "{out}");
}

#[test]
fn a_settled_clean_chain_reads_nothing_in_radar() {
    let _g = lock();
    let first = run().remove(0);
    let mut chain = chain_of(Radar, vec![first], "one");
    chain.close(ChainState::Done);
    assert!(render(&mut chain).trim().is_empty());
    assert!(chain.is_summarised());
}

#[test]
fn an_orphan_single_call_in_radar_keeps_the_group_summary_row() {
    // A chain before the first mission has no flight-log row above it, so
    // radar keeps its old summary line.
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
    chain.set_orphan(true);
    chain.close(ChainState::Done);
    let lines = chain.render(100);
    assert_eq!(lines[0], "");
    let row = strip(&lines[1]);
    assert!(row.starts_with(" ● Ran npm run check"), "{row:?}");
    assert!(row.ends_with("1 call · 3 lines"), "{row:?}");
    assert!(!row.contains("Shell"));
}

#[test]
fn a_running_single_call_in_radar_reads_as_a_live_step() {
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
    assert!(out.contains("◐"), "{out}");
    assert!(out.contains("01"), "{out}");
    assert!(out.contains("npm run check"), "{out}");
    assert!(!out.contains("running"), "{out}");
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
    assert!(out.contains("⊗"), "{out}");
    assert!(out.contains("bun test"), "{out}");
    assert!(out.contains("HIT"), "{out}");
    assert!(out.contains("ASSERTION FAILED"), "{out}");
    assert!(!out.contains("1 call"), "{out}");
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
    // ` 01 ● Shell  command` with `exit 0 · 1 line` right-aligned, then
    // `   ┊    l1` — with no summary row.
    let rows: Vec<&str> = out.lines().map(str::trim_end).collect();
    // Stem ` 01 ● Shell  npm run check` is 26 cells, stats 15: 59 spaces.
    let header = format!(
        " 01 ● Shell  npm run check{}exit 0 · 1 line",
        " ".repeat(59)
    );
    assert!(
        rows.windows(2)
            .any(|w| w == [header.as_str(), "   ┊    l1"]),
        "{out}"
    );
    assert!(!out.contains("1 call"), "{out}");
    assert!(!chain.is_summarised());
}

#[test]
fn two_calls_with_one_running_show_the_live_step_in_radar() {
    let _g = lock();
    let mut calls = run();
    calls.truncate(2);
    calls[1].pending = true;
    let mut chain = chain_of(Radar, calls, "two");
    assert!(chain.is_summarised());
    let out = render(&mut chain);
    assert!(out.contains("◐"), "{out}");
    assert!(out.contains("02"), "{out}");
    assert!(!out.contains("CodeSearch › Read"), "{out}");
}

#[test]
fn a_settled_chain_attaches_straight_under_its_mission_row() {
    // No lead-in blank: the flight-log rows sit directly under the mission.
    let _g = lock();
    let mut chain = chain_of(Radar, run(), "lead");
    chain.close(ChainState::Done);
    assert!(chain.render(100).is_empty());
}

#[test]
fn a_settled_run_of_one_leaves_no_lead_in_either() {
    let _g = lock();
    let first = run().remove(0);
    let mut chain = chain_of(Radar, vec![first], "lead-one");
    chain.close(ChainState::Done);
    assert!(chain.render(100).is_empty());
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
    // The `⊘` reads on the mission row; the chain adds nothing.
    assert!(render(&mut chain).trim().is_empty());
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
    assert!(out.contains("npm run dev"), "{out}");
    assert!(!out.contains('●'), "{out}");
}

#[test]
fn a_replayed_background_call_with_no_live_job_reads_done() {
    let _g = lock();
    set_job_row("j1", None);
    assert!(render(&mut chain_with_job(ChainState::Done))
        .trim()
        .is_empty());
}

#[test]
fn an_aborted_chain_keeps_a_live_background_job_running() {
    let _g = lock();
    // Aborting the turn does not stop a job: the chain still reads running.
    set_job_row("j1", Some(TaskStatus::InProgress));
    let out = render(&mut chain_with_job(ChainState::Interrupted));
    set_job_row("j1", None);
    assert!(out.contains("◐"), "{out}");
    assert!(out.contains("npm run dev"), "{out}");
    assert!(!out.contains('⊘'), "{out}");
}

#[test]
fn an_aborted_chain_whose_job_has_ended_reads_interrupted() {
    let _g = lock();
    set_job_row("j1", Some(TaskStatus::Done));
    assert!(render(&mut chain_with_job(ChainState::Interrupted))
        .trim()
        .is_empty());
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
    assert!(render(&mut chain).trim().is_empty());
}

#[test]
fn a_hit_row_names_the_first_line_of_the_failure() {
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
    let rows: Vec<&str> = out.split('\n').filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(rows.len(), 1, "{out}");
    assert!(rows[0].contains("l1"), "{out}");
    assert!(rows[0].trim_end().ends_with("HIT"), "{out}");
    assert!(!out.contains("l3"), "{out}");
    assert!(!out.contains("more line"), "{out}");
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
    assert!(out.contains("l1"), "{out}");
    assert!(out.contains("HIT"), "{out}");
    assert!(!out.contains("more line"), "{out}");
}

/// The stats are whole. A label that does not fit beside them is dropped;
/// the stats are cut only when they do not fit alone.
#[test]
fn a_hit_row_keeps_hit_whole_and_cuts_the_subject_first() {
    let _g = lock();
    let long = format!("npm run check -- {}", "--flag-with-a-long-name ".repeat(6));
    let mut chain = chain_of(
        Radar,
        vec![Call {
            is_error: true,
            ..call("Shell", json!({ "command": long }), "boom")
        }],
        "drop",
    );
    chain.close(ChainState::Done);
    // The hit's line is indented 12, so a subject shows a cut at 36 columns.
    let lines = chain.render(36);
    assert_eq!(lines.len(), 1, "{lines:?}");
    let row = strip(&lines[0]);
    assert!(row.trim_end().ends_with("HIT"), "{row:?}");
    assert!(row.contains('…'), "{row:?}");
    assert!(hoocode_tui_util::visible_width(&lines[0]) <= 36, "{row:?}");
}

#[test]
fn stats_that_do_not_fit_alone_are_cut_last() {
    let _g = lock();
    let mut chain = chain_of(
        Radar,
        vec![Call {
            is_error: true,
            ..call("Shell", json!({ "command": "npm test" }), "boom")
        }],
        "cut",
    );
    chain.close(ChainState::Done);
    let lines = chain.render(12);
    assert_eq!(lines.len(), 1, "{lines:?}");
    let row = strip(&lines[0]);
    assert!(hoocode_tui_util::visible_width(&lines[0]) <= 12, "{row:?}");
    assert!(row.contains('…'), "{row:?}");
    assert!(row.trim_end().ends_with("HIT"), "{row:?}");
}

#[test]
fn the_stats_stay_at_eighty_columns_and_the_label_gives_way() {
    let _g = lock();
    let long = format!("npm run check -- {}", "--flag-with-a-long-name ".repeat(6));
    let mut chain = chain_of(
        Radar,
        vec![Call {
            is_error: true,
            ..call("Shell", json!({ "command": long }), "boom")
        }],
        "wide",
    );
    chain.close(ChainState::Done);
    let lines = chain.render(80);
    assert_eq!(lines.len(), 1, "{lines:?}");
    let row = strip(&lines[0]);
    assert!(hoocode_tui_util::visible_width(&lines[0]) <= 80, "{row:?}");
    assert!(row.trim_end().ends_with("HIT"), "{row:?}");
    assert!(row.contains('…'), "{row:?}");
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

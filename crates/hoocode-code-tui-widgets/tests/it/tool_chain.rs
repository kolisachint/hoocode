//! Port of the pin's `test/tool-chain.test.ts`.

use std::cell::RefCell;
use std::rc::Rc;

use crate::support::{lock, strip};
use hoocode_ai_types::{Content, ThinkingContent};
use hoocode_code_task_store::{task_store, CreateTaskOptions, TaskPatch, TaskSource, TaskStatus};
use hoocode_code_tui_widgets::outline::{turn_rows, FlightItem, FlightMark, FlightStats};
use hoocode_code_tui_widgets::tool_chain::{speaking_ends_chain, ToolChainComponent};
use hoocode_code_tui_widgets::tool_chain_summary::ChainState;
use hoocode_code_tui_widgets::tool_execution::{ToolExecutionComponent, ToolExecutionOptions};
use hoocode_code_tui_widgets::tool_output_view::ToolOutputView;
use hoocode_code_tui_widgets::tool_signal::ToolResult;
use hoocode_tui_ratatui::Palette;
use hoocode_tui_render::Component;
use serde_json::{json, Value};

use ToolOutputView::{Outline, Peek};

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

/// The turn's outline block over the chain's calls, plain, one row per
/// entry, trailing blanks trimmed. A turn's chain draws nothing in outline: its
/// header draws the calls (`outline::turn_rows`), so the chain's own rows are
/// read through it here.
fn mission(chain: &ToolChainComponent, width: usize) -> Vec<String> {
    let open = chain.is_open();
    let mark = if open {
        FlightMark::InFlight
    } else if chain.chain_state() == ChainState::Interrupted {
        FlightMark::Aborted
    } else {
        FlightMark::Landed
    };
    let stats = FlightStats {
        mark,
        steps: chain.tool_blocks().len(),
        landed_time: (!open).then(|| "1s".to_string()),
        started: open.then(std::time::Instant::now),
        items: vec![FlightItem::Batch(chain.tool_blocks().to_vec())],
        ..Default::default()
    };
    turn_rows("run it", &stats, &Palette::default(), width)
        .iter()
        .map(|row| strip(row).trim_end().to_string())
        .collect()
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
fn outline_shows_a_live_step_for_the_call_still_running() {
    let _g = lock();
    // The recap reads live: the call still running is named with the
    // settled calls before it.
    let mut calls = run();
    calls[2].pending = true;
    let chain = chain_of(Outline, calls, "c");
    let rows = mission(&chain, 100);
    let recap: Vec<&String> = rows.iter().filter(|r| r.starts_with(" ● ")).collect();
    assert_eq!(recap.len(), 1, "{rows:?}");
    assert!(recap[0].contains("CodeSearch"), "{rows:?}");
    assert!(
        recap[0].contains(" · Read src/keys.ts · Edit src/keys.ts"),
        "{rows:?}"
    );
    assert!(!recap[0].contains("calls"), "{rows:?}");
}

#[test]
fn shows_the_calls_while_running_and_once_done() {
    let _g = lock();
    let mut calls = run();
    calls.push(Call {
        pending: true,
        ..call("Shell", json!({"command": "x"}), "")
    });
    let running = chain_of(Outline, calls, "r");
    let rows = mission(&running, 100);
    assert!(
        rows.iter()
            .any(|r| r.starts_with(" ● ") && r.contains("Shell x")),
        "{rows:?}"
    );

    // Settled clean: one recap row, no failed row, and the chain itself draws nothing.
    let mut done = chain_of(Outline, run(), "d");
    done.close(ChainState::Done);
    let rows = mission(&done, 100);
    assert!(rows.iter().any(|r| r.starts_with(" ● ")), "{rows:?}");
    assert!(rows.iter().all(|r| !r.starts_with(" ⊗")), "{rows:?}");
    assert!(render(&mut done).trim().is_empty());
}

#[test]
fn an_interrupted_chain_with_no_hits_leaves_outline_to_its_mission_row() {
    let _g = lock();
    let mut chain = chain_of(Outline, run(), "i");
    chain.close(ChainState::Interrupted);
    assert!(render(&mut chain).trim().is_empty());
}

#[test]
fn a_collapsed_chain_still_shows_why_a_call_failed() {
    let _g = lock();
    let first = run().remove(0);
    let mut chain = chain_of(
        Outline,
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
    let rows = mission(&chain, 100);
    assert!(
        rows.contains(&" ⊗ Shell bun test  ASSERTION FAILED".to_string()),
        "{rows:?}"
    );
    let recap = rows
        .iter()
        .find(|r| r.starts_with(" ● "))
        .expect("the recap");
    assert!(!recap.contains("Shell"), "{rows:?}");
    assert!(!rows.join("\n").contains("1 failed"), "{rows:?}");
}

#[test]
fn the_dial_not_a_per_chain_toggle_is_what_turns_the_line_back_into_calls() {
    let _g = lock();
    let mut chain = chain_of(Outline, run(), "o");
    chain.close(ChainState::Done);
    // In outline a mission's chain draws nothing; its header draws the calls.
    assert!(render(&mut chain).trim().is_empty());
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
    chain.set_view(Outline);
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
fn a_failed_row_carries_tool_subject_and_reason_on_one_line() {
    let _g = lock();
    let mut chain = chain_of(
        Outline,
        vec![Call {
            is_error: true,
            ..call("Shell", json!({"command": "bun test"}), "ASSERTION FAILED")
        }],
        "indent",
    );
    chain.close(ChainState::Done);
    let rows = mission(&chain, 100);
    let failed: Vec<&String> = rows.iter().filter(|r| r.starts_with(" ⊗ ")).collect();
    assert_eq!(
        failed,
        vec![" ⊗ Shell bun test  ASSERTION FAILED"],
        "{rows:?}"
    );
    // Its only call failed: the batch has no recap row of its own.
    assert!(rows.iter().all(|r| !r.starts_with(" ● ")), "{rows:?}");
}

#[test]
fn a_settled_clean_chain_reads_nothing_in_outline() {
    let _g = lock();
    let first = run().remove(0);
    let mut chain = chain_of(Outline, vec![first], "one");
    chain.close(ChainState::Done);
    assert!(render(&mut chain).trim().is_empty());
    assert!(chain.is_summarised());
}

#[test]
fn an_orphan_single_call_in_outline_reads_as_a_recap_row() {
    // A chain before the first turn has no recap row above it, so outline
    // draws its own: ` ● Shell npm run`.
    let _g = lock();
    let mut chain = chain_of(
        Outline,
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
    assert!(row.starts_with(" ● Shell npm run"), "{row:?}");
    assert!(!row.contains("1 call"), "{row:?}");
}

#[test]
fn a_running_single_call_in_outline_reads_as_a_live_step() {
    let _g = lock();
    let chain = chain_of(
        Outline,
        vec![Call {
            pending: true,
            ..call("Shell", json!({"command": "npm run check"}), "")
        }],
        "running1",
    );
    let out = mission(&chain, 100).join("\n");
    assert!(out.contains(" ● Shell npm run"), "{out}");
    assert!(!out.contains("running"), "{out}");
}

#[test]
fn a_failed_single_call_in_outline_still_shows_its_reason() {
    let _g = lock();
    let mut chain = chain_of(
        Outline,
        vec![Call {
            is_error: true,
            ..call("Shell", json!({"command": "bun test"}), "ASSERTION FAILED")
        }],
        "failed1",
    );
    chain.close(ChainState::Done);
    let out = mission(&chain, 100).join("\n");
    assert!(out.contains("⊗ Shell bun test  ASSERTION FAILED"), "{out}");
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
    // Peek: ` ● Shell npm run check  exit 0` with its facts right after the
    // subject, then the output at column 4 — no step number, no rail, no
    // summary row.
    let rows: Vec<&str> = out.lines().map(str::trim_end).collect();
    assert!(
        rows.windows(2)
            .any(|w| w == [" ● Shell npm run check  exit 0", "    l1"]),
        "{out}"
    );
    assert!(!out.contains("1 call"), "{out}");
    assert!(!chain.is_summarised());
}

#[test]
fn two_calls_with_one_running_show_the_live_step_in_outline() {
    let _g = lock();
    let mut calls = run();
    calls.truncate(2);
    calls[1].pending = true;
    let chain = chain_of(Outline, calls, "two");
    assert!(chain.is_summarised());
    let out = mission(&chain, 100).join("\n");
    // The live recap names both calls, the settled one first.
    assert!(out.contains(" ● CodeSearch"), "{out}");
    assert!(out.contains(" · Read src/keys.ts"), "{out}");
}

#[test]
fn a_settled_chain_attaches_straight_under_its_mission_row() {
    // No lead-in blank: the flight-log rows sit directly under the mission.
    let _g = lock();
    let mut chain = chain_of(Outline, run(), "lead");
    chain.close(ChainState::Done);
    assert!(chain.render(100).is_empty());
}

#[test]
fn a_settled_run_of_one_leaves_no_lead_in_either() {
    let _g = lock();
    let first = run().remove(0);
    let mut chain = chain_of(Outline, vec![first], "lead-one");
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
    let mut chain = chain_of(Outline, run(), "s");
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
            view: Outline,
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
fn an_interrupted_chain_with_no_failure_is_stopped_never_green() {
    let _g = lock();
    let mut chain = chain_of(Outline, run(), "cancel");
    for block in chain.tool_blocks() {
        block.borrow_mut().set_interrupted();
    }
    chain.close(ChainState::Interrupted);
    // The recap reads dim `⊗`, never green `●`.
    let rows = mission(&chain, 100);
    assert!(
        rows.iter().any(|row| row.starts_with(" ⊗ CodeSearch")),
        "{rows:?}"
    );
    assert!(rows.iter().all(|row| !row.starts_with(" ● ")), "{rows:?}");
    assert!(rows.contains(&" ⊗ stopped · 1s".to_string()), "{rows:?}");
    // The chain itself draws nothing in outline: its turn's header does.
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
    let mut chain = ToolChainComponent::new(Outline);
    chain.add(Rc::new(RefCell::new(background_shell())));
    chain.close(state);
    chain
}

#[test]
fn a_background_call_is_in_the_recap_while_its_job_runs() {
    let _g = lock();
    set_job_row("j1", Some(TaskStatus::InProgress));
    let rows = mission(&chain_with_job(ChainState::Done), 100);
    set_job_row("j1", None);
    assert!(
        rows.iter().any(|r| r.starts_with(" ● Shell npm run")),
        "{rows:?}"
    );
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
    // Aborting the turn does not stop a job: its recap does not read stopped.
    set_job_row("j1", Some(TaskStatus::InProgress));
    let rows = mission(&chain_with_job(ChainState::Interrupted), 100);
    set_job_row("j1", None);
    assert!(
        rows.iter().any(|r| r.starts_with(" ● Shell npm run")),
        "{rows:?}"
    );
    assert!(rows.iter().all(|r| !r.starts_with(" ⊗ Shell")), "{rows:?}");
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
        Outline,
        vec![call("Shell", json!({"command": "npm test"}), "ok")],
        "fin",
    );
    assert!(render(&mut chain).trim().is_empty());
}

#[test]
fn a_failed_row_names_the_first_line_of_the_failure() {
    let _g = lock();
    let mut chain = chain_of(
        Outline,
        vec![Call {
            is_error: true,
            ..call("Shell", json!({"command": "make"}), "l1\nl2\nl3\nl4\nl5")
        }],
        "cap",
    );
    chain.close(ChainState::Done);
    let rows = mission(&chain, 100);
    assert!(rows.contains(&" ⊗ Shell make  l1".to_string()), "{rows:?}");
    let out = rows.join("\n");
    assert!(!out.contains("l2"), "{out}");
    assert!(!out.contains("more line"), "{out}");
}

#[test]
fn a_failed_call_with_two_lines_or_fewer_shows_no_fold() {
    let _g = lock();
    let mut chain = chain_of(
        Outline,
        vec![Call {
            is_error: true,
            ..call("Shell", json!({"command": "make"}), "l1\nl2")
        }],
        "short",
    );
    chain.close(ChainState::Done);
    let out = mission(&chain, 100).join("\n");
    assert!(out.contains("l1"), "{out}");
    assert!(!out.contains("more line"), "{out}");
}

#[test]
fn a_failed_row_wraps_a_long_command_within_the_width() {
    let _g = lock();
    let long = format!("npm run check -- {}", "--flag-with-a-long-name ".repeat(6));
    let mut chain = chain_of(
        Outline,
        vec![Call {
            is_error: true,
            ..call("Shell", json!({ "command": long }), "boom")
        }],
        "drop",
    );
    chain.close(ChainState::Done);
    let rows = mission(&chain, 36);
    assert!(
        rows.iter().any(|r| r.starts_with(" ⊗ Shell npm run")),
        "{rows:?}"
    );
    assert!(rows.iter().any(|r| r.contains("boom")), "{rows:?}");
    assert!(rows.iter().all(|r| r.chars().count() <= 36), "{rows:?}");
}

#[test]
fn a_failed_row_fits_a_very_narrow_width() {
    let _g = lock();
    let mut chain = chain_of(
        Outline,
        vec![Call {
            is_error: true,
            ..call("Shell", json!({ "command": "npm test" }), "boom")
        }],
        "cut",
    );
    chain.close(ChainState::Done);
    let rows = mission(&chain, 12);
    assert!(rows.iter().all(|r| r.chars().count() <= 12), "{rows:?}");
}

#[test]
fn a_failed_row_at_eighty_columns_keeps_its_reason() {
    let _g = lock();
    let long = format!("npm run check -- {}", "--flag-with-a-long-name ".repeat(6));
    let mut chain = chain_of(
        Outline,
        vec![Call {
            is_error: true,
            ..call("Shell", json!({ "command": long }), "boom")
        }],
        "wide",
    );
    chain.close(ChainState::Done);
    let rows = mission(&chain, 80);
    assert!(rows.iter().all(|r| r.chars().count() <= 80), "{rows:?}");
    assert!(
        rows.iter().any(|r| r.trim_end().ends_with("boom")),
        "{rows:?}"
    );
}

#[test]
fn an_agent_failure_names_the_description_not_the_whole_prompt() {
    let _g = lock();
    let mut chain = chain_of(
        Outline,
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
    let out = mission(&chain, 100).join("\n");
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

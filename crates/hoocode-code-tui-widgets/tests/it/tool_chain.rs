//! Port of the pin's `test/tool-chain.test.ts`.

use std::cell::RefCell;
use std::rc::Rc;

use crate::support::{lock, strip};
use hoocode_ai_types::{Content, ThinkingContent};
use hoocode_code_task_store::{task_store, CreateTaskOptions, TaskPatch, TaskSource, TaskStatus};
use hoocode_code_tui_widgets::tool_chain::{speaking_ends_chain, ChainState, ToolChainComponent};
use hoocode_code_tui_widgets::tool_execution::{ToolExecutionComponent, ToolExecutionOptions};
use hoocode_code_tui_widgets::tool_signal::ToolResult;
use hoocode_tui_render::Component;
use serde_json::{json, Value};

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

fn chain_of(calls: Vec<Call>, id: &str) -> ToolChainComponent {
    let mut chain = ToolChainComponent::new();
    for (i, c) in calls.into_iter().enumerate() {
        // `createAllToolDefinitions(cwd)[tool]`: the built-in table.
        let mut block = ToolExecutionComponent::new(
            c.tool,
            &format!("{id}-{i}"),
            c.args,
            ToolExecutionOptions::default(),
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

/// The chain's own rows, plain, trailing blanks trimmed: in the single view
/// a chain always draws its calls.
fn mission(chain: &mut ToolChainComponent, width: u16) -> Vec<String> {
    chain
        .render(width)
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
fn running_calls_render_a_live_step_row() {
    let _g = lock();
    // The rows read live: the call still running carries the live mark
    // while the settled calls before it carry the done mark.
    let mut calls = run();
    calls[2].pending = true;
    let mut chain = chain_of(calls, "c");
    let rows = mission(&mut chain, 100);
    let live: Vec<&String> = rows.iter().filter(|r| r.starts_with(" ◉ ")).collect();
    assert_eq!(live.len(), 1, "{rows:?}");
    assert!(live[0].contains("Edit"), "{rows:?}");
    assert!(!live[0].contains("calls"), "{rows:?}");
    assert!(
        rows.iter()
            .any(|r| r.starts_with(" ● ") && r.contains("CodeSearch")),
        "{rows:?}"
    );
}

#[test]
fn shows_the_calls_while_running_and_once_done() {
    let _g = lock();
    let mut calls = run();
    calls.push(Call {
        pending: true,
        ..call("Shell", json!({"command": "x"}), "")
    });
    let mut running = chain_of(calls, "r");
    let rows = mission(&mut running, 100);
    assert!(
        rows.iter()
            .any(|r| r.starts_with(" ◉ ") && r.contains("Shell x")),
        "{rows:?}"
    );

    // Settled clean: one row per call, no failed row.
    let mut done = chain_of(run(), "d");
    done.close(ChainState::Done);
    let rows = mission(&mut done, 100);
    assert_eq!(
        rows.iter().filter(|r| r.starts_with(" ● ")).count(),
        3,
        "{rows:?}"
    );
    assert!(rows.iter().all(|r| !r.starts_with(" ⊗")), "{rows:?}");
}

#[test]
fn an_interrupted_chain_still_lists_its_calls() {
    let _g = lock();
    let mut chain = chain_of(run(), "i");
    chain.close(ChainState::Interrupted);
    let rows = mission(&mut chain, 100);
    assert!(rows.iter().any(|r| r.contains("CodeSearch")), "{rows:?}");
}

#[test]
fn a_collapsed_chain_still_shows_why_a_call_failed() {
    let _g = lock();
    let first = run().remove(0);
    let mut chain = chain_of(
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
    let rows = mission(&mut chain, 100);
    let failed = rows
        .iter()
        .find(|r| r.starts_with(" ⊗ "))
        .expect("the failed row");
    assert!(
        failed.contains("⊗ Shell bun test  ASSERTION FAILED"),
        "{rows:?}"
    );
    assert!(
        rows.iter()
            .any(|r| r.starts_with(" ● ") && r.contains("CodeSearch")),
        "{rows:?}"
    );
    assert!(!rows.join("\n").contains("1 failed"), "{rows:?}");
}

#[test]
fn a_shut_chain_lists_every_call() {
    let _g = lock();
    let mut chain = chain_of(run(), "o");
    chain.close(ChainState::Done);
    // Shut: one header row per call.
    let rows = mission(&mut chain, 100);
    assert_eq!(
        rows.iter().filter(|r| r.starts_with(" ● ")).count(),
        3,
        "{rows:?}"
    );
    assert!(rows.iter().any(|r| r.contains("CodeSearch")), "{rows:?}");
    assert!(rows.iter().any(|r| r.contains("Read")), "{rows:?}");
    assert!(rows.iter().any(|r| r.contains("Edit")), "{rows:?}");
}

#[test]
fn is_a_plain_pass_through() {
    let _g = lock();
    let mut chain = chain_of(run(), "p");
    chain.close(ChainState::Done);
    let out = render(&mut chain);
    assert!(!out.contains('›'));
    assert!(out.contains("CodeSearch"));
}

#[test]
fn the_toggle_opens_and_closes_a_call_body() {
    let _g = lock();
    let mut chain = chain_of(run(), "v");
    chain.close(ChainState::Done);
    assert!(!render(&mut chain).contains('›'));
    chain.tool_blocks()[0].borrow_mut().set_expanded(true);
    let opened = render(&mut chain);
    assert!(opened.lines().any(|l| l.trim() == "a"), "{opened}");
    chain.tool_blocks()[0].borrow_mut().set_expanded(false);
    let shut = render(&mut chain);
    assert!(!shut.lines().any(|l| l.trim() == "a"), "{shut}");
}

#[test]
fn every_tools_call_line_starts_in_the_same_column() {
    let _g = lock();
    let cwd = cwd();
    let mut chain = chain_of(
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
        vec![Call {
            is_error: true,
            ..call("Shell", json!({"command": "bun test"}), "ASSERTION FAILED")
        }],
        "indent",
    );
    chain.close(ChainState::Done);
    let rows = mission(&mut chain, 100);
    let failed: Vec<&String> = rows.iter().filter(|r| r.starts_with(" ⊗ ")).collect();
    assert_eq!(failed.len(), 1, "{rows:?}");
    assert!(
        failed[0].contains("⊗ Shell bun test  ASSERTION FAILED"),
        "{rows:?}"
    );
    // Its only call failed: the batch has no recap row of its own.
    assert!(rows.iter().all(|r| !r.starts_with(" ● ")), "{rows:?}");
}

#[test]
fn a_settled_clean_chain_lists_its_call_shut() {
    let _g = lock();
    let first = run().remove(0);
    let mut chain = chain_of(vec![first], "one");
    chain.close(ChainState::Done);
    let rows = mission(&mut chain, 100);
    assert_eq!(
        rows.iter().filter(|r| r.starts_with(" ● ")).count(),
        1,
        "{rows:?}"
    );
    assert!(!chain.tool_blocks()[0].borrow().is_expanded());
}

#[test]
fn a_settled_single_call_reads_as_its_own_header_row() {
    // A chain before the first turn has no recap row above it: its own
    // header row carries the call.
    let _g = lock();
    let mut chain = chain_of(
        vec![call(
            "Shell",
            json!({"command": "npm run check"}),
            "l1\nl2\nl3",
        )],
        "single",
    );
    chain.close(ChainState::Done);
    let rows = mission(&mut chain, 100);
    let row = rows
        .iter()
        .find(|r| r.starts_with(" ● "))
        .expect("the header row");
    assert!(row.starts_with(" ● Shell npm run"), "{row:?}");
    assert!(!row.contains("1 call"), "{row:?}");
}

#[test]
fn a_running_single_call_reads_as_a_live_step() {
    let _g = lock();
    let mut chain = chain_of(
        vec![Call {
            pending: true,
            ..call("Shell", json!({"command": "npm run check"}), "")
        }],
        "running1",
    );
    let out = mission(&mut chain, 100).join("\n");
    assert!(out.contains(" ◉ Shell npm run"), "{out}");
    assert!(!out.contains("running"), "{out}");
}

#[test]
fn a_failed_single_call_still_shows_its_reason() {
    let _g = lock();
    let mut chain = chain_of(
        vec![Call {
            is_error: true,
            ..call("Shell", json!({"command": "bun test"}), "ASSERTION FAILED")
        }],
        "failed1",
    );
    chain.close(ChainState::Done);
    let out = mission(&mut chain, 100).join("\n");
    assert!(out.contains("⊗ Shell bun test  ASSERTION FAILED"), "{out}");
}

#[test]
fn a_single_call_is_still_its_own_block() {
    let _g = lock();
    let mut chain = chain_of(
        vec![call("Shell", json!({"command": "npm run check"}), "l1")],
        "peek1",
    );
    chain.close(ChainState::Done);
    // Shut: ` ● Shell npm run check  exit 0 · 1 line` alone — no step number, no
    // rail, no summary row, no body. The toggle opens the output at column 4.
    let shut = render(&mut chain);
    assert!(
        shut.contains(" ● Shell npm run check  exit 0 · 1 line"),
        "{shut}"
    );
    assert!(!shut.contains("l1"), "{shut}");
    assert!(!shut.contains("1 call"), "{shut}");
    assert!(!chain.tool_blocks()[0].borrow().is_expanded());
    chain.tool_blocks()[0].borrow_mut().set_expanded(true);
    let out = render(&mut chain);
    let rows: Vec<&str> = out.lines().map(str::trim_end).collect();
    assert!(
        rows.windows(2)
            .any(|w| w == [" ● Shell npm run check  exit 0 · 1 line ▾", "    l1"]),
        "{out}"
    );
}

#[test]
fn two_calls_with_one_running_show_the_live_step() {
    let _g = lock();
    let mut calls = run();
    calls.truncate(2);
    calls[1].pending = true;
    let mut chain = chain_of(calls, "two");
    let out = mission(&mut chain, 100).join("\n");
    // The live row names the running call; the settled one reads done.
    assert!(out.contains(" ◉ Read"), "{out}");
    assert!(out.contains(" ● CodeSearch"), "{out}");
}

#[test]
fn a_settled_chain_draws_its_rows() {
    let _g = lock();
    let mut chain = chain_of(run(), "lead");
    chain.close(ChainState::Done);
    let out = render(&mut chain);
    assert!(out.contains("CodeSearch"), "{out}");
}

#[test]
fn a_settled_run_of_one_draws_its_row() {
    let _g = lock();
    let first = run().remove(0);
    let mut chain = chain_of(vec![first], "lead-one");
    chain.close(ChainState::Done);
    let out = render(&mut chain);
    assert!(out.contains("CodeSearch"), "{out}");
}

#[test]
fn does_not_double_the_gap_where_the_blocks_already_draw_one() {
    let _g = lock();
    let mut chain = chain_of(run(), "nolead");
    chain.close(ChainState::Done);
    let lines = chain.render(100);
    assert_eq!(lines.iter().take(2).filter(|l| l.is_empty()).count(), 1);
}

#[test]
fn reports_whether_it_is_still_collecting_calls() {
    let _g = lock();
    let mut chain = chain_of(run(), "s");
    assert!(chain.is_open());
    chain.close(ChainState::Done);
    assert!(!chain.is_open());
}

fn background_shell() -> ToolExecutionComponent {
    let mut block = ToolExecutionComponent::new(
        "Shell",
        "bg-0",
        json!({"command": "npm run dev", "run_in_background": true}),
        ToolExecutionOptions::default(),
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
    let mut chain = chain_of(run(), "cancel");
    for block in chain.tool_blocks() {
        block.borrow_mut().set_interrupted();
    }
    chain.close(ChainState::Interrupted);
    // Interrupted calls read `⊗`, never green `●`.
    let rows = mission(&mut chain, 100);
    assert!(
        rows.iter().any(|row| row.starts_with(" ⊗ CodeSearch")),
        "{rows:?}"
    );
    assert!(rows.iter().all(|row| !row.starts_with(" ● ")), "{rows:?}");
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
    let mut chain = ToolChainComponent::new();
    chain.add(Rc::new(RefCell::new(background_shell())));
    chain.close(state);
    chain
}

#[test]
fn a_background_call_lists_its_job_while_it_runs() {
    let _g = lock();
    set_job_row("j1", Some(TaskStatus::InProgress));
    let mut replayed = chain_with_job(ChainState::Done);
    let rows = mission(&mut replayed, 100);
    set_job_row("j1", None);
    assert!(
        rows.iter()
            .any(|r| r.starts_with(" ● ") && r.contains("Shell npm run")),
        "{rows:?}"
    );
    assert!(rows.iter().any(|r| r.contains("background j1")), "{rows:?}");
}

#[test]
fn a_replayed_background_call_with_no_live_job_reads_done() {
    let _g = lock();
    set_job_row("j1", None);
    let mut chain = chain_with_job(ChainState::Done);
    let out = render(&mut chain);
    assert!(out.contains(" ● Shell"), "{out}");
}

#[test]
fn an_aborted_chain_keeps_a_live_background_job_listed() {
    let _g = lock();
    // Aborting the turn does not stop a job: its row does not read stopped.
    set_job_row("j1", Some(TaskStatus::InProgress));
    let mut aborted = chain_with_job(ChainState::Interrupted);
    let rows = mission(&mut aborted, 100);
    set_job_row("j1", None);
    assert!(rows.iter().any(|r| r.contains("Shell npm run")), "{rows:?}");
    assert!(rows.iter().all(|r| !r.starts_with(" ⊗ Shell")), "{rows:?}");
}

#[test]
fn an_aborted_chain_keeps_its_calls_and_reads_interrupted() {
    let _g = lock();
    set_job_row("j1", Some(TaskStatus::Done));
    let mut chain = chain_with_job(ChainState::Interrupted);
    assert_eq!(chain.chain_state(), ChainState::Interrupted);
    let rows = mission(&mut chain, 100);
    assert!(
        rows.iter()
            .any(|r| r.starts_with(" ● ") && r.contains("Shell npm run")),
        "{rows:?}"
    );
}

#[test]
fn a_finished_shell_reads_done_as_soon_as_its_result_arrives() {
    let _g = lock();
    // The chain is still open: the model has not answered yet.
    let mut chain = chain_of(
        vec![call("Shell", json!({"command": "npm test"}), "ok")],
        "fin",
    );
    let out = render(&mut chain);
    assert!(out.contains(" ● Shell npm test"), "{out}");
}

#[test]
fn a_failed_row_names_the_first_line_of_the_failure() {
    let _g = lock();
    let mut chain = chain_of(
        vec![Call {
            is_error: true,
            ..call("Shell", json!({"command": "make"}), "l1\nl2\nl3\nl4\nl5")
        }],
        "cap",
    );
    chain.close(ChainState::Done);
    let rows = mission(&mut chain, 100);
    let failed = rows
        .iter()
        .find(|r| r.starts_with(" ⊗ "))
        .expect("the failed row");
    assert!(failed.contains("⊗ Shell make  l1"), "{rows:?}");
    let out = rows.join("\n");
    assert!(!out.contains("l2"), "{out}");
    assert!(!out.contains("more line"), "{out}");
}

#[test]
fn a_failed_call_with_two_lines_or_fewer_shows_no_fold() {
    let _g = lock();
    let mut chain = chain_of(
        vec![Call {
            is_error: true,
            ..call("Shell", json!({"command": "make"}), "l1\nl2")
        }],
        "short",
    );
    chain.close(ChainState::Done);
    let out = mission(&mut chain, 100).join("\n");
    assert!(out.contains("l1"), "{out}");
    assert!(!out.contains("more line"), "{out}");
}

#[test]
fn a_failed_row_wraps_a_long_command_within_the_width() {
    let _g = lock();
    let long = format!("npm run check -- {}", "--flag-with-a-long-name ".repeat(6));
    let mut chain = chain_of(
        vec![Call {
            is_error: true,
            ..call("Shell", json!({ "command": long }), "boom")
        }],
        "drop",
    );
    chain.close(ChainState::Done);
    let rows = mission(&mut chain, 36);
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
        vec![Call {
            is_error: true,
            ..call("Shell", json!({ "command": "npm test" }), "boom")
        }],
        "cut",
    );
    chain.close(ChainState::Done);
    let rows = mission(&mut chain, 12);
    assert!(rows.iter().all(|r| r.chars().count() <= 12), "{rows:?}");
}

#[test]
fn a_failed_row_at_eighty_columns_keeps_its_reason() {
    let _g = lock();
    let long = format!("npm run check -- {}", "--flag-with-a-long-name ".repeat(6));
    let mut chain = chain_of(
        vec![Call {
            is_error: true,
            ..call("Shell", json!({ "command": long }), "boom")
        }],
        "wide",
    );
    chain.close(ChainState::Done);
    let rows = mission(&mut chain, 80);
    assert!(rows.iter().all(|r| r.chars().count() <= 80), "{rows:?}");
    assert!(
        rows.iter().any(|r| r.trim_end().ends_with("boom ▸")),
        "{rows:?}"
    );
}

#[test]
fn an_agent_failure_names_the_description_not_the_whole_prompt() {
    let _g = lock();
    let mut chain = chain_of(
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
    let rows = mission(&mut chain, 100);
    let header = rows
        .iter()
        .find(|r| r.starts_with(" ⊗ "))
        .expect("the failed row");
    assert!(header.contains("Survey repo"), "{rows:?}");
    assert!(!header.contains("Read every file"), "{rows:?}");
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

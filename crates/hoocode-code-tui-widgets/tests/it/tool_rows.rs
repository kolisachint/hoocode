//! The declutter tool rows (WP6a): ` ● Name subject  facts`, an indented dim
//! body, per-view body rules, no step numbers, rails, `Took` lines or gutters.
//! Goldens pin the screens in `docs/design/tui-declutter/mockups/dark/screens`
//! (peek, full, failed, agent) at 100 columns.

use std::cell::RefCell;
use std::rc::Rc;

use crate::support::{lock, strip};
use hoocode_ai_types::Content;
use hoocode_code_subagents::live::apply;
use hoocode_code_tui_widgets::bash_execution::BashExecutionComponent;
use hoocode_code_tui_widgets::tool_chain::ToolChainComponent;
use hoocode_code_tui_widgets::tool_chain_summary::ChainState;
use hoocode_code_tui_widgets::tool_execution::{ToolExecutionComponent, ToolExecutionOptions};
use hoocode_code_tui_widgets::tool_output_view::ToolOutputView;
use hoocode_code_tui_widgets::tool_signal::ToolResult;
use hoocode_code_tui_widgets::tools::{builtin_tool_definition, registered_tool_definition};
use hoocode_tui_render::golden::render_golden;
use hoocode_tui_render::{assert_golden, Component};
use serde_json::{json, Value};

const CWD: &str = "/work";

fn block(
    view: ToolOutputView,
    tool: &str,
    args: Value,
    output: &str,
    details: Value,
    is_error: bool,
) -> Rc<RefCell<ToolExecutionComponent>> {
    let definition =
        builtin_tool_definition(tool).unwrap_or_else(|| registered_tool_definition(tool));
    let mut b = ToolExecutionComponent::new(
        tool,
        "id",
        args,
        ToolExecutionOptions {
            view,
            ..Default::default()
        },
        Some(definition),
        CWD,
    );
    b.update_result(
        ToolResult {
            content: vec![Content::text(output)],
            details,
            is_error,
        },
        false,
    );
    Rc::new(RefCell::new(b))
}

fn chain(
    view: ToolOutputView,
    blocks: Vec<Rc<RefCell<ToolExecutionComponent>>>,
) -> ToolChainComponent {
    let mut chain = ToolChainComponent::new(view);
    for b in blocks {
        chain.add(b);
    }
    chain.close(ChainState::Done);
    chain
}

fn numbered(prefix: &str, n: usize) -> String {
    (1..=n)
        .map(|i| format!("{prefix}{i:02}"))
        .collect::<Vec<_>>()
        .join("\n")
}

const DIFF: &str = " 12     let mut cfg = Config::default();\n-14     let raw = read_file(path)?;\n-15     apply_env(&mut cfg);\n+14     apply_env(&mut cfg);\n+15     let raw = read_file(path)?;\n 16     parse_into(&mut cfg, &raw)?;\n 17     Ok(cfg)";

/// The same run at 40 and 60 columns: the single-view width pins (UI-21 P0).
/// Today's wrapping behaviour is pinned; P3 replaces it with the cut order.
#[test]
fn tool_rows_peek_40() {
    let _g = lock();
    let mut c = config_run(ToolOutputView::Peek);
    assert_golden!("tool_rows_peek_40", render_golden(&mut c, 40));
}

#[test]
fn tool_rows_peek_60() {
    let _g = lock();
    let mut c = config_run(ToolOutputView::Peek);
    assert_golden!("tool_rows_peek_60", render_golden(&mut c, 60));
}

/// Read, Edit and Shell: the mockup's `peek` and `full` screens.
fn config_run(view: ToolOutputView) -> ToolChainComponent {
    chain(
        view,
        vec![
            block(
                view,
                "Read",
                json!({"file_path": "/work/src/config.rs"}),
                &numbered("setting_", 12),
                json!({}),
                false,
            ),
            block(
                view,
                "Edit",
                json!({"path": "/work/src/config.rs", "oldText": "a", "newText": "b"}),
                "ok",
                json!({"diff": DIFF}),
                false,
            ),
            block(
                view,
                "Shell",
                json!({"command": "cargo test -p hoocode-config 2>&1 | tail -40"}),
                &numbered("test config::tests::case_", 12),
                json!({"durationMs": 8200}),
                false,
            ),
        ],
    )
}

#[test]
fn tool_rows_peek_100() {
    let _g = lock();
    let mut c = config_run(ToolOutputView::Peek);
    assert_golden!("tool_rows_peek_100", render_golden(&mut c, 100));
}

#[test]
fn tool_rows_full_100() {
    let _g = lock();
    let mut c = config_run(ToolOutputView::Full);
    assert_golden!("tool_rows_full_100", render_golden(&mut c, 100));
}

#[test]
fn tool_rows_failed_100() {
    let _g = lock();
    let view = ToolOutputView::Peek;
    let mut c = chain(
        view,
        vec![
            block(
                view,
                "Shell",
                json!({"command": "cargo test -p hoocode-parser escaped_quote"}),
                "a\nb\nc\nd\ne\nf\nerror: could not compile `hoocode-parser` (lib)\n\nCommand exited with code 1",
                json!({"durationMs": 3100}),
                true,
            ),
            block(
                view,
                "Read",
                json!({"file_path": "/work/docs/screens/parser-failure.png"}),
                "line one\nline two",
                json!({}),
                false,
            ),
        ],
    );
    assert_golden!("tool_rows_failed_100", render_golden(&mut c, 100));
}

/// A settled agent with one failed call of its own.
fn settled_agent(view: ToolOutputView, run: &str) -> Rc<RefCell<ToolExecutionComponent>> {
    apply(run, &json!({"type": "task_started"}));
    for (id, tool, fails) in [("s1", "Read", false), ("s2", "Grep", true)] {
        apply(
            run,
            &json!({"type": "tool_execution_start", "toolCallId": id, "toolName": tool, "args": {"pattern": "\\\\\""}}),
        );
        apply(
            run,
            &json!({"type": "tool_execution_end", "toolCallId": id, "isError": fails,
                    "result": {"content": [{"type": "text", "text": "regex parse error: unclosed group"}]}}),
        );
    }
    apply(run, &json!({"type": "task_done"}));
    block(
        view,
        "Agent",
        json!({"subagent_type": "explore", "description": "Trace escaped quotes"}),
        "Found it in lex.rs.\nSecond line.",
        json!({"poolTaskId": run}),
        false,
    )
}

#[test]
fn tool_rows_agent_100() {
    let _g = lock();
    let mut c = chain(
        ToolOutputView::Peek,
        vec![settled_agent(ToolOutputView::Peek, "tool-rows-agent-peek")],
    );
    assert_golden!("tool_rows_agent_100", render_golden(&mut c, 100));
}

#[test]
fn tool_rows_agent_full_100() {
    let _g = lock();
    let mut c = chain(
        ToolOutputView::Full,
        vec![settled_agent(ToolOutputView::Full, "tool-rows-agent-full")],
    );
    assert_golden!("tool_rows_agent_full_100", render_golden(&mut c, 100));
}

#[test]
fn a_collapsed_run_is_tight_and_an_open_row_is_followed_by_one_blank() {
    let _g = lock();
    let mut c = config_run(ToolOutputView::Peek);
    let rows: Vec<String> = c
        .render(100)
        .iter()
        .map(|l| strip(l).trim_end().to_string())
        .collect();
    // Collapsed: one blank opens the run; every row is its header.
    assert_eq!(rows[0], "", "{rows:?}");
    assert!(rows[1].starts_with(" ● Read "), "{rows:?}");
    assert!(rows[2].starts_with(" ● Edit "), "{rows:?}");
    assert!(rows.iter().all(|r| !r.contains("test config")), "{rows:?}");
    // An open row's body is followed by exactly one blank row.
    let shell = c.tool_blocks()[2].clone();
    shell.borrow_mut().set_expanded(true);
    let rows: Vec<String> = c
        .render(100)
        .iter()
        .map(|l| strip(l).trim_end().to_string())
        .collect();
    assert!(rows.iter().any(|r| r.contains("test config")), "{rows:?}");
    let end = rows.iter().position(|r| r.contains("case_12")).unwrap();
    // The open body runs to the end of the run: no trailing blank row.
    assert_eq!(end + 1, rows.len(), "{rows:?}");
}

#[test]
fn facts_follow_the_subject_and_zero_values_stay_out() {
    let _g = lock();
    let view = ToolOutputView::Peek;
    let mut c = chain(
        view,
        vec![
            block(
                view,
                "Edit",
                json!({"path": "/work/a.rs", "oldText": "a", "newText": "b"}),
                "ok",
                json!({"diff": "-old\n+new"}),
                false,
            ),
            block(
                view,
                "Edit",
                json!({"path": "/work/b.rs", "oldText": "a", "newText": "b"}),
                "ok",
                json!({"diff": "+only added"}),
                false,
            ),
            block(
                view,
                "Read",
                json!({"file_path": "/work/empty.rs"}),
                "",
                json!({}),
                false,
            ),
        ],
    );
    let out = strip(&c.render(100).join("\n"));
    assert!(out.contains(" ● Edit /work/a.rs  +1 −1"), "{out}");
    assert!(
        out.contains(" ● Edit /work/b.rs  +1\n") || out.contains(" ● Edit /work/b.rs  +1 "),
        "{out}"
    );
    assert!(!out.contains("−0") && !out.contains("+0"), "{out}");
    assert!(!out.contains("0 lines"), "{out}");
}

#[test]
fn a_long_label_wraps_and_the_facts_close_the_last_row() {
    let _g = lock();
    let view = ToolOutputView::Peek;
    let command = format!(
        "cargo test {}",
        (1..=20)
            .map(|i| format!("flag{i:02}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let mut c = chain(
        view,
        vec![block(
            view,
            "Shell",
            json!({ "command": command }),
            "ok",
            json!({}),
            false,
        )],
    );
    let rows: Vec<String> = c
        .render(50)
        .iter()
        .map(|l| strip(l).trim_end().to_string())
        .collect();
    let header: Vec<&String> = rows
        .iter()
        .filter(|r| !r.is_empty() && !r.starts_with("    ok"))
        .collect();
    assert!(header.len() >= 3, "{rows:?}");
    assert!(header[0].starts_with(" ● Shell cargo test"), "{rows:?}");
    assert!(
        header[1].starts_with("   flag") || header[1].starts_with("   "),
        "{rows:?}"
    );
    assert!(header.last().unwrap().ends_with("exit 0"), "{rows:?}");
    assert!(!rows.iter().any(|r| r.contains('…')), "{rows:?}");
}

#[test]
fn a_collapsed_row_shows_no_body_and_the_toggle_opens_it_whole() {
    let _g = lock();
    let view = ToolOutputView::Peek;
    // Single view: every block starts as one row; the toggle opens the
    // whole output, however long, with no fold hint.
    for lines in [4, 12] {
        let b = block(
            view,
            "Shell",
            json!({"command": "x"}),
            &numbered("row-", lines),
            json!({}),
            false,
        );
        let mut c = chain(view, vec![b.clone()]);
        let shut = strip(&c.render(100).join("\n"));
        assert!(!shut.contains("row-01"), "{shut}");
        b.borrow_mut().set_expanded(true);
        let open = strip(&c.render(100).join("\n"));
        assert!(
            open.contains("row-01") && open.contains(&format!("row-{lines:02}")),
            "{open}"
        );
        assert!(!open.contains("earlier"), "{open}");
    }
}

#[test]
fn a_running_call_has_an_accent_fisheye_and_no_facts() {
    let _g = lock();
    let view = ToolOutputView::Peek;
    let b = ToolExecutionComponent::new(
        "Shell",
        "id",
        json!({"command": "sleep 5"}),
        ToolExecutionOptions {
            view,
            ..Default::default()
        },
        Some(builtin_tool_definition("Shell").unwrap()),
        CWD,
    );
    let mut c = chain(view, vec![Rc::new(RefCell::new(b))]);
    let rows: Vec<String> = c
        .render(80)
        .iter()
        .map(|l| strip(l).trim_end().to_string())
        .collect();
    assert_eq!(rows[1], " ◉ Shell sleep 5", "{rows:?}");
}

// ---- the user's `!` command ----

#[test]
fn user_bash_row_reads_like_a_tool_row() {
    let _g = lock();
    let mut passed = BashExecutionComponent::new("cargo test", false);
    passed.append_output(&numbered("test case_", 9));
    passed.set_complete(Some(0), false, None, None);
    assert_golden!("user_bash_passed_80", render_golden(&mut passed, 80));

    let mut failed = BashExecutionComponent::new("cargo build", false);
    failed.append_output(&numbered("error ", 7));
    failed.set_complete(Some(101), false, None, None);
    assert_golden!("user_bash_failed_80", render_golden(&mut failed, 80));

    let mut cancelled = BashExecutionComponent::new("sleep 99", false);
    cancelled.set_complete(None, true, None, None);
    assert_golden!("user_bash_cancelled_80", render_golden(&mut cancelled, 80));
}

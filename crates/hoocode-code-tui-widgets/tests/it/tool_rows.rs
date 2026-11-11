//! The declutter tool rows (WP6a): ` ● Name subject  facts`, an indented dim
//! body, single-view body rules, no step numbers, rails, `Took` lines or gutters.
//! The goldens in `tests/golden/hoocode-code-tui-widgets/` are the source for
//! these screens (peek, full, failed, agent) at 100 columns.

use std::cell::RefCell;
use std::rc::Rc;

use crate::support::{lock, strip, strip_all};
use hoocode_ai_types::Content;
use hoocode_code_subagents::live::apply;
use hoocode_code_tui_widgets::bash_execution::BashExecutionComponent;
use hoocode_code_tui_widgets::tool_chain::{ChainState, ToolChainComponent};
use hoocode_code_tui_widgets::tool_execution::{ToolExecutionComponent, ToolExecutionOptions};
use hoocode_code_tui_widgets::tool_row::{fit_header, header_spans, Mark};
use hoocode_code_tui_widgets::tool_signal::ToolResult;
use hoocode_code_tui_widgets::tools::{builtin_tool_definition, registered_tool_definition};
use hoocode_tui_render::golden::render_golden;
use hoocode_tui_render::{assert_golden, Component};
use serde_json::{json, Value};

const CWD: &str = "/work";

fn block_with_id(
    tool: &str,
    id: &str,
    args: Value,
    output: &str,
    details: Value,
    is_error: bool,
) -> Rc<RefCell<ToolExecutionComponent>> {
    let definition =
        builtin_tool_definition(tool).unwrap_or_else(|| registered_tool_definition(tool));
    let mut b = ToolExecutionComponent::new(
        tool,
        id,
        args,
        ToolExecutionOptions::default(),
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

fn block(
    tool: &str,
    args: Value,
    output: &str,
    details: Value,
    is_error: bool,
) -> Rc<RefCell<ToolExecutionComponent>> {
    block_with_id(tool, "id", args, output, details, is_error)
}

fn chain(blocks: Vec<Rc<RefCell<ToolExecutionComponent>>>) -> ToolChainComponent {
    let mut chain = ToolChainComponent::new();
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
    let mut c = config_run();
    assert_golden!("tool_rows_peek_40", render_golden(&mut c, 40));
}

#[test]
fn tool_rows_peek_60() {
    let _g = lock();
    let mut c = config_run();
    assert_golden!("tool_rows_peek_60", render_golden(&mut c, 60));
}

/// Read, Edit and Shell: the mockup's `peek` and `full` screens.
fn config_run() -> ToolChainComponent {
    chain(vec![
        block(
            "Read",
            json!({"file_path": "/work/src/config.rs"}),
            &numbered("setting_", 12),
            json!({}),
            false,
        ),
        block(
            "Edit",
            json!({"path": "/work/src/config.rs", "oldText": "a", "newText": "b"}),
            "ok",
            json!({"diff": DIFF}),
            false,
        ),
        block(
            "Shell",
            json!({"command": "cargo test -p hoocode-config 2>&1 | tail -40"}),
            &numbered("test config::tests::case_", 12),
            json!({"durationMs": 8200}),
            false,
        ),
    ])
}

#[test]
fn tool_rows_peek_100() {
    let _g = lock();
    let mut c = config_run();
    assert_golden!("tool_rows_peek_100", render_golden(&mut c, 100));
}

#[test]
fn tool_rows_full_100() {
    let _g = lock();
    let mut c = config_run();
    assert_golden!("tool_rows_full_100", render_golden(&mut c, 100));
}

#[test]
fn tool_rows_failed_100() {
    let _g = lock();
    let mut c = chain(
        vec![
            block(
                "Shell",
                json!({"command": "cargo test -p hoocode-parser escaped_quote"}),
                "a\nb\nc\nd\ne\nf\nerror: could not compile `hoocode-parser` (lib)\n\nCommand exited with code 1",
                json!({"durationMs": 3100}),
                true,
            ),
            block(
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
fn settled_agent(run: &str) -> Rc<RefCell<ToolExecutionComponent>> {
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
    let mut c = chain(vec![settled_agent("tool-rows-agent-peek")]);
    assert_golden!("tool_rows_agent_100", render_golden(&mut c, 100));
}

#[test]
fn tool_rows_agent_full_100() {
    let _g = lock();
    let mut c = chain(vec![settled_agent("tool-rows-agent-full")]);
    assert_golden!("tool_rows_agent_full_100", render_golden(&mut c, 100));
}

/// An open Agent row: the settled children read as one header row each at
/// indent 3, the failed one with its dim reason after two spaces.
#[test]
fn tool_rows_agent_open_100() {
    let _g = lock();
    let agent = settled_agent("tool-rows-agent-open");
    agent.borrow_mut().set_expanded(true);
    let mut c = chain(vec![agent]);
    assert_golden!("tool_rows_agent_open_100", render_golden(&mut c, 100));
}

#[test]
fn a_collapsed_run_is_tight_and_an_open_row_is_followed_by_one_blank() {
    let _g = lock();
    let mut c = config_run();
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
    let mut c = chain(vec![
        block(
            "Edit",
            json!({"path": "/work/a.rs", "oldText": "a", "newText": "b"}),
            "ok",
            json!({"diff": "-old\n+new"}),
            false,
        ),
        block(
            "Edit",
            json!({"path": "/work/b.rs", "oldText": "a", "newText": "b"}),
            "ok",
            json!({"diff": "+only added"}),
            false,
        ),
        block(
            "Read",
            json!({"file_path": "/work/empty.rs"}),
            "",
            json!({}),
            false,
        ),
    ]);
    let out = strip(&c.render(100).join("\n"));
    // Chain subjects read relative to the cwd, as the outline recap does.
    assert!(out.contains(" ● Edit a.rs  +1 −1"), "{out}");
    assert!(
        out.contains(" ● Edit b.rs  +1\n") || out.contains(" ● Edit b.rs  +1 "),
        "{out}"
    );
    assert!(!out.contains("−0") && !out.contains("+0"), "{out}");
    assert!(!out.contains("0 lines"), "{out}");
}

/// Structured header cut order (UI-21 P3 header slice): at 40 columns a Shell
/// failure keeps `⊗` and the primary `exit 1` while the secondary duration
/// drops right-to-left and the reason cuts, then drops.
#[test]
fn shell_failure_at_40_keeps_exit_while_dropping_duration_then_reason() {
    let _g = lock();
    let target = "cargo test -p hoocode-parser escaped_quote";
    let reason = "error: could not compile `hoocode-parser`";
    // Plain contract: duration drops first, then the reason drops; the
    // primary exit never drops and the command keeps its minimum.
    let row = fit_header(
        '⊗',
        "Shell",
        target,
        &["exit 1"],
        &["3.1s"],
        Some(reason),
        40,
    );
    assert!(row.contains('⊗'), "{row}");
    assert!(row.contains("exit 1"), "{row}");
    assert!(row.contains("cargo test"), "{row}");
    assert!(!row.contains("3.1s"), "{row}");
    assert!(!row.contains("could not compile"), "{row}");
    assert!(row.chars().count() <= 40, "{row}");
    // The styled header carries the same cut through spans: mark, bold tool,
    // target, two spaces, dim facts with the exit red on a failed row.
    let palette = hoocode_tui_ratatui::Palette::current();
    let spans = header_spans(
        Mark::Failed,
        "Shell",
        target,
        &["exit 1"],
        &["3.1s"],
        Some(reason),
        40,
        &palette,
    );
    let text: String = spans.iter().map(|s| s.content.to_string()).collect();
    assert!(text.contains('⊗'), "{text}");
    assert!(text.contains("exit 1"), "{text}");
    assert!(!text.contains("3.1s"), "{text}");
    assert!(!text.contains("could not compile"), "{text}");
    assert!(text.chars().count() <= 40, "{text}");
}

#[test]
fn a_long_label_cuts_to_one_row_and_keeps_the_facts() {
    let _g = lock();
    let command = format!(
        "cargo test {}",
        (1..=20)
            .map(|i| format!("flag{i:02}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let mut c = chain(vec![block(
        "Shell",
        json!({ "command": command }),
        "ok",
        json!({}),
        false,
    )]);
    let rows: Vec<String> = c
        .render(50)
        .iter()
        .map(|l| strip(l).trim_end().to_string())
        .collect();
    // A settled, collapsed Shell shows one header row and no body.
    let header: Vec<&String> = rows.iter().filter(|r| !r.is_empty()).collect();
    assert_eq!(header.len(), 1, "{rows:?}");
    assert!(header[0].starts_with(" ● Shell cargo test"), "{rows:?}");
    // The target cuts at a word while the primary facts stay on the row.
    assert!(header[0].contains('…'), "the target cuts: {rows:?}");
    assert!(header[0].contains("exit 0"), "{rows:?}");
    assert!(!header[0].contains("flag20"), "late words cut: {rows:?}");
    assert!(header[0].chars().count() <= 50, "{rows:?}");
}

#[test]
fn a_collapsed_row_shows_no_body_and_the_toggle_opens_it_whole() {
    let _g = lock();
    // Single view: every block starts as one row; the toggle opens the
    // whole output, however long, with no fold hint.
    for lines in [4, 12] {
        let b = block(
            "Shell",
            json!({"command": "x"}),
            &numbered("row-", lines),
            json!({}),
            false,
        );
        let mut c = chain(vec![b.clone()]);
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
    let b = ToolExecutionComponent::new(
        "Shell",
        "id",
        json!({"command": "sleep 5"}),
        ToolExecutionOptions::default(),
        Some(builtin_tool_definition("Shell").unwrap()),
        CWD,
    );
    let mut c = chain(vec![Rc::new(RefCell::new(b))]);
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

/// The focused header draws the row grammar's selection: `›` in the gutter
/// and the paneSel fill across the row (reverse video with no colour). An
/// expanded focused block keeps both: the `›` header and the open body.
#[test]
fn a_focused_header_draws_the_gutter_and_band() {
    let _g = lock();
    let first = block_with_id(
        "Read",
        "call-1",
        json!({"path": "src/a.ts"}),
        "line one\nline two",
        Value::Null,
        false,
    );
    let second = block_with_id(
        "Shell",
        "call-2",
        json!({"command": "npm test -- --watch"}),
        "ok",
        Value::Null,
        false,
    );
    let mut c = chain(vec![first.clone(), second.clone()]);
    // Unfocused: the gutter keeps column 0 and no row carries a fill.
    let plain: Vec<String> = c.render(80);
    assert_eq!(plain.len(), 3, "{plain:?}");
    for row in &plain {
        assert!(!strip(row).contains('›'), "{row}");
        assert!(!row.contains("48;2"), "{row}");
    }
    // Focused: the second header draws `›` and the paneSel fill; the
    // first header stays plain.
    c.set_focused_call(Some("call-2"));
    let rows: Vec<String> = c.render(80);
    assert_eq!(rows.len(), 3, "{rows:?}");
    let stripped: Vec<String> = rows
        .iter()
        .map(|line| strip(line).trim().to_string())
        .collect();
    assert!(stripped[1].starts_with('●'), "{}", stripped[1]);
    assert!(stripped[2].starts_with('›'), "{}", stripped[2]);
    assert!(!rows[1].contains("48;2"), "{}", rows[1]);
    assert!(rows[2].contains("48;2"), "{}", rows[2]);
    // Expanded and focused compose: the body opens under the `›` header.
    second.borrow_mut().set_expanded(true);
    let rows: Vec<String> = c.render(80);
    let text = strip_all(&rows);
    assert!(text.contains("ok"), "{text}");
    assert!(
        strip(&rows[2]).trim_start().starts_with('›'),
        "{}",
        strip(&rows[2])
    );
    assert!(rows[2].contains("48;2"), "{}", rows[2]);
    // With no colour the same row reads reverse video plus `›`.
    let chalk = ChalkGuard::off();
    let rows: Vec<String> = c.render(80);
    drop(chalk);
    assert!(
        strip(&rows[2]).trim_start().starts_with('›'),
        "{}",
        strip(&rows[2])
    );
    assert!(!rows[2].contains("48;2"), "{}", rows[2]);
    assert!(rows[2].contains("[0;7m"), "{}", rows[2]);
}

/// Disables chalk's colour switch until dropped, so the monochrome branch
/// of a paint test restores the shared switch even on failure.
struct ChalkGuard;

impl ChalkGuard {
    fn off() -> Self {
        hoocode_code_tui_theme::chalk::set_enabled(false);
        Self
    }
}

impl Drop for ChalkGuard {
    fn drop(&mut self) {
        hoocode_code_tui_theme::chalk::set_enabled(true);
    }
}

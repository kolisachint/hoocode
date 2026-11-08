//! The frozen-row defect (tui.md "Frozen rows", UI-21 P0).
//!
//! Blocks past the live window freeze and release their payloads. The chain
//! must still draw the settled mark and facts from the snapshot taken at
//! freeze: a done Edit keeps its green `●` and `+2 −2`, a failed Shell its
//! red `⊗` and `exit 1`, a Read its `1451 lines`. Renders twice, then once
//! more after a theme change: the rows must read the same.

use std::cell::RefCell;
use std::rc::Rc;

use crate::support::{lock, strip};
use hoocode_ai_types::Content;
use hoocode_code_subagents::live::apply;
use hoocode_code_tui_widgets::tool_chain::{ChainState, ToolChainComponent};
use hoocode_code_tui_widgets::tool_execution::{ToolExecutionComponent, ToolExecutionOptions};
use hoocode_code_tui_widgets::tool_signal::ToolResult;
use hoocode_code_tui_widgets::tools::{builtin_tool_definition, registered_tool_definition};
use hoocode_tui_render::Component;
use serde_json::{json, Value};

const CWD: &str = "/work";

fn block(
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

fn numbered(prefix: &str, n: usize) -> String {
    (1..=n)
        .map(|i| format!("{prefix}{i:02}"))
        .collect::<Vec<_>>()
        .join("\n")
}

const DIFF: &str = " 12     let mut cfg = Config::default();\n-14     let raw = read_file(path)?;\n-15     apply_env(&mut cfg);\n+14     apply_env(&mut cfg);\n+15     let raw = read_file(path)?;\n 16     parse_into(&mut cfg, &raw)?;\n 17     Ok(cfg)";

/// A settled Agent block with two calls of its own, the second failed.
fn agent_block() -> Rc<RefCell<ToolExecutionComponent>> {
    let run = "frozen-agent-row";
    apply(run, &json!({"type": "task_started"}));
    for (id, tool, fails) in [("s1", "Read", false), ("s2", "Grep", true)] {
        apply(
            run,
            &json!({"type": "tool_execution_start", "toolCallId": id, "toolName": tool, "args": {"pattern": "x"}}),
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

/// A settled chain with its payloads released, as `trim_transcript_memory`
/// leaves it: every block frozen.
fn frozen_chain() -> ToolChainComponent {
    let blocks = vec![
        block(
            "Edit",
            json!({"path": "/work/src/lex.rs", "oldText": "a", "newText": "b"}),
            "ok",
            json!({"diff": DIFF}),
            false,
        ),
        block(
            "Shell",
            json!({"command": "cargo test -p hoocode-parser escaped_quote"}),
            "a\nb\nc\nerror: could not compile `hoocode-parser` (lib)\n\nCommand exited with code 1",
            json!({"durationMs": 3100}),
            true,
        ),
        block(
            "Shell",
            json!({"command": "cargo test -p hoocode-config 2>&1 | tail -40"}),
            &numbered("out_", 40),
            json!({"durationMs": 8200}),
            false,
        ),
        agent_block(),
        block(
            "TodoWrite",
            json!({"todos": [
                {"status": "completed", "content": "a"},
                {"status": "completed", "content": "b"},
                {"status": "in_progress", "content": "c"},
                {"status": "pending", "content": "d"},
                {"status": "pending", "content": "e"},
            ]}),
            "[]",
            json!({}),
            false,
        ),
        block(
            "WebFetch",
            json!({"url": "https://example.com/docs"}),
            "docs text",
            json!({"tokenEstimate": 8100}),
            false,
        ),
        block(
            "WebSearch",
            json!({"query": "frozen rows snapshot"}),
            "result text",
            json!({"tokenEstimate": 2500}),
            false,
        ),
        block(
            "Read",
            json!({"file_path": "/work/crates/hoocode-code-tui-widgets/src/tool_chain.rs"}),
            &numbered("setting_", 1451),
            json!({}),
            false,
        ),
    ];
    for b in &blocks {
        b.borrow_mut().freeze();
    }
    let mut chain = ToolChainComponent::new();
    for b in blocks {
        chain.add(b);
    }
    chain.close(ChainState::Done);
    chain
}

fn text(chain: &mut ToolChainComponent, width: u16) -> String {
    strip(&chain.render(width).join("\n"))
}

#[test]
fn frozen_rows_keep_their_marks_and_facts() {
    let _g = lock();
    let mut chain = frozen_chain();
    // Twice: the second render draws from the capture, not the payloads.
    let first = text(&mut chain, 80);
    let second = text(&mut chain, 80);
    for out in [&first, &second] {
        assert!(
            out.lines()
                .any(|r| r.starts_with(" ● Edit ") && r.contains("+2 −2")),
            "done Edit keeps green ● and +2 −2:\n{out}"
        );
        // A failed row's facts overflow the 80-column header onto their own
        // line, so the reason and exit read from the whole render.
        assert!(
            out.lines().any(|r| r.starts_with(" ⊗ Shell ")),
            "failed Shell keeps red ⊗:\n{out}"
        );
        assert!(
            out.contains("exit 1 · error: could not compile"),
            "failed Shell keeps exit 1 and the first error line:\n{out}"
        );
        assert!(
            out.lines().any(|r| {
                r.starts_with(" ● Shell ") && r.contains("exit 0 · 40 lines · 8.2s")
            }),
            "done Shell keeps exit 0 · size · duration in order:\n{out}"
        );
        assert!(
            out.lines().any(|r| {
                r.starts_with(" ● Agent ") && r.contains("2 calls") && r.contains("1 failed")
            }),
            "Agent keeps N calls · M failed:\n{out}"
        );
        assert!(
            out.lines()
                .any(|r| { r.starts_with(" ● TodoWrite ") && r.contains("5 items · 2 done") }),
            "TodoWrite keeps N items · M done:\n{out}"
        );
        assert!(
            out.lines()
                .any(|r| { r.starts_with(" ● WebFetch ") && r.contains("~8.1k tokens") }),
            "WebFetch keeps ~N tokens:\n{out}"
        );
        assert!(
            out.lines()
                .any(|r| { r.starts_with(" ● WebSearch ") && r.contains("~2.5k tokens") }),
            "WebSearch keeps ~N tokens:\n{out}"
        );
        assert!(
            out.lines()
                .any(|r| r.starts_with(" ● Read ") && r.contains("1451 lines")),
            "Read keeps its line count:\n{out}"
        );
    }
}

#[test]
fn frozen_rows_survive_a_theme_change() {
    let _g = lock();
    let mut chain = frozen_chain();
    let dark = text(&mut chain, 80);
    hoocode_code_tui_theme::init_theme(Some("light"), false);
    chain.invalidate();
    let light = text(&mut chain, 80);
    assert_eq!(dark, light, "same rows under a new theme");
    assert!(
        light.contains("exit 1")
            && light.contains("error: could not compile")
            && light.contains("exit 0 · 40 lines · 8.2s")
            && light.contains("2 calls")
            && light.contains("1 failed")
            && light.contains("5 items · 2 done")
            && light.contains("~8.1k tokens")
            && light.contains("~2.5k tokens")
            && light.contains("+2 −2")
            && light.contains("1451 lines"),
        "facts survive the theme change:\n{light}"
    );
}

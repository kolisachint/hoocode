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
use hoocode_code_tui_widgets::tool_chain::ToolChainComponent;
use hoocode_code_tui_widgets::tool_chain_summary::ChainState;
use hoocode_code_tui_widgets::tool_execution::{ToolExecutionComponent, ToolExecutionOptions};
use hoocode_code_tui_widgets::tool_output_view::ToolOutputView;
use hoocode_code_tui_widgets::tool_signal::ToolResult;
use hoocode_code_tui_widgets::tools::{builtin_tool_definition, registered_tool_definition};
use hoocode_tui_render::Component;
use serde_json::{json, Value};

const CWD: &str = "/work";
const VIEW: ToolOutputView = ToolOutputView::Peek;

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
        ToolExecutionOptions {
            view: VIEW,
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

fn numbered(prefix: &str, n: usize) -> String {
    (1..=n)
        .map(|i| format!("{prefix}{i:02}"))
        .collect::<Vec<_>>()
        .join("\n")
}

const DIFF: &str = " 12     let mut cfg = Config::default();\n-14     let raw = read_file(path)?;\n-15     apply_env(&mut cfg);\n+14     apply_env(&mut cfg);\n+15     let raw = read_file(path)?;\n 16     parse_into(&mut cfg, &raw)?;\n 17     Ok(cfg)";

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
    let mut chain = ToolChainComponent::new(VIEW);
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
        assert!(
            out.lines()
                .any(|r| { r.starts_with(" ⊗ Shell ") && r.contains("exit 1") }),
            "failed Shell keeps red ⊗ and exit 1:\n{out}"
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
        light.contains("exit 1") && light.contains("+2 −2") && light.contains("1451 lines"),
        "facts survive the theme change:\n{light}"
    );
}

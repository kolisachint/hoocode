//! N11 follow-up: the Edit and AgentOutput blocks honour the peek dial
//! through `peek_block`, as Read and WebFetch do.

use crate::support::{lock, strip};
use hoocode_ai_types::Content;
use hoocode_code_tui_widgets::tool_execution::{
    ToolRenderContext, ToolRenderResultOptions, ToolResultView,
};
use hoocode_code_tui_widgets::tool_output_view::PEEK_LINES;
use hoocode_code_tui_widgets::tools::builtin_tool_definition;
use hoocode_code_tui_widgets::tools::subagent::format_task_output_result;
use serde_json::{json, Value};

const CWD: &str = "/work/project";

fn text_content(text: &str) -> Vec<Content> {
    vec![Content::Text(hoocode_ai_types::TextContent {
        text: text.to_string(),
        ..Default::default()
    })]
}

/// `n` lines of a subagent's output card body.
fn card_text(n: usize) -> String {
    (1..=n)
        .map(|i| format!("output line {i}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A unified-style diff of `n` removed and `n` added lines.
fn diff_text(n: usize) -> String {
    let removed = (1..=n).map(|i| format!("-{i:>4} old line {i}"));
    let added = (1..=n).map(|i| format!("+{i:>4} new line {i}"));
    removed.chain(added).collect::<Vec<_>>().join("\n")
}

mod agent_output {
    use super::*;

    /// The roster (`list`) keeps the peek trim: the first PEEK_LINES rows,
    /// then a count of the rest. The full stop keeps every row.
    #[test]
    fn roster_is_peek_trimmed() {
        let _g = lock();
        let mut roster = vec!["8 background subagents (8 running):".to_string()];
        roster.extend((1..=7).map(|i| format!("- explore#{i}  running  3s")));
        let content = text_content(&roster.join("\n"));
        let details = json!({"status": "list", "ok": true});
        let view = ToolResultView {
            content: &content,
            details: &details,
            text: &hoocode_code_tui_widgets::tool_signal::ResultText::default(),
        };
        let peek = strip(&format_task_output_result(&view, false));
        assert!(peek.contains("- explore#4  running  3s"), "{peek}");
        assert!(!peek.contains("- explore#5  running  3s"), "{peek}");
        assert!(peek.contains("more lines"), "{peek}");
        let _ = (card_text(1), PEEK_LINES);
    }

    /// A roster line names the run's description (2026-10-30): the label,
    /// description and status still parse, the description is dim and the
    /// status keeps its colour.
    #[test]
    fn roster_line_with_a_description_keeps_its_colours() {
        let _g = lock();
        let t = hoocode_code_tui_theme::theme();
        let content = text_content(
            "1 background subagent (1 running):\n- explore#2 (\"audit the db layer\")  running  3s",
        );
        let details = json!({"status": "list", "ok": true});
        let view = ToolResultView {
            content: &content,
            details: &details,
            text: &hoocode_code_tui_widgets::tool_signal::ResultText::default(),
        };
        let raw = format_task_output_result(&view, false);
        assert!(
            strip(&raw).contains("- explore#2 (\"audit the db layer\")  running  3s"),
            "{raw}"
        );
        assert!(
            raw.contains(&t.fg("dim", " (\"audit the db layer\")")),
            "{raw}"
        );
        assert!(raw.contains(&t.fg("warning", "running")), "{raw}");
    }
}

mod edit {
    use super::*;

    /// The Edit definition's result slot, for a result that carries `diff`
    /// and no call slot (so the diff is shown as the result).
    fn render_result(diff: &str, expanded: bool) -> String {
        let def = builtin_tool_definition("Edit").expect("Edit renderer");
        let args = json!({"path": "src/a.rs", "oldText": "x", "newText": "y"});
        let details = json!({"diff": diff});
        let content: Vec<Content> = Vec::new();
        let mut state = serde_json::Map::new();
        let mut objects = std::collections::HashMap::new();
        let mut ctx = ToolRenderContext {
            args: &args,
            tool_call_id: "t",
            last_component: None,
            state: &mut state,
            objects: &mut objects,
            cwd: CWD,
            execution_started: true,
            args_complete: true,
            is_partial: false,
            expanded,
            show_images: false,
            is_error: false,
            has_result: true,
        };
        let result = ToolResultView {
            content: &content,
            details: &details,
            text: &hoocode_code_tui_widgets::tool_signal::ResultText::default(),
        };
        let options = ToolRenderResultOptions {
            expanded,
            is_partial: false,
        };
        let render = def.render_result.expect("Edit result slot");
        let handle = render(&result, options, &mut ctx).expect("render");
        let lines = handle.borrow_mut().render(120);
        lines.join("\n")
    }

    #[test]
    fn result_diff_follows_the_peek_dial() {
        let _g = lock();
        let diff = diff_text(8);
        let peek = strip(&render_result(&diff, false));
        assert!(peek.contains("more lines"), "{peek}");
        assert!(peek.contains("old line 1"), "{peek}");
        assert!(!peek.contains("new line 8"), "{peek}");
    }

    /// The full stop keeps the whole diff and no hint.
    #[test]
    fn result_diff_is_whole_at_the_full_stop() {
        let _g = lock();
        let diff = diff_text(8);
        let full = strip(&render_result(&diff, true));
        assert!(full.contains("new line 8"), "{full}");
        assert!(!full.contains("more lines"), "{full}");
    }

    /// A mid-file edit: the window starts at the first change (keeping
    /// one context line), so the peek shows the `-`/`+` lines instead of
    /// the gap and context above them.
    #[test]
    fn result_diff_window_starts_at_first_change() {
        let _g = lock();
        let diff = [
            "  1 a", "  2 b", "  3 c", "  4 d", "  5 e", "- 6 old", "+ 6 new", "  7 f",
        ]
        .join("\n");
        let peek = strip(&render_result(&diff, false));
        assert!(peek.contains("- 6 old"), "{peek}");
        assert!(peek.contains("+ 6 new"), "{peek}");
        assert!(peek.contains("  5 e"), "{peek}");
        assert!(!peek.contains("  1 a"), "{peek}");
        assert!(!peek.contains("more lines"), "{peek}");
    }

    #[test]
    fn short_result_diff_has_no_hint_at_peek() {
        let _g = lock();
        let peek = strip(&render_result(&diff_text(1), false));
        assert!(peek.contains("new line 1"), "{peek}");
        assert!(!peek.contains("more lines"), "{peek}");
    }

    #[test]
    fn call_preview_follows_the_peek_dial() {
        let _g = lock();
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sample.txt");
        let original: String = (1..=10).map(|i| format!("old {i}\n")).collect();
        std::fs::write(&path, &original).expect("write");
        let new_text: String = (1..=10).map(|i| format!("new {i}\n")).collect();
        let args: Value = json!({
            "path": path.to_string_lossy(),
            "edits": [{"oldText": original, "newText": new_text}],
        });
        let def = builtin_tool_definition("Edit").expect("Edit renderer");

        {
            let expanded = false;
            let mut state = serde_json::Map::new();
            let mut objects = std::collections::HashMap::new();
            let mut ctx = ToolRenderContext {
                args: &args,
                tool_call_id: "t",
                last_component: None,
                state: &mut state,
                objects: &mut objects,
                cwd: "/",
                execution_started: true,
                args_complete: true,
                is_partial: false,
                expanded,
                show_images: false,
                has_result: false,
                is_error: false,
            };
            let call = (def.render_call.as_ref().expect("Edit call slot"))(&args, &mut ctx)
                .expect("render call");
            let plain = strip(&call.borrow_mut().render(120).join("\n"));
            assert!(!plain.contains("new 10"), "{plain}");
            assert!(plain.contains("more lines"), "{plain}");
        }
    }
}

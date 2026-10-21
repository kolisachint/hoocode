//! The three transcript layers: radar, peek, and full (peek with nothing cut).

use std::cell::RefCell;
use std::rc::Rc;

use crate::support::{lock, strip};
use hoocode_ai_types::{Content, ImageContent};
use hoocode_code_tui_widgets::fold::{fold_rows, fold_rows_tail, parse_more_lines};
use hoocode_code_tui_widgets::tool_chain::{ToolBlock, ToolChainComponent};
use hoocode_code_tui_widgets::tool_chain_summary::ChainState;
use hoocode_code_tui_widgets::tool_execution::{ToolExecutionComponent, ToolExecutionOptions};
use hoocode_code_tui_widgets::tool_output_view::{
    cycle_tool_output_view, ToolOutputView, TOOL_OUTPUT_VIEWS,
};
use hoocode_code_tui_widgets::tool_signal::ToolResult;
use hoocode_code_tui_widgets::tools::registered_tool_definition;
use hoocode_code_tui_widgets::trajectory::MissionHeader;
use hoocode_tui_render::Component;
use serde_json::{json, Value};

use ToolOutputView::{Full, Peek, Radar};

const PNG_1X1: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

fn cwd() -> String {
    std::env::current_dir()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

fn rows_of(n: usize) -> String {
    (1..=n)
        .map(|i| format!("row-{i:02}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn block(view: ToolOutputView, tool: &str, args: Value, content: Vec<Content>) -> ToolBlock {
    let mut component = ToolExecutionComponent::new(
        tool,
        "t-0",
        args,
        ToolExecutionOptions {
            view,
            ..Default::default()
        },
        Some(registered_tool_definition(tool)),
        &cwd(),
    );
    component.update_result(
        ToolResult {
            content,
            details: json!({}),
            is_error: false,
        },
        false,
    );
    Rc::new(RefCell::new(component))
}

fn chain_with(view: ToolOutputView, blocks: Vec<ToolBlock>) -> ToolChainComponent {
    let mut chain = ToolChainComponent::new(view);
    for block in blocks {
        chain.add(block);
    }
    chain.close(ChainState::Done);
    chain
}

fn render(chain: &mut ToolChainComponent, width: u16) -> Vec<String> {
    chain
        .render(width)
        .iter()
        .map(|l| strip(l))
        .filter(|l| !l.trim().is_empty())
        .collect()
}

fn read_chain(view: ToolOutputView, lines: usize) -> ToolChainComponent {
    chain_with(
        view,
        vec![block(
            view,
            "Read",
            json!({"file_path": "notes.txt"}),
            vec![Content::text(rows_of(lines))],
        )],
    )
}

#[test]
fn the_dial_steps_radar_peek_full_and_wraps() {
    assert_eq!(TOOL_OUTPUT_VIEWS, [Radar, Peek, Full]);
    let mut view = Radar;
    let mut seen = Vec::new();
    for _ in 0..4 {
        view = cycle_tool_output_view(view, true);
        seen.push(view);
    }
    assert_eq!(seen, [Peek, Full, Radar, Peek]);
}

#[test]
fn full_shows_a_long_tool_output_whole_while_peek_folds_it() {
    let _g = lock();
    let peek = render(&mut read_chain(Peek, 40), 100).join("\n");
    assert!(peek.contains("row-01"), "{peek}");
    assert!(!peek.contains("row-40"), "{peek}");
    assert!(peek.contains("more lines"), "{peek}");

    let full = render(&mut read_chain(Full, 40), 100).join("\n");
    for i in 1..=40 {
        assert!(full.contains(&format!("row-{i:02}")), "row {i}: {full}");
    }
    assert!(!full.contains("more line"), "{full}");
    assert!(!full.contains('…'), "{full}");
}

#[test]
fn the_chain_fold_reports_the_true_hidden_count_once() {
    let _g = lock();
    let rows = render(&mut read_chain(Peek, 40), 100);
    let hints: Vec<&String> = rows.iter().filter(|r| r.contains("more line")).collect();
    assert_eq!(hints.len(), 1, "the hint shows once: {rows:?}");
    // 40 lines, the first three shown on the rail: 37 hidden. The block's own
    // `… 35 more lines` is folded into the count, not shown next to it.
    assert_eq!(
        parse_more_lines(hints[0].trim_start_matches(['┊', ' '])),
        Some(37)
    );
    assert_eq!(
        rows.iter().filter(|r| r.contains("row-0")).count(),
        3,
        "{rows:?}"
    );
}

#[test]
fn fold_rows_adds_the_renderers_own_hint_to_its_count() {
    let rows: Vec<String> = ["a", "b", "c", "d", "… 7 more lines"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let folded = fold_rows(rows.clone(), Some(3));
    assert_eq!(folded.shown, ["a", "b", "c"]);
    assert_eq!(folded.hidden, 8);
    let whole = fold_rows(rows.clone(), None);
    assert_eq!(whole.shown, rows);
    assert_eq!(whole.hidden, 0);
}

#[test]
fn a_long_tool_header_wraps_in_peek_and_full_instead_of_being_cut() {
    let _g = lock();
    let words: Vec<String> = (1..=30).map(|i| format!("flag{i:02}")).collect();
    let command = format!("cargo test {}", words.join(" "));
    for view in [Peek, Full] {
        let shell = block(
            view,
            "Shell",
            json!({ "command": command }),
            vec![Content::text("ok")],
        );
        let mut chain = chain_with(view, vec![shell]);
        let rows = render(&mut chain, 60);
        let joined = rows.join(" ");
        for word in &words {
            assert!(
                joined.contains(word.as_str()),
                "{view:?} lost {word}: {rows:?}"
            );
        }
        assert!(!rows.iter().any(|r| r.contains('…')), "{view:?}: {rows:?}");
        assert!(rows[0].contains("01"), "the step number leads: {rows:?}");
        assert!(rows.len() >= 3, "the header took several rows: {rows:?}");
        // The wrapped header rows are not body: the output still follows.
        assert!(joined.contains("ok"), "{rows:?}");
    }
}

#[test]
fn a_long_user_prompt_wraps_whole_in_peek_and_full() {
    let _g = lock();
    let words: Vec<String> = (1..=40).map(|i| format!("word{i:02}")).collect();
    let prompt = format!("{}\nsecond line of the prompt", words.join(" "));
    for view in [Peek, Full] {
        let mut header = MissionHeader::new(1, &prompt, None);
        header.set_view(view);
        let lines = header.render(50);
        let text = strip(&lines.join("\n"));
        for word in &words {
            assert!(text.contains(word.as_str()), "{view:?} lost {word}: {text}");
        }
        assert!(text.contains("second line of the prompt"), "{text}");
        assert!(!text.contains('…'), "{text}");
        assert!(lines.len() >= 4, "{text}");
    }
}

fn image_block(view: ToolOutputView) -> ToolBlock {
    block(
        view,
        "Read",
        json!({"file_path": "shot.png"}),
        vec![
            Content::text(rows_of(8)),
            Content::Image(ImageContent {
                data: PNG_1X1.into(),
                media_type: "image/png".into(),
            }),
        ],
    )
}

#[test]
fn an_image_result_is_not_folded_with_the_body() {
    let _g = lock();
    hoocode_tui_images::set_capabilities(hoocode_tui_images::TerminalCapabilities {
        images: Some(hoocode_tui_images::ImageProtocol::Kitty),
        true_color: true,
        hyperlinks: false,
    });
    let mut chain = chain_with(Peek, vec![image_block(Peek)]);
    let lines = chain.render(100);
    // The body folds; the image rows after it do not.
    let fold = lines
        .iter()
        .position(|l| strip(l).contains("more line"))
        .expect("the text body folds");
    assert!(
        lines[fold + 1..]
            .iter()
            .any(|l| hoocode_tui_images::is_image_line(l)),
        "the image draws after the fold: {lines:?}"
    );
    hoocode_tui_images::set_capabilities(hoocode_tui_images::TerminalCapabilities {
        images: None,
        true_color: true,
        hyperlinks: false,
    });
}

#[test]
fn the_image_placeholder_shows_when_the_terminal_draws_no_images() {
    let _g = lock();
    for view in [Peek, Full] {
        let mut chain = chain_with(view, vec![image_block(view)]);
        let rows = render(&mut chain, 100);
        assert!(
            rows.iter().any(|r| r.contains("[Image: ")),
            "{view:?} dropped the placeholder: {rows:?}"
        );
    }
}

#[test]
fn a_subagent_result_is_whole_in_full_and_summarised_in_peek() {
    let _g = lock();
    let body = (1..=12)
        .map(|i| format!("finding {i:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    let make = |view| {
        chain_with(
            view,
            vec![block(
                view,
                "Agent",
                json!({"subagent_type": "explore"}),
                vec![Content::text(body.clone())],
            )],
        )
    };
    let peek = render(&mut make(Peek), 80).join("\n");
    assert!(!peek.contains("finding 12"), "{peek}");
    let full = render(&mut make(Full), 80).join("\n");
    for i in 1..=12 {
        assert!(full.contains(&format!("finding {i:02}")), "{full}");
    }
    assert!(!full.contains("more line"), "{full}");
}

#[test]
fn a_frozen_block_keeps_the_whole_output_for_the_full_stop() {
    let _g = lock();
    let shell = block(
        Peek,
        "Shell",
        json!({"command": "seq 1 30"}),
        vec![Content::text(rows_of(30))],
    );
    shell.borrow_mut().freeze();
    let mut chain = chain_with(Peek, vec![shell.clone()]);
    let peek = render(&mut chain, 100).join("\n");
    assert!(peek.contains("27 earlier lines"), "{peek}");
    assert!(
        peek.contains("row-30"),
        "peek shows the end of the output: {peek}"
    );
    assert!(
        !peek.contains("row-01"),
        "peek shows the end of the output only: {peek}"
    );
    chain.set_view(Full);
    let full = render(&mut chain, 100).join("\n");
    for i in 1..=30 {
        assert!(full.contains(&format!("row-{i:02}")), "row {i}: {full}");
    }
}

#[test]
fn full_wraps_a_long_output_row_where_peek_keeps_it_on_the_rail() {
    let _g = lock();
    let long = "Q".repeat(150);
    let chain = |view| {
        chain_with(
            view,
            vec![block(
                view,
                "Read",
                json!({"file_path": "notes.txt"}),
                vec![Content::text(long.clone())],
            )],
        )
    };
    let full = render(&mut chain(Full), 60).join("");
    assert_eq!(full.matches('Q').count(), 150, "{full}");
    assert!(!full.contains('…'), "{full}");
}

#[test]
fn an_image_only_result_shows_just_its_placeholder() {
    let _g = lock();
    let image = Content::Image(ImageContent {
        data: PNG_1X1.into(),
        media_type: "image/png".into(),
    });
    for tool in ["Read", "mystery"] {
        let view = Peek;
        let mut chain = chain_with(
            view,
            vec![block(
                view,
                tool,
                json!({"file_path": "shot.png"}),
                vec![image.clone()],
            )],
        );
        let rows = render(&mut chain, 100);
        let body: Vec<&String> = rows.iter().filter(|r| r.contains("┊")).collect();
        assert_eq!(body.len(), 1, "{tool}: {rows:?}");
        assert!(body[0].contains("[Image: "), "{tool}: {rows:?}");
        assert!(
            !rows.iter().any(|r| r.contains("more line")),
            "{tool}: {rows:?}"
        );
    }
}

#[test]
fn a_frozen_block_wraps_at_a_narrower_terminal_in_full() {
    let _g = lock();
    let long = "Q".repeat(120);
    let read = block(
        Full,
        "Read",
        json!({"file_path": "notes.txt"}),
        vec![Content::text(long)],
    );
    read.borrow_mut().freeze();
    let mut chain = chain_with(Full, vec![read]);
    let wide = render(&mut chain, 160).join("");
    assert_eq!(wide.matches('Q').count(), 120, "{wide}");
    let narrow = render(&mut chain, 50);
    assert_eq!(narrow.join("").matches('Q').count(), 120, "{narrow:?}");
    assert!(!narrow.iter().any(|r| r.contains('…')), "{narrow:?}");
}

/// A Shell block with a stored duration, as a rebuilt session carries it.
fn shell_block(view: ToolOutputView, output: &str, is_error: bool) -> ToolBlock {
    let mut component = ToolExecutionComponent::new(
        "Shell",
        "t-0",
        json!({"command": "cargo test"}),
        ToolExecutionOptions {
            view,
            ..Default::default()
        },
        Some(registered_tool_definition("Shell")),
        &cwd(),
    );
    component.update_result(
        ToolResult {
            content: vec![Content::text(output)],
            details: json!({"durationMs": 8200}),
            is_error,
        },
        false,
    );
    Rc::new(RefCell::new(component))
}

#[test]
fn a_shell_step_shows_the_end_of_its_output_and_counts_only_hidden_output_lines() {
    let _g = lock();
    let mut chain = chain_with(Peek, vec![shell_block(Peek, &rows_of(40), false)]);
    let rows = render(&mut chain, 100);
    let hint = rows
        .iter()
        .position(|r| r.contains("earlier lines"))
        .unwrap_or_else(|| panic!("no fold hint: {rows:?}"));
    // 40 lines, the last three shown: 37 hidden, the footer not among them.
    assert_eq!(
        parse_more_lines(rows[hint].trim_start_matches(['┊', ' '])),
        Some(37)
    );
    let shown: Vec<&String> = rows.iter().filter(|r| r.contains("row-")).collect();
    assert_eq!(shown.len(), 3, "{rows:?}");
    assert!(
        rows[hint + 1].contains("row-38"),
        "the hint sits above: {rows:?}"
    );
    assert!(rows.last().unwrap().contains("Took 8.2s"), "{rows:?}");
    assert!(!rows.iter().any(|r| r.contains("more line")), "{rows:?}");
}

#[test]
fn a_failed_shell_step_names_its_exit_status_and_keeps_the_last_lines() {
    let _g = lock();
    let output = "error[E0308]: mismatched types\n   --> src/lex.rs:88:21\n    |\n\
                  88  |     return Ok(Token::Str(text));\n    |\n\
                  error: could not compile `parser`\n\nCommand exited with code 1";
    let mut chain = chain_with(Peek, vec![shell_block(Peek, output, true)]);
    let rows = render(&mut chain, 100);
    assert!(
        rows[0].contains("HIT") && rows[0].contains("exit 1"),
        "{rows:?}"
    );
    let joined = rows.join("\n");
    assert!(joined.contains("earlier lines"), "{joined}");
    assert!(joined.contains("error: could not compile"), "{joined}");
    assert!(joined.contains("Command exited with code 1"), "{joined}");
    assert!(!joined.contains("mismatched types"), "{joined}");
}

#[test]
fn an_interrupted_step_is_no_hit_and_leaves_the_reason_to_the_message() {
    let _g = lock();
    let shell = shell_block(Peek, "Operation aborted", true);
    shell.borrow_mut().set_interrupted();
    assert!(!shell.borrow().is_error());
    assert!(!shell.borrow().chain_entry().is_error);
    let mut chain = chain_with(Peek, vec![shell]);
    let rows = render(&mut chain, 100);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(
        rows[0].contains('⊘') && rows[0].contains("interrupted"),
        "{rows:?}"
    );
    assert!(!rows[0].contains("HIT"), "{rows:?}");
}

#[test]
fn fold_rows_tail_keeps_the_last_rows_and_adds_the_renderers_hint() {
    let rows: Vec<String> = ["… 7 earlier lines", "a", "b", "c", "d"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let folded = fold_rows_tail(rows.clone(), Some(3));
    assert_eq!(folded.shown, ["b", "c", "d"]);
    assert_eq!(folded.hidden, 8);
    let whole = fold_rows_tail(rows, None);
    assert_eq!(whole.hidden, 0);
    assert_eq!(parse_more_lines("… 1 earlier line"), Some(1));
}

/// An Agent block whose run (`run`) settled, with two calls of its own, the
/// second failed.
fn settled_agent(view: ToolOutputView, run: &str) -> ToolBlock {
    use hoocode_code_subagents::live::apply;
    apply(run, &json!({"type": "task_started"}));
    for (id, tool, fails) in [("s1", "Read", false), ("s2", "Grep", true)] {
        apply(
            run,
            &json!({"type": "tool_execution_start", "toolCallId": id, "toolName": tool, "args": {"pattern": "x"}}),
        );
        apply(
            run,
            &json!({"type": "tool_execution_end", "toolCallId": id, "isError": fails,
                    "result": {"content": [{"type": "text", "text": "bad regex"}]}}),
        );
    }
    apply(run, &json!({"type": "task_done"}));
    let mut component = ToolExecutionComponent::new(
        "Agent",
        "t-agent",
        json!({"subagent_type": "explore", "description": "look around"}),
        ToolExecutionOptions {
            view,
            ..Default::default()
        },
        Some(registered_tool_definition("Agent")),
        &cwd(),
    );
    component.update_result(
        ToolResult {
            content: vec![Content::text("Found it.")],
            details: json!({ "poolTaskId": run }),
            is_error: false,
        },
        false,
    );
    Rc::new(RefCell::new(component))
}

#[test]
fn a_settled_agent_keeps_its_failed_calls_in_peek_and_every_call_in_full() {
    let _g = lock();
    let mut peek = chain_with(Peek, vec![settled_agent(Peek, "it-agent-peek")]);
    let rows = render(&mut peek, 100).join("\n");
    assert!(rows.contains("⊗ 01.2 Grep"), "{rows}");
    assert!(rows.contains("→ bad regex"), "{rows}");
    assert!(rows.contains("HIT"), "{rows}");
    assert!(
        !rows.contains("01.1 Read"),
        "peek shows problems only: {rows}"
    );

    let mut full = chain_with(Full, vec![settled_agent(Full, "it-agent-full")]);
    let rows = render(&mut full, 100).join("\n");
    assert!(rows.contains("● 01.1 Read"), "{rows}");
    assert!(rows.contains("⊗ 01.2 Grep"), "{rows}");

    // Radar: the problem row only, under the agent step's number.
    let mut radar = chain_with(Radar, vec![settled_agent(Radar, "it-agent-radar")]);
    radar.set_orphan(false);
    let rows = render(&mut radar, 100);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(
        rows[0].contains("⊗ 01.2 Grep") && rows[0].contains("HIT"),
        "{rows:?}"
    );
}

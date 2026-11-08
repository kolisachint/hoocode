//! The single transcript view: collapsed rows, the toggle, whole bodies.

use std::cell::RefCell;
use std::rc::Rc;

use crate::support::{lock, strip};
use hoocode_ai_types::{Content, ImageContent};
use hoocode_code_tui_widgets::fold::{fold_rows, fold_rows_tail, parse_more_lines};
use hoocode_code_tui_widgets::tool_chain::{ChainState, ToolBlock, ToolChainComponent};
use hoocode_code_tui_widgets::tool_execution::{ToolExecutionComponent, ToolExecutionOptions};
use hoocode_code_tui_widgets::tool_signal::ToolResult;
use hoocode_code_tui_widgets::tools::registered_tool_definition;
use hoocode_code_tui_widgets::trajectory::MissionHeader;
use hoocode_tui_render::Component;
use serde_json::{json, Value};

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

fn block(tool: &str, args: Value, content: Vec<Content>) -> ToolBlock {
    let mut component = ToolExecutionComponent::new(
        tool,
        "t-0",
        args,
        ToolExecutionOptions::default(),
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

fn chain_with(blocks: Vec<ToolBlock>) -> ToolChainComponent {
    let mut chain = ToolChainComponent::new();
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

/// A chain of one Write call whose content is `lines` rows: a tool that shows
/// its first lines at peek (a Read shows no body there).
fn read_chain(lines: usize) -> ToolChainComponent {
    chain_with(vec![block(
        "Write",
        json!({"file_path": "notes.txt", "content": rows_of(lines)}),
        vec![Content::text("ok")],
    )])
}

#[test]
fn collapsed_hides_the_body_and_the_toggle_opens_it_whole() {
    let _g = lock();
    let shut = render(&mut read_chain(40), 100).join("\n");
    assert!(!shut.contains("row-01"), "{shut}");
    assert!(!shut.contains("more line"), "{shut}");

    let mut open = read_chain(40);
    open.tool_blocks()[0].borrow_mut().set_expanded(true);
    let whole = render(&mut open, 100).join("\n");
    for i in 1..=40 {
        assert!(whole.contains(&format!("row-{i:02}")), "row {i}: {whole}");
    }
    assert!(!whole.contains("more line"), "{whole}");
    assert!(!whole.contains('…'), "{whole}");
}

#[test]
fn a_collapsed_chain_hides_every_row_and_the_toggle_opens_them_all() {
    let _g = lock();
    let mut chain = read_chain(40);
    let shut = render(&mut chain, 100);
    assert!(
        !shut.iter().any(|r| r.contains("more line")),
        "no fold hint while shut: {shut:?}"
    );
    assert!(
        !shut.iter().any(|r| r.contains("row-0")),
        "no body rows while shut: {shut:?}"
    );
    chain.tool_blocks()[0].borrow_mut().set_expanded(true);
    let rows = render(&mut chain, 100);
    assert_eq!(
        rows.iter().filter(|r| r.contains("row-")).count(),
        40,
        "{rows:?}"
    );
    assert!(
        !rows.iter().any(|r| r.contains("more line")),
        "open shows every row, no hint: {rows:?}"
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
fn a_long_tool_header_cuts_to_one_row_until_the_toggle_opens_it() {
    let _g = lock();
    let words: Vec<String> = (1..=30).map(|i| format!("flag{i:02}")).collect();
    let command = format!("cargo test {}", words.join(" "));
    let shell = block(
        "Shell",
        json!({ "command": command }),
        vec![Content::text("ok")],
    );
    let mut chain = chain_with(vec![shell]);
    // Collapsed: the single cut header, no body.
    let rows = render(&mut chain, 60);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(rows[0].contains("● Shell"), "the glyph leads: {rows:?}");
    assert!(rows[0].contains('…'), "the target cuts: {rows:?}");
    assert!(rows[0].chars().count() <= 60, "the header fits: {rows:?}");
    assert!(rows[0].contains("exit 0"), "primary facts stay: {rows:?}");
    assert!(!rows[0].contains("flag30"), "late words cut: {rows:?}");
    // The cut header is not body: the toggle opens the output whole.
    chain.tool_blocks()[0].borrow_mut().set_expanded(true);
    let rows = render(&mut chain, 60);
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(rows[1].trim(), "ok", "{rows:?}");
}

#[test]
fn a_long_user_prompt_wraps_whole() {
    let _g = lock();
    let words: Vec<String> = (1..=40).map(|i| format!("word{i:02}")).collect();
    let prompt = format!("{}\nsecond line of the prompt", words.join(" "));
    let mut header = MissionHeader::new(&prompt);
    let lines = header.render(50);
    let text = strip(&lines.join("\n"));
    for word in &words {
        assert!(text.contains(word.as_str()), "lost {word}: {text}");
    }
    assert!(text.contains("second line of the prompt"), "{text}");
    assert!(!text.contains('…'), "{text}");
    assert!(lines.len() >= 4, "{text}");
}

fn image_block() -> ToolBlock {
    block(
        "mystery",
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
    let mut chain = chain_with(vec![image_block()]);
    chain.tool_blocks()[0].borrow_mut().set_expanded(true);
    let lines = chain.render(100);
    // The open body shows whole; the image still draws after the text.
    let last_text = lines
        .iter()
        .rposition(|l| strip(l).contains("row-08"))
        .expect("the text body shows whole");
    assert!(
        lines[last_text + 1..]
            .iter()
            .any(|l| hoocode_tui_images::is_image_line(l)),
        "the image draws after the body: {lines:?}"
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
    let block = image_block();
    block.borrow_mut().set_expanded(true);
    let mut chain = chain_with(vec![block]);
    let rows = render(&mut chain, 100);
    assert!(
        rows.iter().any(|r| r.contains("[Image: ")),
        "dropped the placeholder: {rows:?}"
    );
}

#[test]
fn a_subagent_result_hides_until_the_toggle_opens_it_whole() {
    let _g = lock();
    let body = (1..=12)
        .map(|i| format!("finding {i:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    let make = || {
        chain_with(vec![block(
            "Agent",
            json!({"subagent_type": "explore"}),
            vec![Content::text(body.clone())],
        )])
    };
    let shut = render(&mut make(), 80).join("\n");
    assert!(!shut.contains("finding 01"), "{shut}");
    let mut open = make();
    open.tool_blocks()[0].borrow_mut().set_expanded(true);
    let whole = render(&mut open, 80).join("\n");
    for i in 1..=12 {
        assert!(whole.contains(&format!("finding {i:02}")), "{whole}");
    }
    assert!(!whole.contains("more line"), "{whole}");
}

#[test]
fn a_frozen_block_keeps_the_whole_output_for_the_toggle() {
    let _g = lock();
    let shell = block(
        "Shell",
        json!({"command": "seq 1 30"}),
        vec![Content::text(rows_of(30))],
    );
    shell.borrow_mut().freeze();
    let mut chain = chain_with(vec![shell.clone()]);
    let peek = render(&mut chain, 100).join("\n");
    assert!(!peek.contains("row-01"), "shut, no body: {peek}");
    assert!(!peek.contains("earlier lines"), "shut, no hint: {peek}");
    shell.borrow_mut().set_expanded(true);
    let full = render(&mut chain, 100).join("\n");
    for i in 1..=30 {
        assert!(full.contains(&format!("row-{i:02}")), "row {i}: {full}");
    }
}

#[test]
fn the_toggle_wraps_a_long_output_row_whole() {
    let _g = lock();
    let long = "Q".repeat(150);
    let mut chain = chain_with(vec![block(
        "Read",
        json!({"file_path": "notes.txt"}),
        vec![Content::text(long.clone())],
    )]);
    chain.tool_blocks()[0].borrow_mut().set_expanded(true);
    let open = render(&mut chain, 60).join("");
    assert_eq!(open.matches('Q').count(), 150, "{open}");
    assert!(!open.contains('…'), "{open}");
}

#[test]
fn an_image_only_result_shows_just_its_placeholder() {
    let _g = lock();
    let image = Content::Image(ImageContent {
        data: PNG_1X1.into(),
        media_type: "image/png".into(),
    });
    {
        let tool = "mystery";
        let only = block(tool, json!({"file_path": "shot.png"}), vec![image.clone()]);
        only.borrow_mut().set_expanded(true);
        let mut chain = chain_with(vec![only]);
        let rows = render(&mut chain, 100);
        let body: Vec<&String> = rows.iter().filter(|r| r.contains("[Image: ")).collect();
        assert_eq!(body.len(), 1, "{tool}: {rows:?}");
        assert!(body[0].starts_with("    [Image: "), "{tool}: {rows:?}");
        assert!(
            !rows.iter().any(|r| r.contains("more line")),
            "{tool}: {rows:?}"
        );
    }
}

#[test]
fn a_frozen_block_keeps_the_whole_output_at_its_width() {
    let _g = lock();
    let long = "Q".repeat(120);
    // A frozen capture is taken at the width it first renders: a fresh
    // block keeps the whole output, wrapped, at either terminal.
    let make = || {
        let read = block(
            "Read",
            json!({"file_path": "notes.txt"}),
            vec![Content::text(long.clone())],
        );
        read.borrow_mut().freeze();
        read.borrow_mut().set_expanded(true);
        chain_with(vec![read])
    };
    let wide = render(&mut make(), 160).join("");
    assert_eq!(wide.matches('Q').count(), 120, "{wide}");
    let narrow = render(&mut make(), 50).join("");
    assert_eq!(narrow.matches('Q').count(), 120, "{narrow}");
    assert!(!narrow.contains('…'), "{narrow}");
}

/// A Shell block with a stored duration, as a rebuilt session carries it.
fn shell_block(output: &str, is_error: bool) -> ToolBlock {
    let mut component = ToolExecutionComponent::new(
        "Shell",
        "t-0",
        json!({"command": "cargo test"}),
        ToolExecutionOptions::default(),
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
    let block = shell_block(&rows_of(40), false);
    let mut chain = chain_with(vec![block.clone()]);
    let shut = render(&mut chain, 100);
    assert!(shut[0].ends_with("exit 0 · 40 lines · 8.2s"), "{shut:?}");
    assert!(
        !shut.iter().any(|r| r.contains("row-")),
        "shut, no body: {shut:?}"
    );
    // The toggle opens the whole output: the tail, no hint, no `Took`.
    block.borrow_mut().set_expanded(true);
    let rows = render(&mut chain, 100);
    assert_eq!(
        rows.iter().filter(|r| r.contains("row-")).count(),
        40,
        "{rows:?}"
    );
    assert!(!rows.iter().any(|r| r.contains("Took")), "{rows:?}");
    assert!(!rows.iter().any(|r| r.contains("more line")), "{rows:?}");
    assert!(rows[0].ends_with("exit 0 · 40 lines · 8.2s"), "{rows:?}");
}

#[test]
fn a_failed_shell_step_names_its_exit_status_and_keeps_the_last_lines() {
    let _g = lock();
    let output = "error[E0308]: mismatched types\n   --> src/lex.rs:88:21\n    |\n\
                  88  |     return Ok(Token::Str(text));\n    |\n\
                  error: could not compile `parser`\n\nCommand exited with code 1";
    let block = shell_block(output, true);
    let mut chain = chain_with(vec![block.clone()]);
    let shut = render(&mut chain, 100);
    assert!(
        shut[0].contains('⊗')
            && shut[0].ends_with("exit 1 · error[E0308]: mismatched types · 8 lines · 8.2s"),
        "{shut:?}"
    );
    assert!(!shut.join("\n").contains("could not compile"), "{shut:?}");
    // The toggle opens the whole failed output, head first.
    block.borrow_mut().set_expanded(true);
    let rows = render(&mut chain, 100);
    assert!(
        rows[0].contains('⊗')
            && rows[0].ends_with("exit 1 · error[E0308]: mismatched types · 8 lines · 8.2s"),
        "{rows:?}"
    );
    let joined = rows.join("\n");
    assert!(!joined.contains("HIT"), "{joined}");
    assert!(!joined.contains("earlier lines"), "{joined}");
    assert!(joined.contains("error: could not compile"), "{joined}");
    assert!(joined.contains("mismatched types"), "{joined}");
    // The exit line stays out of the body: the header names it.
    assert!(!joined.contains("Command exited with code 1"), "{joined}");
}

#[test]
fn an_interrupted_step_is_no_hit_and_leaves_the_reason_to_the_message() {
    let _g = lock();
    let shell = shell_block("Operation aborted", true);
    shell.borrow_mut().set_interrupted();
    assert!(!shell.borrow().is_error());
    assert!(!shell.borrow().chain_entry().is_error);
    let mut chain = chain_with(vec![shell]);
    let rows = render(&mut chain, 100);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(
        rows[0].contains('⊗') && rows[0].ends_with("stopped"),
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
fn settled_agent(run: &str) -> ToolBlock {
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
        ToolExecutionOptions::default(),
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
fn a_settled_agent_hides_its_calls_until_the_toggle_opens_them() {
    let _g = lock();
    let agent = settled_agent("it-agent-single");
    let mut chain = chain_with(vec![agent.clone()]);
    let shut = render(&mut chain, 100).join("\n");
    assert!(
        !shut.contains("Found it"),
        "a settled agent shows no body: {shut}"
    );
    assert!(!shut.contains("   ⊗ Grep"), "shut, no child rows: {shut}");
    agent.borrow_mut().set_expanded(true);
    let whole = render(&mut chain, 100).join("\n");
    assert!(whole.contains("Found it."), "{whole}");
    assert!(whole.contains("   ⊗ Grep"), "{whole}");
    assert!(!whole.contains("more line"), "{whole}");
}

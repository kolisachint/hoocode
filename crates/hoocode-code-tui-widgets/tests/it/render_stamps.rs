//! Render stamps: a container keeps the lines of settled transcript items, so
//! every change that shows must take a new stamp. Each test draws through a
//! container (which re-draws on a hit and compares, in debug builds), changes
//! the item, and checks the screen followed.

use std::cell::RefCell;
use std::rc::Rc;

use crate::support::{assistant, lock, strip};
use hoocode_ai_types::Content;
use hoocode_code_tui_widgets::outline::{FlightMark, FlightStats};
use hoocode_code_tui_widgets::tool_chain::ChainState;
use hoocode_code_tui_widgets::tool_chain::{ToolBlock, ToolChainComponent};
use hoocode_code_tui_widgets::tool_execution::{ToolExecutionComponent, ToolExecutionOptions};
use hoocode_code_tui_widgets::tool_signal::ToolResult;
use hoocode_code_tui_widgets::tools::registered_tool_definition;
use hoocode_code_tui_widgets::trajectory::{LandingLine, MissionHeader};
use hoocode_code_tui_widgets::AssistantMessageComponent;
use hoocode_code_tui_widgets::ThinkingDisplay;
use hoocode_tui_components::{Spacer, Text};
use hoocode_tui_render::{note_change, Component, Container};
use serde_json::json;

fn joined(container: &mut Container, width: u16) -> String {
    strip(&container.render(width).join("\n"))
}

fn shell_block(output: &str) -> ToolBlock {
    let mut component = ToolExecutionComponent::new(
        "Shell",
        "t-0",
        json!({"command": "ls"}),
        ToolExecutionOptions::default(),
        Some(registered_tool_definition("Shell")),
        "/tmp",
    );
    component.update_result(
        ToolResult {
            content: vec![Content::text(output)],
            details: json!({}),
            is_error: false,
        },
        false,
    );
    Rc::new(RefCell::new(component))
}

#[test]
fn a_stamped_item_changes_when_its_text_does() {
    let _g = lock();
    let text = Rc::new(RefCell::new(Text::new("first", 0, 0)));
    let spacer = Rc::new(RefCell::new(Spacer::new(1)));
    let mut container = Container::new();
    container.add_child(text.clone());
    container.add_child(spacer.clone());
    assert!(joined(&mut container, 20).contains("first"));
    assert!(joined(&mut container, 20).contains("first"));
    text.borrow_mut().set_text("second");
    spacer.borrow_mut().set_lines(3);
    let after = container.render(20);
    assert!(strip(&after.join("\n")).contains("second"));
    assert_eq!(after.len(), 4);
}

#[test]
fn an_assistant_message_follows_its_updates() {
    let _g = lock();
    let message = |text: &str| assistant(vec![Content::text(text)]);
    let component = Rc::new(RefCell::new(AssistantMessageComponent::new(
        Some(&message("hello there")),
        ThinkingDisplay::Full,
    )));
    let mut container = Container::new();
    container.add_child(component.clone());
    assert!(joined(&mut container, 40).contains("hello there"));
    assert!(joined(&mut container, 40).contains("hello there"));

    component
        .borrow_mut()
        .update_content(&message("goodbye now"), false);
    let after = joined(&mut container, 40);
    assert!(after.contains("goodbye now") && !after.contains("hello there"));

    // Expanding redraws: the prose stays, with no rail.
    component.borrow_mut().set_expanded(true);
    let open = joined(&mut container, 40);
    assert!(open.contains("goodbye now") && !open.contains('┊'));
    component.borrow_mut().invalidate();
    joined(&mut container, 40);
}

#[test]
fn a_streaming_message_has_no_stamp() {
    let _g = lock();
    let mut component = AssistantMessageComponent::new(None, ThinkingDisplay::Full);
    component.update_content(&assistant(vec![Content::text("part")]), true);
    assert_eq!(component.render_stamp(), None);
    component.update_content(&assistant(vec![Content::text("part and more")]), false);
    assert!(component.render_stamp().is_some());
}

#[test]
fn a_chain_is_stamped_once_it_has_settled() {
    let _g = lock();
    let block = shell_block("a\nb\nc");
    let mut chain = ToolChainComponent::new();
    chain.add(block.clone());
    assert_eq!(chain.render_stamp(), None, "still collecting calls");
    chain.close(ChainState::Done);
    let settled = chain.render_stamp().expect("a settled chain");
    block.borrow_mut().set_expanded(true);
    assert_ne!(chain.render_stamp(), Some(settled));
}

#[test]
fn a_chain_follows_its_blocks() {
    let _g = lock();
    let output = (1..=30)
        .map(|n| format!("row-{n:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    let block = shell_block(&output);
    let chain = Rc::new(RefCell::new(ToolChainComponent::new()));
    chain.borrow_mut().add(block.clone());
    chain.borrow_mut().close(ChainState::Done);
    let mut container = Container::new();
    container.add_child(chain.clone());
    // Single view: the row is its header until the toggle opens it.
    let peek = joined(&mut container, 100);
    assert!(!peek.contains("row-01"), "{peek}");
    assert!(!peek.contains("earlier lines"), "{peek}");
    assert_eq!(joined(&mut container, 100), peek);

    block.borrow_mut().set_expanded(true);
    let open = joined(&mut container, 100);
    assert!(open.contains("row-01") && open.contains("row-30"), "{open}");
    assert!(!open.contains("earlier lines"), "{open}");

    // A frozen block draws from its capture, which the chain keeps stamped.
    block.borrow_mut().freeze();
    let frozen = joined(&mut container, 100);
    assert!(
        frozen.contains("row-01") && frozen.contains("row-30"),
        "{frozen}"
    );
    assert_eq!(joined(&mut container, 100), frozen);

    block.borrow_mut().set_expanded(false);
    let shut_again = joined(&mut container, 100);
    assert!(
        !shut_again.contains("row-01") && !shut_again.contains("earlier lines"),
        "{shut_again}"
    );
}

#[test]
fn a_running_block_is_not_stamped() {
    let _g = lock();
    let component = ToolExecutionComponent::new(
        "Shell",
        "t-1",
        json!({"command": "sleep 5"}),
        ToolExecutionOptions::default(),
        Some(registered_tool_definition("Shell")),
        "/tmp",
    );
    assert_eq!(component.render_stamp(), None);
}

#[test]
fn a_turn_header_keeps_its_prompt_band() {
    let _g = lock();
    let header = Rc::new(RefCell::new(MissionHeader::new("fix the build")));
    let mut container = Container::new();
    container.add_child(header.clone());
    let one = joined(&mut container, 100);
    assert!(one.contains("› fix the build"), "{one}");
    assert_eq!(joined(&mut container, 100), one);
}

#[test]
fn a_turn_header_is_always_stamped() {
    // The band draws what it was built with: no live data, no running state.
    let _g = lock();
    let header = MissionHeader::new("go");
    assert!(header.render_stamp().is_some());
}

#[test]
fn an_end_line_follows_its_outline_data() {
    let _g = lock();
    let flight = Rc::new(RefCell::new(FlightStats {
        mark: FlightMark::Aborted,
        landed_time: Some("1.0s".into()),
        ..Default::default()
    }));
    let line = Rc::new(RefCell::new(LandingLine::new(flight.clone())));
    let mut container = Container::new();
    container.add_child(line.clone());
    let before = joined(&mut container, 100);
    assert!(before.contains("⊗ stopped · 1.0s"), "{before}");
    flight.borrow_mut().landed_time = Some("2.0s".into());
    note_change();
    let changed = joined(&mut container, 100);
    assert!(changed.contains("⊗ stopped · 2.0s"), "{changed}");
}

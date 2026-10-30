//! Render stamps: a container keeps the lines of settled transcript items, so
//! every change that shows must take a new stamp. Each test draws through a
//! container (which re-draws on a hit and compares, in debug builds), changes
//! the item, and checks the screen followed.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::support::{assistant, lock, strip};
use hoocode_ai_types::Content;
use hoocode_code_tui_widgets::radar::{FlightMark, FlightStats, PeekOnly};
use hoocode_code_tui_widgets::tool_chain::{ToolBlock, ToolChainComponent};
use hoocode_code_tui_widgets::tool_chain_summary::ChainState;
use hoocode_code_tui_widgets::tool_execution::{ToolExecutionComponent, ToolExecutionOptions};
use hoocode_code_tui_widgets::tool_output_view::ToolOutputView;
use hoocode_code_tui_widgets::tool_signal::ToolResult;
use hoocode_code_tui_widgets::tools::registered_tool_definition;
use hoocode_code_tui_widgets::trajectory::{LandingLine, MissionHeader};
use hoocode_code_tui_widgets::AssistantMessageComponent;
use hoocode_code_tui_widgets::ThinkingDisplay;
use hoocode_tui_components::{Spacer, Text};
use hoocode_tui_render::{note_change, Component, ComponentHandle, Container};
use serde_json::json;

fn joined(container: &mut Container, width: u16) -> String {
    strip(&container.render(width).join("\n"))
}

fn shell_block(view: ToolOutputView, output: &str) -> ToolBlock {
    let mut component = ToolExecutionComponent::new(
        "Shell",
        "t-0",
        json!({"command": "ls"}),
        ToolExecutionOptions {
            view,
            ..Default::default()
        },
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
fn an_assistant_message_follows_its_updates_and_view() {
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

    // The peek view draws the prose on the rail; radar draws none of it.
    component.borrow_mut().set_view(ToolOutputView::Peek);
    assert!(joined(&mut container, 40).contains('┊'));
    component.borrow_mut().set_view(ToolOutputView::Radar);
    assert!(!joined(&mut container, 40).contains('┊'));
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
    let block = shell_block(ToolOutputView::Peek, "a\nb\nc");
    let mut chain = ToolChainComponent::new(ToolOutputView::Peek);
    chain.add(block.clone());
    assert_eq!(chain.render_stamp(), None, "still collecting calls");
    chain.close(ChainState::Done);
    let settled = chain.render_stamp().expect("a settled chain");
    chain.set_view(ToolOutputView::Full);
    assert_ne!(chain.render_stamp(), Some(settled));
}

#[test]
fn a_chain_follows_its_view_and_its_blocks() {
    let _g = lock();
    let output = (1..=30)
        .map(|n| format!("row-{n:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    let block = shell_block(ToolOutputView::Peek, &output);
    let chain = Rc::new(RefCell::new(ToolChainComponent::new(ToolOutputView::Peek)));
    chain.borrow_mut().add(block.clone());
    chain.borrow_mut().close(ChainState::Done);
    let mut container = Container::new();
    container.add_child(chain.clone());
    let peek = joined(&mut container, 100);
    assert!(peek.contains("earlier lines"), "{peek}");
    assert_eq!(joined(&mut container, 100), peek);

    chain.borrow_mut().set_view(ToolOutputView::Full);
    let full = joined(&mut container, 100);
    assert!(
        full.contains("row-01") && !full.contains("earlier lines"),
        "{full}"
    );

    // A frozen block draws from its capture, which the chain keeps stamped.
    block.borrow_mut().freeze();
    let frozen = joined(&mut container, 100);
    assert!(
        frozen.contains("row-01") && frozen.contains("row-30"),
        "{frozen}"
    );
    assert_eq!(joined(&mut container, 100), frozen);

    chain.borrow_mut().set_view(ToolOutputView::Peek);
    let peek_again = joined(&mut container, 100);
    assert!(peek_again.contains("earlier lines"), "{peek_again}");
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
fn a_mission_header_follows_the_view_and_its_flight_data() {
    let _g = lock();
    let flight = Rc::new(RefCell::new(FlightStats {
        mark: FlightMark::Landed,
        steps: 1,
        landed_time: Some("0.8s".into()),
        ..Default::default()
    }));
    let mut header = MissionHeader::new(1, "fix the build", None);
    header.set_flight(flight.clone());
    header.set_view(ToolOutputView::Radar);
    let header = Rc::new(RefCell::new(header));
    let mut container = Container::new();
    container.add_child(header.clone());
    let one = joined(&mut container, 100);
    assert_eq!(joined(&mut container, 100), one);
    flight.borrow_mut().steps = 4;
    flight.borrow_mut().hits = 2;
    // Shared data the stamp reads: its writer says so.
    note_change();
    let two = joined(&mut container, 100);
    assert_ne!(two, one);
    header.borrow_mut().set_view(ToolOutputView::Peek);
    let peek = joined(&mut container, 100);
    assert!(
        peek.contains("▲") && peek.contains("fix the build"),
        "{peek}"
    );
}

#[test]
fn a_flying_mission_header_has_no_stamp() {
    let _g = lock();
    let flight = Rc::new(RefCell::new(FlightStats {
        started: Some(std::time::Instant::now()),
        ..Default::default()
    }));
    let mut header = MissionHeader::new(1, "go", None);
    header.set_flight(flight.clone());
    header.set_view(ToolOutputView::Radar);
    assert_eq!(header.render_stamp(), None);
    // Landing sets the mark and the landed time together.
    let mut landed = flight.borrow_mut();
    landed.mark = FlightMark::Landed;
    landed.landed_time = Some("1s".into());
    drop(landed);
    assert!(header.render_stamp().is_some());
}

#[test]
fn a_landing_line_follows_its_counts_and_the_peek_flag() {
    let _g = lock();
    let line = Rc::new(RefCell::new(LandingLine::new(
        FlightMark::Landed,
        1,
        0,
        "1.0s",
        "10 tokens",
        "$0.001",
    )));
    let hidden = Rc::new(Cell::new(false));
    let wrapped: ComponentHandle = PeekOnly::wrap(hidden.clone(), line.clone());
    let mut container = Container::new();
    container.add_child(wrapped);
    let before = joined(&mut container, 100);
    assert!(before.contains("LANDED"), "{before}");
    line.borrow_mut().set_counts(5, 0);
    let counted = joined(&mut container, 100);
    assert_ne!(counted, before);
    // Radar hides the row; flipping the shared flag must show.
    hidden.set(true);
    note_change();
    assert_eq!(joined(&mut container, 100).trim(), "");
    hidden.set(false);
    note_change();
    assert_eq!(joined(&mut container, 100), counted);
}

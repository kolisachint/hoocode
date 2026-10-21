//! The turn's end in every layer: the landing line's outcome, the abort and
//! error reason, and the agent's whole final text in radar.

use std::rc::Rc;

use crate::support::{assistant, lock, strip};
use hoocode_ai_types::{Content, StopReason, ToolCallContent};
use hoocode_code_tui_widgets::radar::{FlightMark, FlightStats};
use hoocode_code_tui_widgets::tool_chain_summary::ChainEntry;
use hoocode_code_tui_widgets::trajectory::{LandingLine, MissionHeader};
use hoocode_code_tui_widgets::{AssistantMessageComponent, ThinkingDisplay};
use hoocode_tui_render::Component;
use std::cell::RefCell;

fn landing(mark: FlightMark) -> String {
    let mut line = LandingLine::new(mark, 2, 1, "0.5s", "0.8k tokens", "$0.002");
    strip(&line.render(100).join("\n"))
}

#[test]
fn the_landing_line_carries_the_missions_outcome() {
    let _g = lock();
    let landed = landing(FlightMark::Landed);
    assert!(landed.contains("● LANDED"), "{landed}");
    assert!(landed.contains("2 steps · 1 hit"), "{landed}");

    let failed = landing(FlightMark::DidNotLand);
    assert!(failed.contains("⊗ DID NOT LAND"), "{failed}");
    assert!(!failed.contains("LANDED ·"), "{failed}");
    assert!(failed.contains("2 steps · 1 hit"), "{failed}");

    let aborted = landing(FlightMark::Aborted);
    assert!(aborted.contains("⊘ ABORTED"), "{aborted}");
    assert!(!aborted.contains("LANDED"), "{aborted}");
}

#[test]
fn a_stepless_landing_leaves_the_step_counts_out() {
    let _g = lock();
    let mut line = LandingLine::new(FlightMark::Landed, 0, 0, "1.2s", "40 tokens", "$0.001");
    let text = strip(&line.render(100).join("\n"));
    assert!(
        text.contains("● LANDED · 1.2s · 40 tokens · $0.001"),
        "{text}"
    );
    assert!(!text.contains("step"), "{text}");
}

#[test]
fn the_landing_line_sums_up_the_turn_on_one_dim_line_after_it() {
    let _g = lock();
    let mut line = LandingLine::new(FlightMark::Landed, 5, 0, "1.2s", "40 tokens", "$0.001");
    line.set_summary(Some("Edited 3 files · ran 2 commands".to_string()));
    let rows: Vec<String> = line.render(100).iter().map(|row| strip(row)).collect();
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert!(rows[0].contains("LANDED"), "{rows:?}");
    assert_eq!(rows[1].trim(), "Edited 3 files · ran 2 commands");
    // A narrow width wraps it, never cuts it.
    let rows: Vec<String> = line.render(24).iter().map(|row| strip(row)).collect();
    assert!(!rows.iter().any(|row| row.contains('…')), "{rows:?}");
    let flat = rows.join(" ");
    for word in ["Edited", "3", "files", "ran", "2", "commands"] {
        assert!(flat.contains(word), "{rows:?}");
    }
}

fn tool_call() -> Content {
    Content::ToolCall(ToolCallContent {
        id: "tool-1".into(),
        name: "Read".into(),
        arguments: serde_json::json!({ "path": "file.txt" }),
        thought_signature: None,
    })
}

fn ended(
    stop: StopReason,
    reason: Option<&str>,
    content: Vec<Content>,
) -> AssistantMessageComponent {
    let mut message = assistant(content);
    message.stop_reason = stop;
    message.error_message = reason.map(str::to_string);
    AssistantMessageComponent::new(Some(&message), ThinkingDisplay::Full)
}

#[test]
fn the_reason_shows_in_peek_even_when_the_message_has_tool_calls() {
    let _g = lock();
    let mut aborted = ended(
        StopReason::Aborted,
        Some("Operation aborted"),
        vec![Content::text("calling"), tool_call()],
    );
    let text = strip(&aborted.render(60).join("\n"));
    assert!(text.contains("Operation aborted"), "{text}");
    let mut errored = ended(
        StopReason::Error,
        Some("boom"),
        vec![Content::text("calling"), tool_call()],
    );
    let text = strip(&errored.render(60).join("\n"));
    assert!(text.contains("Error: boom"), "{text}");
}

fn radar_text(
    stop: StopReason,
    reason: Option<&str>,
    content: Vec<Content>,
    width: u16,
) -> Vec<String> {
    let mut message = assistant(content);
    message.stop_reason = stop;
    message.error_message = reason.map(str::to_string);
    let mut component = AssistantMessageComponent::new(Some(&message), ThinkingDisplay::Omit);
    component.set_radar_prose_shown(true);
    component
        .render(width)
        .iter()
        .map(|row| strip(row))
        .filter(|row| !row.trim().is_empty())
        .collect()
}

#[test]
fn radar_shows_the_whole_final_text_wrapped_at_the_step_indent() {
    let _g = lock();
    let text = "First I read the wrap helper and found the overflow in the second branch.\n\n\
                - It cut every row at the edge\n- It dropped the rest of the prompt\n\n\
                Then I fixed **both** and the `suite` passes.";
    let rows = radar_text(StopReason::Stop, None, vec![Content::text(text)], 40);
    assert!(rows.len() >= 6, "{rows:#?}");
    for row in &rows {
        assert!(row.starts_with("        "), "{row:?}");
        assert!(!row.contains('…'), "{row:?}");
        assert!(row.chars().count() <= 39, "{row:?}");
    }
    let flat = rows
        .iter()
        .map(|row| row.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join(" ");
    for needle in [
        "First I read the wrap helper and found the overflow in the second branch.",
        "It cut every row at the edge",
        "It dropped the rest of the prompt",
        "Then I fixed both and the suite passes.",
    ] {
        assert!(flat.contains(needle), "{needle}: {flat}");
    }
}

#[test]
fn radar_shows_the_abort_and_error_reason_after_the_text() {
    let _g = lock();
    let rows = radar_text(
        StopReason::Aborted,
        Some("Aborted after 2 retry attempts"),
        vec![Content::text("Started the build."), tool_call()],
        60,
    );
    let text = rows.join("\n");
    assert!(text.contains("Started the build."), "{text}");
    assert!(text.contains("Aborted after 2 retry attempts"), "{text}");
    let rows = radar_text(StopReason::Error, Some("503 overloaded"), vec![], 60);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(rows[0].contains("Error: 503 overloaded"), "{rows:?}");
}

#[test]
fn a_radar_mission_row_lists_its_progress_under_the_whole_prompt() {
    let _g = lock();
    let entry = |tool: &str, subject: &str, partial: bool| ChainEntry {
        tool: tool.into(),
        subject: subject.into(),
        is_partial: partial,
        ..Default::default()
    };
    let stats = Rc::new(RefCell::new(FlightStats {
        mark: FlightMark::InFlight,
        steps: 3,
        chain: vec![
            entry("Read", "a.rs", false),
            entry("Edit", "a.rs", false),
            entry("Shell", "cargo test", true),
        ],
        ..Default::default()
    }));
    let mut header = MissionHeader::new(
        1,
        "fix the overflow in the wrap helper and make sure that the whole suite still passes",
        None,
    );
    header.set_view(hoocode_code_tui_widgets::tool_output_view::ToolOutputView::Radar);
    header.set_flight(stats.clone());
    let rows: Vec<String> = header.render(50).iter().map(|row| strip(row)).collect();
    assert!(rows.len() >= 3, "{rows:#?}");
    assert!(
        rows.iter()
            .all(|row| !row.contains('…') || row.contains(" …")),
        "{rows:#?}"
    );
    // The progress row is last, after every prompt row.
    assert_eq!(
        rows.last().unwrap().trim_end(),
        "        ◐ Read › Edit › Shell …"
    );
    assert!(rows[..rows.len() - 1]
        .iter()
        .any(|row| row.contains("suite still passes")));

    // Landed, the same row reads the settled summary.
    {
        let mut flight = stats.borrow_mut();
        flight.mark = FlightMark::Landed;
        flight.chain = vec![
            entry("Edit", "a.rs", false),
            entry("Edit", "b.rs", false),
            entry("Shell", "cargo test", false),
        ];
    }
    let rows: Vec<String> = header.render(50).iter().map(|row| strip(row)).collect();
    assert!(
        rows.last()
            .unwrap()
            .contains("● Edited 2 files · ran cargo test"),
        "{rows:#?}"
    );
}

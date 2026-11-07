//! The turn's end in every layer: no end line for a turn that finished, the
//! failed or stopped end line, the error reason, and the agent's whole final
//! text in outline.

use std::rc::Rc;

use crate::support::{assistant, lock, strip};
use hoocode_ai_types::{Content, StopReason, ToolCallContent};
use hoocode_code_tui_widgets::outline::{FlightMark, FlightStats, TurnEnd};
use hoocode_code_tui_widgets::trajectory::{LandingLine, MissionHeader};
use hoocode_code_tui_widgets::{AssistantMessageComponent, ThinkingDisplay};
use hoocode_tui_render::Component;
use std::cell::RefCell;

fn stats(mark: FlightMark, failure: Option<&str>) -> Rc<RefCell<FlightStats>> {
    Rc::new(RefCell::new(FlightStats {
        mark,
        steps: 2,
        hits: 1,
        landed_time: Some("0.5s".to_string()),
        tokens: Some("0.8k tokens".to_string()),
        cost: Some("$0.002".to_string()),
        failure: failure.map(str::to_string),
        ..Default::default()
    }))
}

fn end_rows(mark: FlightMark, failure: Option<&str>) -> Vec<String> {
    let mut line = LandingLine::new(stats(mark, failure));
    line.render(100)
        .iter()
        .map(|row| strip(row).trim_end().to_string())
        .collect()
}

#[test]
fn a_finished_turn_has_no_end_line() {
    let _g = lock();
    assert!(end_rows(FlightMark::Landed, None).is_empty());
    // Failed calls alone do not fail the turn: the status line counts them.
    assert!(end_rows(FlightMark::DidNotLand, None).is_empty());
    assert_eq!(
        LandingLine::new(stats(FlightMark::Landed, None)).end(),
        None
    );
}

#[test]
fn a_failed_or_stopped_turn_ends_with_one_plain_line() {
    let _g = lock();
    assert_eq!(
        end_rows(FlightMark::DidNotLand, Some("boom")),
        vec!["", " ⊗ failed · 0.5s"]
    );
    assert_eq!(
        end_rows(FlightMark::Aborted, None),
        vec!["", " ⊗ stopped · 0.5s"]
    );
    assert_eq!(
        LandingLine::new(stats(FlightMark::Aborted, None)).end(),
        Some(TurnEnd::Stopped)
    );
    // Tokens and cost are the status line's.
    let text = end_rows(FlightMark::DidNotLand, Some("boom")).join("\n");
    assert!(!text.contains("tokens") && !text.contains('$'), "{text}");
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
        Some("Aborted after 2 retry attempts"),
        vec![Content::text("calling"), tool_call()],
    );
    let text = strip(&aborted.render(60).join("\n"));
    assert!(text.contains("⊗ Aborted after 2 retry attempts"), "{text}");
    let mut errored = ended(
        StopReason::Error,
        Some("boom"),
        vec![Content::text("calling"), tool_call()],
    );
    let text = strip(&errored.render(60).join("\n"));
    assert!(text.contains(" ⊗ boom"), "{text}");
    assert!(!text.contains("Error:"), "{text}");
}

#[test]
fn a_plain_abort_inside_a_turn_leaves_the_word_to_the_end_line() {
    let _g = lock();
    let mut aborted = ended(
        StopReason::Aborted,
        Some("Operation aborted"),
        vec![Content::text("calling"), tool_call()],
    );
    aborted.set_outline_mission(Some(1));
    let text = strip(&aborted.render(60).join("\n"));
    assert!(!text.contains("Operation aborted"), "{text}");
    // Outside every turn there is no end line: the abort line stays.
    aborted.set_outline_mission(None);
    let text = strip(&aborted.render(60).join("\n"));
    assert!(text.contains("⊗ Operation aborted"), "{text}");
}

fn outline_text(
    stop: StopReason,
    reason: Option<&str>,
    content: Vec<Content>,
    width: u16,
) -> Vec<String> {
    let mut message = assistant(content);
    message.stop_reason = stop;
    message.error_message = reason.map(str::to_string);
    let mut component = AssistantMessageComponent::new(Some(&message), ThinkingDisplay::Omit);
    component.set_outline_prose_shown(true);
    component
        .render(width)
        .iter()
        .map(|row| strip(row))
        .filter(|row| !row.trim().is_empty())
        .collect()
}

#[test]
fn outline_shows_the_whole_final_text_wrapped_at_the_prose_indent() {
    let _g = lock();
    let text = "First I read the wrap helper and found the overflow in the second branch.\n\n\
                - It cut every row at the edge\n- It dropped the rest of the prompt\n\n\
                Then I fixed **both** and the `suite` passes.";
    let rows = outline_text(StopReason::Stop, None, vec![Content::text(text)], 40);
    assert!(rows.len() >= 5, "{rows:#?}");
    for row in &rows {
        assert!(row.starts_with(' ') && !row.starts_with("  "), "{row:?}");
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
fn outline_shows_the_abort_and_error_reason_after_the_text() {
    let _g = lock();
    let rows = outline_text(
        StopReason::Aborted,
        Some("Aborted after 2 retry attempts"),
        vec![Content::text("Started the build."), tool_call()],
        60,
    );
    let text = rows.join("\n");
    assert!(text.contains("Started the build."), "{text}");
    assert!(text.contains("⊗ Aborted after 2 retry attempts"), "{text}");
    let rows = outline_text(StopReason::Error, Some("503 overloaded"), vec![], 60);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(rows[0].contains("⊗ 503 overloaded"), "{rows:?}");
}

#[test]
fn the_header_band_lists_the_whole_prompt() {
    let _g = lock();
    let prompt =
        "fix the overflow in the wrap helper and make sure that the whole suite still passes";
    let mut header = MissionHeader::new(prompt);
    let rows: Vec<String> = header
        .render(50)
        .iter()
        .map(|row| strip(row).trim_end().to_string())
        .collect();
    assert!(rows[0].starts_with(" › fix the overflow"), "{rows:#?}");
    // The prompt is never cut: every word of it shows in the band rows.
    let band: Vec<&String> = rows.iter().take_while(|row| !row.is_empty()).collect();
    assert!(band.iter().all(|row| !row.contains('…')), "{rows:#?}");
    let flat = band
        .iter()
        .map(|row| row.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join(" ");
    for word in prompt.split_whitespace() {
        assert!(flat.contains(word), "{word}: {rows:#?}");
    }
    // The prompt band is the whole header: no mission number.
    assert!(!flat.contains("M1"), "{rows:#?}");
}

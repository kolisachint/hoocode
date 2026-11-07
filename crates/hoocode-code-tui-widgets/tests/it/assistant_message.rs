//! Port of the pin's `test/assistant-message.test.ts`.

use crate::support::{assistant, lock};
use hoocode_ai_types::{Content, ThinkingContent, ToolCallContent};
use hoocode_code_tui_widgets::{
    AssistantMessageComponent, ThinkingDisplay, OSC133_ZONE_END, OSC133_ZONE_FINAL,
    OSC133_ZONE_START,
};
use hoocode_tui_render::Component;

fn thinking(text: &str) -> Content {
    Content::Thinking(ThinkingContent {
        thinking: text.into(),
        signature: Some(String::new()),
        ..Default::default()
    })
}

fn tool_call() -> Content {
    Content::ToolCall(ToolCallContent {
        id: "tool-1".into(),
        name: "Read".into(),
        arguments: serde_json::json!({ "path": "file.txt" }),
        thought_signature: None,
    })
}

#[test]
fn adds_osc_133_zone_markers_to_assistant_messages_without_tool_calls() {
    let _g = lock();
    let mut c = AssistantMessageComponent::new(
        Some(&assistant(vec![Content::text("hello")])),
        ThinkingDisplay::Full,
    );
    let lines = c.render(40);
    assert!(!lines.is_empty());
    assert!(lines[0].contains(OSC133_ZONE_START));
    assert!(lines
        .last()
        .unwrap()
        .starts_with(&format!("{OSC133_ZONE_END}{OSC133_ZONE_FINAL}")));
}

#[test]
fn does_not_add_osc_133_zone_markers_when_assistant_message_contains_tool_calls() {
    let _g = lock();
    let mut c = AssistantMessageComponent::new(
        Some(&assistant(vec![Content::text("calling tool"), tool_call()])),
        ThinkingDisplay::Full,
    );
    let rendered = c.render(60).join("\n");
    assert!(!rendered.contains(OSC133_ZONE_START));
    assert!(!rendered.contains(OSC133_ZONE_END));
    assert!(!rendered.contains(OSC133_ZONE_FINAL));
}

#[test]
fn omit_renders_nothing_for_a_message_that_only_thought_and_called_a_tool() {
    let _g = lock();
    let mut c = AssistantMessageComponent::new(
        Some(&assistant(vec![
            thinking("weighing the options"),
            tool_call(),
        ])),
        ThinkingDisplay::Omit,
    );
    assert_eq!(c.render(60), Vec::<String>::new());
}

#[test]
fn omit_drops_the_thinking_block_but_keeps_the_text_around_it() {
    let _g = lock();
    let mut c = AssistantMessageComponent::new(
        Some(&assistant(vec![
            thinking("weighing the options"),
            Content::text("here is the answer"),
        ])),
        ThinkingDisplay::Omit,
    );
    // The transcript shows the prose once the mission has ended.
    c.set_outline_prose_shown(true);
    let rendered = c.render(60).join("\n");
    assert!(rendered.contains("here is the answer"));
    assert!(!rendered.contains("weighing the options"));
    assert!(!rendered.contains("Thinking..."));
}

#[test]
fn thinking_is_one_dim_line_with_no_glyph_and_no_header() {
    // Peek and hidden thinking: the first sentence, `…` when more follows.
    let _g = lock();
    for display in [ThinkingDisplay::Label, ThinkingDisplay::Full] {
        let mut c = AssistantMessageComponent::new(
            Some(&assistant(vec![thinking(
                "Weighing the options. The first one hides a trap.",
            )])),
            display,
        );
        let rows: Vec<String> = c
            .render(60)
            .iter()
            .map(|l| crate::support::strip(l).trim_end().to_string())
            .filter(|l| !l.is_empty())
            .collect();
        assert_eq!(rows, vec![" Weighing the options…"], "{display:?}");
    }
    // One sentence reads whole.
    let mut c = AssistantMessageComponent::new(
        Some(&assistant(vec![thinking("weighing the options")])),
        ThinkingDisplay::Full,
    );
    let rendered = crate::support::strip(&c.render(60).join("\n"));
    assert!(rendered.contains(" weighing the options"), "{rendered}");
    for gone in ["Think", "··", "✻", "ctrl+t"] {
        assert!(!rendered.contains(gone), "{gone}: {rendered}");
    }
}

#[test]
fn single_view_shows_the_whole_trace_on_the_toggle() {
    let _g = lock();
    let gist = "weighing the options one by one because the first idea hides a trap \
                in the second step of the plan";
    let mut c = AssistantMessageComponent::new(
        Some(&assistant(vec![thinking(gist), tool_call()])),
        ThinkingDisplay::Full,
    );
    let shut = crate::support::strip(&c.render(50).join("\n"));
    assert!(shut.contains('…'), "{shut}");
    c.set_expanded(true);
    let full = crate::support::strip(&c.render(50).join("\n"));
    assert!(!full.contains('…'), "{full}");
    let flat = full.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(flat.contains(gist), "{full}");
}

#[test]
fn prose_opens_with_a_blank_and_sits_at_the_prose_indent_with_no_rail() {
    let _g = lock();
    let mut c = AssistantMessageComponent::new(
        Some(&assistant(vec![Content::text("The answer.")])),
        ThinkingDisplay::Full,
    );
    let rows: Vec<String> = c
        .render(60)
        .iter()
        .map(|l| crate::support::strip(l).trim_end().to_string())
        .collect();
    assert_eq!(rows, vec!["", " The answer."], "{rows:?}");
}

#[test]
fn outline_reason_is_the_first_line_of_the_failure_trimmed() {
    let _g = lock();
    let message = hoocode_ai_types::AssistantMessage {
        stop_reason: hoocode_ai_types::StopReason::Error,
        error_message: Some("\n  Request failed: boom  \nstack line two".into()),
        ..Default::default()
    };
    let component = AssistantMessageComponent::new(Some(&message), ThinkingDisplay::Omit);
    assert_eq!(
        component.outline_reason().as_deref(),
        Some("Request failed: boom")
    );
}

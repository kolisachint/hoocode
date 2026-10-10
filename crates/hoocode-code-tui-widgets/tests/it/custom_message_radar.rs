//! The folded (radar) custom message is one row at any width: its summary,
//! label and peek hint are cut to the width, never wrapped onto a second row.

use std::rc::Rc;

use crate::support::{lock, strip};
use hoocode_agent_types::CustomMessage;
use hoocode_ai_types::UserContent;
use hoocode_code_tui_theme::get_markdown_theme;
use hoocode_code_tui_widgets::custom_message::CustomMessageComponent;
use hoocode_tui_components::MarkdownTheme;
use hoocode_tui_render::Component;
use hoocode_tui_util::visible_width;

fn radar(text: &str) -> CustomMessageComponent {
    let md: Rc<dyn Fn() -> MarkdownTheme> = Rc::new(get_markdown_theme);
    let mut message = CustomMessageComponent::new(
        CustomMessage {
            custom_type: "skill".into(),
            content: UserContent::Text(text.into()),
            display: true,
            details: None,
            timestamp: 0,
        },
        md,
    );
    message.set_expanded(false);
    message
}

/// The rows that carry text, once the padding and spacer rows are dropped.
fn text_rows(rows: &[String]) -> Vec<String> {
    rows.iter()
        .map(|row| strip(row))
        .filter(|row| !row.trim().is_empty())
        .collect()
}

#[test]
fn a_long_radar_line_is_one_row_at_width_80() {
    let _g = lock();
    let long = "a summary that goes on and on well past the width of any terminal line we know about and then some more words";
    let rows = radar(long).render(80);
    let text = text_rows(&rows);
    assert_eq!(text.len(), 1, "{text:?}");
    assert!(text[0].contains("for peek"), "{text:?}");
    assert!(text[0].contains('…'), "{text:?}");
    assert!(rows.iter().all(|row| visible_width(row) <= 80), "{rows:?}");
}

#[test]
fn a_long_radar_line_is_one_row_at_width_40() {
    let _g = lock();
    let long = "a summary that goes on and on well past the width of any terminal line we know about and then some more words";
    let rows = radar(long).render(40);
    let text = text_rows(&rows);
    assert_eq!(text.len(), 1, "{text:?}");
    assert!(text[0].contains('…'), "{text:?}");
    assert!(rows.iter().all(|row| visible_width(row) <= 40), "{rows:?}");
}

#[test]
fn the_radar_row_keeps_the_same_height_at_every_width() {
    let _g = lock();
    let text = "short";
    assert_eq!(radar(text).render(80).len(), radar(text).render(40).len());
}

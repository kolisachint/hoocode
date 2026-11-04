//! The user message band (declutter spec "Transcript").

use crate::support::{lock, strip};
use hoocode_code_tui_widgets::UserMessageComponent;
use hoocode_tui_render::Component;

#[test]
fn a_user_message_is_one_surface_row_per_line_with_the_marker_first() {
    let _g = lock();
    let lines = UserMessageComponent::new("hello").render(20);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(strip(&lines[0]), format!(" › hello{}", " ".repeat(12)));
    assert_eq!(hoocode_tui_util::visible_width(&lines[0]), 20);
    // The surface colour runs to the last cell.
    assert!(lines[0].contains("48;"), "{:?}", lines[0]);
}

#[test]
fn wrapped_and_later_lines_start_under_the_text() {
    let _g = lock();
    let lines = UserMessageComponent::new("one two three four five six\nseven").render(16);
    let plain: Vec<String> = lines
        .iter()
        .map(|l| strip(l).trim_end().to_string())
        .collect();
    assert_eq!(plain[0], " › one two");
    assert!(plain[1..]
        .iter()
        .all(|l| l.starts_with("   ") && !l.starts_with("    ")));
    assert_eq!(plain.last().unwrap(), "   seven");
}

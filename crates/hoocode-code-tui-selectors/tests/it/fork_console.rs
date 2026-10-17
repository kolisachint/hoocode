//! Component goldens: the `/fork` picker as the STAGE SEPARATION console,
//! at 100 and 80 columns. Hints use literal keys (no `alt`), so the bytes
//! hold on every platform. Accept a deliberate change with
//! `UPDATE_GOLDENS=1 cargo nextest run -p hoocode-code-tui-selectors fork_console`.

use hoocode_code_tui_selectors::user_message_selector::{
    UserMessageEvent, UserMessageItem, UserMessageSelectorComponent,
};
use hoocode_tui_render::assert_golden;
use hoocode_tui_render::golden::render_golden;
use hoocode_tui_render::Component;

use crate::support::lock;

fn item(id: &str, text: &str) -> UserMessageItem {
    UserMessageItem {
        id: id.into(),
        text: text.into(),
        timestamp: None,
    }
}

fn fixture() -> Vec<UserMessageItem> {
    vec![
        item("m1", "what does the wrap crate do?"),
        item("m2", "add a width check to wrap_text in src/wrap.rs"),
        item(
            "m3",
            "lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor",
        ),
    ]
}

fn check(name: &str, width: u16) {
    let _g = lock();
    let mut selector = UserMessageSelectorComponent::new(fixture(), Some("m2"));
    let rendered = render_golden(&mut selector, width);
    let text = rendered.split("--- styles ---").next().unwrap_or_default();
    assert!(
        text.contains("STAGE SEPARATION · /fork"),
        "{name}: missing title in\n{text}"
    );
    assert!(
        text.lines()
            .any(|l| l.contains('›') && l.contains("width check")),
        "{name}: missing selected message in\n{text}"
    );
    assert!(
        text.contains("Message 2 of 3"),
        "{name}: missing position row in\n{text}"
    );
    assert!(
        text.contains("enter fork"),
        "{name}: missing hint foot in\n{text}"
    );
    assert_golden!(name, rendered);
}

#[test]
fn stage_separation_100() {
    check("stage_separation_100", 100);
}

#[test]
fn stage_separation_80() {
    check("stage_separation_80", 80);
}

/// Enter forks before the selected message: it opens on the most recent.
#[test]
fn enter_selects_the_most_recent_message() {
    let _g = lock();
    let mut selector = UserMessageSelectorComponent::new(fixture(), None);
    selector.handle_input("\r");
    assert_eq!(
        selector.poll(std::time::Instant::now()),
        vec![UserMessageEvent::Select("m3".to_string())]
    );
}

/// Esc cancels the fork.
#[test]
fn esc_cancels() {
    let _g = lock();
    let mut selector = UserMessageSelectorComponent::new(fixture(), None);
    selector.handle_input("\x1b");
    assert_eq!(
        selector.poll(std::time::Instant::now()),
        vec![UserMessageEvent::Cancel]
    );
}

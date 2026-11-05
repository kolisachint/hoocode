//! The letter gate of the list pickers: list mode, where a bare letter is a key,
//! and typing mode after `/`, where printable keys type into the query. Esc
//! clears a query, then leaves typing, then closes. Every test uses the
//! keymap the lock installs.

use hoocode_ai_types::Model;
use hoocode_code_tui_selectors::scoped_models_selector::{
    ScopedModelsEvent, ScopedModelsSelectorComponent,
};
use hoocode_tui_render::Component;
use hoocode_tui_util::strip_vt_control_characters;
use serde_json::json;

use crate::support::lock;

fn model(provider: &str, id: &str, name: &str) -> Model {
    serde_json::from_value(json!({
        "id": id, "name": name, "api": "openai-completions", "provider": provider,
        "baseUrl": "https://example.test", "reasoning": true, "input": ["text"],
        "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0},
        "contextWindow": 128000, "maxTokens": 8192,
    }))
    .unwrap()
}

fn picker() -> ScopedModelsSelectorComponent {
    ScopedModelsSelectorComponent::new(
        vec![
            model("openai", "gpt-5-mini", "GPT-5 mini"),
            model("anthropic", "claude-haiku-5-5", "Claude Haiku 5.5"),
        ],
        None,
    )
}

fn press(p: &mut ScopedModelsSelectorComponent, keys: &str) {
    for ch in keys.chars() {
        p.handle_input(&ch.to_string());
    }
}

fn text(p: &mut ScopedModelsSelectorComponent) -> String {
    strip_vt_control_characters(&p.render(100).join("\n"))
}

#[test]
fn slash_then_letters_filter_and_do_not_save() {
    // `s` saves in list mode. After `/`, the same letter types.
    let _g = lock();
    let mut p = picker();
    p.handle_input("/");
    press(&mut p, "sonnet");
    assert!(
        text(&mut p).contains("No matching models"),
        "the query filtered the list"
    );
    assert!(
        p.take_events().is_empty(),
        "typing sonnet does not save the models"
    );
}

#[test]
fn an_unbound_letter_in_list_mode_does_nothing() {
    let _g = lock();
    let mut p = picker();
    press(&mut p, "z");
    assert!(
        !text(&mut p).contains("No matching models"),
        "z did not type"
    );
    assert!(p.take_events().is_empty());
}

#[test]
fn escape_clears_the_query_then_leaves_typing_then_closes() {
    let _g = lock();
    let mut p = picker();
    p.handle_input("/");
    press(&mut p, "z");
    assert!(text(&mut p).contains("No matching models"));

    p.handle_input("\x1b");
    assert!(p.take_events().is_empty(), "esc clears, it does not close");
    assert!(
        !text(&mut p).contains("No matching models"),
        "the query is cleared"
    );

    press(&mut p, "x");
    assert!(
        p.take_events().is_empty(),
        "still typing: x types instead of clearing every model"
    );

    p.handle_input("\x1b");
    p.handle_input("\x1b");
    assert!(p.take_events().is_empty(), "the second esc leaves typing");
    press(&mut p, "q");
    assert_eq!(
        p.take_events(),
        vec![ScopedModelsEvent::Cancel],
        "q closes in list mode"
    );
}

#[test]
fn escape_in_list_mode_closes_the_picker() {
    let _g = lock();
    let mut p = picker();
    p.handle_input("\x1b");
    assert_eq!(p.take_events(), vec![ScopedModelsEvent::Cancel]);
}

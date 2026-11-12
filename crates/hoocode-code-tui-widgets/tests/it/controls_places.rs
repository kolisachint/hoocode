//! Behavior: the Keys pane groups each layer's keys by where they act, the
//! five rows of "Where a key acts" in docs/design/tui.md. The groups come in
//! table order, a bare key with a meaning in two places lists both, and
//! non-letter keys (digits, brackets, named keys) are listed, not dropped.

use hoocode_code_tui_widgets::controls::ControlsComponent;
use hoocode_tui_render::Component;

use crate::support::lock;

const PLACES: [&str; 5] = [
    "Prompt",
    "Block focused (pinned, prompt empty)",
    "Picker or pane, empty filter",
    "Blocking dialog, empty reply",
    "Task panel focused",
];

/// The pane's text on the plain layer (`tab` three times from the default ctrl).
fn plain_layer(width: u16) -> String {
    let mut controls = ControlsComponent::new();
    for _ in 0..3 {
        controls.handle_input("\t");
    }
    controls
        .render(width)
        .join("\n")
        .split("--- styles ---")
        .next()
        .unwrap_or_default()
        .to_string()
}

/// The text from one group's header up to the next group's header (or the end).
fn section(text: &str, place: &str) -> String {
    let Some(start) = text.find(place) else {
        return String::new();
    };
    let rest = &text[start + place.len()..];
    let end = PLACES
        .iter()
        .filter(|p| **p != place)
        .filter_map(|p| rest.find(p))
        .min()
        .unwrap_or(rest.len());
    rest[..end].to_string()
}

#[test]
fn the_five_groups_appear_in_table_order() {
    let _g = lock();
    for width in [80, 100] {
        let text = plain_layer(width);
        let mut last = 0;
        for place in PLACES {
            let at = text
                .find(place)
                .unwrap_or_else(|| panic!("missing group {place:?} at width {width}:\n{text}"));
            assert!(
                at >= last,
                "group {place:?} out of order at width {width}:\n{text}"
            );
            last = at;
        }
    }
}

#[test]
fn a_bare_key_with_two_meanings_lists_both() {
    let _g = lock();
    let text = plain_layer(100);
    // `y` copies the focused block (Block focused) and answers yes (Blocking dialog).
    let block = section(&text, PLACES[1]);
    let dialog = section(&text, PLACES[3]);
    assert!(
        block.contains("Copy the focused block's raw output"),
        "block group lacks copy:\n{block}"
    );
    assert!(
        block.split_whitespace().any(|w| w == "y"),
        "block group lacks y:\n{block}"
    );
    assert!(
        dialog.contains("Dialog: yes"),
        "dialog group lacks yes:\n{dialog}"
    );
    assert!(
        dialog.split_whitespace().any(|w| w == "y"),
        "dialog group lacks y:\n{dialog}"
    );
}

#[test]
fn non_letter_keys_are_listed() {
    let _g = lock();
    let text = plain_layer(100);
    // A digit (permission pick), a bracket (focus previous block), a function key (list help).
    assert!(
        text.contains("Permission: pick option 1"),
        "digit key dropped:\n{text}"
    );
    assert!(
        text.split_whitespace().any(|w| w == "["),
        "bracket key dropped:\n{text}"
    );
    assert!(
        text.split_whitespace().any(|w| w == "f1"),
        "function key dropped:\n{text}"
    );
}

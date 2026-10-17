//! Component goldens: the `/tree` picker as the FLIGHT PATH console, at 100
//! and 80 columns. Hints use literal keys (no `alt`), so the bytes hold on
//! every platform. Accept a deliberate change with
//! `UPDATE_GOLDENS=1 cargo nextest run -p hoocode-code-tui-selectors flight_path`.

use hoocode_code_session::{FileEntry, SessionTreeNode};
use hoocode_code_tui_selectors::tree_selector::TreeSelectorComponent;
use hoocode_tui_render::assert_golden;
use hoocode_tui_render::golden::render_golden;
use hoocode_tui_render::Component;
use serde_json::{json, Value};

use crate::support::lock;

const TS: &str = "2026-01-01T00:00:00.000Z";

fn entry(value: Value) -> FileEntry {
    serde_json::from_value(value).expect("a session entry")
}

fn user_message(id: &str, parent: Option<&str>, content: &str) -> FileEntry {
    entry(json!({
        "type": "message", "id": id, "parentId": parent, "timestamp": TS,
        "message": {"role": "user", "content": content, "timestamp": 0}
    }))
}

fn assistant_message(id: &str, parent: Option<&str>, text: &str) -> FileEntry {
    entry(json!({
        "type": "message", "id": id, "parentId": parent, "timestamp": TS,
        "message": {
            "role": "assistant",
            "content": [{"type": "text", "text": text}],
            "api": "anthropic-messages", "provider": "anthropic", "model": "claude-sonnet-4.5",
            "usage": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 0,
                      "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}},
            "stopReason": "stop", "timestamp": 0
        }
    }))
}

/// A user question, its answer, and a follow-up: the console opens on the
/// follow-up.
fn fixture() -> (Vec<SessionTreeNode>, &'static str) {
    let entries = vec![
        user_message("user-1", None, "what does the wrap crate do?"),
        assistant_message("asst-1", Some("user-1"), "It wraps text greedily."),
        user_message("user-2", Some("asst-1"), "add a width check to wrap_text"),
    ];
    let tree = {
        fn children_of(entries: &[FileEntry], parent: Option<&str>) -> Vec<SessionTreeNode> {
            entries
                .iter()
                .filter(|e| e.parent_id() == parent)
                .map(|e| SessionTreeNode {
                    entry: e.clone(),
                    children: children_of(entries, e.id()),
                    label: None,
                    label_timestamp: None,
                })
                .collect()
        }
        children_of(&entries, None)
    };
    (tree, "user-2")
}

fn check(name: &str, width: u16) {
    let _g = lock();
    let (tree, leaf) = fixture();
    let mut selector = TreeSelectorComponent::new(&tree, Some(leaf), 24, None, None);
    let rendered = render_golden(&mut selector, width);
    let text = rendered.split("--- styles ---").next().unwrap_or_default();
    assert!(
        text.contains("FLIGHT PATH · /tree"),
        "{name}: missing title in\n{text}"
    );
    assert!(
        text.lines()
            .any(|l| l.contains('›') && l.contains("width check")),
        "{name}: missing selected entry in\n{text}"
    );
    assert!(
        text.lines().any(|l| l.contains('▸')),
        "{name}: missing query row in\n{text}"
    );
    assert!(
        text.contains("enter go here"),
        "{name}: missing hint foot in\n{text}"
    );
    assert_golden!(name, rendered);
}

#[test]
fn flight_path_100() {
    check("flight_path_100", 100);
}

#[test]
fn flight_path_80() {
    check("flight_path_80", 80);
}

/// Typing narrows the entries and the title counts the matches.
#[test]
fn flight_path_search_counts_the_matches() {
    let _g = lock();
    let (tree, leaf) = fixture();
    let mut selector = TreeSelectorComponent::new(&tree, Some(leaf), 24, None, None);
    for ch in "width".chars() {
        selector.handle_input(&ch.to_string());
    }
    let rendered = render_golden(&mut selector, 100);
    let text = rendered.split("--- styles ---").next().unwrap_or_default();
    assert!(
        text.contains("1 node of 3"),
        "missing filtered count in\n{text}"
    );
}

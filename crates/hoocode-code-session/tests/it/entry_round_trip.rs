//! Every session entry and message type survives a parse and serialize round trip.
//! Fixture: tests/fixtures/all-entry-types.jsonl (current session version).

use hoocode_agent_types::AgentMessage;
use hoocode_code_session::{load_session_file, FileEntry};
use serde_json::Value;
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// Semantic equality: JS-style `0` equals serde's `0.0`, and `content: "text"` equals
/// `[{"type":"text","text":"text"}]`.
fn canon(v: &Value) -> Value {
    match v {
        Value::Number(n) => serde_json::json!(n.as_f64().unwrap()),
        Value::Array(a) => Value::Array(a.iter().map(canon).collect()),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, v)| match (k.as_str(), v) {
                    ("content", Value::String(s)) => {
                        (k.clone(), serde_json::json!([{"type": "text", "text": s}]))
                    }
                    _ => (k.clone(), canon(v)),
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

#[test]
fn every_entry_and_message_type_round_trips() {
    let text = std::fs::read_to_string(fixture("all-entry-types.jsonl")).unwrap();
    for (i, line) in text.lines().enumerate() {
        let original: Value = serde_json::from_str(line).unwrap();
        let entry: FileEntry = serde_json::from_value(original.clone())
            .unwrap_or_else(|e| panic!("line {}: does not parse: {e}\n{line}", i + 1));
        let back = serde_json::to_value(&entry).unwrap();
        assert_eq!(canon(&back), canon(&original), "line {}", i + 1);
    }
}

#[test]
fn string_content_round_trips_as_a_string() {
    let loaded = load_session_file(fixture("all-entry-types.jsonl"));
    assert_eq!(loaded.unrecognized, 0);
    let custom = loaded
        .entries
        .iter()
        .find_map(|e| match e {
            FileEntry::Message {
                message: AgentMessage::Custom(c),
                ..
            } => Some(c.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        serde_json::to_value(&custom.content).unwrap(),
        serde_json::json!("plain string content")
    );
}

#[test]
fn a_session_of_another_version_does_not_load() {
    let path =
        std::env::temp_dir().join(format!("hoocode-old-session-{}.jsonl", std::process::id()));
    std::fs::write(
        &path,
        "{\"type\":\"session\",\"version\":2,\"id\":\"x\",\"timestamp\":\"2026-01-01T00:00:00.000Z\",\"cwd\":\"/w\"}\n",
    )
    .unwrap();
    let loaded = load_session_file(&path);
    let _ = std::fs::remove_file(&path);
    assert!(loaded.entries.is_empty());
}

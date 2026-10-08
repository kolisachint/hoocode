//! Helpers shared by the integration tests in `tests/it/`.

pub(crate) mod mock_llm;

use serde_json::Value;

/// `json.dumps(value)` with Python's defaults (`", "`/`": "` separators,
/// `ensure_ascii`): the mock streams tool-call arguments in this form.
pub(crate) fn py_dumps(value: &Value) -> String {
    fn string(s: &str, out: &mut String) {
        out.push('"');
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                '\x08' => out.push_str("\\b"),
                '\x0c' => out.push_str("\\f"),
                c if (c as u32) < 0x20 || (c as u32) > 0x7e => {
                    let mut buf = [0u16; 2];
                    for unit in c.encode_utf16(&mut buf) {
                        out.push_str(&format!("\\u{unit:04x}"));
                    }
                }
                c => out.push(c),
            }
        }
        out.push('"');
    }
    fn walk(v: &Value, out: &mut String) {
        match v {
            Value::Object(map) => {
                out.push('{');
                for (i, (k, x)) in map.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    string(k, out);
                    out.push_str(": ");
                    walk(x, out);
                }
                out.push('}');
            }
            Value::Array(items) => {
                out.push('[');
                for (i, x) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    walk(x, out);
                }
                out.push(']');
            }
            Value::String(s) => string(s, out),
            Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Value::Null => out.push_str("null"),
            Value::Number(n) => out.push_str(&n.to_string()),
        }
    }
    let mut out = String::new();
    walk(value, &mut out);
    out
}

/// `json.dumps(v, ensure_ascii=False, separators=(",", ":"))` with key order kept.
pub(crate) fn compact(value: &Value) -> String {
    serde_json::to_string(value).expect("serialize")
}

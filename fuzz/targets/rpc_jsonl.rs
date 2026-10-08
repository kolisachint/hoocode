//! Target `rpc_jsonl`: LF-delimited JSON framing for RPC mode (`hoocode-code-rpc`).
//!
//! Must not panic. Lines must not depend on how the input was chunked. Each line that
//! is JSON must survive `serialize_json_line` and parse back to the same value.

use hoocode_code_rpc::jsonl::{serialize_json_line, JsonlLineReader};
use serde_json::Value;

pub fn run(data: &[u8]) {
    let whole = {
        let mut reader = JsonlLineReader::new();
        let mut lines = reader.push(data);
        lines.extend(reader.finish());
        lines
    };

    let step = 1 + usize::from(data.first().copied().unwrap_or(0)) % 9;
    let chunked = {
        let mut reader = JsonlLineReader::new();
        let mut lines = Vec::new();
        for chunk in data.chunks(step) {
            lines.extend(reader.push(chunk));
        }
        lines.extend(reader.finish());
        lines
    };
    assert_eq!(whole, chunked, "lines depend on chunk boundaries");

    // The bounded reader must cope with oversized and unterminated lines too.
    let mut bounded = JsonlLineReader::with_max_buffer(64);
    bounded.push(data);
    bounded.finish();

    for line in &whole {
        if let Ok(value) = serde_json::from_str::<Value>(line) {
            let framed = serialize_json_line(&value);
            assert!(framed.ends_with('\n') && framed.matches('\n').count() == 1);
            let back: Value = serde_json::from_str(framed.trim_end_matches('\n'))
                .expect("serialized JSON parses");
            assert_eq!(value, back, "round trip changed the value");
        }
    }
}

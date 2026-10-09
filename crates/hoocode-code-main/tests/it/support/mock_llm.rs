//! In-process mock LLM for the integration tests: an OpenAI-compatible chat-completions
//! server that streams a scripted list of turns and records every request. A port of
//! `scripts/tui/mockllm.py`, so tests that need a model need no Python.

#![allow(clippy::disallowed_methods)] // test code: a listener thread and one thread per connection

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{Map, Value};

use super::{compact, py_dumps};

const CHUNK_SIZE: usize = 8;
const CHUNK_DELAY: Duration = Duration::from_millis(10);

pub(crate) struct MockLlm {
    pub(crate) port: u16,
    log: Arc<Mutex<Vec<String>>>,
}

impl MockLlm {
    pub(crate) fn start(script: Vec<Value>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock");
        let port = listener.local_addr().expect("addr").port();
        let log = Arc::new(Mutex::new(Vec::new()));
        let turns = Arc::new(Mutex::new((script, 0usize)));
        let log2 = log.clone();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (log, turns) = (log2.clone(), turns.clone());
                thread::spawn(move || {
                    let _ = serve(stream, &log, &turns);
                });
            }
        });
        Self { port, log }
    }

    pub(crate) fn requests(&self) -> Vec<String> {
        self.log.lock().expect("log").clone()
    }
}

type Turns = Mutex<(Vec<Value>, usize)>;

fn serve(stream: TcpStream, log: &Mutex<Vec<String>>, turns: &Turns) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut out = stream;
    loop {
        let mut request_line = String::new();
        if reader.read_line(&mut request_line)? == 0 {
            return Ok(());
        }
        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or_default().to_string();
        let path = parts.next().unwrap_or_default().to_string();
        let mut length = 0usize;
        loop {
            let mut header = String::new();
            reader.read_line(&mut header)?;
            let header = header.trim_end();
            if header.is_empty() {
                break;
            }
            if let Some((k, v)) = header.split_once(':') {
                if k.eq_ignore_ascii_case("content-length") {
                    length = v.trim().parse().unwrap_or(0);
                }
            }
        }
        let mut raw = vec![0u8; length];
        reader.read_exact(&mut raw)?;
        let trimmed = path.trim_end_matches('/');
        if method == "GET" {
            if trimmed.ends_with("/models") {
                send_json(
                    &mut out,
                    200,
                    &serde_json::json!({"object": "list", "data": [{"id": "mock-model", "object": "model"}]}),
                )?;
            } else {
                send_json(&mut out, 404, &serde_json::json!({"error": "not found"}))?;
            }
            continue;
        }
        let body: Value = if raw.is_empty() {
            Value::Object(Map::new())
        } else {
            serde_json::from_slice(&raw).unwrap_or_else(
                |_| serde_json::json!({"_unparseable": String::from_utf8_lossy(&raw)}),
            )
        };
        log.lock()
            .expect("log")
            .push(compact(&serde_json::json!({"path": path, "body": body})));
        if !trimmed.ends_with("/chat/completions") {
            send_json(
                &mut out,
                404,
                &serde_json::json!({"error": format!("unsupported path {path}")}),
            )?;
            continue;
        }
        let turn = {
            let mut t = turns.lock().expect("turns");
            let turn =
                t.0.get(t.1)
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({"text": "[mockllm: script exhausted]"}));
            t.1 += 1;
            turn
        };
        if let Some(delay) = turn.get("delay_s").and_then(Value::as_f64) {
            thread::sleep(Duration::from_secs_f64(delay));
        }
        if let Some(error) = turn.get("error").and_then(Value::as_str) {
            let status = turn.get("status").and_then(Value::as_u64).unwrap_or(500) as u16;
            send_json(
                &mut out,
                status,
                &serde_json::json!({"error": {"message": error}}),
            )?;
            continue;
        }
        if !body.get("stream").and_then(Value::as_bool).unwrap_or(false) {
            send_json(
                &mut out,
                400,
                &serde_json::json!({"error": {"message": "mockllm only supports stream=true"}}),
            )?;
            continue;
        }
        stream_turn(&mut out, &turn, &body)?;
    }
}

fn send_json(out: &mut TcpStream, status: u16, payload: &Value) -> std::io::Result<()> {
    let data = py_dumps(payload);
    write!(
        out,
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{data}",
        data.len()
    )?;
    out.flush()
}

fn chunks(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return vec![String::new()];
    }
    chars
        .chunks(CHUNK_SIZE)
        .map(|c| c.iter().collect())
        .collect()
}

fn stream_turn(out: &mut TcpStream, turn: &Value, body: &Value) -> std::io::Result<()> {
    write!(
        out,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nTransfer-Encoding: chunked\r\n\r\n"
    )?;
    let model = body
        .get("model")
        .cloned()
        .unwrap_or(Value::from("mock-model"));
    let delta = |d: Value, finish: Value| {
        serde_json::json!({"id": "chatcmpl-mock", "object": "chat.completion.chunk", "created": 0, "model": model,
            "choices": [{"index": 0, "delta": d, "finish_reason": finish}]})
    };
    let mut send = |line: String| -> std::io::Result<()> {
        let data = format!("data: {line}\n\n");
        write!(out, "{:x}\r\n{data}\r\n", data.len())?;
        out.flush()?;
        thread::sleep(CHUNK_DELAY);
        Ok(())
    };
    send(compact(&delta(
        serde_json::json!({"role": "assistant", "content": ""}),
        Value::Null,
    )))?;
    for (key, field) in [("thinking", "reasoning_content"), ("text", "content")] {
        if let Some(text) = turn
            .get(key)
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
        {
            for part in chunks(text) {
                send(compact(&delta(
                    serde_json::json!({ field: part }),
                    Value::Null,
                )))?;
            }
        }
    }
    let calls = turn
        .get("tool_calls")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for (i, call) in calls.iter().enumerate() {
        let id = call
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or(format!("call_{i}"));
        let args = py_dumps(call.get("arguments").unwrap_or(&Value::Object(Map::new())));
        let start = serde_json::json!({"tool_calls": [{"index": i, "id": id, "type": "function",
            "function": {"name": call["name"], "arguments": ""}}]});
        send(compact(&delta(start, Value::Null)))?;
        for part in chunks(&args) {
            let d =
                serde_json::json!({"tool_calls": [{"index": i, "function": {"arguments": part}}]});
            send(compact(&delta(d, Value::Null)))?;
        }
    }
    let usage = turn.get("usage").cloned().unwrap_or(
        serde_json::json!({"prompt_tokens": 100, "completion_tokens": 20, "total_tokens": 120}),
    );
    let finish = if calls.is_empty() {
        "stop"
    } else {
        "tool_calls"
    };
    let mut last = delta(Value::Object(Map::new()), Value::from(finish));
    last["usage"] = usage;
    send(compact(&last))?;
    send("[DONE]".to_string())?;
    write!(out, "0\r\n\r\n")?;
    out.flush()
}

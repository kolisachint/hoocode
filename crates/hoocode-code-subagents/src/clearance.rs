//! Clearance relay between a subagent child and the process that started it.
//!
//! A subagent runs as `hoocode --mode json --task-id <id>` with no screen, so
//! it cannot ask for approval itself. When its permission gate needs a human,
//! it asks its parent instead, over the pipes it already has:
//!
//! - child stdout: `{"type":"clearance_request","id":7,"tool":"Shell","origin":{"kind":"builtIn"},
//!   "input":{..}}`, plus `"via":"<subagent chain>"` when the call came from a deeper subagent
//!   the child is relaying for;
//! - child stdin: `{"type":"clearance_response","id":7,"allow":false,"reason":".."}`.
//!
//! The parent's own gate decides (its config rules, its session grants, its
//! card), so an `always` answered once covers every subagent. Everything that
//! can go wrong is a denial: no UI in the parent, a closed pipe, a cancelled
//! task. A child that is itself a parent relays upward the same way, so a
//! grandchild's request reaches the screen through every level.

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use hoocode_agent_types::ToolOrigin;
use hoocode_code_permissions::{ClearanceRelay, RelayRequest};
use serde_json::{json, Value};

/// `type` of the child's request line.
pub const REQUEST_TYPE: &str = "clearance_request";
/// `type` of the parent's answer line.
pub const RESPONSE_TYPE: &str = "clearance_response";
/// `type` of a child's EnsureHelper request line (spec D5): the child asks
/// its parent for the socket path of a shared tier C helper.
pub const ENSURE_REQUEST_TYPE: &str = "native_ensure_helper";
/// `type` of the parent's EnsureHelper answer line.
pub const ENSURE_RESPONSE_TYPE: &str = "native_ensure_helper_response";

/// Why a call is denied when the parent has nobody to ask.
pub const NO_UI_REASON: &str =
    "Tool needs approval, but the session that started this subagent has no UI to ask. Add the tool to auto_allow for the mode in hoo-config.json to allow it.";
/// Why a call is denied when the parent's end of the pipe is gone.
const PARENT_GONE: &str = "Tool needs approval, but the parent hoocode is no longer reachable.";
/// Why a call is denied when the task ended while it waited.
pub const CANCELLED_REASON: &str = "The subagent ended before the call was answered.";
/// A request line larger than this is denied rather than relayed.
const MAX_REQUEST_LINE: usize = 4 * 1024 * 1024;
/// How often a waiting request checks whether it was cancelled.
const POLL: Duration = Duration::from_millis(250);

/// The child's request line (without the newline). `origin` is where the tool comes from; the
/// parent gates on it, never on the name.
pub fn request_line(
    id: u64,
    tool: &str,
    origin: &ToolOrigin,
    input: &Value,
    via: Option<&str>,
) -> String {
    let mut line = json!({
        "type": REQUEST_TYPE, "id": id, "tool": tool,
        "origin": serde_json::to_value(origin).unwrap_or(Value::Null), "input": input,
    });
    if let (Some(via), Some(map)) = (via, line.as_object_mut()) {
        map.insert("via".into(), Value::String(via.to_string()));
    }
    line.to_string()
}

/// A child's EnsureHelper request line for `plugin` (spec D5): the parent
/// ensures its own connection, registers the child, and answers with the
/// helper's socket path.
pub fn ensure_request_line(id: u64, plugin: &str) -> String {
    json!({"type": ENSURE_REQUEST_TYPE, "id": id, "plugin": plugin}).to_string()
}

/// The parent's EnsureHelper answer for `id`: `Ok(socket)` is the helper's
/// socket path, `Err(error)` is why the child cannot use the helper.
pub fn ensure_response_line(id: u64, result: &Result<String, String>) -> String {
    match result {
        Ok(socket) => json!({"type": ENSURE_RESPONSE_TYPE, "id": id, "socket": socket}).to_string(),
        Err(error) => json!({"type": ENSURE_RESPONSE_TYPE, "id": id, "error": error}).to_string(),
    }
    .to_string()
}

/// Read one EnsureHelper answer line: the id and either the socket path or
/// the error. `None` when the line is not one.
pub fn parse_ensure_response(line: &str) -> Option<(u64, Result<String, String>)> {
    let Value::Object(map) = serde_json::from_str::<Value>(line.trim()).ok()? else {
        return None;
    };
    if map.get("type").and_then(Value::as_str) != Some(ENSURE_RESPONSE_TYPE) {
        return None;
    }
    let id = map.get("id").and_then(Value::as_u64)?;
    if let Some(socket) = map.get("socket").and_then(Value::as_str) {
        return Some((id, Ok(socket.to_owned())));
    }
    let error = map
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or("the parent denied the helper")
        .to_owned();
    Some((id, Err(error)))
}

/// The parent's answer line (without the newline). `verdict` is the gate's
/// own shape: `None` allows, `Some(reason)` denies.
pub fn response_line(id: u64, verdict: &Option<String>) -> String {
    match verdict {
        None => json!({"type": RESPONSE_TYPE, "id": id, "allow": true}),
        Some(reason) => {
            json!({"type": RESPONSE_TYPE, "id": id, "allow": false, "reason": reason})
        }
    }
    .to_string()
}

/// The id and verdict of an answer line. `None` for any other line.
pub fn parse_response(line: &str) -> Option<(u64, Option<String>)> {
    let Value::Object(map) = serde_json::from_str::<Value>(line.trim()).ok()? else {
        return None;
    };
    if map.get("type").and_then(Value::as_str) != Some(RESPONSE_TYPE) {
        return None;
    }
    let id = map.get("id").and_then(Value::as_u64)?;
    // Anything but an explicit allow is a denial.
    let verdict = match map.get("allow").and_then(Value::as_bool) {
        Some(true) => None,
        _ => Some(
            map.get("reason")
                .and_then(Value::as_str)
                .filter(|reason| !reason.trim().is_empty())
                .unwrap_or("Denied by permission gate")
                .to_string(),
        ),
    };
    Some((id, verdict))
}

#[derive(Default)]
struct RelayState {
    /// Requests waiting for an answer, by id.
    pending: HashMap<u64, Sender<Option<String>>>,
    /// EnsureHelper answers the child waits for (spec D5), by request id.
    ensure_pending: HashMap<u64, Sender<Result<String, String>>>,
    /// The parent's stdin ended: nothing will be answered any more.
    closed: bool,
}

/// The child's end of the relay: asks the parent over stdout, hears the answer
/// on stdin. One per process, shared by its gate and (for grandchildren) its
/// subagent pool.
pub struct ChildRelay {
    out: Mutex<Box<dyn Write + Send>>,
    state: Mutex<RelayState>,
    next: AtomicU64,
    /// How often a waiting request writes `{"ping":true}`, so a child parked on
    /// a human still counts as alive.
    ping_every: Duration,
}

impl ChildRelay {
    /// A relay writing to `out`. Call [`serve`](Self::serve) to hear answers.
    pub fn new(out: Box<dyn Write + Send>, ping_every: Duration) -> Arc<Self> {
        Arc::new(Self {
            out: Mutex::new(out),
            state: Mutex::new(RelayState::default()),
            next: AtomicU64::new(1),
            ping_every,
        })
    }

    fn state(&self) -> MutexGuard<'_, RelayState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Reads answers from `input` on a thread of its own. When `input` ends
    /// (the parent closed it or died) every waiting request, and every later
    /// one, is denied.
    pub fn serve(self: &Arc<Self>, input: impl BufRead + Send + 'static) {
        let relay = self.clone();
        let started = hoocode_runtime::spawn_named_thread("hoocode-clearance-in", move || {
            for line in input.lines() {
                let Ok(line) = line else { break };
                relay.route(&line);
            }
            relay.close();
        });
        if started.is_err() {
            // No reader: nothing can ever be answered.
            self.close();
        }
    }

    /// Hands one answer line to the request waiting for it: a clearance
    /// answer to its clearance request, an EnsureHelper answer to its
    /// EnsureHelper request. Anything else is not for the relay.
    fn route(&self, line: &str) {
        if let Some((id, verdict)) = parse_response(line) {
            if let Some(waiting) = self.state().pending.remove(&id) {
                let _ = waiting.send(verdict);
            }
            return;
        }
        if let Some((id, result)) = parse_ensure_response(line) {
            if let Some(waiting) = self.state().ensure_pending.remove(&id) {
                let _ = waiting.send(result);
            }
        }
    }

    /// The input ended: deny everything waiting, now and later.
    fn close(&self) {
        let mut state = self.state();
        state.closed = true;
        for (_, waiting) in state.pending.drain() {
            let _ = waiting.send(Some(PARENT_GONE.to_string()));
        }
        for (_, waiting) in state.ensure_pending.drain() {
            let _ = waiting.send(Err(PARENT_GONE.to_string()));
        }
    }

    /// One whole line in one write, so it cannot interleave with the lines the
    /// run loop writes to the same stdout from another thread.
    fn write_line(&self, line: &str) -> bool {
        let mut out = self.out.lock().unwrap_or_else(|e| e.into_inner());
        out.write_all(format!("{line}\n").as_bytes())
            .and_then(|()| out.flush())
            .is_ok()
    }
}

impl ChildRelay {
    /// Ask the parent for the socket path of the shared tier C helper
    /// `plugin` (spec D5): the parent ensures its own connection, registers
    /// this agent, and answers. `Err` when the parent denies or is gone.
    pub fn ask_ensure(&self, plugin: &str) -> Result<String, String> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = channel();
        {
            let mut state = self.state();
            if state.closed {
                return Err(PARENT_GONE.to_string());
            }
            state.ensure_pending.insert(id, tx);
        }
        if !self.write_line(&ensure_request_line(id, plugin)) {
            self.state().ensure_pending.remove(&id);
            return Err(PARENT_GONE.to_string());
        }
        // The same wait as `ask`: ping while nothing arrives (liveness),
        // and an error when the input ends (the parent is gone).
        let mut last_ping = Instant::now();
        loop {
            match rx.recv_timeout(POLL) {
                Ok(result) => return result,
                Err(RecvTimeoutError::Disconnected) => return Err(PARENT_GONE.to_string()),
                Err(RecvTimeoutError::Timeout) => {}
            }
            let pinged = last_ping.elapsed() >= self.ping_every;
            if pinged && !self.write_line(r#"{"ping":true}"#) {
                self.state().ensure_pending.remove(&id);
                return Err(PARENT_GONE.to_string());
            }
            if pinged {
                last_ping = Instant::now();
            }
        }
    }
}

impl ClearanceRelay for ChildRelay {
    fn ask(&self, request: &RelayRequest) -> Option<String> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (reply, answer) = channel();
        {
            let mut state = self.state();
            if state.closed {
                return Some(PARENT_GONE.to_string());
            }
            state.pending.insert(id, reply);
        }
        let line = request_line(
            id,
            &request.tool,
            &request.origin,
            &request.input,
            request.via.as_deref(),
        );
        if line.len() > MAX_REQUEST_LINE || !self.write_line(&line) {
            self.state().pending.remove(&id);
            return Some(PARENT_GONE.to_string());
        }
        let mut last_ping = Instant::now();
        loop {
            match answer.recv_timeout(POLL) {
                Ok(verdict) => return verdict,
                Err(RecvTimeoutError::Disconnected) => return Some(PARENT_GONE.to_string()),
                Err(RecvTimeoutError::Timeout) => {}
            }
            let gone = request.cancelled.load(Ordering::SeqCst);
            let pinged = last_ping.elapsed() >= self.ping_every;
            if gone || (pinged && !self.write_line(r#"{"ping":true}"#)) {
                self.state().pending.remove(&id);
                return Some(if gone { CANCELLED_REASON } else { PARENT_GONE }.to_string());
            }
            if pinged {
                last_ping = Instant::now();
            }
        }
    }
}

//! T3.1: a subagent run's live events reach the UI, with text deltas coalesced
//! to the streaming render rate; T3.2's run lookup. See `live.rs`.

#![allow(clippy::disallowed_methods)] // test code: #[tokio::test] expands to a runtime builder

use std::sync::Arc;
use std::time::{Duration, Instant};

use hoocode_code_subagents::instance::{set_live_sink, wire_live_events};
use hoocode_code_subagents::live::{
    apply, child_stdout_events, is_running, remember_run, run_for_tool_call, tail, DeltaCoalescer,
    LiveForwarder, LiveSink, DELTA_THROTTLE,
};
use hoocode_code_subagents::pool::{PoolEvent, SubagentPool, SubagentPoolOptions};
use serde_json::{json, Value};

use crate::SERIAL;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn delta_event(text: &str) -> Value {
    json!({
        "type": "message_update",
        "assistantMessageEvent": {
            "type": "text_delta",
            "contentIndex": 0,
            "delta": text,
            "partial": {"role": "assistant", "content": []},
        },
        "message": {"role": "assistant", "content": []},
    })
}

fn progress(task: &str, event: Value) -> PoolEvent {
    PoolEvent {
        name: "task_progress",
        data: json!({"task_id": task, "agent_type": "explore", "event": event}),
    }
}

fn lifecycle(name: &'static str, task: &str) -> PoolEvent {
    PoolEvent {
        name,
        data: json!({"task_id": task}),
    }
}

/// What a forwarded pair reads as: `text:<delta>` for a text delta, else its type.
fn kinds(pairs: Vec<(String, Value)>) -> Vec<String> {
    pairs
        .into_iter()
        .map(|(_, event)| {
            let kind = event["type"].as_str().unwrap_or("").to_string();
            match event.get("assistantMessageEvent") {
                Some(inner) if kind == "message_update" => {
                    format!("text:{}", inner["delta"].as_str().unwrap_or(""))
                }
                _ => kind,
            }
        })
        .collect()
}

#[test]
fn a_text_delta_inside_the_window_is_held_and_released_with_the_next_one() {
    let mut forwarder = LiveForwarder::default();
    let t0 = Instant::now();
    let w = DELTA_THROTTLE;
    let out = forwarder.forward(&progress("t1", delta_event("a")), t0, w);
    assert_eq!(kinds(out), ["text:a"]);
    assert!(forwarder
        .forward(&progress("t1", delta_event("b")), t0 + ms(10), w)
        .is_empty());
    assert!(forwarder
        .forward(&progress("t1", delta_event("c")), t0 + ms(50), w)
        .is_empty());
    let out = forwarder.forward(&progress("t1", delta_event("d")), t0 + ms(120), w);
    assert_eq!(
        kinds(out),
        ["text:bcd"],
        "held text is grouped, not dropped"
    );
}

#[test]
fn the_end_of_a_message_releases_held_text_before_the_event() {
    let mut forwarder = LiveForwarder::default();
    let t0 = Instant::now();
    let w = DELTA_THROTTLE;
    forwarder.forward(&progress("t1", delta_event("a")), t0, w);
    forwarder.forward(&progress("t1", delta_event("b")), t0 + ms(10), w);
    let end = json!({"type": "message_end", "message": {"role": "assistant"}});
    let out = forwarder.forward(&progress("t1", end), t0 + ms(20), w);
    assert_eq!(kinds(out), ["text:b", "message_end"]);
}

#[test]
fn a_new_message_starts_a_fresh_window_after_the_end_of_the_last() {
    let mut forwarder = LiveForwarder::default();
    let t0 = Instant::now();
    let w = DELTA_THROTTLE;
    forwarder.forward(&progress("t1", delta_event("a")), t0, w);
    forwarder.forward(&progress("t1", delta_event("b")), t0 + ms(10), w);
    let end = json!({"type": "message_end", "message": {"role": "assistant"}});
    forwarder.forward(&progress("t1", end), t0 + ms(20), w);
    // The next message's first delta goes out at once, not held behind the last one's window.
    let out = forwarder.forward(&progress("t1", delta_event("c")), t0 + ms(21), w);
    assert_eq!(kinds(out), ["text:c"]);
}

#[test]
fn a_settle_releases_held_text_then_names_the_lifecycle_step() {
    let mut forwarder = LiveForwarder::default();
    let t0 = Instant::now();
    let w = DELTA_THROTTLE;
    forwarder.forward(&progress("t1", delta_event("a")), t0, w);
    forwarder.forward(&progress("t1", delta_event("b")), t0 + ms(10), w);
    let out = forwarder.forward(&lifecycle("task_done", "t1"), t0 + ms(20), w);
    assert_eq!(kinds(out), ["text:b", "task_done"]);
    // The run's throttle state went with it: its next delta is not held.
    let out = forwarder.forward(&progress("t1", delta_event("x")), t0 + ms(21), w);
    assert_eq!(kinds(out), ["text:x"]);
}

#[test]
fn each_run_is_throttled_on_its_own() {
    let mut forwarder = LiveForwarder::default();
    let t0 = Instant::now();
    let w = DELTA_THROTTLE;
    assert_eq!(
        kinds(forwarder.forward(&progress("t1", delta_event("a")), t0, w)),
        ["text:a"]
    );
    assert_eq!(
        kinds(forwarder.forward(&progress("t2", delta_event("p")), t0 + ms(5), w)),
        ["text:p"]
    );
}

#[test]
fn progress_and_lifecycle_events_reach_the_listener_in_order() {
    let mut forwarder = LiveForwarder::default();
    let now = Instant::now();
    let w = DELTA_THROTTLE;
    let start = json!({"type": "tool_execution_start", "toolName": "Read", "args": {}});
    assert_eq!(
        kinds(forwarder.forward(&progress("t1", start), now, w)),
        ["tool_execution_start"]
    );
    assert_eq!(
        kinds(forwarder.forward(&progress("t1", json!({"type": "turn_end"})), now, w)),
        ["turn_end"]
    );
    assert_eq!(
        kinds(forwarder.forward(&lifecycle("task_started", "t1"), now, w)),
        ["task_started"]
    );
}

#[test]
fn an_event_without_a_task_id_is_not_forwarded() {
    let mut forwarder = LiveForwarder::default();
    let event = PoolEvent {
        name: "task_progress",
        data: json!({"event": {"type": "turn_end"}}),
    };
    assert!(forwarder
        .forward(&event, Instant::now(), DELTA_THROTTLE)
        .is_empty());
}

#[test]
fn the_child_keeps_text_deltas_without_the_partial_message() {
    let mut coalescer = DeltaCoalescer::default();
    let t0 = Instant::now();
    let out = child_stdout_events(delta_event("a"), &mut coalescer, t0);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0]["type"], "message_update");
    let inner = &out[0]["assistantMessageEvent"];
    assert_eq!(inner["type"], "text_delta");
    assert_eq!(inner["delta"], "a");
    assert!(inner.get("partial").is_none(), "the snapshot is dropped");
    assert!(out[0].get("message").is_none());
}

#[test]
fn the_child_drops_non_text_updates_and_other_per_delta_events() {
    let mut coalescer = DeltaCoalescer::default();
    let t0 = Instant::now();
    let thinking = json!({"type": "message_update", "assistantMessageEvent": {"type": "thinking_delta", "delta": "x"}});
    assert!(child_stdout_events(thinking, &mut coalescer, t0).is_empty());
    let start = json!({"type": "message_start", "message": {}});
    assert!(child_stdout_events(start, &mut coalescer, t0).is_empty());
    let update = json!({"type": "tool_execution_update", "toolCallId": "c"});
    assert!(child_stdout_events(update, &mut coalescer, t0).is_empty());
}

#[test]
fn the_child_holds_fast_deltas_and_releases_them_before_message_end() {
    let mut coalescer = DeltaCoalescer::default();
    let t0 = Instant::now();
    let first = child_stdout_events(delta_event("a"), &mut coalescer, t0);
    assert_eq!(first.len(), 1);
    assert!(child_stdout_events(delta_event("b"), &mut coalescer, t0 + ms(10)).is_empty());
    let end = json!({"type": "message_end", "message": {"role": "assistant"}});
    let out = child_stdout_events(end, &mut coalescer, t0 + ms(20));
    assert_eq!(
        kinds(out.into_iter().map(|v| (String::new(), v)).collect()),
        ["text:b", "message_end"]
    );
}

#[test]
fn a_new_message_starts_a_fresh_throttle_window() {
    let mut coalescer = DeltaCoalescer::default();
    let t0 = Instant::now();
    let end = json!({"type": "message_end", "message": {"role": "assistant"}});
    assert_eq!(
        child_stdout_events(delta_event("a"), &mut coalescer, t0).len(),
        1
    );
    child_stdout_events(end, &mut coalescer, t0 + ms(10));
    // The next message starts 5 ms later, inside the old window: still released at once.
    let out = child_stdout_events(delta_event("b"), &mut coalescer, t0 + ms(15));
    assert_eq!(
        kinds(out.into_iter().map(|v| (String::new(), v)).collect()),
        ["text:b"]
    );
}

#[test]
fn a_run_tail_keeps_its_last_five_rows_and_closes_on_settle() {
    let run = "tail-run-five";
    apply(run, &json!({"type": "task_started"}));
    assert!(is_running(run));
    for i in 1..=7 {
        let start = json!({
            "type": "tool_execution_start",
            "toolName": "Read",
            "args": {"file_path": format!("src/f{i}.rs")},
        });
        apply(run, &start);
    }
    let rows = tail(run);
    assert_eq!(rows.len(), 5, "{rows:?}");
    assert_eq!(rows.first().map(String::as_str), Some("→ Read src/f3.rs"));
    assert_eq!(rows.last().map(String::as_str), Some("→ Read src/f7.rs"));
    apply(run, &json!({"type": "task_done"}));
    assert!(!is_running(run));
    assert!(tail(run).is_empty());
}

#[test]
fn streamed_text_shows_as_its_last_row_until_the_line_ends() {
    let run = "tail-run-text";
    apply(run, &json!({"type": "task_started"}));
    apply(
        run,
        &json!({"type": "message_update", "assistantMessageEvent": {"type": "text_delta", "delta": "Looking at the footer\nthen"}}),
    );
    assert_eq!(tail(run), ["Looking at the footer", "then"]);
    apply(run, &json!({"type": "message_end"}));
    assert_eq!(tail(run), ["Looking at the footer", "then"]);
    apply(run, &json!({"type": "task_done"}));
}

#[test]
fn a_long_streamed_line_keeps_only_its_newest_characters() {
    let run = "tail-run-long";
    apply(run, &json!({"type": "task_started"}));
    let long: String = (0..300)
        .map(|i| char::from(b'a' + (i % 26) as u8))
        .collect();
    apply(
        run,
        &json!({"type": "message_update", "assistantMessageEvent": {"type": "text_delta", "delta": long}}),
    );
    let rows = tail(run);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].chars().count(), 240);
    assert!(rows[0].ends_with(&long[long.len() - 10..]));
    apply(run, &json!({"type": "task_done"}));
}

#[test]
fn activity_of_a_run_that_never_started_is_not_kept() {
    let run = "tail-run-unknown";
    apply(
        run,
        &json!({"type": "tool_execution_start", "toolName": "Read"}),
    );
    assert!(!is_running(run));
    assert!(tail(run).is_empty());
}

#[test]
fn an_agent_call_finds_the_run_it_dispatched() {
    remember_run("call-lookup-a", "run-lookup-a");
    assert_eq!(
        run_for_tool_call("call-lookup-a").as_deref(),
        Some("run-lookup-a")
    );
    assert_eq!(run_for_tool_call("call-lookup-missing"), None);
}

#[test]
fn the_run_lookup_forgets_the_oldest_calls() {
    remember_run("call-old-0", "run-old-0");
    for i in 0..300 {
        remember_run(&format!("call-old-fill-{i}"), &format!("run-fill-{i}"));
    }
    assert_eq!(run_for_tool_call("call-old-0"), None);
    assert_eq!(
        run_for_tool_call("call-old-fill-299").as_deref(),
        Some("run-fill-299")
    );
}

#[test]
fn held_text_goes_out_before_a_later_event_of_another_kind() {
    let mut coalescer = DeltaCoalescer::default();
    let t0 = Instant::now();
    let first = child_stdout_events(delta_event("a"), &mut coalescer, t0);
    assert_eq!(first.len(), 1);
    assert!(child_stdout_events(delta_event("b"), &mut coalescer, t0 + ms(10)).is_empty());
    let start = json!({"type": "tool_execution_start", "toolName": "Read", "args": {}});
    let out = child_stdout_events(start, &mut coalescer, t0 + ms(20));
    assert_eq!(
        kinds(out.into_iter().map(|v| (String::new(), v)).collect()),
        ["text:b", "tool_execution_start"],
        "the text the child held comes before the event that follows it"
    );
}

#[test]
fn a_non_text_update_releases_held_text() {
    let mut coalescer = DeltaCoalescer::default();
    let t0 = Instant::now();
    child_stdout_events(delta_event("a"), &mut coalescer, t0);
    assert!(child_stdout_events(delta_event("c"), &mut coalescer, t0 + ms(10)).is_empty());
    let thinking = json!({"type": "message_update", "assistantMessageEvent": {"type": "thinking_delta", "delta": "x"}});
    let out = child_stdout_events(thinking, &mut coalescer, t0 + ms(20));
    assert_eq!(
        kinds(out.into_iter().map(|v| (String::new(), v)).collect()),
        ["text:c"]
    );
}

#[test]
fn control_characters_never_reach_a_tail_row() {
    let run = "tail-run-escapes";
    apply(run, &json!({"type": "task_started"}));
    apply(
        run,
        &json!({"type": "message_update", "assistantMessageEvent": {"type": "text_delta", "delta": "ok\u{1b}]52;c;Zm9v\u{7}\tdone\n"}}),
    );
    apply(
        run,
        &json!({"type": "tool_execution_start", "toolName": "Bash", "args": {"command": "echo \u{1b}[2J"}}),
    );
    let rows = tail(run);
    assert!(
        rows.iter().all(|row| !row.chars().any(char::is_control)),
        "{rows:?}"
    );
    assert_eq!(rows[0], "ok]52;c;Zm9v done");
    apply(run, &json!({"type": "task_done"}));
}

/// The pool's listener delivers a run's events to the sink the UI set.
#[tokio::test]
async fn the_pool_listener_sends_a_runs_events_to_the_live_sink() {
    let _serial = SERIAL.lock().await;
    let (tx, rx) = std::sync::mpsc::channel::<(String, Value)>();
    set_live_sink(Some(LiveSink {
        throttle: DELTA_THROTTLE,
        send: Arc::new(move |task, event| {
            let _ = tx.send((task, event));
        }),
    }));
    let pool = SubagentPool::new(SubagentPoolOptions::default());
    wire_live_events(&pool);
    pool.emit_for_testing(
        "task_progress",
        json!({"task_id": "wire-run", "event": {"type": "tool_execution_start", "toolName": "Grep"}}),
    );
    pool.emit_for_testing(
        "task_progress",
        json!({"task_id": "wire-run", "event": delta_event("a")}),
    );
    // Within the window of the previous delta: held until the message ends.
    pool.emit_for_testing(
        "task_progress",
        json!({"task_id": "wire-run", "event": delta_event("b")}),
    );
    pool.emit_for_testing("task_done", json!({"task_id": "wire-run"}));
    let got: Vec<(String, Value)> = rx
        .try_iter()
        .filter(|(task, _)| task == "wire-run")
        .collect();
    assert_eq!(
        kinds(got),
        ["tool_execution_start", "text:a", "text:b", "task_done"]
    );
    set_live_sink(None);
    pool.dispose();
}

#[test]
fn sub_steps_keep_running_and_failed_calls_and_drop_clean_ones() {
    // The run's own calls: a clean one settles away, a failed one stays with
    // its first output line, and the run's end clears them all.
    let task = "live-sub-steps-radar";
    apply(task, &json!({"type": "task_started"}));
    apply(
        task,
        &json!({"type": "tool_execution_start", "toolCallId": "c1", "toolName": "Read", "args": {"path": "a.rs"}}),
    );
    apply(
        task,
        &json!({"type": "tool_execution_start", "toolCallId": "c2", "toolName": "Read", "args": {"path": "b.rs"}}),
    );
    apply(
        task,
        &json!({"type": "tool_execution_end", "toolCallId": "c1", "isError": false,
                "result": {"content": [{"type": "text", "text": "fine"}]}}),
    );
    apply(
        task,
        &json!({"type": "tool_execution_end", "toolCallId": "c2", "isError": true,
                "result": {"content": [{"type": "text", "text": "\n  no such file\nmore"}]}}),
    );
    let steps = hoocode_code_subagents::live::sub_steps(task);
    assert_eq!(steps.len(), 1, "{steps:?}");
    assert_eq!(steps[0].tool, "Read");
    assert_eq!(steps[0].subject, "b.rs");
    assert_eq!(steps[0].failure.as_deref(), Some("no such file"));

    apply(task, &json!({"type": "task_done"}));
    assert!(hoocode_code_subagents::live::sub_steps(task).is_empty());
}

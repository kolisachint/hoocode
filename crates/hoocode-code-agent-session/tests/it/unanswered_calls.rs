#![allow(clippy::disallowed_methods)] // test code: #[tokio::test] expands to a runtime builder
//! A session that ends mid-call persists a result for every open tool call.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use crate::common::{text_result, tool, Harness, HarnessOptions};
use hoocode_agent_types::AgentMessage;
use hoocode_ai_provider_faux::{
    faux_assistant_message, faux_tool_call, FauxMessageOptions, FauxResponseStep,
};
use hoocode_ai_types::{Content, StopReason, ToolResultMessage};
use hoocode_code_agent_session::PromptOptions;
use hoocode_code_session::FileEntry;
use hoocode_code_tool_api::ToolDefinition;

const SESSION_ENDED: &str = "Session ended before this call finished";

/// A `wait` tool that ignores the abort: it marks itself running, then blocks
/// until released.
struct StuckTool {
    definition: ToolDefinition,
    running: Arc<AtomicBool>,
    release: Arc<(Mutex<bool>, Condvar)>,
}

impl StuckTool {
    fn new() -> Self {
        let running = Arc::new(AtomicBool::new(false));
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let (flag, gate) = (running.clone(), release.clone());
        let definition = tool("wait", move |_| {
            flag.store(true, Ordering::SeqCst);
            let (lock, cvar) = &*gate;
            let released = lock.lock().unwrap();
            let _ = cvar
                .wait_timeout_while(released, Duration::from_secs(10), |r| !*r)
                .unwrap();
            text_result("released")
        });
        Self {
            definition,
            running,
            release,
        }
    }

    async fn running(&self) {
        while !self.running.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    fn release(&self) {
        let (lock, cvar) = &*self.release;
        *lock.lock().unwrap() = true;
        cvar.notify_all();
    }
}

fn wait_call() -> FauxResponseStep {
    faux_assistant_message(
        faux_tool_call("wait", serde_json::json!({}), None),
        FauxMessageOptions {
            stop_reason: Some(StopReason::ToolUse),
            ..Default::default()
        },
    )
    .into()
}

fn persisted_results(h: &Harness) -> Vec<ToolResultMessage> {
    h.session
        .session_manager()
        .entries()
        .iter()
        .filter_map(|entry| match entry {
            FileEntry::Message {
                message: AgentMessage::ToolResult(result),
                ..
            } => Some(result.clone()),
            _ => None,
        })
        .collect()
}

fn text_of(result: &ToolResultMessage) -> String {
    match result.content.first() {
        Some(Content::Text(t)) => t.text.clone(),
        other => format!("{other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shutdown_abort_persists_session_ended_for_a_call_that_ignores_it() {
    let wait = StuckTool::new();
    let h = Harness::new(HarnessOptions {
        tools: vec![wait.definition.clone()],
        ..Default::default()
    });
    h.set_responses(vec![
        wait_call(),
        faux_assistant_message("done", Default::default()).into(),
    ]);
    let session = h.session.clone();
    let run = tokio::spawn(async move { session.prompt("go", PromptOptions::default()).await });
    wait.running().await;

    // The wait tool ignores the abort; the loop answers it after the grace.
    h.session.abort_for_shutdown().await;
    run.await.unwrap().unwrap();
    wait.release();

    let results = persisted_results(&h);
    assert_eq!(results.len(), 1);
    assert_eq!(text_of(&results[0]), SESSION_ENDED);
    assert!(results[0].is_error);
    assert_eq!(h.session.close_unanswered_tool_calls(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn closing_open_calls_at_shutdown_persists_one_result_per_call() {
    let wait = StuckTool::new();
    let h = Harness::new(HarnessOptions {
        tools: vec![wait.definition.clone()],
        ..Default::default()
    });
    h.set_responses(vec![
        wait_call(),
        faux_assistant_message("done", Default::default()).into(),
    ]);
    let session = h.session.clone();
    let run = tokio::spawn(async move { session.prompt("go", PromptOptions::default()).await });
    wait.running().await;

    // A shutdown that could not wait for the run (a signal, a stuck abort).
    assert_eq!(h.session.try_close_unanswered_tool_calls(), Some(1));
    let results = persisted_results(&h);
    assert_eq!(results.len(), 1);
    assert_eq!(text_of(&results[0]), SESSION_ENDED);

    // The late real result is not persisted a second time.
    wait.release();
    run.await.unwrap().unwrap();
    let results = persisted_results(&h);
    assert_eq!(results.len(), 1, "{results:?}");
    assert_eq!(text_of(&results[0]), SESSION_ENDED);
}

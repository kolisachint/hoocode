//! An abort answers every tool call of the assistant message: calls still
//! running get "Aborted by the user", calls not yet started get "Not run".

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use hoocode_agent_loop::{default_convert_to_llm, run_agent_loop, AgentEventSink};
use hoocode_agent_types::{
    AgentContext, AgentEvent, AgentLoopConfig, AgentMessage, AgentTool, AgentToolResult,
    ToolExecutionMode,
};
use hoocode_ai_stream::create_assistant_message_event_stream;
use hoocode_ai_types::{
    AbortSignal, AssistantMessage, AssistantMessageEvent, Content, Model, ModelCost, StopReason,
    ToolCallContent, ToolResultMessage,
};

const ABORTED: &str = "Aborted by the user before this call finished";
const NOT_RUN: &str = "Not run: the turn was aborted";

fn model() -> Model {
    Model {
        id: "mock".into(),
        name: "mock".into(),
        api: "openai-responses".into(),
        provider: "openai".into(),
        base_url: "https://example.invalid".into(),
        reasoning: false,
        thinking_level_map: None,
        input: vec!["text".into()],
        cost: ModelCost::default(),
        context_window: 8192,
        max_tokens: 2048,
        headers: None,
        compat: None,
    }
}

fn assistant(content: Vec<Content>, stop_reason: StopReason) -> AssistantMessage {
    AssistantMessage {
        content,
        api: "openai-responses".into(),
        provider: "openai".into(),
        model: "mock".into(),
        stop_reason,
        ..Default::default()
    }
}

/// The first response calls `names` (ids `c0`, `c1`, ...); the next one ends the run.
fn config_calling(names: &[&str], signal: &AbortSignal) -> AgentLoopConfig {
    let calls: Vec<Content> = names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            Content::ToolCall(ToolCallContent {
                id: format!("c{i}"),
                name: (*name).into(),
                arguments: serde_json::json!({}),
                thought_signature: None,
            })
        })
        .collect();
    let responses = AtomicUsize::new(0);
    let mut config = AgentLoopConfig::new(model());
    config.convert_to_llm = Some(Box::new(default_convert_to_llm));
    config.signal = Some(signal.clone());
    config.stream_fn = Some(Box::new(move |_model, _context, _options| {
        let message = if responses.fetch_add(1, Ordering::SeqCst) == 0 {
            assistant(calls.clone(), StopReason::ToolUse)
        } else {
            assistant(vec![Content::text("done")], StopReason::Stop)
        };
        let stream = create_assistant_message_event_stream();
        stream.push(AssistantMessageEvent::Done { message });
        Ok(stream)
    }));
    config
}

/// Released at the end of a test, so a tool that ignores the abort returns.
#[derive(Default)]
struct Gate(Mutex<bool>, Condvar);

impl Gate {
    fn wait(&self) {
        let open = self.0.lock().unwrap();
        let _ = self
            .1
            .wait_timeout_while(open, Duration::from_secs(10), |open| !*open)
            .unwrap();
    }

    fn open(&self) {
        *self.0.lock().unwrap() = true;
        self.1.notify_all();
    }
}

fn ok_result(text: &str) -> AgentToolResult {
    AgentToolResult {
        content: vec![Content::text(text)],
        details: serde_json::json!({}),
        terminate: false,
    }
}

/// A tool that ignores the abort: it reports its start, then blocks on `gate`.
fn stuck_tool(name: &str, started: Arc<AtomicUsize>, gate: Arc<Gate>) -> AgentTool {
    AgentTool::new(
        name,
        "blocks",
        serde_json::json!({"type": "object"}),
        Box::new(move |_id, _args, _signal, _update| {
            started.fetch_add(1, Ordering::SeqCst);
            gate.wait();
            Ok(ok_result("finished late"))
        }),
    )
}

/// A tool that aborts the run itself (as the user would mid-call) and returns.
fn aborting_tool(name: &str, signal: AbortSignal) -> AgentTool {
    AgentTool::new(
        name,
        "aborts",
        serde_json::json!({"type": "object"}),
        Box::new(move |_id, _args, _signal, _update| {
            signal.abort();
            Ok(ok_result("aborted the run"))
        }),
    )
}

fn counting_tool(name: &str, runs: Arc<AtomicUsize>) -> AgentTool {
    AgentTool::new(
        name,
        "counts",
        serde_json::json!({"type": "object"}),
        Box::new(move |_id, _args, _signal, _update| {
            runs.fetch_add(1, Ordering::SeqCst);
            Ok(ok_result("ran"))
        }),
    )
}

/// The tool results the run announced with `message_end`: what a session persists.
type Persisted = Arc<Mutex<Vec<ToolResultMessage>>>;

fn sink() -> (AgentEventSink, Persisted) {
    let persisted: Persisted = Arc::default();
    let seen = persisted.clone();
    let emit: AgentEventSink = Box::new(move |event| {
        if let AgentEvent::MessageEnd {
            message: AgentMessage::ToolResult(result),
        } = event
        {
            seen.lock().unwrap().push(result);
        }
    });
    (emit, persisted)
}

async fn run(tools: Vec<AgentTool>, config: AgentLoopConfig) -> Vec<ToolResultMessage> {
    let context = AgentContext::new(String::new(), vec![], tools);
    let (mut emit, persisted) = sink();
    let messages = run_agent_loop(
        vec![AgentMessage::user_text("go")],
        context,
        &config,
        &mut emit,
    )
    .await
    .expect("the run succeeded");
    let returned: Vec<ToolResultMessage> = messages
        .into_iter()
        .filter_map(|m| match m {
            AgentMessage::ToolResult(r) => Some(r),
            _ => None,
        })
        .collect();
    let persisted = persisted.lock().unwrap().clone();
    assert_eq!(
        returned
            .iter()
            .map(|r| r.tool_call_id.clone())
            .collect::<Vec<_>>(),
        persisted
            .iter()
            .map(|r| r.tool_call_id.clone())
            .collect::<Vec<_>>(),
        "every returned result was announced for persisting"
    );
    persisted
}

fn summary(results: &[ToolResultMessage]) -> Vec<(String, String, bool)> {
    results
        .iter()
        .map(|r| {
            let text = match r.content.first() {
                Some(Content::Text(t)) => t.text.clone(),
                other => format!("{other:?}"),
            };
            (r.tool_call_id.clone(), text, r.is_error)
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn abort_with_two_parallel_running_calls_persists_two_results() {
    let signal = AbortSignal::new();
    let started = Arc::new(AtomicUsize::new(0));
    let gate = Arc::new(Gate::default());
    let tools = vec![
        stuck_tool("stuck_a", started.clone(), gate.clone()),
        stuck_tool("stuck_b", started.clone(), gate.clone()),
    ];
    let config = config_calling(&["stuck_a", "stuck_b"], &signal);

    let run = tokio::spawn(run(tools, config));
    while started.load(Ordering::SeqCst) < 2 {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let aborted_at = Instant::now();
    signal.abort();
    let results = run.await.unwrap();
    // The loop stopped waiting after the grace, not when the tools returned.
    assert!(aborted_at.elapsed() < Duration::from_secs(5));
    gate.open();

    assert_eq!(
        summary(&results),
        vec![
            ("c0".into(), ABORTED.into(), true),
            ("c1".into(), ABORTED.into(), true),
        ]
    );
}

#[tokio::test]
async fn abort_before_a_queued_sequential_call_starts_answers_it_not_run() {
    let signal = AbortSignal::new();
    let runs = Arc::new(AtomicUsize::new(0));
    let tools = vec![
        aborting_tool("first", signal.clone()),
        counting_tool("second", runs.clone()),
    ];
    let mut config = config_calling(&["first", "second"], &signal);
    config.tool_execution = ToolExecutionMode::Sequential;

    let results = run(tools, config).await;

    assert_eq!(runs.load(Ordering::SeqCst), 0, "the queued call never ran");
    assert_eq!(
        summary(&results),
        vec![
            ("c0".into(), "aborted the run".into(), false),
            ("c1".into(), NOT_RUN.into(), true),
        ]
    );
}

#[tokio::test]
async fn abort_before_a_waiting_parallel_call_gets_a_slot_answers_it_not_run() {
    let signal = AbortSignal::new();
    let runs = Arc::new(AtomicUsize::new(0));
    let tools = vec![
        aborting_tool("first", signal.clone()),
        counting_tool("second", runs.clone()),
    ];
    let mut config = config_calling(&["first", "second"], &signal);
    config.max_parallel_tools = 1;

    let results = run(tools, config).await;

    assert_eq!(runs.load(Ordering::SeqCst), 0, "the waiting call never ran");
    assert_eq!(
        summary(&results),
        vec![
            ("c0".into(), "aborted the run".into(), false),
            ("c1".into(), NOT_RUN.into(), true),
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tool_that_honours_the_abort_keeps_its_own_result() {
    let signal = AbortSignal::new();
    let started = Arc::new(AtomicBool::new(false));
    let seen = started.clone();
    let tool = AgentTool::new(
        "honours",
        "returns on abort",
        serde_json::json!({"type": "object"}),
        Box::new(move |_id, _args, signal, _update| {
            seen.store(true, Ordering::SeqCst);
            let signal = signal.expect("the run's signal");
            while !signal.aborted() {
                std::thread::sleep(Duration::from_millis(2));
            }
            Err("Command aborted".into())
        }),
    );
    let config = config_calling(&["honours"], &signal);

    let run = tokio::spawn(run(vec![tool], config));
    while !started.load(Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    signal.abort();
    let results = run.await.unwrap();

    assert_eq!(
        summary(&results),
        vec![("c0".into(), "Command aborted".into(), true)]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_shutdown_abort_says_the_session_ended() {
    let signal = AbortSignal::new();
    let started = Arc::new(AtomicUsize::new(0));
    let gate = Arc::new(Gate::default());
    let runs = Arc::new(AtomicUsize::new(0));
    let tools = vec![
        stuck_tool("stuck", started.clone(), gate.clone()),
        counting_tool("later", runs.clone()),
    ];
    let mut config = config_calling(&["stuck", "later"], &signal);
    config.tool_execution = ToolExecutionMode::Sequential;
    let session_ending = Arc::new(AtomicBool::new(false));
    config.session_ending = Some(session_ending.clone());

    let run = tokio::spawn(run(tools, config));
    while started.load(Ordering::SeqCst) < 1 {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    session_ending.store(true, Ordering::SeqCst);
    signal.abort();
    let results = run.await.unwrap();
    gate.open();

    assert_eq!(runs.load(Ordering::SeqCst), 0);
    assert_eq!(
        summary(&results),
        vec![
            (
                "c0".into(),
                "Session ended before this call finished".into(),
                true
            ),
            ("c1".into(), "Not run: the session ended".into(), true),
        ]
    );
}

//! Layer goldens: one scripted session rendered in each transcript layer
//! (radar, peek, full) at 80 and 120 columns.
//!
//! `Mode` needs a live session, so a session cannot be built here as a whole.
//! [`Replay`] plays the script through the same `Transcript` calls the rebuild
//! path makes (`render_session_context`, with `add_message_to_chat`,
//! `attach_tool_block`, `close_open_chain`, `show_rebuilt_turn_cost` and
//! `refresh_flight_stats`), keeping the session's own state (clock, usage
//! anchor, settings) to the script's numbers. Thinking is shown (hidden
//! thinking is off), so each layer's thinking rule shows.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Mutex, MutexGuard};

use hoocode_ai_types::{
    AssistantMessage, Content, Cost, ImageContent, StopReason, ThinkingContent, ToolCallContent,
    ToolResultMessage, Usage,
};
use hoocode_code_agent_session::stats::{add_assistant_usage, AssistantUsageTotals};
use hoocode_code_settings::ToolOutputView;
use hoocode_code_subagents::live;
use hoocode_code_tui_theme::{get_markdown_theme, init_theme};
use hoocode_code_tui_widgets::tool_chain::{speaking_ends_chain, ToolBlock, ToolChainComponent};
use hoocode_code_tui_widgets::tool_chain_summary::ChainState;
use hoocode_code_tui_widgets::tool_execution::{ToolExecutionComponent, ToolExecutionOptions};
use hoocode_code_tui_widgets::tool_signal::ToolResult;
use hoocode_code_tui_widgets::tools::registered_tool_definition;
use hoocode_code_tui_widgets::{AssistantMessageComponent, ThinkingDisplay};
use hoocode_tui_components::MarkdownTheme;
use hoocode_tui_render::golden::render_golden;
use hoocode_tui_render::{assert_golden, ComponentHandle, Container};
use serde_json::{json, Value};

use super::{rebuilt_details, Transcript};
use crate::mode::{as_component, handle};

const CWD: &str = "/work/project";
const PNG_1X1: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

static LOCK: Mutex<()> = Mutex::new(());

/// Serialises the goldens (the theme and the live subagent store are global)
/// and fixes the theme and colour depth the goldens were captured in.
fn guard() -> MutexGuard<'static, ()> {
    let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("COLORTERM", "truecolor");
    init_theme(Some("dark"), false);
    guard
}

/// The turn the replay is in: its clock and what its messages spent.
#[derive(Default)]
struct Turn {
    started_ms: i64,
    ended_ms: i64,
    usage: AssistantUsageTotals,
    stop: Option<StopReason>,
}

/// A transcript under construction, fed one message at a time.
struct Replay {
    chat: Rc<RefCell<Container>>,
    t: Transcript,
    view: ToolOutputView,
    /// Calls waiting for their results, by call id (`rendered_pending`).
    pending: Vec<(String, ToolBlock)>,
    turn: Option<Turn>,
    last_stop: Option<StopReason>,
}

fn markdown_theme() -> Rc<dyn Fn() -> MarkdownTheme> {
    Rc::new(|| MarkdownTheme {
        code_block_indent: Some("  ".to_string()),
        ..get_markdown_theme()
    })
}

impl Replay {
    fn new(view: ToolOutputView) -> Self {
        let chat = Rc::new(RefCell::new(Container::new()));
        let t = Transcript::new(chat.clone());
        // `apply_tool_output_view`: radar hides the peek-only rows (the
        // landing line and its summary); the full stop opens `!` output.
        t.radar_active.set(view == ToolOutputView::Radar);
        t.full_active.set(view == ToolOutputView::Full);
        hoocode_tui_render::note_change();
        Self {
            t,
            chat,
            view,
            pending: Vec::new(),
            turn: None,
            last_stop: None,
        }
    }

    /// `refreshFlightStats` with no live turn: the flight rows follow the
    /// transcript, and an unlanded mission has no clock or cost yet.
    fn refresh(&mut self) {
        self.t.refresh_flight_marks();
        self.t.sync_radar_prose(false);
        if let Some(mission) = self.t.missions.last() {
            if !mission.landed {
                let mut flight = mission.flight.borrow_mut();
                hoocode_tui_render::note_change();
                flight.started = None;
                flight.cost = None;
            }
        }
    }

    fn thinking_display(&self) -> ThinkingDisplay {
        if self.view == ToolOutputView::Radar {
            ThinkingDisplay::Omit
        } else {
            ThinkingDisplay::Full
        }
    }

    /// A user message: its turn settles the one before, then it opens a mission.
    fn user(&mut self, text: &str, timestamp: i64) {
        if let Some(previous) = self.turn.take() {
            self.settle_turn(previous);
        }
        self.t.begin_mission(text, None, self.view);
        self.refresh();
        self.turn = Some(Turn {
            started_ms: timestamp,
            ended_ms: timestamp,
            ..Turn::default()
        });
    }

    /// An assistant message, with the calls it makes.
    fn assistant(&mut self, message: &AssistantMessage) {
        if let Some(turn) = self.turn.as_mut() {
            add_assistant_usage(&mut turn.usage, &message.usage);
            turn.ended_ms = message.timestamp;
            turn.stop = Some(message.stop_reason);
        }
        self.last_stop = Some(message.stop_reason);
        if speaking_ends_chain(&message.content) {
            self.close_open_chain(ChainState::Done);
        }
        let component = handle(AssistantMessageComponent::with_theme(
            Some(message),
            self.thinking_display(),
            markdown_theme(),
            "Thinking...",
        ));
        {
            let mut c = component.borrow_mut();
            c.set_view(self.view);
            c.set_radar_above(self.t.radar_chain_above());
            c.set_radar_mission(self.t.current_mission());
        }
        self.t.assistant_components.push(component.clone());
        self.t.chat.borrow_mut().add_child(as_component(&component));
        self.t.sync_radar_prose(false);
        if message
            .content
            .iter()
            .any(|c| matches!(c, Content::ToolCall(_)))
        {
            self.place_trace_in_chain(&component);
        }
        for content in &message.content {
            let Content::ToolCall(ToolCallContent {
                id,
                name,
                arguments,
                ..
            }) = content
            else {
                continue;
            };
            let block = self.new_tool_block(name, id, arguments.clone());
            self.attach_tool_block(block.clone());
            if matches!(message.stop_reason, StopReason::Aborted | StopReason::Error) {
                let error = if message.stop_reason == StopReason::Aborted {
                    "Operation aborted".to_string()
                } else {
                    message
                        .error_message
                        .clone()
                        .filter(|m| !m.is_empty())
                        .unwrap_or_else(|| "Error".into())
                };
                block.borrow_mut().update_result(
                    ToolResult {
                        content: vec![Content::text(error)],
                        details: Value::Null,
                        is_error: true,
                    },
                    false,
                );
                if message.stop_reason == StopReason::Aborted {
                    block.borrow_mut().set_interrupted();
                }
            } else {
                block.borrow_mut().set_args_complete();
                self.pending.push((id.clone(), block));
            }
        }
    }

    /// A tool result for a call still waiting on it.
    fn result(
        &mut self,
        call_id: &str,
        content: Vec<Content>,
        is_error: bool,
        details: Option<Value>,
        duration_ms: Option<u64>,
        timestamp: i64,
    ) {
        if let Some(turn) = self.turn.as_mut() {
            turn.ended_ms = timestamp;
        }
        let Some(i) = self.pending.iter().position(|(id, _)| id == call_id) else {
            panic!("no pending call {call_id}");
        };
        let (_, block) = self.pending.remove(i);
        let tool_name = block.borrow().tool_name().to_string();
        let message = ToolResultMessage {
            tool_call_id: call_id.to_string(),
            tool_name,
            content: content.clone(),
            details,
            is_error,
            timestamp,
            duration_ms,
        };
        block.borrow_mut().update_result(
            ToolResult {
                content,
                details: rebuilt_details(&message),
                is_error,
            },
            false,
        );
    }

    /// `renderSessionContext`'s end: a settled turn lands its mission.
    fn finish(&mut self) {
        if !self.pending.is_empty() {
            panic!("calls still waiting for results: {:?}", self.pending.len());
        }
        if let Some(turn) = self.turn.take() {
            self.settle_turn(turn);
        }
        let outcome = if self.last_stop == Some(StopReason::Stop) {
            ChainState::Done
        } else {
            ChainState::Interrupted
        };
        self.close_open_chain(outcome);
    }

    /// `showRebuiltTurnCost`: close the turn's chain, then land its mission
    /// with the turn's tokens, time and cost. Every script opens a mission,
    /// so the accounting row beside it never draws.
    fn settle_turn(&mut self, turn: Turn) {
        let outcome = if turn.stop == Some(StopReason::Stop) {
            ChainState::Done
        } else {
            ChainState::Interrupted
        };
        self.close_open_chain(outcome);
        let secs = (turn.ended_ms - turn.started_ms).max(0) as f64 / 1000.0;
        if let Some(seq) = self.t.missions.last().map(|m| m.seq) {
            self.t.land_mission(
                seq,
                turn.usage.input as i64,
                turn.usage.output as i64,
                turn.usage.cost,
                secs,
                turn.stop,
            );
            self.refresh();
        }
    }

    /// `closeOpenChain`.
    fn close_open_chain(&mut self, outcome: ChainState) {
        let Some(chain) = self.t.open_chain.take() else {
            return;
        };
        if chain.borrow().is_empty() {
            self.t.remove_chain(&chain);
        } else {
            chain.borrow_mut().close(outcome);
        }
        self.refresh();
    }

    /// `placeTraceInChain`: a message's trace moves into the open chain.
    fn place_trace_in_chain(&mut self, component: &Rc<RefCell<AssistantMessageComponent>>) {
        let Some(chain) = self.t.open_chain.clone() else {
            return;
        };
        if !chain.borrow().is_open() || chain.borrow().has_lead_in(component) {
            return;
        }
        self.t
            .chat
            .borrow_mut()
            .remove_child(&as_component(component));
        chain.borrow_mut().add_lead_in(component.clone());
    }

    /// `newToolBlock`, with the image placeholder path (`show_images` off).
    fn new_tool_block(&self, name: &str, id: &str, args: Value) -> ToolBlock {
        handle(ToolExecutionComponent::new(
            name,
            id,
            args,
            ToolExecutionOptions {
                show_images: false,
                view: self.view,
                ..Default::default()
            },
            Some(registered_tool_definition(name)),
            CWD,
        ))
    }

    /// `attachToolBlock`: into the open chain, or a new one.
    fn attach_tool_block(&mut self, block: ToolBlock) {
        let chain = match &self.t.open_chain {
            Some(chain) if chain.borrow().is_open() => chain.clone(),
            _ => {
                let chain = handle(ToolChainComponent::new(self.view));
                chain.borrow_mut().set_base_step(self.t.mission_tally().0);
                self.t.chat.borrow_mut().add_child(as_component(&chain));
                self.t.chains.push(chain.clone());
                self.t.open_chain = Some(chain.clone());
                chain
            }
        };
        chain.borrow_mut().add(block.clone());
        if let Some(previous) = self.t.latest_block.replace(block.clone()) {
            previous.borrow_mut().set_latest(false);
        }
        block.borrow_mut().set_latest(true);
        let same = self
            .t
            .latest_chain
            .as_ref()
            .is_some_and(|c| Rc::ptr_eq(c, &chain));
        if !same {
            if let Some(previous) = self.t.latest_chain.replace(chain.clone()) {
                previous.borrow_mut().set_latest(false);
            }
            chain.borrow_mut().set_latest(true);
        }
        self.refresh();
    }

    /// The chat, as the screen draws it.
    fn render(self, width: u16) -> String {
        let chat: ComponentHandle = as_component(&self.chat);
        let mut chat = chat.borrow_mut();
        render_golden(&mut *chat, width)
    }
}

// ---------------------------------------------------------------------------
// Script helpers
// ---------------------------------------------------------------------------

fn text(s: &str) -> Content {
    Content::text(s)
}

fn thinking(s: &str) -> Content {
    Content::Thinking(ThinkingContent {
        thinking: s.to_string(),
        signature: None,
        redacted: false,
    })
}

fn call(id: &str, name: &str, arguments: Value) -> Content {
    Content::ToolCall(ToolCallContent {
        id: id.to_string(),
        name: name.to_string(),
        arguments,
        thought_signature: None,
    })
}

/// An assistant message with its tokens (input, output) and cost in USD.
fn assistant(
    content: Vec<Content>,
    stop: StopReason,
    timestamp: i64,
    (input, output, cost): (u64, u64, f64),
) -> AssistantMessage {
    AssistantMessage {
        content,
        usage: Usage {
            input,
            output,
            cost: Cost {
                total: cost,
                ..Cost::default()
            },
            ..Usage::default()
        },
        stop_reason: stop,
        timestamp,
        error_message: None,
        ..Default::default()
    }
}

fn with_error(mut message: AssistantMessage, error: &str) -> AssistantMessage {
    message.error_message = Some(error.to_string());
    message
}

fn lines(n: usize, f: impl Fn(usize) -> String) -> String {
    (1..=n).map(f).collect::<Vec<_>>().join("\n")
}

/// A diff row in the Edit renderer's shape: sign, line number, text.
fn diff_row(sign: char, number: usize, body: &str) -> String {
    format!("{sign}{number:>4} {body}")
}

// ---------------------------------------------------------------------------
// Scenarios: each returns the chat the session leaves behind.
// ---------------------------------------------------------------------------

const LANDED_PROMPT: &str = "Make the config loader apply environment overrides before the file is parsed, so an override can set the parser's own options. Keep the public API the same for every caller in the workspace.\nUpdate the configuration section of the README so the order of resolution matches the code.\nRun the config tests afterwards and report anything that fails.";

/// A mission that lands: a Read, an Edit, a Shell, then the answer.
fn landed(view: ToolOutputView, _run: &str) -> Replay {
    let mut r = Replay::new(view);
    r.user(LANDED_PROMPT, 1_000);
    r.assistant(&assistant(
        vec![
            thinking("The loader parses the file first and reads the environment after, so an override arrives too late. The file has to be read before I change anything."),
            call("read-1", "Read", json!({"file_path": "src/config.rs"})),
        ],
        StopReason::ToolUse,
        2_000,
        (4_200, 310, 0.0121),
    ));
    let source = lines(30, |i| {
        format!("    setting_{i:02} = parse_value(&raw[{i}]).unwrap_or_default();")
    });
    r.result("read-1", vec![text(&source)], false, None, None, 2_400);
    let diff = [
        diff_row(' ', 12, "    let mut cfg = Config::default();"),
        diff_row('-', 14, "    let raw = read_file(path)?;"),
        diff_row('-', 15, "    apply_env(&mut cfg);"),
        diff_row('+', 14, "    apply_env(&mut cfg);"),
        diff_row('+', 15, "    let raw = read_file(path)?;"),
        diff_row(' ', 16, "    parse_into(&mut cfg, &raw)?;"),
        diff_row(' ', 17, "    Ok(cfg)"),
    ]
    .join("\n");
    r.assistant(&assistant(
        vec![call(
            "edit-1",
            "Edit",
            json!({
                "path": "src/config.rs",
                "oldText": "    let raw = read_file(path)?;\n    apply_env(&mut cfg);",
                "newText": "    apply_env(&mut cfg);\n    let raw = read_file(path)?;",
            }),
        )],
        StopReason::ToolUse,
        2_600,
        (5_100, 460, 0.0158),
    ));
    r.result(
        "edit-1",
        vec![text("Successfully replaced 1 block(s) in src/config.rs.")],
        false,
        Some(json!({ "diff": diff, "firstChangedLine": 14 })),
        None,
        2_900,
    );
    let output = lines(40, |i| format!("test config::tests::case_{i:02} ... ok"));
    r.assistant(&assistant(
        vec![call(
            "shell-1",
            "Shell",
            json!({"command": "cargo test -p hoocode-config 2>&1 | tail -40"}),
        )],
        StopReason::ToolUse,
        3_000,
        (5_600, 120, 0.0177),
    ));
    r.result(
        "shell-1",
        vec![text(&output)],
        false,
        None,
        Some(8_200),
        11_200,
    );
    r.assistant(&assistant(
        vec![text(
            "The overrides now apply before the file is parsed, so an override can set the parser's own options.\n\nThe public API is unchanged. `Config::load` keeps its signature, and the two callers in `hoocode-cli` need no edits.\n\nThe config tests pass: 40 of 40. The README's resolution order now reads environment first, then the file, then the defaults.",
        )],
        StopReason::Stop,
        12_000,
        (6_300, 540, 0.0214),
    ));
    r.finish();
    r
}

const FAILED_PROMPT: &str = "The parser test fails on the escaped-quote case. Find the cause, fix it, and check the screenshot of the failing output too.";

/// A mission that does not land: a failing Shell, a PNG Read, then an error.
fn failed(view: ToolOutputView, _run: &str) -> Replay {
    let mut r = Replay::new(view);
    r.user(FAILED_PROMPT, 1_000);
    r.assistant(&assistant(
        vec![call(
            "shell-1",
            "Shell",
            json!({"command": "cargo test -p hoocode-parser escaped_quote"}),
        )],
        StopReason::ToolUse,
        2_000,
        (3_900, 180, 0.0097),
    ));
    let stderr = [
        "error[E0308]: mismatched types",
        "   --> crates/hoocode-parser/src/lex.rs:88:21",
        "    |",
        "88  |         return Ok(Token::Str(text));",
        "    |                     ^^^^ expected `String`, found `&str`",
        "",
        "error: could not compile `hoocode-parser` (lib) due to 1 previous error",
        // The Shell tool ends a failed run's output with its exit status.
        "",
        "Command exited with code 1",
    ]
    .join("\n");
    r.result(
        "shell-1",
        vec![text(&stderr)],
        true,
        None,
        Some(3_100),
        5_100,
    );
    r.assistant(&assistant(
        vec![call(
            "read-1",
            "Read",
            json!({"file_path": "docs/screens/parser-failure.png"}),
        )],
        StopReason::ToolUse,
        5_300,
        (4_400, 90, 0.0105),
    ));
    r.result(
        "read-1",
        vec![
            text("Read image file [image/png]"),
            Content::Image(ImageContent {
                data: PNG_1X1.into(),
                media_type: "image/png".into(),
            }),
        ],
        false,
        None,
        None,
        5_500,
    );
    r.assistant(&with_error(
        assistant(
            vec![text(
                "The escaped-quote case still fails. The lexer builds the token from a `&str` slice, but the parser expects an owned `String`, so the fix has to change the token type in `lex.rs` before the test can compile.\n\nI have not changed any file yet.",
            )],
            StopReason::Error,
            6_000,
            (4_800, 130, 0.0119),
        ),
        "Request failed: connection reset by peer",
    ));
    r.finish();
    r
}

const ABORTED_PROMPT: &str = "Run the full integration suite and report the first failure.";

/// A mission cut short: the long-running Shell is interrupted and the turn aborted.
fn aborted(view: ToolOutputView, _run: &str) -> Replay {
    let mut r = Replay::new(view);
    r.user(ABORTED_PROMPT, 1_000);
    r.assistant(&assistant(
        vec![
            thinking("The suite takes a few minutes. I will run it once and wait for the output."),
            call(
                "shell-1",
                "Shell",
                json!({"command": "./scripts/integration.sh --all"}),
            ),
        ],
        StopReason::Aborted,
        2_000,
        (2_300, 60, 0.0064),
    ));
    r.finish();
    r
}

const AGENT_PROMPT: &str = "Find where the parser handles escaped quotes in string literals.";

/// A mission that delegates to a subagent; one of its sub-steps fails.
fn agent(view: ToolOutputView, run: &str) -> Replay {
    live::apply(run, &json!({"type": "task_started"}));
    let steps = [
        (
            "s1",
            "Read",
            json!({"path": "src/parser/lex.rs"}),
            false,
            "fn read_string(&mut self) -> Token { ... }",
        ),
        (
            "s2",
            "Grep",
            json!({"pattern": "\\\\\""}),
            true,
            "rg: regex parse error: unclosed group",
        ),
        (
            "s3",
            "Read",
            json!({"path": "src/parser/tests.rs"}),
            false,
            "#[test] fn escaped_quote() { ... }",
        ),
        (
            "s4",
            "Read",
            json!({"path": "src/parser/token.rs"}),
            false,
            "pub enum Token { Str(String), ... }",
        ),
    ];
    for (id, tool, args, is_error, output) in steps {
        live::apply(
            run,
            &json!({"type": "tool_execution_start", "toolCallId": id, "toolName": tool, "args": args}),
        );
        live::apply(
            run,
            &json!({"type": "tool_execution_end", "toolCallId": id, "isError": is_error,
                    "result": {"content": [{"type": "text", "text": output}]}}),
        );
    }
    live::apply(run, &json!({"type": "task_done"}));

    let mut r = Replay::new(view);
    r.user(AGENT_PROMPT, 1_000);
    r.assistant(&assistant(
        vec![
            thinking("A subagent can trace the string-literal path while I wait."),
            call(
                "agent-1",
                "Agent",
                json!({
                    "subagent_type": "explore",
                    "description": "Trace escaped quotes",
                    "prompt": "Find where the parser handles escaped quotes in string literals.",
                }),
            ),
        ],
        StopReason::ToolUse,
        2_000,
        (3_100, 150, 0.0088),
    ));
    r.result(
        "agent-1",
        vec![text(
            "Escaped quotes are handled in read_string() in src/parser/lex.rs. The token type is in src/parser/token.rs.",
        )],
        false,
        Some(json!({ "poolTaskId": run })),
        None,
        9_000,
    );
    r.assistant(&assistant(
        vec![text(
            "The escaped-quote handling lives in `read_string()` in `src/parser/lex.rs`. The subagent's search with the first pattern failed, but the read of the lexer and the token type answered the question.",
        )],
        StopReason::Stop,
        9_500,
        (3_600, 210, 0.0102),
    ));
    r.finish();
    r
}

const STEPLESS_PROMPT: &str = "What does CONTRIBUTING say about commit messages?";

/// A mission with no calls: an answer only.
fn stepless(view: ToolOutputView, _run: &str) -> Replay {
    let mut r = Replay::new(view);
    r.user(STEPLESS_PROMPT, 1_000);
    r.assistant(&assistant(
        vec![text(
            "Commit messages use a short imperative subject line of about 50 characters, a blank line, then a body that explains why the change was made.",
        )],
        StopReason::Stop,
        2_500,
        (1_200, 96, 0.0031),
    ));
    r.finish();
    r
}

// ---------------------------------------------------------------------------
// Goldens: <scenario>_<layer>_<width>, one test each.
// ---------------------------------------------------------------------------

macro_rules! layer_golden {
    ($name:ident, $scenario:ident, $view:ident, $width:literal) => {
        #[test]
        fn $name() {
            let _guard = guard();
            let replay = $scenario(ToolOutputView::$view, stringify!($name));
            let text = replay.render($width);
            assert_golden!(stringify!($name), text);
        }
    };
}

layer_golden!(layers_landed_radar_80, landed, Radar, 80);
layer_golden!(layers_landed_radar_120, landed, Radar, 120);
layer_golden!(layers_landed_peek_80, landed, Peek, 80);
layer_golden!(layers_landed_peek_120, landed, Peek, 120);
layer_golden!(layers_landed_full_80, landed, Full, 80);
layer_golden!(layers_landed_full_120, landed, Full, 120);

layer_golden!(layers_failed_radar_80, failed, Radar, 80);
layer_golden!(layers_failed_radar_120, failed, Radar, 120);
layer_golden!(layers_failed_peek_80, failed, Peek, 80);
layer_golden!(layers_failed_peek_120, failed, Peek, 120);
layer_golden!(layers_failed_full_80, failed, Full, 80);
layer_golden!(layers_failed_full_120, failed, Full, 120);

layer_golden!(layers_aborted_radar_80, aborted, Radar, 80);
layer_golden!(layers_aborted_radar_120, aborted, Radar, 120);
layer_golden!(layers_aborted_peek_80, aborted, Peek, 80);
layer_golden!(layers_aborted_peek_120, aborted, Peek, 120);
layer_golden!(layers_aborted_full_80, aborted, Full, 80);
layer_golden!(layers_aborted_full_120, aborted, Full, 120);

layer_golden!(layers_agent_radar_80, agent, Radar, 80);
layer_golden!(layers_agent_radar_120, agent, Radar, 120);
layer_golden!(layers_agent_peek_80, agent, Peek, 80);
layer_golden!(layers_agent_peek_120, agent, Peek, 120);
layer_golden!(layers_agent_full_80, agent, Full, 80);
layer_golden!(layers_agent_full_120, agent, Full, 120);

layer_golden!(layers_stepless_radar_80, stepless, Radar, 80);
layer_golden!(layers_stepless_radar_120, stepless, Radar, 120);
layer_golden!(layers_stepless_peek_80, stepless, Peek, 80);
layer_golden!(layers_stepless_peek_120, stepless, Peek, 120);
layer_golden!(layers_stepless_full_80, stepless, Full, 80);
layer_golden!(layers_stepless_full_120, stepless, Full, 120);

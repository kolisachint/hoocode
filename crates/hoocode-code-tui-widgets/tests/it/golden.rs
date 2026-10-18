//! Component goldens (T0.2): the task panel, the Shell and Agent tool blocks,
//! the session chip, and the user and assistant messages, at 80 columns (and
//! 120 for the key cases). Output is pinned to the dark theme and carries no
//! times: tool blocks get no `startedAt` (no `Took`/`Elapsed` line) and the
//! task panel shows only plan items, never delegated runs.

use std::cell::RefCell;
use std::rc::Rc;

use hoocode_ai_types::{AssistantMessage, Content, ThinkingContent};
use hoocode_code_task_store::{task_store, CreateTaskOptions, TaskPatch, TaskSource, TaskStatus};
use hoocode_code_tui_keybindings::AppKeybindingsManager;
use hoocode_code_tui_widgets::session_chip::render_session_chip;
use hoocode_code_tui_widgets::task_panel::{TaskPanelComponent, TaskPanelDensity, TaskPanelView};
use hoocode_code_tui_widgets::tool_chain::ToolChainComponent;
use hoocode_code_tui_widgets::tool_execution::{ToolExecutionComponent, ToolExecutionOptions};
use hoocode_code_tui_widgets::tool_output_view::ToolOutputView;
use hoocode_code_tui_widgets::tool_signal::ToolResult;
use hoocode_code_tui_widgets::tools::{builtin_tool_definition, registered_tool_definition};
use hoocode_code_tui_widgets::trajectory::MissionHeader;
use hoocode_code_tui_widgets::{AssistantMessageComponent, ThinkingDisplay, UserMessageComponent};
use hoocode_tui_components::Text;
use hoocode_tui_render::golden::render_golden;
use hoocode_tui_render::{assert_golden, Component};
use serde_json::{json, Value};

use crate::support::lock;

/// A fixed working directory: the tool renderers print paths relative to it.
const CWD: &str = "/work/project";

fn tool(name: &str, args: Value, view: ToolOutputView) -> ToolExecutionComponent {
    // The registry as the app builds it: built-in renderers first, then the
    // registered ones (Agent, AgentOutput).
    let definition =
        builtin_tool_definition(name).unwrap_or_else(|| registered_tool_definition(name));
    ToolExecutionComponent::new(
        name,
        "call-1",
        args,
        ToolExecutionOptions {
            view,
            ..Default::default()
        },
        Some(definition),
        CWD,
    )
}

fn result(text: &str, is_error: bool, details: Value) -> ToolResult {
    ToolResult {
        content: vec![Content::text(text)],
        details,
        is_error,
    }
}

fn shell_args() -> Value {
    json!({ "command": "cargo test -p hoocode-code-tui-widgets" })
}

/// Lines of test output, `n` of them, numbered so truncation is visible.
fn output_lines(n: usize) -> String {
    (1..=n)
        .map(|i| format!("test case_{i:02} ... ok"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn add_task(title: &str, status: TaskStatus) {
    let id = task_store().create(title, CreateTaskOptions::default()).id;
    if status != TaskStatus::Pending {
        task_store().update(
            id,
            TaskPatch {
                status: Some(status),
                ..Default::default()
            },
        );
    }
}

fn mixed_tasks() {
    add_task("Read the migration plan", TaskStatus::Done);
    add_task("Port the task panel renderer", TaskStatus::InProgress);
    add_task("Add golden snapshots", TaskStatus::Pending);
    add_task("Run the parity check", TaskStatus::Failed);
    add_task("Drop the old lens tabs", TaskStatus::Cancelled);
}

const LONG_TITLE: &str = "Refactor the authentication middleware so that session tokens are rotated on every privileged request and the audit log records each rotation with the caller's address";

fn assistant(content: Vec<Content>) -> AssistantMessage {
    AssistantMessage {
        content,
        api: "openai-responses".into(),
        provider: "openai".into(),
        model: "gpt-4o-mini".into(),
        ..Default::default()
    }
}

fn thinking(text: &str) -> Content {
    Content::Thinking(ThinkingContent {
        thinking: text.into(),
        signature: Some(String::new()),
        ..Default::default()
    })
}

const MARKDOWN: &str = "## Plan\n\nThe fix has two parts:\n\n1. Rotate the token in `middleware.rs`.\n2. Log the rotation.\n\n```rust\nfn rotate(session: &mut Session) {\n    session.token = new_token();\n}\n```\n\nRun `cargo test` afterwards.";

// ---- task panel ----

#[test]
fn task_panel_empty_80() {
    let _g = lock();
    task_store().clear();
    let mut panel = TaskPanelComponent::new();
    assert_golden!("task_panel_empty_80", render_golden(&mut panel, 80));
}

#[test]
fn task_panel_mixed_80() {
    let _g = lock();
    task_store().clear();
    mixed_tasks();
    let mut panel = TaskPanelComponent::new();
    assert_golden!("task_panel_mixed_80", render_golden(&mut panel, 80));
}

#[test]
fn task_panel_mixed_120() {
    let _g = lock();
    task_store().clear();
    mixed_tasks();
    let mut panel = TaskPanelComponent::new();
    assert_golden!("task_panel_mixed_120", render_golden(&mut panel, 120));
}

#[test]
fn task_panel_long_text_80() {
    let _g = lock();
    task_store().clear();
    add_task(LONG_TITLE, TaskStatus::InProgress);
    add_task("Short follow-up", TaskStatus::Pending);
    let mut panel = TaskPanelComponent::new();
    assert_golden!("task_panel_long_text_80", render_golden(&mut panel, 80));
}

#[test]
fn task_panel_long_text_120() {
    let _g = lock();
    task_store().clear();
    add_task(LONG_TITLE, TaskStatus::InProgress);
    add_task("Short follow-up", TaskStatus::Pending);
    let mut panel = TaskPanelComponent::new();
    assert_golden!("task_panel_long_text_120", render_golden(&mut panel, 120));
}

#[test]
fn task_panel_summary_80() {
    let _g = lock();
    task_store().clear();
    mixed_tasks();
    let mut panel = TaskPanelComponent::new();
    panel.set_density(TaskPanelDensity::Summary);
    assert_golden!("task_panel_summary_80", render_golden(&mut panel, 80));
}

// ---- task panel tabs (T4.1) ----

/// One row of each kind, with no timed rows: a plan item in progress, a Done
/// subagent run and a Done MCP call (a running one would show its clock).
fn three_tabs() {
    add_task("Read the migration plan", TaskStatus::Done);
    add_task("Port the task panel renderer", TaskStatus::InProgress);
    add_task("Add golden snapshots", TaskStatus::Pending);
    let run = task_store()
        .create(
            "Trace the auth flow",
            CreateTaskOptions {
                source: Some(TaskSource::Subagent),
                subagent_mode: Some("explore".into()),
                agent: Some("run-1".into()),
                ..Default::default()
            },
        )
        .id;
    set_done(run);
    let call = task_store()
        .create(
            "Fetch the spec",
            CreateTaskOptions {
                source: Some(TaskSource::Mcp),
                subagent_mode: Some("web".into()),
                ..Default::default()
            },
        )
        .id;
    set_done(call);
}

fn set_done(id: u64) {
    task_store().update(
        id,
        TaskPatch {
            status: Some(TaskStatus::Done),
            ..Default::default()
        },
    );
}

#[test]
fn task_panel_tabs_unfocused_80() {
    let _g = lock();
    task_store().clear();
    three_tabs();
    let mut panel = TaskPanelComponent::new();
    assert_golden!(
        "task_panel_tabs_unfocused_80",
        render_golden(&mut panel, 80)
    );
}

#[test]
fn flight_plan_80() {
    let _g = lock();
    AppKeybindingsManager::default().install();
    task_store().clear();
    three_tabs();
    let mut panel = TaskPanelComponent::new();
    panel.set_focused(true);
    assert_golden!("flight_plan_80", render_golden(&mut panel, 80));
}

#[test]
fn flight_plan_100() {
    let _g = lock();
    AppKeybindingsManager::default().install();
    task_store().clear();
    three_tabs();
    let mut panel = TaskPanelComponent::new();
    panel.set_focused(true);
    assert_golden!("flight_plan_100", render_golden(&mut panel, 100));
}

#[test]
fn flight_plan_agents_80() {
    let _g = lock();
    AppKeybindingsManager::default().install();
    task_store().clear();
    three_tabs();
    let mut panel = TaskPanelComponent::new();
    panel.set_view(TaskPanelView::Subagents);
    panel.set_focused(true);
    assert_golden!("flight_plan_agents_80", render_golden(&mut panel, 80));
}

// ---- task panel: Shell tab (T5.3) ----

/// Three background jobs with no timed state: one Done, one Failed with its
/// exit status, one killed.
fn shell_jobs() {
    let shell = |key: &str| CreateTaskOptions {
        source: Some(TaskSource::Shell),
        key: Some(key.into()),
        ..Default::default()
    };
    let done = task_store()
        .create("cargo build --release", shell("shell:job-1"))
        .id;
    set_done(done);
    let failed = task_store()
        .create("cargo test -p nope", shell("shell:job-2"))
        .id;
    task_store().update(
        failed,
        TaskPatch {
            status: Some(TaskStatus::Failed),
            note: Some(Some("exit 101".into())),
            ..Default::default()
        },
    );
    let killed = task_store().create("sleep 600", shell("shell:job-3")).id;
    task_store().update(
        killed,
        TaskPatch {
            status: Some(TaskStatus::Cancelled),
            ..Default::default()
        },
    );
}

#[test]
fn flight_plan_shell_80() {
    let _g = lock();
    AppKeybindingsManager::default().install();
    task_store().clear();
    shell_jobs();
    let mut panel = TaskPanelComponent::new();
    panel.set_view(TaskPanelView::Shell);
    panel.set_focused(true);
    assert_golden!("flight_plan_shell_80", render_golden(&mut panel, 80));
}

#[test]
fn task_panel_tabs_with_shell_unfocused_80() {
    let _g = lock();
    AppKeybindingsManager::default().install();
    task_store().clear();
    three_tabs();
    shell_jobs();
    let mut panel = TaskPanelComponent::new();
    assert_golden!(
        "task_panel_tabs_with_shell_unfocused_80",
        render_golden(&mut panel, 80)
    );
}

#[test]
fn task_panel_empty_tabs_hidden_80() {
    let _g = lock();
    task_store().clear();
    add_task("Read the migration plan", TaskStatus::Done);
    let call = task_store()
        .create(
            "Fetch the spec",
            CreateTaskOptions {
                source: Some(TaskSource::Mcp),
                subagent_mode: Some("web".into()),
                ..Default::default()
            },
        )
        .id;
    set_done(call);
    // No subagent run: the Agents tab is not in the strip.
    let mut panel = TaskPanelComponent::new();
    assert_golden!(
        "task_panel_empty_tabs_hidden_80",
        render_golden(&mut panel, 80)
    );
}

// ---- Shell (bash) tool block ----

#[test]
fn shell_running_80() {
    let _g = lock();
    let mut block = tool("Shell", shell_args(), ToolOutputView::Peek);
    block.update_result(result(&output_lines(2), false, json!({})), true);
    assert_golden!("shell_running_80", render_golden(&mut block, 80));
}

#[test]
fn shell_success_80() {
    let _g = lock();
    let mut block = tool("Shell", shell_args(), ToolOutputView::Peek);
    block.update_result(
        result("test result: ok. 12 passed; 0 failed", false, json!({})),
        false,
    );
    assert_golden!("shell_success_80", render_golden(&mut block, 80));
}

#[test]
fn shell_success_120() {
    let _g = lock();
    let mut block = tool("Shell", shell_args(), ToolOutputView::Peek);
    block.update_result(
        result("test result: ok. 12 passed; 0 failed", false, json!({})),
        false,
    );
    assert_golden!("shell_success_120", render_golden(&mut block, 120));
}

#[test]
fn shell_error_80() {
    let _g = lock();
    let mut block = tool("Shell", shell_args(), ToolOutputView::Peek);
    block.update_result(
        result(
            "error[E0432]: unresolved import `crate::missing`\nexit code 101",
            true,
            json!({}),
        ),
        false,
    );
    assert_golden!("shell_error_80", render_golden(&mut block, 80));
}

#[test]
fn shell_output_collapsed_80() {
    let _g = lock();
    let mut block = tool("Shell", shell_args(), ToolOutputView::Radar);
    block.update_result(result(&output_lines(12), false, json!({})), false);
    assert_golden!("shell_output_collapsed_80", render_golden(&mut block, 80));
}

#[test]
fn shell_output_peek_80() {
    let _g = lock();
    let mut block = tool("Shell", shell_args(), ToolOutputView::Peek);
    block.update_result(result(&output_lines(12), false, json!({})), false);
    assert_golden!("shell_output_peek_80", render_golden(&mut block, 80));
}

// ---- Agent tool block ----

fn agent_args() -> Value {
    json!({
        "subagent_type": "explore",
        "description": "Find the footer code",
        "prompt": "Locate the footer renderer and list its inputs."
    })
}

#[test]
fn agent_call_80() {
    let _g = lock();
    let mut block = tool("Agent", agent_args(), ToolOutputView::Peek);
    assert_golden!("agent_call_80", render_golden(&mut block, 80));
}

#[test]
fn agent_background_call_80() {
    let _g = lock();
    let mut args = agent_args();
    args["background"] = json!(true);
    let mut block = tool("Agent", args, ToolOutputView::Peek);
    assert_golden!("agent_background_call_80", render_golden(&mut block, 80));
}

#[test]
fn agent_result_80() {
    let _g = lock();
    let mut block = tool("Agent", agent_args(), ToolOutputView::Peek);
    block.update_result(
        result("Found the footer in footer.rs.", false, json!({})),
        false,
    );
    assert_golden!("agent_result_80", render_golden(&mut block, 80));
}

#[test]
fn agent_result_120() {
    let _g = lock();
    let mut block = tool("Agent", agent_args(), ToolOutputView::Peek);
    block.update_result(
        result("Found the footer in footer.rs.", false, json!({})),
        false,
    );
    assert_golden!("agent_result_120", render_golden(&mut block, 120));
}

// ---- AgentOutput result card (subagent status) ----

fn agent_output(text: &str, status: &str) -> ToolResult {
    result(
        text,
        false,
        json!({ "task_id": "explore#1", "status": status }),
    )
}

#[test]
fn agent_output_done_80() {
    let _g = lock();
    let mut block = tool(
        "AgentOutput",
        json!({ "task_id": "explore#1" }),
        ToolOutputView::Peek,
    );
    block.update_result(
        agent_output("Footer is in footer.rs, 340 lines.", "done"),
        false,
    );
    assert_golden!("agent_output_done_80", render_golden(&mut block, 80));
}

#[test]
fn agent_output_running_80() {
    let _g = lock();
    let mut block = tool(
        "AgentOutput",
        json!({ "task_id": "explore#1" }),
        ToolOutputView::Peek,
    );
    block.update_result(agent_output("Still reading files.", "running"), true);
    assert_golden!("agent_output_running_80", render_golden(&mut block, 80));
}

#[test]
fn agent_output_failed_80() {
    let _g = lock();
    let mut block = tool(
        "AgentOutput",
        json!({ "task_id": "explore#1" }),
        ToolOutputView::Peek,
    );
    block.update_result(
        agent_output("Stopped: tool limit reached.", "failed"),
        false,
    );
    assert_golden!("agent_output_failed_80", render_golden(&mut block, 80));
}

// ---- session chip ----

#[test]
fn session_chip_80() {
    let _g = lock();
    let chip = render_session_chip("refactor-auth", 1).expect("a chip for a name");
    let mut line = Text::new(chip.styled, 0, 0);
    assert_golden!("session_chip_80", render_golden(&mut line, 80));
}

#[test]
fn session_chip_truncated_80() {
    let _g = lock();
    let chip = render_session_chip("an-extremely-long-session-name-nobody-can-scan", 3)
        .expect("a chip for a name");
    let mut line = Text::new(chip.styled, 0, 0);
    assert_golden!("session_chip_truncated_80", render_golden(&mut line, 80));
}

#[test]
fn session_chip_120() {
    let _g = lock();
    let chip = render_session_chip("amber-harbor", 4).expect("a chip for a name");
    let mut line = Text::new(chip.styled, 0, 0);
    assert_golden!("session_chip_120", render_golden(&mut line, 120));
}

// ---- user message ----

#[test]
fn user_message_plain_80() {
    let _g = lock();
    let mut msg =
        UserMessageComponent::new("Why does the footer hide the model name on narrow terminals?");
    assert_golden!("user_message_plain_80", render_golden(&mut msg, 80));
}

#[test]
fn user_message_markdown_80() {
    let _g = lock();
    let mut msg = UserMessageComponent::new(
        "Please fix this:\n\n```rust\nfn main() {\n    println!(\"hi\");\n}\n```\n\nThanks, see `main.rs`.",
    );
    assert_golden!("user_message_markdown_80", render_golden(&mut msg, 80));
}

#[test]
fn user_message_plain_120() {
    let _g = lock();
    let mut msg =
        UserMessageComponent::new("Why does the footer hide the model name on narrow terminals?");
    assert_golden!("user_message_plain_120", render_golden(&mut msg, 120));
}

// ---- assistant message ----

#[test]
fn assistant_message_plain_80() {
    let _g = lock();
    let mut msg = AssistantMessageComponent::new(
        Some(&assistant(vec![Content::text(
            "The model name is dropped when the line is too long.",
        )])),
        ThinkingDisplay::Full,
    );
    assert_golden!("assistant_message_plain_80", render_golden(&mut msg, 80));
}

#[test]
fn assistant_message_markdown_80() {
    let _g = lock();
    let mut msg = AssistantMessageComponent::new(
        Some(&assistant(vec![Content::text(MARKDOWN)])),
        ThinkingDisplay::Full,
    );
    assert_golden!("assistant_message_markdown_80", render_golden(&mut msg, 80));
}

#[test]
fn assistant_message_markdown_120() {
    let _g = lock();
    let mut msg = AssistantMessageComponent::new(
        Some(&assistant(vec![Content::text(MARKDOWN)])),
        ThinkingDisplay::Full,
    );
    assert_golden!(
        "assistant_message_markdown_120",
        render_golden(&mut msg, 120)
    );
}

#[test]
fn assistant_message_thinking_full_80() {
    let _g = lock();
    let mut msg = AssistantMessageComponent::new(
        Some(&assistant(vec![
            thinking("The footer drops the name first, then the branch."),
            Content::text("Narrow footers drop the session name first."),
        ])),
        ThinkingDisplay::Full,
    );
    assert_golden!(
        "assistant_message_thinking_full_80",
        render_golden(&mut msg, 80)
    );
}

#[test]
fn assistant_message_thinking_label_80() {
    let _g = lock();
    let mut msg = AssistantMessageComponent::new(
        Some(&assistant(vec![
            thinking("The footer drops the name first, then the branch."),
            Content::text("Narrow footers drop the session name first."),
        ])),
        ThinkingDisplay::Label,
    );
    assert_golden!(
        "assistant_message_thinking_label_80",
        render_golden(&mut msg, 80)
    );
}

// ---- Mission Control trajectory (M4) ----

#[test]
fn trajectory_mission_header_80() {
    let _g = lock();
    let mut header = MissionHeader::new(1, "how does wrapping work?", None);
    assert_golden!(
        "trajectory_mission_header_80",
        render_golden(&mut header, 80)
    );
}

#[test]
fn trajectory_mission_header_skill_80() {
    let _g = lock();
    let mut header = MissionHeader::new(6, "check wrap.rs", Some("review"));
    assert_golden!(
        "trajectory_mission_header_skill_80",
        render_golden(&mut header, 80)
    );
}

#[test]
fn trajectory_steps_hit_80() {
    // Numbered steps on the `┊` rail; the failed Shell shows the HIT chip
    // in place of its success stats.
    let _g = lock();
    let mut chain = ToolChainComponent::new(ToolOutputView::Peek);
    let mut read = tool(
        "Read",
        json!({ "path": "src/wrap.rs" }),
        ToolOutputView::Peek,
    );
    read.update_result(result("line one\nline two", false, json!({})), false);
    chain.add(Rc::new(RefCell::new(read)));
    let mut shell = tool("Shell", shell_args(), ToolOutputView::Peek);
    shell.update_result(
        result(
            "boom\nCommand exited with code 3",
            true,
            json!({ "durationMs": 100.0 }),
        ),
        false,
    );
    chain.add(Rc::new(RefCell::new(shell)));
    assert_golden!("trajectory_steps_hit_80", render_golden(&mut chain, 80));
}

// ---- Mission Control flight log (M5, radar) ----

use hoocode_code_tui_widgets::radar::{FlightMark, FlightStats};

fn flight_stats(
    mark: FlightMark,
    steps: usize,
    hits: usize,
    time: Option<&str>,
    cost: Option<&str>,
) -> Rc<RefCell<FlightStats>> {
    Rc::new(RefCell::new(FlightStats {
        mark,
        steps,
        hits,
        landed_time: time.map(str::to_string),
        started: None,
        cost: cost.map(str::to_string),
    }))
}

fn radar_header(seq: usize, prompt: &str, stats: Rc<RefCell<FlightStats>>) -> MissionHeader {
    let mut header = MissionHeader::new(seq, prompt, None);
    header.set_view(ToolOutputView::Radar);
    header.set_flight(stats);
    header
}

#[test]
fn radar_mission_row_80() {
    // One flight-log row: the mark, the prompt, steps, hits, time, cost.
    let _g = lock();
    let mut header = radar_header(
        1,
        "what does the wrap crate do?",
        flight_stats(FlightMark::Landed, 1, 0, Some("0.8s"), Some("$0.003")),
    );
    assert_golden!("radar_mission_row_80", render_golden(&mut header, 80));
}

#[test]
fn radar_mission_row_40() {
    // Narrow: the prompt is cut with `…` first, the stats stay whole.
    let _g = lock();
    let mut header = radar_header(
        2,
        "review the wrap helper for overflow bugs",
        flight_stats(FlightMark::Landed, 4, 1, Some("0.6s"), Some("$0.006")),
    );
    assert_golden!("radar_mission_row_40", render_golden(&mut header, 40));
}

#[test]
fn radar_mission_in_flight_80() {
    // A flying mission reads `◉`; with no landed time the row shows the
    // cost only. Elapsed time comes off the turn clock at render, so the
    // golden pins the row without it.
    let _g = lock();
    let mut header = radar_header(
        2,
        "run a slow check",
        flight_stats(FlightMark::InFlight, 1, 0, None, Some("$0.001")),
    );
    assert_golden!("radar_mission_in_flight_80", render_golden(&mut header, 80));
}

#[test]
fn radar_mission_stepless_80() {
    // No steps: `0 steps 0✕` is omitted, the row reads mark, prompt, cost.
    let _g = lock();
    let mut header = radar_header(
        3,
        "just talking",
        flight_stats(FlightMark::Landed, 0, 0, None, Some("$0.000")),
    );
    assert_golden!("radar_mission_stepless_80", render_golden(&mut header, 80));
}

#[test]
fn radar_chain_hits_80() {
    // Each hit gets its own line under the mission: mission-relative step,
    // tool, subject, first output line, `HIT`. Settled clean calls get none.
    let _g = lock();
    let mut chain = ToolChainComponent::new(ToolOutputView::Radar);
    chain.set_base_step(3);
    let mut read = tool(
        "Read",
        json!({ "path": "src/wrap.rs" }),
        ToolOutputView::Radar,
    );
    read.update_result(result("line one\nline two", false, json!({})), false);
    chain.add(Rc::new(RefCell::new(read)));
    let mut shell = tool("Shell", shell_args(), ToolOutputView::Radar);
    shell.update_result(
        result(
            "boom\nCommand exited with code 3",
            true,
            json!({ "durationMs": 100.0 }),
        ),
        false,
    );
    chain.add(Rc::new(RefCell::new(shell)));
    assert_golden!("radar_chain_hits_80", render_golden(&mut chain, 80));
}

#[test]
fn radar_chain_hits_40() {
    // Narrow: the subject is cut first, `HIT` stays whole.
    let _g = lock();
    let mut chain = ToolChainComponent::new(ToolOutputView::Radar);
    chain.set_base_step(3);
    let mut shell = tool("Shell", shell_args(), ToolOutputView::Radar);
    shell.update_result(
        result(
            "boom\nCommand exited with code 3",
            true,
            json!({ "durationMs": 100.0 }),
        ),
        false,
    );
    chain.add(Rc::new(RefCell::new(shell)));
    assert_golden!("radar_chain_hits_40", render_golden(&mut chain, 40));
}

#[test]
fn radar_chain_live_80() {
    // A flying mission's live step: elapsed only, no progress bar.
    let _g = lock();
    let mut chain = ToolChainComponent::new(ToolOutputView::Radar);
    chain.add(Rc::new(RefCell::new(tool(
        "Shell",
        json!({ "command": "sleep 6; echo done" }),
        ToolOutputView::Radar,
    ))));
    assert_golden!("radar_chain_live_80", render_golden(&mut chain, 80));
}

#[test]
fn radar_think_80() {
    // Thinking under its mission: the last prose line, then
    // `✻ thought Ns` with no gist.
    let _g = lock();
    let mut msg = AssistantMessageComponent::new(None, ThinkingDisplay::Omit);
    msg.update_content(
        &assistant(vec![
            thinking("weighing the options"),
            Content::text("here is the answer"),
        ]),
        true,
    );
    assert_golden!("radar_think_80", render_golden(&mut msg, 80));
}

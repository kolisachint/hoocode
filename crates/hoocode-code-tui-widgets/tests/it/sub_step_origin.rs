//! A subagent's own call under its Agent row carries its origin: a plugin or MCP call takes the
//! external label and colour, a built-in call draws as before (`docs/design/plugins.md`, Naming
//! and identity; `docs/design/tui.md`, External tools).

use std::cell::RefCell;
use std::rc::Rc;

use crate::support::{lock, strip};
use hoocode_code_subagents::live::apply;
use hoocode_code_tui_theme::theme;
use hoocode_code_tui_widgets::tool_chain::ToolChainComponent;
use hoocode_code_tui_widgets::tool_execution::{ToolExecutionComponent, ToolExecutionOptions};
use hoocode_code_tui_widgets::tool_signal::ToolResult;
use hoocode_code_tui_widgets::tools::registered_tool_definition;
use hoocode_tui_render::Component;
use serde_json::{json, Value};

const CWD: &str = "/work/project";
const RUN: &str = "widget-sub-origin-run";
const CALL: &str = "call-sub-origin";

fn event(value: Value) {
    apply(RUN, &value);
}

/// A settled run with a plugin call (it fails, so it stays visible) and a built-in call.
fn settled_run() {
    event(json!({"type": "task_started"}));
    event(json!({
        "type": "tool_execution_start", "toolCallId": "c1", "toolName": "WebFetch",
        "args": {"url": "https://example.com/docs"},
        "origin": {"kind": "plugin", "plugin": "webtools", "tool": "WebFetch"},
        "render": "web-fetch",
    }));
    event(json!({
        "type": "tool_execution_end", "toolCallId": "c1", "isError": true,
        "result": {"content": [{"type": "text", "text": "refused"}]},
    }));
    event(json!({
        "type": "tool_execution_start", "toolCallId": "c2", "toolName": "Read",
        "args": {"file_path": "a.rs"}, "origin": null,
    }));
    event(json!({"type": "task_done"}));
}

/// An open Agent block whose run has settled.
fn agent_block() -> ToolExecutionComponent {
    let args = json!({
        "subagent_type": "explore",
        "description": "Fetch the docs",
        "prompt": "Fetch the docs page.",
    });
    let mut block = ToolExecutionComponent::new(
        "Agent",
        CALL,
        args,
        ToolExecutionOptions::default(),
        Some(registered_tool_definition("Agent")),
        CWD,
    );
    block.set_expanded(true);
    block.update_result(
        ToolResult {
            content: Vec::new(),
            details: json!({"poolTaskId": RUN, "subagent_type": "explore", "status": "done"}),
            is_error: false,
        },
        false,
    );
    block
}

/// The chain's rows for the block, raw (with colour codes).
fn chain_rows(block: ToolExecutionComponent) -> Vec<String> {
    let mut chain = ToolChainComponent::new();
    chain.add(Rc::new(RefCell::new(block)));
    chain.render(100)
}

/// The colour parameters of the escape before `name` in a raw row.
fn name_colour(row: &str, name: &str) -> String {
    let before = &row[..row
        .find(name)
        .unwrap_or_else(|| panic!("{name} in {row:?}"))];
    let open = before.rfind("\u{1b}[").expect("an escape before the name");
    before[open + 2..]
        .split('m')
        .next()
        .unwrap_or_default()
        .split(';')
        .filter(|part| *part != "0" && *part != "1")
        .collect::<Vec<_>>()
        .join(";")
}

/// The colour parameters of a theme token, as `name_colour` reads them.
fn token_colour(token: &str) -> String {
    let t = theme();
    let ansi = t.get_fg_ansi(token);
    ansi.trim_start_matches("\u{1b}[")
        .trim_end_matches('m')
        .to_owned()
}

/// The raw sub-step row that names `tool`, found in the chain's rows.
fn sub_row(rows: &[String], tool: &str) -> String {
    rows.iter()
        .find(|row| strip(row).contains(&format!(" {tool} ")) && row.starts_with("   "))
        .or_else(|| {
            rows.iter()
                .find(|row| strip(row).contains(&format!(" {tool} ")))
        })
        .unwrap_or_else(|| panic!("no sub-step row for {tool}: {rows:?}"))
        .clone()
}

#[test]
fn a_plugin_sub_step_takes_its_plugin_label_and_the_external_colour() {
    let _g = lock();
    settled_run();
    let rows = chain_rows(agent_block());
    let web = sub_row(&rows, "WebFetch");
    let plain = strip(&web);
    assert!(plain.contains("webtools"), "{plain}");
    assert_eq!(
        name_colour(&web, "WebFetch"),
        token_colour("toolExternal"),
        "{web:?}"
    );
}

#[test]
fn a_built_in_sub_step_has_no_label_and_keeps_the_title_colour() {
    let _g = lock();
    settled_run();
    let rows = chain_rows(agent_block());
    let read = sub_row(&rows, "Read");
    let plain = strip(&read);
    assert!(!plain.contains("webtools"), "{plain}");
    assert_ne!(
        name_colour(&read, "Read"),
        token_colour("toolExternal"),
        "{read:?}"
    );
}

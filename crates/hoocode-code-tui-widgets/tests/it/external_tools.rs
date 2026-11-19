//! External tools (plugin and MCP) in the transcript: the name takes `toolExternal`, the
//! origin label follows it dimmed, and the renderer comes from the plugin's render hint
//! (`docs/design/tui.md`, External tools; `docs/design/plugins.md`, Naming and identity).

use std::cell::RefCell;
use std::rc::Rc;

use crate::support::{lock, strip};
use hoocode_agent_types::ToolOrigin;
use hoocode_ai_types::Content;
use hoocode_code_tui_theme::theme;
use hoocode_code_tui_widgets::tool_chain::ToolChainComponent;
use hoocode_code_tui_widgets::tool_execution::{ToolExecutionComponent, ToolExecutionOptions};
use hoocode_code_tui_widgets::tool_row::HeaderOrigin;
use hoocode_code_tui_widgets::tool_signal::ToolResult;
use hoocode_code_tui_widgets::tools::{
    builtin_tool_definition, hinted_definition, renderer_for, RENDER_WEB_FETCH, RENDER_WEB_SEARCH,
};
use hoocode_tui_render::golden::render_golden;
use hoocode_tui_render::{assert_golden, Component};
use serde_json::{json, Value};

const CWD: &str = "/work";

fn webtools(tool: &str) -> ToolOrigin {
    ToolOrigin::Plugin {
        plugin: "webtools".into(),
        tool: tool.into(),
    }
}

fn mcp_github(tool: &str) -> ToolOrigin {
    ToolOrigin::Mcp {
        server: "github".into(),
        tool: tool.into(),
        plugin: None,
    }
}

fn block(
    name: &str,
    origin: Option<ToolOrigin>,
    render: Option<&str>,
    args: Value,
) -> ToolExecutionComponent {
    ToolExecutionComponent::with_origin(
        name,
        "call-1",
        args,
        ToolExecutionOptions::default(),
        None,
        CWD,
        origin,
        render.map(str::to_owned),
    )
}

fn settle(block: &mut ToolExecutionComponent, details: Value, text: &str) {
    block.update_result(
        ToolResult {
            content: vec![Content::text(text)],
            details,
            is_error: false,
        },
        false,
    );
}

/// The chain's rows for one call, raw (with colour codes).
fn chain_rows(block: ToolExecutionComponent) -> Vec<String> {
    let mut chain = ToolChainComponent::new();
    chain.add(Rc::new(RefCell::new(block)));
    chain.render(100)
}

/// The header row of a chain: the row that names the tool.
fn header_row(rows: &[String], name: &str) -> String {
    rows.iter()
        .find(|row| strip(row).contains(&format!(" {name} ")))
        .unwrap_or_else(|| panic!("no header for {name}: {rows:?}"))
        .clone()
}

/// The colour parameters (`38;2;r;g;b`) of the escape that opens `name` in `row`.
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

#[test]
fn a_plugin_tool_name_takes_the_external_colour_and_its_plugin_label() {
    let _g = lock();
    let mut b = block(
        "WebFetch",
        Some(webtools("WebFetch")),
        None,
        json!({"url": "https://example.com/docs"}),
    );
    settle(&mut b, json!({}), "Install it.");
    let rows = chain_rows(b);
    let row = header_row(&rows, "WebFetch");
    assert_eq!(
        name_colour(&row, "WebFetch"),
        token_colour("toolExternal"),
        "{row:?}"
    );
    assert!(
        strip(&row).starts_with(" ● WebFetch webtools https://example.com/docs"),
        "{}",
        strip(&row)
    );
}

#[test]
fn an_mcp_tool_is_labelled_with_its_server() {
    let _g = lock();
    let b = block(
        "mcp__github__create_issue",
        Some(mcp_github("create_issue")),
        None,
        json!({"title": "bug"}),
    );
    let rows = chain_rows(b);
    let plain = strip(&header_row(&rows, "mcp__github__create_issue"));
    assert!(
        plain.contains("mcp__github__create_issue mcp:github"),
        "{plain}"
    );
}

#[test]
fn a_built_in_row_has_no_label_and_keeps_the_title_colour() {
    let _g = lock();
    let mut b = block(
        "Read",
        Some(ToolOrigin::BuiltIn),
        None,
        json!({"path": "src/main.rs"}),
    );
    settle(&mut b, json!({}), "fn main() {}");
    let rows = chain_rows(b);
    let row = header_row(&rows, "Read");
    assert_eq!(
        name_colour(&row, "Read"),
        token_colour("toolTitle"),
        "{row:?}"
    );
    assert!(!strip(&row).contains("mcp:"), "{row:?}");
    assert_eq!(
        block_origin(Some(ToolOrigin::BuiltIn)),
        HeaderOrigin::default()
    );
}

#[test]
fn an_unknown_origin_draws_as_a_built_in() {
    let _g = lock();
    assert_eq!(block_origin(None), HeaderOrigin::default());
}

fn block_origin(origin: Option<ToolOrigin>) -> HeaderOrigin {
    block("Read", origin, None, json!({})).header_origin()
}

#[test]
fn the_label_drops_before_the_target_shortens() {
    let _g = lock();
    let mut b = block(
        "WebFetch",
        Some(webtools("WebFetch")),
        None,
        json!({"url": "https://example.com/a/long/path/index.html"}),
    );
    settle(&mut b, json!({}), "text");
    let wide = strip(&header_row(&chain_rows(b), "WebFetch"));
    assert!(wide.contains("webtools"), "{wide}");

    // A row too narrow for the label loses it first; the name is never cut.
    let mut narrow = block(
        "WebFetch",
        Some(webtools("WebFetch")),
        None,
        json!({"url": "https://example.com/a/long/path/index.html"}),
    );
    settle(&mut narrow, json!({}), "text");
    let mut chain = ToolChainComponent::new();
    chain.add(Rc::new(RefCell::new(narrow)));
    let rows = chain.render(48);
    let row = strip(&header_row(&rows, "WebFetch"));
    assert!(!row.contains("webtools"), "{row}");
    assert!(row.contains("WebFetch"), "{row}");
}

#[test]
fn the_renderer_comes_from_the_hint_not_the_name() {
    // A plugin tool named `Read` with the web-search hint draws as web search.
    assert!(renderer_for("Read", Some(&webtools("Read")), Some(RENDER_WEB_SEARCH)).is_some());
    // A plugin tool named `WebFetch` with no hint, or an unknown id, draws generically.
    assert!(renderer_for("WebFetch", Some(&webtools("WebFetch")), None).is_none());
    assert!(renderer_for("WebFetch", Some(&webtools("WebFetch")), Some("nope")).is_none());
    // MCP tools with no hint draw generically too.
    assert!(renderer_for("mcp__github__x", Some(&mcp_github("x")), None).is_none());
    // A built-in draws by its built-in name; the web names are not built-ins any more.
    assert!(renderer_for("Read", Some(&ToolOrigin::BuiltIn), None).is_some());
    assert!(renderer_for("WebFetch", Some(&ToolOrigin::BuiltIn), None).is_none());
    assert!(builtin_tool_definition("WebFetch").is_none());
    // A call whose origin is not known draws by its built-in name.
    assert!(renderer_for("Read", None, None).is_some());
    assert!(hinted_definition(RENDER_WEB_FETCH).is_some());
    assert!(hinted_definition(RENDER_WEB_SEARCH).is_some());
    assert!(hinted_definition("generic").is_none());
}

#[test]
fn a_hinted_web_fetch_row_shows_its_tokens_and_an_unhinted_one_does_not() {
    let _g = lock();
    let args = json!({"url": "https://example.com/docs"});
    let details = json!({"tokenEstimate": 8100});

    let mut hinted = block(
        "WebFetch",
        Some(webtools("WebFetch")),
        Some(RENDER_WEB_FETCH),
        args.clone(),
    );
    settle(&mut hinted, details.clone(), "Install it.");
    let plain = strip(&header_row(&chain_rows(hinted), "WebFetch"));
    assert!(plain.contains("~8.1k tokens"), "{plain}");

    let mut unhinted = block("WebFetch", Some(webtools("WebFetch")), None, args);
    settle(&mut unhinted, details, "Install it.");
    let plain = strip(&header_row(&chain_rows(unhinted), "WebFetch"));
    assert!(!plain.contains("tokens"), "{plain}");
}

#[test]
fn the_renderer_follows_a_hint_that_arrives_after_the_call_started() {
    let _g = lock();
    // The call streams in with no origin, then its start event names the plugin tool.
    let mut b = block("WebSearch", None, None, json!({"query": "tokio select"}));
    b.set_origin(
        Some(webtools("WebSearch")),
        Some(RENDER_WEB_SEARCH.to_owned()),
    );
    settle(
        &mut b,
        json!({"tokenEstimate": 512}),
        "It waits on several futures.",
    );
    let plain = strip(&header_row(&chain_rows(b), "WebSearch"));
    assert!(plain.contains("~512 tokens"), "{plain}");
    assert!(plain.contains("webtools"), "{plain}");
}

#[test]
fn set_origin_with_the_same_values_changes_nothing() {
    let _g = lock();
    let mut b = block(
        "Read",
        Some(ToolOrigin::BuiltIn),
        None,
        json!({"path": "a"}),
    );
    let before = b.render(60);
    b.set_origin(Some(ToolOrigin::BuiltIn), None);
    assert_eq!(b.render(60), before);
}

#[test]
fn the_external_colour_is_the_only_name_colour_that_differs() {
    let _g = lock();
    let external = theme().get_fg_ansi("toolExternal").to_owned();
    let title = theme().get_fg_ansi("toolTitle").to_owned();
    assert_ne!(external, title);
    assert_ne!(external, theme().get_fg_ansi("error"));
    assert_ne!(external, theme().get_fg_ansi("warning"));
    assert_ne!(external, theme().get_fg_ansi("success"));
}

/// The three origins side by side: a plugin row with its hint, an MCP call still running, and
/// a built-in row with no label.
#[test]
fn external_rows_100() {
    let _g = lock();
    let mut chain = ToolChainComponent::new();
    let mut web = block(
        "WebFetch",
        Some(webtools("WebFetch")),
        Some(RENDER_WEB_FETCH),
        json!({"url": "https://example.com/docs"}),
    );
    settle(&mut web, json!({"tokenEstimate": 8100}), "Install it.");
    chain.add(Rc::new(RefCell::new(web)));
    let mut read = block(
        "Read",
        Some(ToolOrigin::BuiltIn),
        None,
        json!({"path": "src/main.rs"}),
    );
    settle(&mut read, json!({}), "fn main() {}");
    chain.add(Rc::new(RefCell::new(read)));
    chain.add(Rc::new(RefCell::new(block(
        "mcp__github__create_issue",
        Some(mcp_github("create_issue")),
        None,
        json!({"title": "bug"}),
    ))));
    assert_golden!("external_rows_100", render_golden(&mut chain, 100));
}

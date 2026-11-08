//! UI-21 P4 (every block): one toggle for thinking, summaries, skill,
//! notice, `!` and startup details; the running `!` tail line (one dim line,
//! gone on settle); the web, todo and compaction facts with their cut order.

use std::rc::Rc;

use hoocode_agent_types::{BranchSummaryMessage, CompactionSummaryMessage, CustomMessage};
use hoocode_ai_types::{Content, ThinkingContent, UserContent};
use hoocode_code_tui_theme::get_markdown_theme;
use hoocode_code_tui_widgets::background_notice::{
    parse_background_notice, BackgroundNoticeComponent,
};
use hoocode_code_tui_widgets::bash_execution::BashExecutionComponent;
use hoocode_code_tui_widgets::custom_message::{
    BranchSummaryMessageComponent, CompactionSummaryMessageComponent, CustomMessageComponent,
};
use hoocode_code_tui_widgets::skill_block::SkillBlockComponent;
use hoocode_code_tui_widgets::tool_execution::{
    ToolRenderContext, ToolRenderResultOptions, ToolResultView,
};
use hoocode_code_tui_widgets::tool_signal::ResultText;
use hoocode_code_tui_widgets::tools::{todo, web};
use hoocode_code_tui_widgets::{AssistantMessageComponent, ThinkingDisplay};
use hoocode_tui_components::MarkdownTheme;
use hoocode_tui_render::Component;
use hoocode_tui_util::visible_width;

use crate::support::{assistant, lock, strip};

fn thinking(text: &str) -> Content {
    Content::Thinking(ThinkingContent {
        thinking: text.into(),
        signature: Some(String::new()),
        ..Default::default()
    })
}

fn markdown_theme() -> Rc<dyn Fn() -> MarkdownTheme> {
    Rc::new(get_markdown_theme)
}

/// Collapsed thinking is its first sentence on one dim line; the toggle
/// opens the whole trace.
#[test]
fn thinking_toggle_opens_the_whole_trace() {
    let _g = lock();
    let gist = "Weighing the options. The first one hides a trap.";
    let mut c = AssistantMessageComponent::new(
        Some(&assistant(vec![thinking(gist)])),
        ThinkingDisplay::Full,
    );
    assert!(!c.is_expanded());
    let rows: Vec<String> = c
        .render(60)
        .iter()
        .map(|l| strip(l).trim_end().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(rows, vec![" Weighing the options…"], "{rows:?}");

    c.set_expanded(true);
    assert!(c.is_expanded());
    let open = strip(&c.render(60).join("\n"));
    assert!(open.contains("Weighing the options"), "{open}");
    assert!(open.contains("hides a trap"), "{open}");
    assert!(!open.contains('…'), "{open}");

    c.set_expanded(false);
    assert!(!c.is_expanded());
    let rows: Vec<String> = c
        .render(60)
        .iter()
        .map(|l| strip(l).trim_end().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(rows, vec![" Weighing the options…"], "{rows:?}");
}

/// `ctrl+x b` still hides thinking whole, even with the toggle open.
#[test]
fn omit_hides_thinking_despite_the_toggle() {
    let _g = lock();
    let mut c = AssistantMessageComponent::new(
        Some(&assistant(vec![thinking("weighing the options")])),
        ThinkingDisplay::Omit,
    );
    c.set_expanded(true);
    assert_eq!(c.render(60), Vec::<String>::new());
}

/// Collapsed, the skill block is one row with the name in bold and no mark;
/// expanded, the whole body.
#[test]
fn skill_row_has_no_mark_and_toggles_to_the_whole_body() {
    let _g = lock();
    let body = "one\ntwo\nthree\nfour\nfive\nsix";
    let mut block = SkillBlockComponent::new("write-tests", body);
    block.set_expanded(false);
    assert!(!block.is_expanded());
    let rows: Vec<String> = block
        .render(100)
        .iter()
        .map(|l| strip(l).trim_end().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(rows, vec!["skill write-tests"], "{rows:?}");
    assert!(!rows.join("").contains('◆'));

    block.set_expanded(true);
    assert!(block.is_expanded());
    let open = strip(&block.render(100).join("\n"));
    for line in ["one", "three", "six"] {
        assert!(open.contains(line), "{open}");
    }
    assert!(!open.contains("more lines"), "{open}");
}

/// A running `!` command, collapsed, shows one dim tail line: its last
/// output line at column 4, with no spinner.
#[test]
fn running_user_shell_shows_one_dim_tail_line() {
    let _g = lock();
    let mut c = BashExecutionComponent::new("cargo test", false);
    c.append_output("line one\nline two\nline three\n");
    let lines = c.render(80);
    let stripped: Vec<String> = lines
        .iter()
        .map(|l| strip(l).trim_end().to_string())
        .collect();
    // One blank row opens the run; the header follows it.
    assert!(stripped[1].contains("! cargo test"), "{stripped:?}");
    let body: Vec<&String> = stripped
        .iter()
        .skip(2)
        .filter(|l| !l.trim().is_empty())
        .collect();
    assert_eq!(body, vec!["    line three"], "{stripped:?}");
}

/// Once the `!` command settles, the tail line is gone: the header carries
/// the exit, and the collapsed body is the settled context.
#[test]
fn settled_user_shell_has_no_tail_line() {
    let _g = lock();
    let mut c = BashExecutionComponent::new("cargo test", false);
    c.append_output("line one\nline two\nline three\n");
    let running = c.render(80).join("\n");
    c.set_complete(Some(0), false, None, None);
    let settled: Vec<String> = c
        .render(80)
        .iter()
        .map(|l| strip(l).trim_end().to_string())
        .skip(1)
        .collect();
    assert!(settled[0].contains("exit 0"), "{settled:?}");
    assert_ne!(strip(&running), settled.join("\n"));
    // Four lines or fewer settle whole: the collapsed body is the context,
    // not the single tail line.
    assert!(settled.iter().any(|l| l == "    line one"), "{settled:?}");
    assert!(settled.iter().any(|l| l == "    line two"), "{settled:?}");
    assert!(settled.iter().any(|l| l == "    line three"), "{settled:?}");
}

/// Expanded, a running `!` command shows its tail and spinner, not the
/// single tail line.
#[test]
fn expanded_running_user_shell_shows_tail_and_spinner() {
    let _g = lock();
    let mut c = BashExecutionComponent::new("cargo test", false);
    c.append_output("line one\nline two\nline three\n");
    c.set_expanded(true);
    assert!(c.is_expanded());
    let lines: Vec<String> = c
        .render(80)
        .iter()
        .map(|l| strip(l).trim_end().to_string())
        .filter(|l| !l.trim().is_empty())
        .collect();
    assert!(lines.len() > 2, "{lines:?}");
    assert!(lines.iter().any(|l| l == "    line three"), "{lines:?}");
}

fn custom(text: &str, expanded: bool) -> CustomMessageComponent {
    let mut message = CustomMessageComponent::new(
        CustomMessage {
            custom_type: "skill".into(),
            content: UserContent::Text(text.into()),
            display: true,
            details: None,
            timestamp: 0,
        },
        markdown_theme(),
    );
    message.set_expanded(expanded);
    message
}

/// A custom message folds to one row and opens to its body on the toggle.
#[test]
fn custom_message_toggles_between_one_row_and_body() {
    let _g = lock();
    let mut folded = custom("a short summary", false);
    assert!(!folded.is_expanded());
    let rows: Vec<String> = folded
        .render(80)
        .iter()
        .map(|l| strip(l))
        .filter(|l| !l.trim().is_empty())
        .collect();
    assert_eq!(rows.len(), 1, "{rows:?}");

    let mut open = custom("a short summary", true);
    assert!(open.is_expanded());
    let body = strip(&open.render(80).join("\n"));
    assert!(body.contains("a short summary"), "{body}");
}

/// Branch and compaction summaries ride the same toggle.
#[test]
fn branch_and_compaction_summaries_toggle() {
    let _g = lock();
    let mut branch = BranchSummaryMessageComponent::new(
        BranchSummaryMessage {
            summary: "switched lanes".into(),
            from_id: "a".into(),
            timestamp: 0,
        },
        markdown_theme(),
    );
    assert!(!branch.is_expanded());
    branch.set_expanded(true);
    assert!(branch.is_expanded());
    let body = strip(&branch.render(80).join("\n"));
    assert!(body.contains("switched lanes"), "{body}");

    let mut compaction = CompactionSummaryMessageComponent::new(
        CompactionSummaryMessage {
            summary: "kept the lexer".into(),
            tokens_before: 200,
            tokens_after: Some(50),
            timestamp: 0,
            mission_range: Some((1, 3)),
        },
        markdown_theme(),
    );
    assert!(!compaction.is_expanded());
    compaction.set_expanded(true);
    assert!(compaction.is_expanded());
    let body = strip(&compaction.render(80).join("\n"));
    assert!(body.contains("kept the lexer"), "{body}");
    compaction.set_expanded(false);
    let folded = strip(&compaction.render(100).join("\n"));
    assert!(
        folded.contains("Compacted turns 1–3 · saved 150"),
        "{folded}"
    );
}

fn compaction(before: u64, after: Option<u64>) -> CompactionSummaryMessageComponent {
    CompactionSummaryMessageComponent::new(
        CompactionSummaryMessage {
            summary: "kept the lexer".into(),
            tokens_before: before,
            tokens_after: after,
            timestamp: 0,
            mission_range: Some((1, 3)),
        },
        markdown_theme(),
    )
}

/// At 40 columns the secondary token range drops and the primary `saved N`
/// fact stays on the row.
#[test]
fn compaction_cut_order_drops_the_range_first() {
    let _g = lock();
    let plain = strip(&compaction(12_400, Some(3_100)).render(40)[0])
        .trim_end()
        .to_string();
    assert_eq!(plain, "Compacted turns 1–3 · saved 9,300", "{plain:?}");
}

/// At 80 the full row and the hint both fit.
#[test]
fn compaction_full_row_fits_at_80_with_the_hint() {
    let _g = lock();
    let rows = compaction(12_400, Some(3_100)).render(80);
    let plain = strip(&rows[0]).trim_end().to_string();
    assert!(
        plain.contains("Compacted turns 1–3 · saved 9,300 · 12,400 → 3,100 tokens"),
        "{plain:?}"
    );
}

/// At 60 the full row fits but the hint drops.
#[test]
fn compaction_hint_drops_when_only_the_row_fits() {
    let _g = lock();
    let rows = compaction(12_400, Some(3_100)).render(60);
    let plain = strip(&rows[0]).trim_end().to_string();
    assert!(
        plain.contains("Compacted turns 1–3 · saved 9,300 · 12,400 → 3,100 tokens"),
        "{plain:?}"
    );
    assert!(!plain.contains("for peek"), "{plain:?}");
}

/// Past the primary fact the label cuts short, keeping `saved N` readable.
#[test]
fn compaction_label_cuts_before_the_primary_fact() {
    let _g = lock();
    let rows = compaction(12_400, Some(3_100)).render(30);
    assert_eq!(rows.len(), 1, "{rows:?}");
    for row in &rows {
        assert!(visible_width(row) <= 30, "{row:?}");
    }
    let plain = strip(&rows[0]).trim_end().to_string();
    assert!(plain.contains("saved 9,300"), "{plain:?}");
    assert!(plain.contains('…'), "{plain:?}");
}

const HEADER: &str = "Background tool \"Agent\" (id toolu_01Hx7) finished:";

/// The notice toggle reads back what it was set to.
#[test]
fn notice_toggle_mirrors_its_setter() {
    let _g = lock();
    let text = format!("{HEADER}explore#4 finished ✓ — mapped it.\nRead the full result.");
    let notice = parse_background_notice(&text).expect("a background notice");
    let mut component = BackgroundNoticeComponent::new(notice);
    component.set_expanded(false);
    assert!(!component.is_expanded());
    assert_eq!(component.render(100).len(), 1);
    component.set_expanded(true);
    assert!(component.is_expanded());
    assert_eq!(component.render(100).len(), 2);
}

fn render_ctx<'a>(
    args: &'a serde_json::Value,
    state: &'a mut serde_json::Map<String, serde_json::Value>,
    objects: &'a mut std::collections::HashMap<String, Rc<dyn std::any::Any>>,
    expanded: bool,
    is_error: bool,
) -> ToolRenderContext<'a> {
    ToolRenderContext {
        args,
        tool_call_id: "t",
        last_component: None,
        state,
        objects,
        cwd: "/w",
        execution_started: true,
        args_complete: true,
        is_partial: false,
        expanded,
        show_images: false,
        is_error,
        has_result: true,
    }
}

fn text_content(text: &str) -> Vec<Content> {
    vec![Content::text(text)]
}

/// WebFetch leads with the primary `~N tokens` fact, then the secondary
/// sections fact.
#[test]
fn webfetch_facts_lead_with_tokens_then_sections() {
    let _g = lock();
    let args = serde_json::json!({"url": "https://example.com/docs/install"});
    let details = serde_json::json!({"tokenEstimate": 8100, "sectionCount": 12});
    let content = text_content("Install it.");
    let text = ResultText::default();
    let view = ToolResultView {
        content: &content,
        details: &details,
        text: &text,
    };
    let mut state = serde_json::Map::new();
    let mut objects = std::collections::HashMap::new();
    let mut ctx = render_ctx(&args, &mut state, &mut objects, false, false);
    let body = web::webfetch_definition()
        .render_result
        .as_ref()
        .expect("a result renderer")(
        &view,
        ToolRenderResultOptions {
            expanded: false,
            is_partial: false,
        },
        &mut ctx,
    )
    .expect("a body")
    .borrow_mut()
    .render(120)
    .join("\n");
    let body = strip(&body);
    let tokens = body.find("~8100 tokens").expect("tokens first {body}");
    let sections = body.find("12 sections").expect("sections second {body}");
    assert!(tokens < sections, "{body}");
    assert!(body.contains("Install it."), "{body}");
}

/// WebSearch carries the primary tokens fact with no secondary fact.
#[test]
fn websearch_facts_carry_tokens_only() {
    let _g = lock();
    let args = serde_json::json!({"query": "tokio select"});
    let details = serde_json::json!({"tokenEstimate": 512});
    let content = text_content("It waits on several futures.");
    let text = ResultText::default();
    let view = ToolResultView {
        content: &content,
        details: &details,
        text: &text,
    };
    let mut state = serde_json::Map::new();
    let mut objects = std::collections::HashMap::new();
    let mut ctx = render_ctx(&args, &mut state, &mut objects, false, false);
    let body = web::websearch_definition()
        .render_result
        .as_ref()
        .expect("a result renderer")(
        &view,
        ToolRenderResultOptions {
            expanded: false,
            is_partial: false,
        },
        &mut ctx,
    )
    .expect("a body")
    .borrow_mut()
    .render(120)
    .join("\n");
    let body = strip(&body);
    assert!(body.contains("~512 tokens"), "{body}");
    assert!(body.contains("It waits on several futures."), "{body}");
}

/// TodoWrite is not a toggle: the call carries the counts, and the result
/// has no body.
#[test]
fn todowrite_call_carries_counts_and_no_body() {
    let _g = lock();
    let args = serde_json::json!({"todos": [
        {"status": "completed"}, {"status": "completed"},
        {"status": "pending"}, {"status": "pending"}, {"status": "pending"},
    ]});
    let mut state = serde_json::Map::new();
    let mut objects = std::collections::HashMap::new();
    let mut ctx = render_ctx(&args, &mut state, &mut objects, false, false);
    let call = todo::definition()
        .render_call
        .as_ref()
        .expect("a call renderer")(&args, &mut ctx)
    .expect("a header")
    .borrow_mut()
    .render(120)
    .join("\n");
    let call = strip(&call);
    assert!(call.contains("5 items · 2 done"), "{call}");

    let details = serde_json::Value::Null;
    let content: Vec<Content> = Vec::new();
    let text = ResultText::default();
    let view = ToolResultView {
        content: &content,
        details: &details,
        text: &text,
    };
    let body = todo::definition()
        .render_result
        .as_ref()
        .expect("a result renderer")(
        &view,
        ToolRenderResultOptions {
            expanded: false,
            is_partial: false,
        },
        &mut ctx,
    )
    .expect("a body")
    .borrow_mut()
    .render(120);
    assert!(body.is_empty(), "{body:?}");
}

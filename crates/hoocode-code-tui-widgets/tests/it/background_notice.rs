//! The background-subagent notice: parsing the text the loop hands the
//! transcript, and its outline and peek drawings.
//!
//! The inputs are the loop's real text. `create_default_background_result_message`
//! writes the header, the Agent tool writes the body, and the transcript joins
//! the text blocks with no separator, so the header's colon runs straight into
//! the body.

use hoocode_code_tui_widgets::background_notice::{
    parse_background_notice, BackgroundNotice, BackgroundNoticeComponent, NoticeVerdict,
};
use hoocode_tui_render::Component;

use crate::support::{lock, strip};

/// `create_default_background_result_message`'s header, with the verb it
/// writes for an `Ok` result. A failed dispatch still returns `Ok`, so its
/// header says `finished:` too; the verdict is only in the body. The header
/// names the run's resume label and the raw call id.
fn header(label: &str) -> String {
    format!("Background agent {label} (toolu_01Hx7) finished:")
}

/// The header old session files carry: the tool and the call id, no label.
/// It still parses (back-compat).
const OLD_HEADER: &str = "Background tool \"Agent\" (id toolu_01Hx7) finished:";

/// A done notice: the Agent tool's body with the summary, the outstanding
/// count (empty when none are running), and its "Read the full result" line.
fn done_text(summary: &str, tail: &str) -> String {
    format!(
        "{}explore#4 finished ✓ — {summary}.{tail}\n\
         Read the full result with AgentOutput(\"explore#4\").",
        header("explore#4")
    )
}

/// A failed dispatch's body: a single line, no "Read the full result".
fn failed_text() -> String {
    format!(
        "{}explore#1 failed ✗ — provider timed out after 95s",
        header("explore#1")
    )
}

const SUMMARY: &str = "Mapped the module and its three callers";
const MODEL: &str = "[model: anthropic/claude-haiku-5-5, effort high]";

fn render(text: &str, expanded: bool) -> Vec<String> {
    let _guard = lock();
    let notice = parse_background_notice(text).expect("a background notice");
    let mut component = BackgroundNoticeComponent::with_elapsed(notice, Some("1m27s".into()));
    component.set_expanded(expanded);
    component.render(100).iter().map(|l| strip(l)).collect()
}

#[test]
fn parses_a_done_notice() {
    let notice = parse_background_notice(&done_text(SUMMARY, " 4 still running."));
    assert_eq!(
        notice,
        Some(BackgroundNotice {
            label: "explore#4".into(),
            description: String::new(),
            call_id: Some("toolu_01Hx7".into()),
            verdict: NoticeVerdict::Finished,
            partial: false,
            summary: SUMMARY.into(),
            still_running: Some(4),
            model: None,
            effort: None,
        })
    );
}

#[test]
fn parses_a_failed_notice() {
    let notice = parse_background_notice(&failed_text()).expect("a failed notice");
    assert_eq!(notice.label, "explore#1");
    assert_eq!(notice.verdict, NoticeVerdict::Failed);
    assert_eq!(notice.summary, "provider timed out after 95s");
    assert_eq!(notice.still_running, None);
}

#[test]
fn still_running_is_absent_when_nothing_else_runs() {
    let notice = parse_background_notice(&done_text(SUMMARY, "")).expect("a done notice");
    assert_eq!(notice.still_running, None);
    assert_eq!(notice.summary, SUMMARY);
}

#[test]
fn outline_is_one_line_with_the_counts() {
    let lines = render(&done_text(SUMMARY, " 4 still running."), false);
    assert_eq!(
        lines,
        vec!["● explore#4 finished ✓ · 1m27s · 4 still running"]
    );
    assert!(!lines.join("").contains('▏'));
    assert!(!lines.join("").contains("Read the full result"));
}

#[test]
fn outline_omits_the_running_count_when_absent() {
    let lines = render(&done_text(SUMMARY, ""), false);
    assert_eq!(lines, vec!["● explore#4 finished ✓ · 1m27s"]);
}

#[test]
fn peek_adds_a_summary_line_with_the_gutter() {
    let lines = render(&done_text(SUMMARY, " 4 still running."), true);
    assert_eq!(
        lines,
        vec![
            "● explore#4 finished ✓ · 1m27s · 4 still running",
            "  │ Mapped the module and its three callers",
        ]
    );
    assert!(lines[1].starts_with("  │ "));
    assert!(!lines.join("").contains('▏'));
    assert!(!lines.join("").contains("Read the full result"));
}

#[test]
fn peek_adds_the_model_line_when_the_text_has_one() {
    let text = format!("{}\n{MODEL}", done_text(SUMMARY, " 4 still running."));
    let lines = render(&text, true);
    assert_eq!(
        lines,
        vec![
            "● explore#4 finished ✓ · 1m27s · 4 still running",
            "  │ Mapped the module and its three callers",
            "  model haiku-5-5 · effort high",
        ]
    );
    // The marker itself never reaches the drawing, at either density.
    assert!(!lines.join("").contains("[model"));
    assert_eq!(render(&text, false).len(), 1);
}

#[test]
fn a_failed_notice_renders_its_reason_at_peek() {
    let lines = render(&failed_text(), true);
    assert_eq!(
        lines,
        vec![
            "⊗ explore#1 failed · 1m27s",
            "  │ provider timed out after 95s",
        ]
    );
}

#[test]
fn text_that_is_not_a_notice_falls_back() {
    // Plain text, an empty string, and the dispatch placeholder.
    assert_eq!(parse_background_notice(""), None);
    assert_eq!(parse_background_notice("hello there"), None);
    assert_eq!(
        parse_background_notice(
            "Started \"Agent\" in the background. Its result will arrive as a follow-up message once it finishes — keep working in the meantime."
        ),
        None
    );
    // A header with a body that carries no `type#n` label.
    assert_eq!(
        parse_background_notice(&format!("{}Mapped the module.", header("explore#4"))),
        None
    );
    // A notice line without the header it always comes after.
    assert_eq!(
        parse_background_notice("explore#4 finished ✓ — Mapped the module."),
        None
    );
}

/// The notice the Agent tool writes since 2026-10-30 names the run's
/// description after its label; it still parses, and keeps the description.
#[test]
fn parses_a_notice_that_names_its_description() {
    let text = format!(
        "{}explore#2 (\"audit the db layer\") finished ✓ — {SUMMARY}.\n\
         Read the full result with AgentOutput(\"explore#2\").",
        header("explore#2")
    );
    let notice = parse_background_notice(&text).expect("a notice with a description");
    assert_eq!(notice.label, "explore#2");
    assert_eq!(notice.description, "audit the db layer");
    assert_eq!(notice.verdict, NoticeVerdict::Finished);
    assert_eq!(notice.summary, SUMMARY);
    assert_eq!(notice.call_id.as_deref(), Some("toolu_01Hx7"));
    let cancelled = format!(
        "{}explore#3 (\"x\") cancelled ⊘ — cancelled before it started",
        header("explore#3")
    );
    assert_eq!(
        parse_background_notice(&cancelled).map(|n| n.verdict),
        Some(NoticeVerdict::Cancelled)
    );
}

/// Old session files carry the tool-and-call-id header with no resume
/// label. It still parses, with the same call id and body.
#[test]
fn parses_the_old_tool_and_call_id_header() {
    let body = "explore#4 finished ✓ — Mapped the module.\n\
         Read the full result with AgentOutput(\"explore#4\").";
    let new_shape = parse_background_notice(&format!("{}{body}", header("explore#4")))
        .expect("the new header parses");
    let old_shape =
        parse_background_notice(&format!("{OLD_HEADER}{body}")).expect("the old header parses");
    assert_eq!(old_shape.call_id, new_shape.call_id);
    assert_eq!(old_shape.call_id.as_deref(), Some("toolu_01Hx7"));
    assert_eq!(old_shape.label, "explore#4");
    assert_eq!(old_shape.verdict, NoticeVerdict::Finished);
    assert_eq!(old_shape.summary, "Mapped the module");
    let _guard = lock();
    let mut component = BackgroundNoticeComponent::new(old_shape);
    let lines: Vec<String> = component
        .render(100)
        .iter()
        .map(|l| strip(l).trim_end().to_string())
        .collect();
    assert!(lines[0].starts_with("● explore#4 finished ✓"), "{lines:?}");
}

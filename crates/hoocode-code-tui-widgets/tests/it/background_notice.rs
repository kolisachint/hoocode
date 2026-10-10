//! The background-subagent notice: parsing the text the loop hands the
//! transcript, and its radar and peek drawings.
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
/// header says `finished:` too; the verdict is only in the body.
const HEADER: &str = "Background tool \"Agent\" (id toolu_01Hx7) finished:";

/// A done notice: the Agent tool's body with the summary, the outstanding
/// count (empty when none are running), and its "Read the full result" line.
fn done_text(summary: &str, tail: &str) -> String {
    format!(
        "{HEADER}explore#4 finished ✓ — {summary}.{tail}\n\
         Read the full result with AgentOutput(\"explore#4\")."
    )
}

/// A failed dispatch's body: a single line, no "Read the full result".
fn failed_text() -> String {
    format!("{HEADER}explore#1 failed ✗ — provider timed out after 95s")
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
fn radar_is_one_line_with_the_counts() {
    let lines = render(&done_text(SUMMARY, " 4 still running."), false);
    assert_eq!(
        lines,
        vec!["◦ explore#4 finished ✓ · 1m27s · 4 still running"]
    );
    assert!(!lines.join("").contains('▏'));
    assert!(!lines.join("").contains("Read the full result"));
}

#[test]
fn radar_omits_the_running_count_when_absent() {
    let lines = render(&done_text(SUMMARY, ""), false);
    assert_eq!(lines, vec!["◦ explore#4 finished ✓ · 1m27s"]);
}

#[test]
fn peek_adds_a_summary_line_with_the_gutter() {
    let lines = render(&done_text(SUMMARY, " 4 still running."), true);
    assert_eq!(
        lines,
        vec![
            "◦ explore#4 finished ✓ · 1m27s · 4 still running",
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
            "◦ explore#4 finished ✓ · 1m27s · 4 still running",
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
            "◦ explore#1 ✗ failed · 1m27s",
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
        parse_background_notice(&format!("{HEADER}Mapped the module.")),
        None
    );
    // A notice line without the header it always comes after.
    assert_eq!(
        parse_background_notice("explore#4 finished ✓ — Mapped the module."),
        None
    );
}

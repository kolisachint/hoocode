//! The background-subagent notice: parsing the text the loop hands the
//! transcript, and its outline and peek drawings.
//!
//! The inputs are the loop's real text. `create_default_background_result_message`
//! writes the two-line glance header, the Agent tool writes the body, and
//! the transcript joins the text blocks with no separator, so the header's
//! trailing newline opens the body.

use hoocode_code_tui_widgets::background_notice::{
    parse_background_notice, BackgroundNotice, BackgroundNoticeComponent, NoticeVerdict,
};
use hoocode_tui_render::Component;
use hoocode_tui_util::visible_width;

use crate::support::{lock, strip};

const CALL_ID: &str = "toolu_01Hx7";

/// The machine line: the raw call id verbatim, the resume label, the
/// pointer to the full result.
fn machine_line(label: &str) -> String {
    format!("  {CALL_ID} · resume_task_id: {label} · full result below\n")
}

/// The loop's glance header for a done run with no target.
fn header(label: &str) -> String {
    format!("› ● {label} · done\n{}", machine_line(label))
}

/// A done notice: the Agent tool's body with the summary, the outstanding
/// count (empty when none are running), and its "Read the full result" line.
fn done_text(summary: &str, tail: &str) -> String {
    format!(
        "{}explore#4 finished ✓ — {summary}.{tail}\n\
         Read the full result with AgentOutput(\"explore#4\").",
        header("explore#4")
    )
}

/// A done notice that names its target: the glance line carries the
/// description and the short model, and so does the body.
fn described_done_text(summary: &str, tail: &str) -> String {
    format!(
        "› ● explore#4  audit the db layer · done · haiku-5-5\n{}explore#4 (\"audit the db layer\") finished ✓ — {summary}.{tail}\n\
         Read the full result with AgentOutput(\"explore#4\").\n\
         {MODEL}\n\
         resume_task_id: explore#4",
        machine_line("explore#4")
    )
}

/// A failed dispatch's body: a single line, no "Read the full result".
fn failed_text() -> String {
    format!(
        "› ⊗ explore#1 · failed\n{}explore#1 failed ✗ — provider timed out after 95s",
        machine_line("explore#1")
    )
}

/// A cancelled run's notice, with its target on both lines.
fn cancelled_text() -> String {
    format!(
        "› ⊗ explore#3  trace it · cancelled\n{}explore#3 (\"trace it\") cancelled ⊘ — cancelled before it started",
        machine_line("explore#3")
    )
}

/// The header old session files carry: the tool and the call id, no label.
/// It still parses (back-compat).
const OLD_HEADER: &str = "Background tool \"Agent\" (id toolu_01Hx7) finished:";

/// The label-first header the previous look wrote: the resume label with
/// the raw call id in parens. It still parses (back-compat).
const PAREN_HEADER: &str = "Background agent explore#4 (toolu_01Hx7) finished:";

const SUMMARY: &str = "Mapped the module and its three callers";
const MODEL: &str = "[model: anthropic/claude-haiku-5-5, effort high]";

fn render(text: &str, expanded: bool) -> Vec<String> {
    render_at(text, expanded, 100)
}

fn render_at(text: &str, expanded: bool, width: u16) -> Vec<String> {
    let _guard = lock();
    let notice = parse_background_notice(text).expect("a background notice");
    let mut component = BackgroundNoticeComponent::with_elapsed(notice, Some("1m27s".into()));
    component.set_expanded(expanded);
    component.render(width).iter().map(|l| strip(l)).collect()
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
fn parses_a_cancelled_notice() {
    let notice = parse_background_notice(&cancelled_text()).expect("a cancelled notice");
    assert_eq!(notice.label, "explore#3");
    assert_eq!(notice.description, "trace it");
    assert_eq!(notice.verdict, NoticeVerdict::Cancelled);
    assert_eq!(notice.summary, "cancelled before it started");
}

#[test]
fn still_running_is_absent_when_nothing_else_runs() {
    let notice = parse_background_notice(&done_text(SUMMARY, "")).expect("a done notice");
    assert_eq!(notice.still_running, None);
    assert_eq!(notice.summary, SUMMARY);
}

#[test]
fn outline_shows_the_target_and_facts() {
    let lines = render(&described_done_text(SUMMARY, ""), false);
    assert_eq!(
        lines,
        vec!["● explore#4  audit the db layer · 1m27s · haiku-5-5"]
    );
    assert!(!lines.join("").contains('▏'));
    assert!(!lines.join("").contains("Read the full result"));
    assert!(!lines.join("").contains("finished ✓"));
}

#[test]
fn outline_drops_the_running_count() {
    // The task panel owns live counts: the head never names them.
    let lines = render(&done_text(SUMMARY, " 4 still running."), false);
    assert_eq!(lines, vec!["● explore#4 · 1m27s"]);
    assert!(!lines.join("").contains("still running"));
}

#[test]
fn narrow_rows_shorten_the_target_and_keep_the_facts() {
    let lines = render_at(&described_done_text(SUMMARY, ""), false, 41);
    assert_eq!(lines, vec!["● explore#4  audit… · 1m27s · haiku-5-5"]);
    for line in &lines {
        assert!(visible_width(line) <= 41, "{line:?}");
    }
}

#[test]
fn peek_adds_a_summary_line_with_the_gutter() {
    let lines = render(&done_text(SUMMARY, " 4 still running."), true);
    assert_eq!(
        lines,
        vec![
            "● explore#4 · 1m27s",
            "  │ Mapped the module and its three callers",
        ]
    );
    assert!(lines[1].starts_with("  │ "));
    assert!(!lines.join("").contains('▏'));
    assert!(!lines.join("").contains("Read the full result"));
}

#[test]
fn peek_adds_the_model_line_when_the_text_has_one() {
    let lines = render(&described_done_text(SUMMARY, " 4 still running."), true);
    assert_eq!(
        lines,
        vec![
            "● explore#4  audit the db layer · 1m27s · haiku-5-5",
            "  │ Mapped the module and its three callers",
            "  model haiku-5-5 · effort high",
        ]
    );
    // The marker itself never reaches the drawing, at either density.
    assert!(!lines.join("").contains("[model"));
    assert_eq!(render(&described_done_text(SUMMARY, ""), false).len(), 1);
}

#[test]
fn a_failed_notice_renders_its_reason_at_peek() {
    let lines = render(&failed_text(), true);
    assert_eq!(
        lines,
        vec![
            "⊗ explore#1 · failed · 1m27s",
            "  │ provider timed out after 95s",
        ]
    );
}

#[test]
fn a_cancelled_notice_reads_all_dim_with_its_word() {
    let lines = render(&cancelled_text(), true);
    assert_eq!(
        lines,
        vec![
            "⊗ explore#3  trace it · cancelled · 1m27s",
            "  │ cancelled before it started",
        ]
    );
}

#[test]
fn a_partial_notice_marks_the_head() {
    let text = format!(
        "› ● explore#4  audit the db layer · done · haiku-5-5\n{}explore#4 (\"audit the db layer\") finished ✓ (partial — resume to continue) — {SUMMARY}.\n\
         Read the full result with AgentOutput(\"explore#4\").\n{MODEL}",
        machine_line("explore#4")
    );
    let notice = parse_background_notice(&text).expect("a partial notice");
    assert!(notice.partial);
    assert_eq!(
        render(&text, false),
        vec!["● explore#4  audit the db layer (partial) · 1m27s · haiku-5-5"]
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
    // A machine line without the resume label it always carries.
    assert_eq!(
        parse_background_notice(&format!(
            "› ● explore#4 · done\n  {CALL_ID} · full result below\nexplore#4 finished ✓ — Mapped the module."
        )),
        None
    );
}

/// The notice the Agent tool writes since 2026-10-30 names the run's
/// description after its label; it still parses, and keeps the description.
#[test]
fn parses_a_notice_that_names_its_description() {
    let notice = parse_background_notice(&described_done_text(SUMMARY, ""))
        .expect("a notice with a description");
    assert_eq!(notice.label, "explore#4");
    assert_eq!(notice.description, "audit the db layer");
    assert_eq!(notice.verdict, NoticeVerdict::Finished);
    assert_eq!(notice.summary, SUMMARY);
    assert_eq!(notice.call_id.as_deref(), Some("toolu_01Hx7"));
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
    let mut component = BackgroundNoticeComponent::with_elapsed(old_shape, Some("1m27s".into()));
    component.set_expanded(false);
    let lines: Vec<String> = component
        .render(100)
        .iter()
        .map(|l| strip(l).trim_end().to_string())
        .collect();
    assert_eq!(lines, vec!["● explore#4 · 1m27s"]);
}

/// The previous look's label-first header (`Background agent explore#4
/// (id …) finished:`) still parses, with the same call id and body.
#[test]
fn parses_the_label_first_paren_id_header() {
    let body = "explore#4 finished ✓ — Mapped the module.\n\
         Read the full result with AgentOutput(\"explore#4\").";
    let new_shape = parse_background_notice(&format!("{}{body}", header("explore#4")))
        .expect("the new header parses");
    let paren_shape =
        parse_background_notice(&format!("{PAREN_HEADER}{body}")).expect("the paren header parses");
    assert_eq!(paren_shape.call_id, new_shape.call_id);
    assert_eq!(paren_shape.call_id.as_deref(), Some("toolu_01Hx7"));
    assert_eq!(paren_shape.label, "explore#4");
    assert_eq!(paren_shape.verdict, NoticeVerdict::Finished);
    assert_eq!(paren_shape.summary, "Mapped the module");
    let _guard = lock();
    let mut component = BackgroundNoticeComponent::with_elapsed(paren_shape, Some("1m27s".into()));
    component.set_expanded(false);
    let lines: Vec<String> = component
        .render(100)
        .iter()
        .map(|l| strip(l).trim_end().to_string())
        .collect();
    assert_eq!(lines, vec!["● explore#4 · 1m27s"]);
}

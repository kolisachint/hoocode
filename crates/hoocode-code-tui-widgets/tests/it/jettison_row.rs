//! The Mission Control JETTISONED row (M8e6): the folded compaction
//! summary is one vio row drawn with ratatui, with the real token counts.
//! Expanded, the message is still its sheet.

use std::rc::Rc;

use crate::support::{lock, strip};
use hoocode_agent_types::CompactionSummaryMessage;
use hoocode_code_tui_theme::get_markdown_theme;
use hoocode_code_tui_widgets::custom_message::CompactionSummaryMessageComponent;
use hoocode_tui_components::MarkdownTheme;
use hoocode_tui_render::golden::render_golden;
use hoocode_tui_render::{assert_golden, Component};
use hoocode_tui_util::visible_width;

fn folded() -> CompactionSummaryMessageComponent {
    let md: Rc<dyn Fn() -> MarkdownTheme> = Rc::new(get_markdown_theme);
    CompactionSummaryMessageComponent::new(
        CompactionSummaryMessage {
            summary: "s".into(),
            tokens_before: 120,
            tokens_after: Some(144),
            timestamp: 0,
            mission_range: Some((1, 3)),
        },
        md,
    )
}

#[test]
fn jettison_row_golden() {
    let _g = lock();
    assert_golden!("jettison_row_100x1", render_golden(&mut folded(), 100));
}

#[test]
fn folded_is_one_vio_row_with_the_range_and_real_tokens() {
    let _g = lock();
    let rows = folded().render(100);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(visible_width(&rows[0]), 100, "{:?}", rows[0]);
    let plain = strip(&rows[0]);
    assert!(
        plain.contains("JETTISONED M1–M3 into one summary stage"),
        "{plain:?}"
    );
    assert!(plain.contains("120 → 144 tokens (grew 24)"), "{plain:?}");
    assert!(plain.contains("for peek"), "{plain:?}");
}

#[test]
fn a_single_mission_jettison_names_no_range() {
    let _g = lock();
    let md: Rc<dyn Fn() -> MarkdownTheme> = Rc::new(get_markdown_theme);
    let mut component = CompactionSummaryMessageComponent::new(
        CompactionSummaryMessage {
            summary: "s".into(),
            tokens_before: 200,
            tokens_after: Some(50),
            timestamp: 0,
            mission_range: Some((2, 2)),
        },
        md,
    );
    let plain = strip(&component.render(100)[0]);
    assert!(
        plain.contains("JETTISONED M2 into one summary stage"),
        "{plain:?}"
    );
    assert!(plain.contains("200 → 50 tokens (saved 150)"), "{plain:?}");
}

#[test]
fn a_summary_from_before_mission_ranges_names_earlier_missions() {
    let _g = lock();
    let md: Rc<dyn Fn() -> MarkdownTheme> = Rc::new(get_markdown_theme);
    let mut component = CompactionSummaryMessageComponent::new(
        CompactionSummaryMessage {
            summary: "s".into(),
            tokens_before: 100,
            tokens_after: None,
            timestamp: 0,
            mission_range: None,
        },
        md,
    );
    let plain = strip(&component.render(100)[0]);
    assert!(
        plain.contains("JETTISONED earlier missions into one summary stage"),
        "{plain:?}"
    );
    assert!(plain.contains("from 100 tokens"), "{plain:?}");
}

#[test]
fn expanded_is_still_the_summary_sheet() {
    let _g = lock();
    let mut component = folded();
    component.set_expanded(true);
    let rows = component.render(46);
    assert!(rows.len() > 1, "{rows:?}");
    assert!(rows.iter().any(|row| strip(row).contains('s')), "{rows:?}");
}

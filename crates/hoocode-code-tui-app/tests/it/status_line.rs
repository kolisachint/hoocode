//! The status line (declutter WP3): one row, left slot for live state, right
//! slot for session, model, context and cost. Component goldens at 100
//! columns for each state, the narrow drop order at 60 and 40, and the
//! NO_COLOR rules.

use std::time::Duration;

use hoocode_code_tui_app::status_line::{
    format_cost, format_took, status_line_component, status_line_spans, status_line_text, KeyHint,
    LeftSlot, NoticeLevel, ScrollInfo, SearchInfo, StatusLineData, StatusNotice, TurnOutcome,
    TurnResult,
};
use hoocode_tui_ratatui::core::style::Modifier;
use hoocode_tui_ratatui::{spans_width, Palette};
use hoocode_tui_render::assert_golden;
use hoocode_tui_render::golden::render_golden;

use crate::support::lock;

fn secs(s: f64) -> Duration {
    Duration::from_secs_f64(s)
}

/// Signed in, everything set.
fn base() -> StatusLineData {
    StatusLineData {
        session: Some("amber-harbor".into()),
        model: Some("claude-haiku-5-5".into()),
        context_percent: Some(42.0),
        cost: Some(1.23),
        ..StatusLineData::default()
    }
}

fn done() -> TurnResult {
    TurnResult {
        outcome: TurnOutcome::Done,
        elapsed: secs(11.0),
        tokens: 23_000,
        failed_calls: 0,
    }
}

fn golden(name: &str, data: StatusLineData, width: u16) {
    let _g = lock(Some("dark"));
    let mut component = status_line_component(data);
    assert_golden!(name, render_golden(&mut component, width));
}

fn text(data: &StatusLineData, width: usize) -> String {
    let _g = lock(Some("dark"));
    status_line_text(data, width, &Palette::current())
}

#[test]
fn idle_without_a_turn_has_only_the_right_slot() {
    let mut data = base();
    data.effort = Some("high".into());
    golden("status_line_idle_100x1", data, 100);
}

#[test]
fn idle_after_a_turn_shows_the_result() {
    let mut data = base();
    data.last_turn = Some(done());
    golden("status_line_done_100x1", data, 100);
}

#[test]
fn working_keeps_the_slot() {
    let mut data = base();
    data.last_turn = Some(done());
    data.working = Some(secs(4.2));
    golden("status_line_working_100x1", data, 100);
}

#[test]
fn failed_turn() {
    let mut data = base();
    data.last_turn = Some(TurnResult {
        outcome: TurnOutcome::Failed,
        elapsed: secs(5.0),
        tokens: 14_000,
        failed_calls: 0,
    });
    golden("status_line_failed_100x1", data, 100);
}

#[test]
fn stopped_turn_and_failed_call_variants() {
    let mut data = base();
    data.last_turn = Some(TurnResult {
        outcome: TurnOutcome::Stopped,
        elapsed: secs(2.0),
        tokens: 0,
        failed_calls: 0,
    });
    assert!(text(&data, 100).starts_with(" ⊗ stopped · 2.0s  "));
    data.last_turn = Some(TurnResult {
        outcome: TurnOutcome::Done,
        elapsed: secs(8.5),
        tokens: 7_100,
        failed_calls: 1,
    });
    golden("status_line_failed_call_100x1", data, 100);
}

#[test]
fn plan_mode_leads_the_right_slot() {
    let mut data = base();
    data.plan_mode = true;
    data.last_turn = Some(done());
    golden("status_line_plan_100x1", data, 100);
}

#[test]
fn not_signed_in_shows_session_only() {
    let mut data = base();
    data.signed_out = true;
    data.last_turn = Some(done());
    golden("status_line_signed_out_100x1", data, 100);
}

#[test]
fn working_outranks_not_signed_in_and_notices() {
    let mut data = base();
    data.signed_out = true;
    data.working = Some(secs(1.0));
    data.notice = Some(StatusNotice::new(NoticeLevel::Info, "hello"));
    assert!(matches!(data.left_slot(), LeftSlot::Working(_)));
    data.working = None;
    assert!(matches!(data.left_slot(), LeftSlot::Notice(_)));
    data.notice = None;
    assert_eq!(data.left_slot(), LeftSlot::NotSignedIn);
}

#[test]
fn info_notice_with_keys() {
    let mut data = base();
    data.last_turn = Some(done());
    let mut notice = StatusNotice::new(NoticeLevel::Info, "switched to claude-sonnet-5-5");
    notice.keys = vec![KeyHint::new("shift+alt+m", "back")];
    data.notice = Some(notice);
    golden("status_line_notice_100x1", data, 100);
}

#[test]
fn warning_notice() {
    let mut data = base();
    data.notice = Some(StatusNotice::new(
        NoticeLevel::Warning,
        "context is almost full",
    ));
    golden("status_line_warning_100x1", data, 100);
}

#[test]
fn extra_usage_is_a_dim_row_with_the_settings_off_switch() {
    let mut data = base();
    data.extra_usage = true;
    golden("status_line_extra_usage_100x1", data, 100);
}

#[test]
fn skip_permissions_is_a_dim_row_below_extra_usage() {
    let mut data = base();
    data.extra_usage = true;
    data.skip_permissions = true;
    assert_eq!(data.left_slot(), LeftSlot::ExtraUsage);
    data.extra_usage = false;
    assert_eq!(data.left_slot(), LeftSlot::SkipPermissions);
    golden("status_line_skip_permissions_100x1", data, 100);
}

#[test]
fn notices_time_out_but_actions_do_not() {
    let mut data = base();
    data.last_turn = Some(done());
    let mut info = StatusNotice::new(NoticeLevel::Info, "hello");
    info.age = Duration::from_millis(2_900);
    assert!(!info.expired());
    info.age = Duration::from_secs(3);
    assert!(info.expired());
    data.notice = Some(info);
    // An expired notice yields to the last turn's result.
    assert!(matches!(data.left_slot(), LeftSlot::Turn(_)));

    let mut warn = StatusNotice::new(NoticeLevel::Warning, "careful");
    warn.age = Duration::from_secs(7);
    assert!(!warn.expired());
    warn.age = Duration::from_secs(8);
    assert!(warn.expired());

    let mut action = StatusNotice::action("not signed in", "/login");
    action.age = Duration::from_secs(86_400);
    assert!(!action.expired());
    data.notice = Some(action);
    golden("status_line_action_notice_100x1", data, 100);
}

#[test]
fn scrolled_back() {
    let mut data = base();
    data.scroll = Some(ScrollInfo {
        top: 43,
        bottom: 61,
        total: 78,
        new_below: 3,
        keys: vec![KeyHint::new("end", "live")],
    });
    golden("status_line_scroll_100x1", data, 100);
}

#[test]
fn scrolled_back_hides_zero_new() {
    let mut data = base();
    data.scroll = Some(ScrollInfo {
        top: 1,
        bottom: 19,
        total: 78,
        new_below: 0,
        keys: Vec::new(),
    });
    let row = text(&data, 100);
    assert!(row.contains("scrolled 1–19 of 78  "), "{row}");
    assert!(!row.contains("new"), "{row}");
}

#[test]
fn search() {
    let mut data = base();
    data.search = Some(SearchInfo {
        query: "escaped_quote".into(),
        count: 1,
        index: 1,
        keys: vec![KeyHint::new("enter", "next"), KeyHint::new("esc", "close")],
    });
    golden("status_line_search_100x1", data.clone(), 100);
    if let Some(search) = data.search.as_mut() {
        search.count = 5;
        search.index = 2;
    }
    assert!(text(&data, 100).contains("escaped_quote  2/5 matches"));
    if let Some(search) = data.search.as_mut() {
        search.count = 0;
    }
    assert!(text(&data, 100).contains("escaped_quote  no matches"));
}

#[test]
fn narrow_60_drops_cost_then_session() {
    let mut data = base();
    data.effort = Some("high".into());
    data.last_turn = Some(done());
    // 100: everything. 80: cost goes. 60 with the result: session goes too.
    assert!(text(&data, 100).ends_with("claude-haiku-5-5 high · 42% context · $1.23 "));
    golden("status_line_narrow_60x1", data, 60);
}

#[test]
fn narrow_40_keeps_percent_and_model() {
    let mut data = base();
    data.effort = Some("high".into());
    data.last_turn = Some(done());
    golden("status_line_narrow_40x1", data, 40);
}

#[test]
fn drop_order_is_cost_session_context_word_effort() {
    let mut data = base();
    data.effort = Some("high".into());
    let full = "amber-harbor · claude-haiku-5-5 high · 42% context · $1.23";
    let steps = [
        full,
        "amber-harbor · claude-haiku-5-5 high · 42% context",
        "claude-haiku-5-5 high · 42% context",
        "claude-haiku-5-5 high · 42%",
        "claude-haiku-5-5 · 42%",
    ];
    for expected in steps {
        // The narrowest width that holds this step, with one pad each side.
        let width = expected.chars().count() + 2;
        let row = text(&data, width);
        assert_eq!(row, format!(" {expected} "), "width {width}");
        // One column less must drop the next part.
        let narrower = text(&data, width - 1);
        assert_ne!(narrower.trim(), expected, "width {}", width - 1);
    }
}

#[test]
fn row_is_exactly_the_width() {
    let mut data = base();
    data.effort = Some("high".into());
    data.last_turn = Some(done());
    data.plan_mode = true;
    let _g = lock(Some("dark"));
    let pal = Palette::current();
    for width in [10, 20, 30, 40, 60, 80, 100, 140] {
        let spans = status_line_spans(&data, width, &pal);
        assert!(spans_width(&spans) <= width, "width {width}");
    }
    assert_eq!(spans_width(&status_line_spans(&data, 100, &pal)), 100);
}

#[test]
fn very_narrow_cuts_the_left_slot_with_an_ellipsis() {
    let mut data = base();
    data.last_turn = Some(done());
    let row = text(&data, 28);
    assert!(row.contains('…'), "{row}");
    assert!(row.trim_end().ends_with("42%"), "{row}");
}

#[test]
fn zero_values_are_hidden() {
    let mut data = base();
    data.cost = Some(0.0);
    data.context_percent = Some(0.0);
    data.session = Some(String::new());
    data.effort = Some(String::new());
    assert_eq!(text(&data, 100).trim(), "claude-haiku-5-5");
    data.cost = Some(0.004);
    assert_eq!(text(&data, 100).trim(), "claude-haiku-5-5");
    assert_eq!(format_cost(0.004), None);
    assert_eq!(format_cost(1.234).as_deref(), Some("$1.23"));
    data.last_turn = Some(TurnResult {
        outcome: TurnOutcome::Done,
        elapsed: secs(3.0),
        tokens: 0,
        failed_calls: 0,
    });
    assert!(text(&data, 100).starts_with(" ● done · 3.0s  "));
}

#[test]
fn durations() {
    assert_eq!(format_took(secs(5.0)), "5.0s");
    assert_eq!(format_took(secs(8.5)), "8.5s");
    assert_eq!(format_took(secs(10.4)), "10s");
    assert_eq!(format_took(secs(65.0)), "1m 05s");
}

#[test]
fn plan_word_is_bold_and_first() {
    let mut data = base();
    data.plan_mode = true;
    let _g = lock(Some("dark"));
    let spans = status_line_spans(&data, 100, &Palette::current());
    let plan = spans
        .iter()
        .find(|s| s.content.as_ref() == "plan")
        .expect("plan span");
    assert!(plan.style.add_modifier.contains(Modifier::BOLD));
    let row: String = spans.iter().map(|s| s.content.as_ref()).collect();
    assert!(row.trim_start().starts_with("plan · amber-harbor"), "{row}");
}

#[test]
fn no_color_uses_only_bold_and_dim() {
    let _g = lock(Some("dark"));
    hoocode_code_tui_theme::chalk::set_enabled(false);
    let mut data = base();
    data.plan_mode = true;
    data.working = Some(secs(4.0));
    let spans = status_line_spans(&data, 100, &Palette::current());
    hoocode_code_tui_theme::chalk::set_enabled(true);
    for s in &spans {
        assert_eq!(s.style.fg, None, "{:?}", s.content);
        assert_eq!(s.style.bg, None, "{:?}", s.content);
    }
    let find = |needle: &str| {
        spans
            .iter()
            .find(|s| s.content.contains(needle))
            .unwrap_or_else(|| panic!("no span with {needle}"))
    };
    assert!(find("working").style.add_modifier.contains(Modifier::DIM));
    assert!(find("amber-harbor")
        .style
        .add_modifier
        .contains(Modifier::DIM));
    let plan = find("plan").style.add_modifier;
    assert!(plan.contains(Modifier::BOLD) && !plan.contains(Modifier::DIM));
    let row: String = spans.iter().map(|s| s.content.as_ref()).collect();
    assert!(row.contains("◉ working · 4s · esc stop"), "{row}");
}

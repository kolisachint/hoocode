//! Direct Mission Control radar row checks (spec r5).
//!
//! The clearance rows render outside `flight_rows` while the component
//! carries no clearance flag (`is_waiting_for_clearance` is `false` until
//! the permission gate wires one), so pin `waiting_row` here.

use std::cell::RefCell;
use std::rc::Rc;

use crate::support::{assistant, lock, strip};
use hoocode_ai_types::Content;
use hoocode_code_tui_widgets::radar::{
    agent_live_row, hit_row, live_row, mission_row, prose_row, satellite_row, think_row,
    waiting_row, FlightMark, FlightStats, HIT_INDENT, STEP_INDENT,
};
use hoocode_code_tui_widgets::tool_chain::ToolChainComponent;
use hoocode_code_tui_widgets::tool_execution::{ToolExecutionComponent, ToolExecutionOptions};
use hoocode_code_tui_widgets::tool_output_view::ToolOutputView::Radar;
use hoocode_code_tui_widgets::tool_signal::ToolResult;
use hoocode_code_tui_widgets::{AssistantMessageComponent, ThinkingDisplay};
use hoocode_tui_ratatui::Palette;
use hoocode_tui_render::Component;
use serde_json::json;

#[test]
fn waiting_head_row_holds_for_clearance() {
    let palette = Palette::default();
    let row = strip(&waiting_row(1, "Shell", "sleep 6", true, &palette, 80));
    assert!(row.contains("01"), "{row:?}");
    assert!(row.contains("◉"), "{row:?}");
    assert!(row.contains("Shell"), "{row:?}");
    assert!(row.contains("sleep 6"), "{row:?}");
    assert!(row.contains("holding for clearance"), "{row:?}");
}

#[test]
fn waiting_tail_row_reads_as_next() {
    let palette = Palette::default();
    let row = strip(&waiting_row(2, "Shell", "echo done", false, &palette, 80));
    assert!(row.contains("02"), "{row:?}");
    assert!(row.contains("○"), "{row:?}");
    assert!(!row.contains("◉"), "{row:?}");
    assert!(row.trim_end().ends_with("next"), "{row:?}");
}

fn landed(steps: usize, hits: usize) -> FlightStats {
    FlightStats {
        mark: FlightMark::Landed,
        steps,
        hits,
        landed_time: Some("0.8s".to_string()),
        started: None,
        cost: Some("$0.003".to_string()),
    }
}

#[test]
fn live_row_fills_its_bar_by_elapsed_share() {
    // 3s of ~6s is half of the eight cells.
    let palette = Palette::default();
    let row = strip(&live_row(
        1,
        "Shell",
        "sleep 6; echo done",
        3,
        Some(6),
        &palette,
        80,
    ));
    assert_eq!(
        row.trim_end(),
        "        ◐ 01 Shell  sleep 6; echo done  ████░░░░ 3s of ~6s"
    );
}

#[test]
fn live_row_without_an_estimate_reads_elapsed_only() {
    let palette = Palette::default();
    let row = strip(&live_row(1, "Shell", "sleep 6", 3, None, &palette, 80));
    assert_eq!(row.trim_end(), "        ◐ 01 Shell  sleep 6  3s");
}

#[test]
fn hit_row_indents_twelve_and_keeps_the_chip_inline() {
    let palette = Palette::default();
    let row = strip(&hit_row(
        4,
        "Shell",
        "echo boom; exit 3",
        "boom",
        &palette,
        80,
    ));
    assert_eq!(
        row.trim_end(),
        "            ⊗ 04 Shell  echo boom; exit 3  → boom  HIT"
    );
    assert!(row.trim_end().chars().count() <= 79, "{row:?}");
}

#[test]
fn two_digit_hits_grow_leftward_and_keep_the_columns() {
    let palette = Palette::default();
    let row = strip(&mission_row(1, "wrap", &landed(12, 12), &palette, 80));
    assert!(
        row.trim_end().ends_with("12 steps 12✕   0.8s  $0.003"),
        "{row:?}"
    );
    assert_eq!(row.chars().count(), 79, "{row:?}");
}

#[test]
fn stepless_mission_leaves_the_steps_columns_blank() {
    let palette = Palette::default();
    let stats = FlightStats {
        landed_time: None,
        cost: Some("$0.000".to_string()),
        ..landed(0, 0)
    };
    let row = strip(&mission_row(3, "just talking", &stats, &palette, 80));
    assert_eq!(
        row.trim_end(),
        " M3 ●  just talking                                                      $0.000"
    );
}

#[test]
fn prose_indents_eight_under_a_mission_and_twelve_under_a_hit() {
    let palette = Palette::default();
    let under_mission = strip(&prose_row("It wraps text.", STEP_INDENT, &palette, 80));
    assert_eq!(under_mission.trim_end(), "        It wraps text.");
    let under_hit = strip(&prose_row("It wraps text.", HIT_INDENT, &palette, 80));
    assert_eq!(under_hit.trim_end(), "            It wraps text.");
}

#[test]
fn prose_is_cut_with_an_ellipsis_before_the_last_column() {
    let palette = Palette::default();
    let row = strip(&prose_row(
        "one long line of prose that keeps going past the edge",
        STEP_INDENT,
        &palette,
        24,
    ));
    assert!(row.contains('…'), "{row:?}");
    assert!(row.chars().count() <= 23, "{row:?}");
}

#[test]
fn prose_moves_to_the_hit_text_when_the_chain_above_turns_into_a_hit() {
    let _g = lock();
    let cwd = std::env::current_dir()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let options = ToolExecutionOptions {
        view: Radar,
        ..Default::default()
    };
    // The chain above is still running when the prose message appears.
    let call = Rc::new(RefCell::new(ToolExecutionComponent::new(
        "Shell",
        "live-1",
        json!({"command": "bun test"}),
        options,
        None,
        &cwd,
    )));
    let mut chain = ToolChainComponent::new(Radar);
    chain.add(call.clone());
    let chain = Rc::new(RefCell::new(chain));

    let message = assistant(vec![Content::text("It wraps text.")]);
    let mut prose = AssistantMessageComponent::new(Some(&message), ThinkingDisplay::Omit);
    prose.set_radar_above(Some(Rc::downgrade(&chain)));
    let before = strip(&prose.render(40).join("\n"));
    assert!(before.starts_with("        It wraps text."), "{before:?}");

    // The running call fails: the row above is now a hit.
    call.borrow_mut().update_result(
        ToolResult {
            content: vec![Content::text("ASSERTION FAILED")],
            details: json!({}),
            is_error: true,
        },
        false,
    );
    let after = strip(&prose.render(40).join("\n"));
    assert!(after.starts_with("            It wraps text."), "{after:?}");
}

#[test]
fn narrow_rows_keep_the_last_column_blank_and_never_overflow() {
    let palette = Palette::default();
    let stats = landed(12, 12);
    let live = FlightStats {
        mark: FlightMark::InFlight,
        steps: 3,
        hits: 1,
        landed_time: None,
        started: None,
        cost: None,
    };
    for width in [1usize, 10, 12, 20, 24, 40] {
        let rows = [
            mission_row(12, "a long prompt that goes on", &stats, &palette, width),
            mission_row(1, "wrap", &live, &palette, width),
            hit_row(4, "Shell", "echo boom; exit 3", "boom", &palette, width),
            live_row(
                1,
                "Shell",
                "sleep 6; echo done",
                3,
                Some(6),
                &palette,
                width,
            ),
            waiting_row(1, "Shell", "sleep 6", true, &palette, width),
            agent_live_row(
                1,
                "general-purpose",
                "Compute a product",
                2,
                &palette,
                width,
            ),
            satellite_row(1, "Shell", "sleep 60", "job-1", 4, &palette, width),
            think_row(3, Some("The helper is 13 lines"), &palette, width),
            prose_row("It wraps text and goes on", HIT_INDENT, &palette, width),
        ];
        for row in rows {
            let text = strip(&row);
            assert!(
                text.chars().count() < width.max(1),
                "width {width}: {text:?}"
            );
        }
    }
}

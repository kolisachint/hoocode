//! Direct Mission Control radar row checks (spec r5).
//!
//! The clearance rows render outside `flight_rows` while the component
//! carries no clearance flag (`is_waiting_for_clearance` is `false` until
//! the permission gate wires one), so pin `waiting_row` here.

use crate::support::strip;
use hoocode_code_tui_widgets::radar::{
    hit_row, live_row, mission_row, prose_row, waiting_row, FlightMark, FlightStats, HIT_INDENT,
    STEP_INDENT,
};
use hoocode_tui_ratatui::Palette;

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

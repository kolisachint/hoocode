//! Direct Mission Control radar row checks (spec r5).
//!
//! The clearance rows render outside `flight_rows` while the component
//! carries no clearance flag (`is_waiting_for_clearance` is `false` until
//! the permission gate wires one), so pin `waiting_row` here.

use crate::support::strip;
use hoocode_code_tui_widgets::radar::waiting_row;
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

//! Prose-row checks for the mission flight log's helpers: prose outside a
//! turn sits at the prose indent, wrapped and never cut.

use crate::support::{lock, strip};
use hoocode_code_tui_widgets::outline::{prose_rows, STEP_INDENT};
use hoocode_tui_ratatui::Palette;

#[test]
fn prose_outside_a_turn_sits_at_the_prose_indent() {
    let _g = lock();
    let ink = Palette::default().ink();
    let row = strip(&prose_rows(&["It wraps text.".to_string()], STEP_INDENT, ink, 80)[0]);
    assert!(row.starts_with(" It wraps text."), "{row:?}");
}

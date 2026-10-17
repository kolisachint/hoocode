//! `truncate_at_word`: cuts plain text at a word boundary and ends with `…`.

use hoocode_tui_util::{strip_vt_control_characters, truncate_at_word, visible_width};

#[test]
fn text_that_fits_is_unchanged() {
    assert_eq!(truncate_at_word("dark", 20), "dark");
}

#[test]
fn a_cut_inside_a_word_falls_back_to_the_last_whole_word() {
    let cut = truncate_at_word("Low-token preset for small or local models", 20);
    assert_eq!(cut, "Low-token preset…");
    assert!(visible_width(&cut) <= 20);
}

#[test]
fn a_cut_on_a_word_boundary_keeps_every_word() {
    let cut = truncate_at_word("alpha beta gamma", 11);
    assert_eq!(cut, "alpha beta…");
    assert_eq!(visible_width(&cut), 11);
}

#[test]
fn a_single_word_wider_than_the_row_is_cut_inside() {
    assert_eq!(truncate_at_word("Supercalifragilistic word", 8), "Superca…");
}

#[test]
fn a_zero_width_row_is_empty_and_a_one_column_row_is_the_ellipsis() {
    assert_eq!(truncate_at_word("abc def", 0), "");
    assert_eq!(truncate_at_word("abc def", 1), "…");
}

/// Escapes take no columns: a styled hint is cut at its word, never mid-escape.
#[test]
fn styled_text_is_measured_without_its_escapes() {
    let styled = "\x1b[1m↑/↓\x1b[0m move";
    let cut = truncate_at_word(styled, 6);
    assert_eq!(strip_vt_control_characters(&cut), "↑/↓…");
    assert!(visible_width(&cut) <= 6, "{cut:?}");
    assert!(cut.starts_with("\x1b[1m"), "{cut:?}");
    assert!(cut.ends_with("…") || cut.ends_with("\x1b[0m…"), "{cut:?}");
}

/// A cut inside a styled word keeps the style and closes it before the ellipsis.
#[test]
fn a_cut_inside_a_styled_word_closes_the_style() {
    let styled = "\x1b[38;5;208mSupercalifragilistic\x1b[0m word";
    let cut = truncate_at_word(styled, 8);
    assert_eq!(cut, "\x1b[38;5;208mSuperca\x1b[0m…");
    assert_eq!(visible_width(&cut), 8);
}

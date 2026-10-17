//! `truncate_at_word`: cuts plain text at a word boundary and ends with `…`.

use hoocode_tui_util::{truncate_at_word, visible_width};

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

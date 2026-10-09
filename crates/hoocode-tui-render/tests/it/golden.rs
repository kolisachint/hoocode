//! Golden snapshots: the `golden` module itself, on a plain and a styled component.

use crate::support::Lines;
use hoocode_tui_render::assert_golden;
use hoocode_tui_render::golden::{check_golden, render_golden};
use std::path::PathBuf;

#[test]
fn plain_text_matches_golden() {
    let mut component = Lines(vec![
        "hello world".to_string(),
        String::new(),
        "second line".to_string(),
    ]);
    assert_golden!("plain_text", render_golden(&mut component, 20));
}

#[test]
fn styled_text_matches_golden() {
    let mut component = Lines(vec![
        "\x1b[31mred\x1b[0m plain \x1b[1;4mbold underline\x1b[0m".to_string(),
        "\x1b[38;5;208morange\x1b[0m \x1b[7minverse\x1b[0m".to_string(),
        "\x1b[38;2;10;20;30;48;2;255;255;255mtruecolor\x1b[0m".to_string(),
        "\x1b[3mitalic\x1b[0m".to_string(),
    ]);
    assert_golden!("styled_text", render_golden(&mut component, 40));
}

#[test]
fn mismatch_panics_with_line_diff() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("golden-mismatch");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("expected.txt");
    std::fs::write(&path, "alpha\nbeta\n").unwrap();

    let err = std::panic::catch_unwind(|| check_golden(&path, "alpha\ngamma\n", false))
        .expect_err("a mismatched golden must panic");
    let msg = err.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(msg.contains("golden mismatch"), "{msg}");
    assert!(msg.contains("- beta"), "{msg}");
    assert!(msg.contains("+ gamma"), "{msg}");
}

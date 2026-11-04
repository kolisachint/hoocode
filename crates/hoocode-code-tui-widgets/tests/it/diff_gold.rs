//! jsdiff `diffWords` against the real one (`fixtures/diff-gold.json`, from
//! `archive/migration/tools/goldens/diff.mjs`), and the diff's colouring: the
//! sign alone carries colour.

use crate::support::lock;
use hoocode_code_tui_widgets::diff::render_diff;
use hoocode_code_tui_widgets::jsdiff::diff_words;
use serde_json::Value;

fn gold() -> Value {
    serde_json::from_str(include_str!("../fixtures/diff-gold.json")).unwrap()
}

#[test]
fn diff_words_matches_jsdiff() {
    let gold = gold();
    let mut failures = Vec::new();
    for case in gold["words"].as_array().unwrap() {
        let (a, b) = (case["a"].as_str().unwrap(), case["b"].as_str().unwrap());
        let got: Vec<(String, bool, bool)> = diff_words(a, b)
            .into_iter()
            .map(|c| (c.value, c.added, c.removed))
            .collect();
        let want: Vec<(String, bool, bool)> = case["changes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                (
                    c["value"].as_str().unwrap().to_string(),
                    c["added"].as_bool().unwrap(),
                    c["removed"].as_bool().unwrap(),
                )
            })
            .collect();
        if got != want {
            failures.push(format!("{a:?} -> {b:?}\n  want {want:?}\n  got  {got:?}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures
            .iter()
            .take(5)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn only_the_sign_is_coloured_and_the_rest_is_dim() {
    let _g = lock();
    let t = hoocode_code_tui_theme::theme();
    let out = render_diff("-14 let raw = 1;\n+14 let raw = 2;\n 15 let rest = 3;");
    let rows: Vec<&str> = out.split('\n').collect();
    assert_eq!(
        rows[0],
        format!(
            "{}{}",
            t.fg("toolDiffRemoved", "-"),
            t.fg("dim", "14 let raw = 1;")
        )
    );
    assert_eq!(
        rows[1],
        format!(
            "{}{}",
            t.fg("toolDiffAdded", "+"),
            t.fg("dim", "14 let raw = 2;")
        )
    );
    assert_eq!(rows[2], t.fg("dim", " 15 let rest = 3;"));
    assert!(!out.contains("\x1b[7m"), "no word-level inverse: {out:?}");
}

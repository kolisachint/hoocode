//! Timing of the first code block's stall: the first `supports_language`
//! and the first highlight in a fresh process. Ignored by default; run with
//! `cargo nextest run -p hoocode-tui-highlight --run-ignored only -E 'test(first_block)' --no-capture`.

use std::time::Instant;

#[test]
#[ignore = "timing measurement"]
fn first_block_stall() {
    let t0 = Instant::now();
    let supported = hoocode_tui_highlight::supports_language("rust");
    let t_supports = t0.elapsed();
    let t1 = Instant::now();
    let out = hoocode_tui_highlight::highlight("fn main() {}\n", "rust", false, &|_, _| None);
    let t_highlight = t1.elapsed();
    assert!(supported);
    assert!(out.is_some());
    println!(
        "first_block: supports_language={:.3} ms highlight={:.3} ms total={:.3} ms",
        t_supports.as_secs_f64() * 1e3,
        t_highlight.as_secs_f64() * 1e3,
        (t_supports + t_highlight).as_secs_f64() * 1e3,
    );
}

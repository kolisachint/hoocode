//! Stable smoke runs of the `js_regex` and `ansi_wrap` fuzz targets (see `fuzz/README.md`).

#[path = "../../../../fuzz/targets/ansi_wrap.rs"]
mod ansi_wrap;
#[path = "../../../../fuzz/targets/js_regex.rs"]
mod js_regex;
#[path = "../../../../fuzz/smoke.rs"]
mod smoke;

#[test]
fn fuzz_smoke_js_regex() {
    smoke::run("js_regex", js_regex::run);
}

#[test]
fn fuzz_smoke_ansi_wrap() {
    smoke::run("ansi_wrap", ansi_wrap::run);
}

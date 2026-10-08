//! Stable smoke run of the `markdown` fuzz target (see `fuzz/README.md`).

#[path = "../../../../fuzz/targets/markdown.rs"]
mod markdown;
#[path = "../../../../fuzz/smoke.rs"]
mod smoke;

#[test]
fn fuzz_smoke_markdown() {
    smoke::run("markdown", markdown::run);
}

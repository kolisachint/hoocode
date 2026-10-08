//! Stable smoke run of the `sse` fuzz target (see `fuzz/README.md`).

#[path = "../../../fuzz/smoke.rs"]
mod smoke;
#[path = "../../../fuzz/targets/sse.rs"]
mod sse;

#[test]
fn fuzz_smoke_sse() {
    smoke::run("sse", sse::run);
}

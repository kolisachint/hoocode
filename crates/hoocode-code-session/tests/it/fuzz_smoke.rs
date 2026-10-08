//! Stable smoke run of the `session_jsonl` fuzz target (see `fuzz/README.md`).

#[path = "../../../../fuzz/targets/session_jsonl.rs"]
mod session_jsonl;
#[path = "../../../../fuzz/smoke.rs"]
mod smoke;

#[test]
fn fuzz_smoke_session_jsonl() {
    smoke::run("session_jsonl", session_jsonl::run);
}

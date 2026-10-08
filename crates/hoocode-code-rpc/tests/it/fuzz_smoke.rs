//! Stable smoke run of the `rpc_jsonl` fuzz target (see `fuzz/README.md`).

#[path = "../../../../fuzz/targets/rpc_jsonl.rs"]
mod rpc_jsonl;
#[path = "../../../../fuzz/smoke.rs"]
mod smoke;

#[test]
fn fuzz_smoke_rpc_jsonl() {
    smoke::run("rpc_jsonl", rpc_jsonl::run);
}

#![no_main]

use libfuzzer_sys::fuzz_target;

#[path = "../targets/rpc_jsonl.rs"]
mod rpc_jsonl;

fuzz_target!(|data: &[u8]| rpc_jsonl::run(data));

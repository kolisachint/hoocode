#![no_main]

use libfuzzer_sys::fuzz_target;

#[path = "../targets/session_jsonl.rs"]
mod session_jsonl;

fuzz_target!(|data: &[u8]| session_jsonl::run(data));

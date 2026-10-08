#![no_main]

use libfuzzer_sys::fuzz_target;

#[path = "../targets/sse.rs"]
mod sse;

fuzz_target!(|data: &[u8]| sse::run(data));

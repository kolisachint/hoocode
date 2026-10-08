#![no_main]

use libfuzzer_sys::fuzz_target;

#[path = "../targets/markdown.rs"]
mod markdown;

fuzz_target!(|data: &[u8]| markdown::run(data));

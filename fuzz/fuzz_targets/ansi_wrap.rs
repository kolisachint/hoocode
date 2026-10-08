#![no_main]

use libfuzzer_sys::fuzz_target;

#[path = "../targets/ansi_wrap.rs"]
mod ansi_wrap;

fuzz_target!(|data: &[u8]| ansi_wrap::run(data));

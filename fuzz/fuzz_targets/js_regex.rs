#![no_main]

use libfuzzer_sys::fuzz_target;

#[path = "../targets/js_regex.rs"]
mod js_regex;

fuzz_target!(|data: &[u8]| js_regex::run(data));

#![no_main]

use libfuzzer_sys::fuzz_target;

#[path = "../targets/tui_keys.rs"]
mod tui_keys;

fuzz_target!(|data: &[u8]| tui_keys::run(data));

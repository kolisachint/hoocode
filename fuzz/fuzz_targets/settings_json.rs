#![no_main]

use libfuzzer_sys::fuzz_target;

#[path = "../targets/settings_json.rs"]
mod settings_json;

fuzz_target!(|data: &[u8]| settings_json::run(data));

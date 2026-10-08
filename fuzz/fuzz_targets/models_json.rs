#![no_main]

use libfuzzer_sys::fuzz_target;

#[path = "../targets/models_json.rs"]
mod models_json;

fuzz_target!(|data: &[u8]| models_json::run(data));

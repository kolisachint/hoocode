//! Stable smoke run of the `settings_json` fuzz target (see `fuzz/README.md`).

#[path = "../../../../fuzz/targets/settings_json.rs"]
mod settings_json;
#[path = "../../../../fuzz/smoke.rs"]
mod smoke;

#[test]
fn fuzz_smoke_settings_json() {
    smoke::run("settings_json", settings_json::run);
}

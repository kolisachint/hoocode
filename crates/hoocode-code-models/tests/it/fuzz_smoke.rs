//! Stable smoke run of the `models_json` fuzz target (see `fuzz/README.md`).

#[path = "../../../../fuzz/targets/models_json.rs"]
mod models_json;
#[path = "../../../../fuzz/smoke.rs"]
mod smoke;

#[test]
fn fuzz_smoke_models_json() {
    smoke::run("models_json", models_json::run);
}

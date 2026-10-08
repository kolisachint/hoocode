//! Stable smoke run of the `tui_keys` fuzz target (see `fuzz/README.md`).

#[path = "../../../../fuzz/smoke.rs"]
mod smoke;
#[path = "../../../../fuzz/targets/tui_keys.rs"]
mod tui_keys;

#[test]
fn fuzz_smoke_tui_keys() {
    smoke::run("tui_keys", tui_keys::run);
}

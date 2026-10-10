//! The bundled themes are copies of the pinned hoocode's
//! `coding-agent/src/modes/interactive/theme/*.json`. When the pinned checkout
//! is present, check them JSON-equal (ignoring hoocode-only keys) so a pin bump
//! can't leave them stale.

use std::path::Path;

use serde_json::Value;

/// Hoocode-only keys the pinned TS build does not have; dropped before comparing.
const HOOCODE_ONLY: [&str; 3] = ["toolBandBg", "stockNavyDeep", "stockBlueSoft"];

fn strip_hoocode_only(v: &mut Value) {
    match v {
        Value::Object(map) => {
            map.retain(|k, _| !HOOCODE_ONLY.contains(&k.as_str()));
            map.values_mut().for_each(strip_hoocode_only);
        }
        Value::Array(items) => items.iter_mut().for_each(strip_hoocode_only),
        _ => {}
    }
}

#[test]
fn bundled_themes_match_the_pin() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let pin =
        manifest.join("../../target/hoocode-pin/packages/coding-agent/src/modes/interactive/theme");
    if !pin.is_dir() {
        eprintln!("pinned hoocode not checked out: skipping the copy check");
        return;
    }
    let mut stale = Vec::new();
    for entry in std::fs::read_dir(manifest.join("themes")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let name = path.file_name().unwrap();
        let mut ours: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        strip_hoocode_only(&mut ours);
        let theirs: Option<Value> = std::fs::read(pin.join(name))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok());
        if theirs.as_ref() != Some(&ours) {
            stale.push(name.to_string_lossy().into_owned());
        }
    }
    assert!(
        stale.is_empty(),
        "themes differ from the pin (copy them from {}): {stale:?}",
        pin.display()
    );
}

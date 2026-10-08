//! Target `models_json`: the custom `models.json` file (`hoocode-code-models`).
//!
//! Must not panic on any text. The loader strips `//` comments and trailing commas,
//! parses the result as JSON, and reports a broken file through `error()`. Goes
//! through the real loader, using a scratch file per run.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use hoocode_code_models::{strip_json_comments, ModelRegistry};

pub fn run(data: &[u8]) {
    let text = String::from_utf8_lossy(data);
    let _ = strip_json_comments(&text);

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let path: PathBuf = std::env::temp_dir().join(format!(
        "hoocode-fuzz-models-{}-{}.json",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, data).expect("write scratch models file");

    let registry = ModelRegistry::create(&path);
    let _ = registry.error();
    let _ = registry.get_all().len();

    let _ = std::fs::remove_file(&path);
}

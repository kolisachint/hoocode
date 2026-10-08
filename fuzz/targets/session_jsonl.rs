//! Target `session_jsonl`: session files on disk (`hoocode-code-session`).
//!
//! Must not panic on a truncated, corrupt or hostile session file: malformed lines are
//! skipped, and a file without a valid header yields no entries. Goes through the real
//! file loaders, using a scratch file per run.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use hoocode_code_session::manager::{load_raw_entries, load_session_file, migrate_session_entries};

pub fn run(data: &[u8]) {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let path: PathBuf = std::env::temp_dir().join(format!(
        "hoocode-fuzz-session-{}-{}.jsonl",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, data).expect("write scratch session file");

    let _ = load_session_file(&path);
    let mut entries = load_raw_entries(&path);
    migrate_session_entries(&mut entries);

    let _ = std::fs::remove_file(&path);
}

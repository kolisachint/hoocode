// Test code slices literal fixtures; the string_slice lint guards production code.
#![allow(clippy::string_slice)]

use std::sync::{Mutex, MutexGuard};

/// Every test that switches the process-wide theme (or the agent-dir env
/// override it reads) holds this lock, so those tests run one at a time.
/// Two separate locks let them interleave and overwrite each other's state.
pub(crate) fn global_theme_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

mod gold;
mod pin_copies;
mod schema;
mod theme_contrast;
mod theme_cutout_tokens;
mod theme_export;

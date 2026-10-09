// Test code slices literal fixtures; the string_slice lint guards production code.
#![allow(clippy::string_slice)]

use std::sync::{Mutex, MutexGuard};

/// Terminal capabilities (image protocol, hyperlinks, cell size) are process
/// wide. Every test that sets them holds this lock for its whole run, so one
/// test's hyperlinks or image settings never reach another test's output.
pub(crate) fn capabilities_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

mod autocomplete_ts;
mod common;
mod editor;
mod file_search;
mod frame;
mod fuzz_smoke;
mod image_component;
mod lists_and_input;
mod markdown;
mod markdown_gold;
mod paper_sheet;
#[path = "../../../hoocode-tui-render/tests/it/support/mod.rs"]
mod render_support;
mod scroll_images;
mod truncated_text_ts;

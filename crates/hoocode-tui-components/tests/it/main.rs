// Test code slices literal fixtures; the string_slice lint guards production code.
#![allow(clippy::string_slice)]

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
mod sixel;
mod truncated_text_ts;

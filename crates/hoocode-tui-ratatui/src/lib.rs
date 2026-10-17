//! Generic ratatui adapter for the hoocode TUI.
//!
//! Renders ratatui widgets into the line-based `Component` model: a widget draws
//! into a `Buffer`, and [`buffer_to_lines`] turns the buffer into ANSI lines for
//! the differential writer. No agent or app knowledge lives here.
//!
//! Callers depend on this crate alone; `core` and `widgets` re-export the ratatui
//! crates it is built on.

mod component;
mod lines;

pub use component::RatatuiComponent;
pub use lines::buffer_to_lines;

pub use ratatui_core as core;
pub use ratatui_widgets as widgets;

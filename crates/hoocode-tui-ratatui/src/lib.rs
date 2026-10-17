//! Generic ratatui adapter for the hoocode TUI.
//!
//! Renders ratatui widgets into the line-based `Component` model: a widget draws
//! into a `Buffer`, and [`buffer_to_lines`] turns the buffer into ANSI lines for
//! the differential writer. No agent or app knowledge lives here.
//!
//! Callers depend on this crate alone; `core` and `widgets` re-export the ratatui
//! crates it is built on.

mod component;
mod frame;
mod lines;
mod palette;

pub use component::RatatuiComponent;
pub use frame::{flatten, render_chrome, truncate_line, Chrome};
pub use lines::{buffer_to_lines, paint};
pub use palette::{
    acc, bold, dim, err, faint, from_token, gold, ink, italic, ok, reversed, vio, warn, you,
    Palette, PALETTE_TOKENS,
};

/// The ratatui primitives Mission Control surfaces draw with: [`Line`] and
/// [`Span`] for styled rows, [`Style`] for tokens, and the [`Block`]
/// (thick borders), [`List`] and [`Paragraph`] widgets.
pub use ratatui_core::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
};
pub use ratatui_widgets::{
    block::Block,
    borders::BorderType,
    list::{List, ListItem, ListState},
    paragraph::Paragraph,
};

pub use ratatui_core as core;
pub use ratatui_widgets as widgets;

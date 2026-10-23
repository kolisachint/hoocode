//! Differential rendering for the hoocode TUI: the component tree
//! (`Component`/`Container`) and the `TUI` differential terminal writer. See the
//! `tui` module docs for the deliberate design choices.

mod component;
pub mod fullscreen;
#[cfg(any(test, feature = "golden"))]
pub mod golden;
mod scrollbar;
mod tui;

pub use component::{
    bump_render_epoch, fresh_stamp, mix_stamps, render_epoch, Component, ComponentHandle,
    Container, FlexSpacer, Piece, Rendered, Slot,
};
pub use scrollbar::{scrollbar_glyphs, SCROLLBAR_THUMB, SCROLLBAR_TRACK};
pub use tui::{
    default_scroll_status, CanPinScroll, FrameObserver, FrameTiming, HyperlinkHandler,
    InputInterceptor, InputListener, InputListenerResult, ScrollSearchStatus, ScrollStatus,
    ScrollStatusFormatter, Tui, TuiEvent, WakeHandle, CURSOR_MARKER,
};

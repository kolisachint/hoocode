//! Differential rendering for the hoocode TUI.
//!
//! Ported from TypeScript `@kolisachint/hoocode-tui` -> `tui.ts`: the
//! component tree (`Component`/`Container`) and the `TUI` differential
//! terminal writer. See [`tui`] module docs for the (documented, deliberate)
//! simplifications made relative to the original.

mod component;
#[cfg(any(test, feature = "golden"))]
pub mod golden;
mod scrollbar;
mod tui;

pub use component::{Component, ComponentHandle, Container, FlexSpacer, Slot};
pub use scrollbar::{scrollbar_glyphs, SCROLLBAR_THUMB, SCROLLBAR_TRACK};
pub use tui::{
    default_scroll_status, CanPinScroll, FrameObserver, FrameTiming, HyperlinkHandler,
    InputInterceptor, InputListener, InputListenerResult, ScrollSearchStatus, ScrollStatus,
    ScrollStatusFormatter, Tui, TuiEvent, CURSOR_MARKER,
};

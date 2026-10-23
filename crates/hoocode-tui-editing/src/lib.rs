//! Text editing primitives for the hoocode TUI: an Emacs-style kill/yank ring
//! and a clone-on-push undo stack.
//!
//! The `EditorComponent` trait (extending `hoocode_tui_render::Component`) has no
//! runtime logic of its own and is tightly coupled to `AutocompleteProvider`
//! and the concrete `Editor` component, so it is defined alongside those in
//! `hoocode-tui-components` instead of here.

mod kill_ring;
mod undo_stack;

pub use kill_ring::{KillPushOptions, KillRing};
pub use undo_stack::UndoStack;

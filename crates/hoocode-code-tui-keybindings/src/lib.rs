//! The coding agent's keyboard map: app keybindings, `keybindings.json` loading
//! and key text for hint lines.

pub mod config;
pub mod hints;
pub mod keybindings;
pub mod which_key;

pub use config::{normalize_key, Context, ContextOverride, KeymapConfig};
pub use hints::{
    app_key_label, format_key_text, key_display_label, key_display_text, key_hint, key_hint_text,
    key_text, matches_app_key, raw_key_hint,
};
pub use keybindings::{
    app_keybindings, keybinding, keybinding_definitions, keybinding_definitions_for, keybindings,
    order_keybindings_config, to_keybindings_config, AppKeybindingsManager, KeybindingEntry,
    LeaderEntry,
};
pub use which_key::{leader_label, which_key_rows, WhichKeyPopup, WHICH_KEY_DELAY};

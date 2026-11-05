//! The coding agent's keyboard map: app keybindings, `keybindings.json` loading
//! and key text for hint lines.

pub mod config;
pub mod defaults;
pub mod hints;
pub mod keybindings;
pub mod notices;
pub mod which_key;

pub use defaults::{
    context_of, defaults_document, defaults_path, write_defaults_file, DEFAULTS_FILE_NAME,
};
pub use notices::{alt_keys, alt_notice, take_alt_notice, STATE_FILE_NAME};

pub use config::{
    normalize_key, Context, ContextOverride, KeymapConfig, RESERVED_KEYS, WARNED_KEYS,
};
pub use hints::{
    app_key_label, format_key_text, key_display_label, key_display_text, key_glyph_text, key_hint,
    key_hint_text, key_pair_hint, key_text, matches_app_key, raw_key_hint, SPACE_KEY,
};
pub use keybindings::{
    app_keybindings, check_file, keybinding, keybinding_definitions, keybinding_definitions_for,
    keybindings, order_keybindings_config, to_keybindings_config, AppKeybindingsManager,
    CheckReport, KeybindingEntry, LeaderEntry,
};
pub use which_key::{leader_label, which_key_rows, WhichKeyPopup, WHICH_KEY_DELAY};

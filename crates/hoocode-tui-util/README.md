# hoocode-tui-util

Shared utilities for the hoocode TUI.

Part of the [hoocode](https://github.com/kolisachint/hoocode) Rust workspace.

ANSI-aware terminal string handling: grapheme-cluster-based visible-width
calculation (CJK/emoji-aware), word wrapping and truncation that preserve
SGR styling and OSC-8 hyperlinks across line breaks, and column-range
slicing.

Emoji-width classification uses a fast heuristic pre-filter as the classifier
(there is no exact `\p{RGI_Emoji}` check). See the module docs in
`src/width.rs` for details.

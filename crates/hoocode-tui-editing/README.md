# hoocode-tui-editing

Text editing primitives for the hoocode TUI: an Emacs-style kill/yank ring
and a generic clone-on-push undo stack.

Part of the [hoocode](https://github.com/kolisachint/hoocode) Rust workspace.

The `EditorComponent` interface (a pure type interface) lives alongside the
`Editor` component in `hoocode-tui-components` instead, since it
depends on that crate's `AutocompleteProvider` type.

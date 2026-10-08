# hoocode-tui-editing

Text editing primitives for the hoocode TUI: an Emacs-style kill/yank ring
and a generic clone-on-push undo stack.

Part of the [hoocode](https://github.com/kolisachint/hoocode) Rust workspace.

Ported from `@kolisachint/hoocode-tui`'s `kill-ring.ts` and `undo-stack.ts`.
`editor-component.ts` (a pure type interface) is ported alongside the
`Editor` component in `hoocode-tui-components` instead, since it
depends on that phase's `AutocompleteProvider` type.

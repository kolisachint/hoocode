# hoocode-tui-components

UI widgets for the hoocode TUI: `Spacer`, `Text`, `TruncatedText`,
`BoxComponent`, `Image`, `Loader`, `CancellableLoader`, `SelectList`,
`SettingsList`, `Input`, `Markdown`, `Editor`, and the
`AutocompleteProvider`.

Part of the [hoocode](https://github.com/kolisachint/hoocode) Rust workspace.

The autocomplete provider and the editor component interface live next to the
`Editor`, which they are tightly coupled to.

`Loader`'s animation timing and `Tui`'s render coalescing share the same
design: the component tree is `Rc<RefCell<...>>` and not shared across threads, so
there are no background timers. `Loader::tick()` is a plain method the
owning event loop calls periodically.

`Markdown` parses with `pulldown-cmark` (flat events), via a small intermediate
AST; see `src/markdown/mod.rs` for the resulting fidelity trade-offs
(table column sizing, nested-style ANSI-reset recovery, no footnotes/
definition lists/inline images).

`Editor` (the multi-line text editor) provides the buffer, word-wrap-aware layout,
vertical cursor movement (including the sticky preferred-column algorithm),
undo/kill-ring, prompt history, synchronous autocomplete, paste markers and a
character-jump mode; see `src/editor/editor.rs`.

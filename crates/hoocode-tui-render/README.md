# hoocode-tui-render

Differential rendering for the hoocode TUI: the component tree
(`Component`/`Container`), overlay positioning, and the `Tui` differential
terminal writer that only rewrites changed lines.

Part of the [hoocode](https://github.com/kolisachint/hoocode) Rust workspace.

Design choices (documented in `src/tui.rs`): no reference-identity flatten
memoization (the writer diffs by line content instead) and no 16ms render
coalescing timer (every `request_render()` renders immediately).

Threading model: `Tui`'s component tree uses `Rc<RefCell<dyn Component>>`
and is not `Send`, while `Terminal::start` reads stdin on a background
thread. `Tui::start` bridges the two via an `mpsc` channel of `TuiEvent`s
that the owning thread drains into `Tui::process_event`.

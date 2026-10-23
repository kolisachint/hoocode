# hoocode-tui-terminal

Terminal abstraction for the hoocode TUI: raw mode, dimensions, cursor
control, bracketed paste, Kitty keyboard protocol negotiation, and OSC
progress/title sequences.

Part of the [hoocode](https://github.com/kolisachint/hoocode) Rust workspace.

Includes the stdin buffer. Raw mode and terminal size are handled via `crossterm`; other escape
sequences are written directly so the output bytes are exact.

On Windows the virtual-terminal input console mode is enabled, so Shift+Tab can be
distinguished from Tab.

# hoocode-code-cli

CLI argument parsing and mode dispatch for the `hoocode` coding agent.

Part of the [hoocode](https://github.com/kolisachint/hoocode) Rust workspace.

- `args`: `parseArgs` and `printHelp`, with the flag set and its quirks (`-nt`-style short
  aliases, unknown `--flags` captured as extension flags, `-p <prompt>`).
- `run`: the main entry composition: diagnostics, `--version`/`--help`, app-mode
  resolution, then print / json / rpc / interactive dispatch. Flags that parse but
  whose behavior is not implemented yet fail with `Error: <flag> is not yet supported by hoocode`.

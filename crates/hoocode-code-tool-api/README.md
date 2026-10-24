# hoocode-code-tool-api

Shared plumbing for the built-in tools of the hoocode coding agent.

Part of the [hoocode](https://github.com/kolisachint/hoocode) Rust workspace.

- Truncation: `truncate_head`, `truncate_tail`, `truncate_line`, `format_size`, and the
  800-line / 32KB defaults. The harness variant uses 2000 lines / 50KB
  (`HARNESS_DEFAULT_MAX_*`).
- Path helpers: `expand_path`, `resolve_to_cwd`, `resolve_read_path` (with the macOS
  screenshot variants: AM/PM narrow no-break space, NFD, curly apostrophe).
- `ToolDefinition` (a tool plus its prompt metadata and a
  context-aware `execute`) and `wrap_tool_definition` into an `AgentTool`.
- `fs_error`: Node-style fs error messages (`ENOENT: no such file or directory, access '/x'`),
  so tool errors read the same everywhere.

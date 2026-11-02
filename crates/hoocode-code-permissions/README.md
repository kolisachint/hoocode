# hoocode-code-permissions

hoocode's permission gate as a [`PermissionGate`](../hoocode-agent-types):

- Hard enforcement from the active mode in the merged `hoo-config.json`, with or without a UI:
  `denied_tools`, the `enabled_tools` allowlist, `denied_bash_commands` / `allowed_bash_commands`.
- With a UI: `allowed_write_paths` for edit/write, then `skip_permissions` (global config or
  `HOOCODE_SKIP_PERMISSIONS`), then `safe_shell` (the read-only Shell allowlist in
  `safe_shell.rs`), then `auto_allow`, else a prompt (Yes once / No / Always).
- The shipped mode rules (`hoocode_code_modes::config::default_config`) fill in any mode key
  the user's global file leaves out, and are written to `~/.hoocode/hoo-config.json` on first run.

The `.webtoolsignore` host rule for webfetch is not applied yet.
Bash patterns are JavaScript-style regexes; here they compile with the `regex` crate
(no look-around or backreferences; a pattern that does not compile never matches).

The `hoo-config.json` format is kept stable across releases.

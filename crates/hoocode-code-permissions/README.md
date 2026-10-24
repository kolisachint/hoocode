# hoocode-code-permissions

hoocode's permission gate as a [`PermissionGate`](../hoocode-agent-types):

- Hard enforcement from the active mode in the merged `hoo-config.json`, with or without a UI:
  `denied_tools`, the `enabled_tools` allowlist, `denied_bash_commands` / `allowed_bash_commands`.
- With a UI: `allowed_write_paths` for edit/write, `auto_allow`, else an
  `Allow: …` prompt (Yes once / No / Always, which appends to the mode's global `auto_allow`).

The `.webtoolsignore` host rule for webfetch is not applied yet.
Bash patterns are JavaScript-style regexes; here they compile with the `regex` crate
(no look-around or backreferences; a pattern that does not compile never matches).

The `hoo-config.json` format is kept stable across releases.

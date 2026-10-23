# hoocode-code-resources

Resources for the `hoocode` coding agent:

- `frontmatter` — YAML frontmatter parsing (YAML via `hoocode-agent-harness`).
- `source_info`, `diagnostics` — where a resource came from, and load diagnostics.
- `agent_frontmatter` — agent definitions and the Claude Code
  tool-name shim (`normalize_tools`).
- `skills` — discovery (`SKILL.md` roots, ignore files, `.claude/skills`,
  plugin roots skipped), validation, and the `<available_skills>` prompt block.
- `prompt_templates` — prompt/slash-command templates, argument
  parsing and `$1` / `$@` / `$ARGUMENTS` / `${@:N:L}` substitution.
- `slash_commands` — the built-in slash command list.

These file formats are kept stable across releases.

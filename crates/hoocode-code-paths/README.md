# hoocode-code-paths

Where the coding agent keeps its files: hoocode `packages/coding-agent/src/config.ts` (the
app identity, directory and env-override half; v0.5.89) and `src/utils/paths.ts`.

- Data lives in `~/.hoocode/` (project: `<repo>/.hoocode/`), shared with hoocode-ts.
  The one-time merge of the pre-1.2 folders is in `hoocode-code-migrate`.
- Env overrides take the `HOOCODE_` prefix only: `HOOCODE_CODING_AGENT_DIR`,
  `HOOCODE_CODING_AGENT_SESSION_DIR`, `HOOCODE_USER_AGENTS_DIR`, `HOOCODE_SHARE_VIEWER_URL`
  (naming-and-paths.md §2).

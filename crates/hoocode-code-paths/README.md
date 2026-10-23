# hoocode-code-paths

Where the coding agent keeps its files: the app identity, directory and env-override
configuration, and path helpers.

- Data lives in `~/.hoocode/` (project: `<repo>/.hoocode/`).
  The one-time merge of the pre-1.2 folders is in `hoocode-code-migrate`.
- `lockfile`: the proper-lockfile-compatible `<file>.lock` directory lock taken on
  `auth.json` and `settings.json`. Auth and settings both use it.
- Env overrides take the `HOOCODE_` prefix only: `HOOCODE_CODING_AGENT_DIR`,
  `HOOCODE_CODING_AGENT_SESSION_DIR`, `HOOCODE_USER_AGENTS_DIR`, `HOOCODE_SHARE_VIEWER_URL`
  (naming-and-paths.md §2).

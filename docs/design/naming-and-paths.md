# Naming and paths: one hoocode

Status: **agreed 2026-10-08** ([decisions-2026-10-08.md](decisions-2026-10-08.md)),
design only. Replaces the 2026-10-01 design (separate `~/.hoocode/rust/`; see git
history) and item 2 of [reliability.md](reliability.md).

## Goal

The Rust hoocode is a **drop-in replacement** for hoocode-ts. It is called `hoocode`
everywhere, keeps its data in `~/.hoocode` next to hoocode-ts, and reads and writes
the same files. Nothing visible or internal says cortex any more. There is one user
(the owner), so we change things in one go, with no deprecation period.

## Decisions

| Decision | Why |
|---|---|
| **Data lives in `~/.hoocode`, shared with hoocode-ts.** Project data lives in `<repo>/.hoocode/`. | One config, one login, one session list. Safe today: hoocode-ts merges only the settings fields it changed (unknown keys survive), and our `auth.json` lock is `proper-lockfile` compatible. |
| **Rename everything**: crates `cortexcode-*` → `hoocode-*`, Rust paths `cortexcode_*` → `hoocode_*`, the cargo binary `cortex` → `hoocode`, `APP_NAME` → `hoocode`, `APP_TITLE` → `HooCode`, the system prompt, help, logs, thread names, scripts, CI, docs. | No "two names for one thing". About 740 files and 6,400 occurrences; mechanical. |
| **`HOOCODE_` is the only env prefix.** Every `CORTEX_` and `CORTEXCODE_` variable is renamed; no aliases. | One user; the release notes list the renames. |
| **One-time full merge of `~/.cortexcode` into `~/.hoocode`**; on a conflict **`~/.cortexcode` wins**. Every file it overwrites is backed up first. | The owner's choice. **Risk:** after the first Rust run, hoocode-ts uses the Rust settings and logins. The backup undoes it. |
| **Project folders merge too**: `<repo>/.cortexcode/` → `<repo>/.hoocode/`, same rules, skipping `dispatch/`. | The owner's choice. **Risk:** it writes into git working trees; the changes show in `git status`. |
| **Copy, never move or delete.** `~/.cortexcode` and `<repo>/.cortexcode/` stay as they are; deleting them is the owner's call. | A half-finished move can't be undone; a copy can. |
| **Shared files keep hoocode-ts's shape**: new settings keys are fine, new session data goes in `custom` entries, the `auth.json` lock stays `proper-lockfile` compatible. | Both tools must keep reading what the other wrote. |
| **All docs are rewritten too**, history included. The migration plan becomes `ts-to-rust-migration.md`. | The owner's choice. Git history keeps the old names. |

## What we build

### 1. The rename (first coding session, step 0)

One mechanical commit, after the 11 crate deletions and before any other branch is
open, so nothing conflicts with it.

- `git mv crates/cortexcode-<x> crates/hoocode-<x>`; package names, `[workspace]`
  members, `use cortexcode_<x>` → `use hoocode_<x>`; `Cargo.lock` regenerated.
- `[[bin]] cortex` → `hoocode`. `scripts/install.sh`, `binaries.yml` and the npm
  packaging stop renaming at install. `hoo` stays a link. The `hoocode-ts` shim is
  unchanged.
- `[workspace.metadata.cortex]` → `[workspace.metadata.hoocode]`; `CORTEX_BIN` →
  `HOOCODE_BIN`.
- Parity harness, `ledger.json`, `dep-firewall.json`, `pin_drift.py`,
  `gen_help_text.py`, eval scripts and CI: new names. Harness reports label the two
  sides `ts` and `rust`.
- Docs: every `cortexcode` and `cortex` becomes `hoocode` (or `hoocode-<x>` for a
  crate), including `docs/maps/`, the migration plan, `PROGRESS.md`, the ledger notes
  and `.claude/skills/`. The only exceptions are this card and the decision pages,
  which must name the old names.
  - `hoocode-to-cortexcode-migration.md` → `ts-to-rust-migration.md`, with links
    updated.
- Guard: `scripts/ci/no_cortex.sh` fails CI if `cortex` (any case) appears outside
  an allowlist (this card, decision pages, the merge code that reads the old
  folders).
- Done when: fmt, clippy, all tests, the dep firewall and the L2 parity checks pass,
  and the guard is green.

### 2. Paths and environment (reliability card, item 2)

| Item | Now | Then |
|---|---|---|
| `CONFIG_DIR_NAME` | `.cortexcode` (reads `.hoocode` as a fallback) | `.hoocode`, no fallback |
| Agent dir | `~/.cortexcode` | `~/.hoocode` |
| Managed binaries (fd, rg) | `~/.cortexcode/bin` | **None**: fd and rg are dropped (reliability item 6). `~/.hoocode/bin` belongs to the installer. |
| Debug log | `~/.cortexcode/cortex-debug.log` | `~/.hoocode/hoocode-debug.log` (the same file hoocode-ts writes) |
| Dispatch dirs | `<cwd>/.cortexcode/dispatch/` | `<cwd>/.hoocode/dispatch/` |
| `ENV_PREFIXES` | `CORTEXCODE_`, `CORTEX_`, `HOOCODE_` | `HOOCODE_` only |
| Env variables | about 25 `CORTEX*` names | the same suffixes with `HOOCODE_` (e.g. `HOOCODE_CA_CERT`, `HOOCODE_MOUSE`, `HOOCODE_IMAGE_PROTOCOL`) |

`hoobot/src/config.ts` hardcodes `<workspace>/.cortexcode`. It changes to `.hoocode`
in the same release (in the hoobot repo).

### 3. The one-time merge

Runs at the first start after the upgrade, in every mode (`print`, `json` and `rpc`
report to stderr). It is also available as `hoocode migrate [--dry-run]`.

| Source | Into | Rule (`~/.cortexcode` wins) |
|---|---|---|
| `settings.json` | `~/.hoocode/settings.json` | Deep merge of objects; on a leaf conflict the cortexcode value wins; arrays are replaced whole |
| `auth.json` | `~/.hoocode/auth.json` | Per provider: the cortexcode entry wins. Written under the `auth.json` lock. |
| `models.json`, `keybindings.json`, `hoo-config.json` | same names | Deep merge, cortexcode wins |
| `sessions/`, `themes/` | same folders | Copy every file; a same-named file is replaced by cortexcode's |
| `bin/`, `cache/`, `embsearch/`, debug log | — | Skipped (re-downloaded or regenerated) |

- **Backup first:** each `~/.hoocode` file the merge changes is copied to
  `~/.hoocode/backup-cortexcode-<timestamp>/` before it is written.
- **Report:** what was added, what was overwritten (with its backup path) and what
  was skipped, printed once and saved as `~/.hoocode/merge-report-<timestamp>.txt`.
- **Once:** a marker `~/.hoocode/.merged-from-cortexcode` stops a second run. A merge
  lock stops two processes merging at the same time.
- **Project folders:** when hoocode starts in a folder whose `.cortexcode/` has not
  been merged (no `.hoocode/.merged-from-cortexcode` there), it merges it into
  `.hoocode/` by the same rules. It skips `dispatch/`, backs up into
  `.hoocode/backup-cortexcode-<timestamp>/`, and shows one notice: "merged
  `.cortexcode/` into `.hoocode/`; review with `git status`; add `.hoocode/dispatch/`
  and `.hoocode/backup-*/` to `.gitignore`". It never edits `.gitignore` itself.
- After the merge, nothing reads `.cortexcode` again.

### 4. Tests

- Rename: the guard script; the L2 parity suite; a golden test that `--version`, the
  help text and the window title say hoocode.
- Paths: agent dir, auth path, sessions dir, dispatch root and bin dir all resolve
  under `.hoocode`; only `HOOCODE_` variables are read.
- Merge: each row of the table, with conflicts, for home and project folders;
  backups made before every overwrite; the marker stops a re-run; `--dry-run` writes
  nothing; a missing or partial `~/.cortexcode` still works; two processes merge
  once.
- Shared files: a session, settings and auth file written by Rust is read by
  hoocode-ts (fixtures through the harness), and the reverse.

## Not doing

- A separate `~/.hoocode/rust/` or `~/.hoocode/ts/` namespace.
- Deprecated `CORTEX_` and `CORTEXCODE_` aliases.
- Deleting `~/.cortexcode` or `<repo>/.cortexcode/`.
- Editing `.gitignore` files.
- Changing hoocode-ts (it is the reference; it already uses `~/.hoocode`).

## Open questions

- None. Risks accepted on 2026-10-08: hoocode-ts adopts the Rust settings and logins
  after the first merge; the project merge writes into repos; rewritten history docs
  no longer show the old names (git does).

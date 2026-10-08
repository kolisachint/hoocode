# Reliability first

Status: **agreed 2026-10-07**, design only. Build this before any new feature.

## Goal

Fix the known ways `hoocode` is unsafe, wrong or crashes today, and add checks that
stop the same bug classes coming back.

## Decisions

| Decision | Why |
|---|---|
| In `--mode rpc`, a tool that needs approval is **denied** when no client can answer | Today it runs without asking (`code-permissions` `evaluate`: `!has_ui` returns `Allow`; `code-cli` `build_permission_gate` attaches a UI only in interactive mode). hoobot users could run bash and edit files unchecked. |
| `--print` and `--mode json` keep today's behaviour (allow), but say so | You ran the command yourself, and headless scripts depend on it. A one-line stderr notice lists which tools ran without approval. |
| Data directories move to `~/.hoocode`, shared with hoocode-ts, after a one-time merge with backups ([naming-and-paths.md](naming-and-paths.md); changed 2026-10-08) | Rust is a drop-in replacement: one config, one login, one session list |
| macOS is a supported dev platform: its test failures are bugs | It's the user's local machine |
| Panics on user or model input are bugs | One crashed the TUI on 2026-10-02 (byte-slicing inside a multi-byte character) |

## What we build

1. **rpc fail-closed.** Built 2026-10-08 (`claude/1.1-rpc-fails-closed`).
   - `evaluate` takes an `ApprovalChannel` (`Ui`, or `Headless { fail_closed }`).
     A gated call that needs approval is `Block`ed when `fail_closed`, with a
     message that names `auto_allow` in hoo-config.json.
   - rpc mode is fail-closed. Its children inherit the policy through the internal
     env `HOOCODE_INTERNAL_APPROVALS_FAIL_CLOSED`, so json subagents are closed too.
     Warm workers opt out of their own gate with the internal `HOOCODE_INTERNAL_WARM_WORKER`,
     unless their parent is fail-closed (the inherited flag wins).
   - print and json keep `Allow` and print a one-line stderr note.
   - MCP and plugin tools stay ungated (TODO in `code-permissions`).
   - Tests: a gated tool in rpc is blocked, in print is allowed with the notice,
     and in interactive mode still prompts.
   - Approval dialogs over rpc are dropped (2026-10-08); hoobot uses the
     app-server ([rpc-approvals.md](rpc-approvals.md) is kept for the record).
2. **Paths** (changed 2026-10-08): [naming-and-paths.md](naming-and-paths.md) §2–4.
   Data moves to `~/.hoocode`, **shared with hoocode-ts** (drop-in replacement);
   `HOOCODE_` is the only env prefix; a one-time merge copies `~/.hoocode` and
   `<repo>/.hoocode/` in (hoocode wins, backups first). The crate and binary
   rename is §1 of that card and happens in step 0, before this card.
3. **macOS test failures** (all four fail on a clean checkout):
   - `code-main` replay: temp paths resolve under `/private` on macOS, so
     canonicalize both sides;
   - `code-tool-api`: macOS file names come back NFD-normalized, so compare
     normalized names;
   - `tui-app` `option+a` tip: the key name differs on macOS, so test per platform;
   - `tui-selectors` scoped-models hint: find the cause, then fix.
   - Add a macOS test job to `ci.yml`. `binaries.yml` already uses `macos-latest`
     runners for release builds, but no tests run there.
   - **Status (2026-10-08, branch `claude/1.3-macos-tests`): fixes done, macOS job staged.**
     Fixed: replay masks both spellings of each temp path (`/private`), `code-tool-api`
     compares names with `same_file_name` (NFC both sides), the `option+a` tip expects
     `option` on macOS, and the scoped-models footer test renders at 200 columns (cause
     below). The job is `migration/ci/macos-tests.patch`, not yet applied. Not run on
     macOS (Linux only here).
     Scoped-models cause: `alt` is printed as `option` on macOS, so the 5-key footer is
     about 20 columns wider and wraps at 120, splitting "all enabled".
4. **Panic audit.**
   - Scope: about 170 byte-index slices (`[..n]`, `[n..]`) on strings, and about
     815 `unwrap`/`expect` calls in non-test code. Most are fine.
   - Triage by input source: anything touched by user text, model output, files or
     network is fixed (char-boundary-safe helpers, `?`, or a logged fallback).
   - Add `clippy::string_slice` as a warning in the TUI and tool crates so new
     slices get a second look.
5. **Fuzzing.** Property tests with `proptest` (runs in the normal test suite, stable
   Rust) for: session JSONL parse, settings and `models.json` parse, SSE parser,
   terminal key parser, markdown renderer, and list-item detection. An optional
   nightly `cargo-fuzz` CI job runs the same targets for longer.
   - **Status (2026-10-08, branch `claude/1.6-fuzzing`): done, except the CI job is staged.**
     Targets are in `fuzz/` (see `fuzz/README.md`): `sse`, `rpc_jsonl`, `session_jsonl`,
     `settings_json`, `models_json`, `tui_keys`, `js_regex`, `ansi_wrap`, `markdown`
     (list items are covered through the lexer; `list_item_regex` is crate-private).
     Smoke tests run on stable as `fuzz_smoke` in each owning crate. They replay seeds,
     their prefixes and deterministic mutations, so no `proptest` dependency was added.
     The nightly job is `migration/ci/fuzz.patch`, not yet applied. The first run found
     a panic in `js_regex` `translate` (truncated `\u`, `\x`, `\p`), now fixed.

6. **`@file` autocomplete without `fd`** (2026-10-08).
   - Status (2026-10-08, `claude/1.4b-finder-async`): **done.** The walk is in
     `hoocode-code-tools` (`file_finder`, the `ignore` crate) and the app injects it,
     so `@` works with no `fd` installed. Results are capped at 100 matches and 20 shown,
     as before. The walk runs on a worker thread (`tui-components/src/autocomplete/file_search.rs`):
     typing never waits on it, matches for a query typed past are dropped, and the editor
     asks again when the walk finishes. The external-tools layer is removed: the `fd` and
     `rg` lookups, the `external_tools` table and its `/settings` pane, `bin_dir()`, and
     `HOOCODE_RG_BINARY`, `HOOCODE_FD_BINARY`, `HOOCODE_NATIVE_SEARCH`. The parity harness
     masks the `/settings` pane (`mask_snapshots` in `settings-pane.json`).
   - A file finder in a `code-*` crate walks with the `ignore` crate using fd's
     rules: files and folders, hidden included, follow links, honour `.gitignore`,
     skip `.git`, match the name (the full path when the query has a `/`),
     smart case, at most 100 results.
   - The autocomplete gets it injected (the `tui-*` crates stay free of it), runs it
     off the UI thread, and drops results for a query the user has typed past.
   - Remove the `fd` and `rg` lookups, the external-tools table and its `/settings`
     pane, `bin_dir()`, and the `RG_BINARY`, `FD_BINARY` and `NATIVE_SEARCH` env
     variables. Mask the `/settings` pane difference in the parity harness.
   - Tests: the same suggestions as `fd` on a fixture tree (hidden files,
     `.gitignore`, symlinks, `.git` skipped, path queries); typing never waits on
     the walk.

Done when: all six land, CI is green on Linux and macOS, and every item has
regression tests.

## Not doing

- Approval dialogs over rpc (dropped 2026-10-08).
- Rewriting every `unwrap`; only ones reachable from input.

## Open questions

- None blocking. The exact `tui-selectors` macOS cause is found during the work.

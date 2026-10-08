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

1. **rpc fail-closed.**
   - `evaluate` takes a third case, "no UI and not allowed to auto-approve", and
     returns `Block` with a clear message ("needs approval; no client attached").
   - rpc mode uses it; print and json keep `Allow` and print the notice.
   - Tests: a gated tool in rpc is blocked, in print is allowed with the notice,
     and in interactive mode still prompts.
   - Approval dialogs over rpc are dropped (2026-10-08); hoobot uses the
     app-server ([rpc-approvals.md](rpc-approvals.md) is kept for the record).
2. **Paths** (changed 2026-10-08): [naming-and-paths.md](naming-and-paths.md) §2–4.
   Data moves to `~/.hoocode`, **shared with hoocode-ts** (drop-in replacement);
   `HOOCODE_` is the only env prefix; a one-time merge copies `~/.cortexcode` and
   `<repo>/.cortexcode/` in (hoocode wins, backups first). The crate and binary
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

6. **`@file` autocomplete without `fd`** (2026-10-08).
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

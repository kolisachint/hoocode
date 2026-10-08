# Reliability first

Status: **agreed 2026-10-07**, design only. Build this before any new feature.

## Goal

Fix the known ways `cortex` is unsafe, wrong or crashes today, and add checks that
stop the same bug classes coming back.

## Decisions

| Decision | Why |
|---|---|
| In `--mode rpc`, a tool that needs approval is **denied** when no client can answer | Today it runs without asking (`code-permissions` `evaluate`: `!has_ui` returns `Allow`; `code-cli` `build_permission_gate` attaches a UI only in interactive mode). hoobot users could run bash and edit files unchecked. |
| `--print` and `--mode json` keep today's behaviour (allow), but say so | You ran the command yourself, and headless scripts depend on it. A one-line stderr notice lists which tools ran without approval. |
| Data directories move to `~/.hoocode/rust/` with a backup ([naming-and-paths.md](naming-and-paths.md) steps 1–2) | Rust reads hoocode-ts's `~/.hoocode` files today, and the two tools must not share private state |
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
2. **Paths.** [naming-and-paths.md](naming-and-paths.md) steps 1–2:
   - new layout in `code-paths`;
   - a one-time migration with a backup and a marker file;
   - `--no-migrate` and `--migrate-only` flags;
   - tests for all four cases: new location only, old only, both, neither.
   - Also ship a `.gitignore` snippet for `<repo>/.hoocode/rust/dispatch/`.
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

Done when: all five land, CI is green on Linux and macOS, and every item has
regression tests.

## Not doing

- Approval dialogs over rpc (dropped 2026-10-08).
- Rewriting every `unwrap`; only ones reachable from input.

## Open questions

- None blocking. The exact `tui-selectors` macOS cause is found during the work.

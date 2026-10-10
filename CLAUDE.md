# hoocode (Rust)

hoocode (Rust) fully replaces the TypeScript hoocode-ts (decided 2026-10-10). No TS
reference, pin, shim or parity checks remain; don't ask about TS parity.

- The TS→Rust migration is finished; its records are in `archive/`.
- **Disk (2026-10-10):** dev builds have `debug = false`. At session start run
  `scripts/dev/prune_target.sh --apply --if-over 15` without asking. It keeps current
  builds and deletes stale units (older than 3 days, old `cortex*` names) and an
  oversized incremental cache. Never `cargo clean` unless the user asks.
- Build speed / agent loop design (profiles, nextest, test layout, hooks, CI):
  `docs/design/build-speed.md`. Its §4.3 rules join this file as they are implemented.
- Done = `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo nextest run -p <touched crates>`. CI runs the full workspace. Component goldens run in nextest (`UPDATE_GOLDENS=1` accepts them). Screen goldens (about 10 key screens, `scripts/tui/goldens.py`) are a pre-release check, not per change.

- **MIT only.** Never copy code, schemas or generated types from non-MIT projects (e.g.
  `../codex`, Apache-2.0) into this repo. Running such tools in tests is fine.
- **What to build next:** `docs/design/README.md` (cards in build order, plus the user's
  design preferences). Start with step 0a of the plan in `docs/design/README.md`.
  A design session changes docs only.
- **Two plans side by side:** the core plan (`docs/design/README.md`) and the TUI plan
  (`docs/design/tui-activity.md`, 2026-10-09). When the user asks to proceed or implement,
  ask which plan and which step first.
- **TUI work is done by Haiku subagents** (user, 2026-10-09): review, implement and test.
  The main session orchestrates, briefs and merges. In the TUI plan, simplification
  (Phase S of `docs/design/tui-activity.md`) comes before the feature phases.
- **Maps:** `docs/maps/packages.md` (crates) and `docs/maps/ui.md` (screen, pickers, slash
  commands). Read them to find code; update them in the same commit as any crate, screen
  or command change.

- Naming (2026-10-08, `archive/docs/naming-and-paths.md`): everything is named hoocode
  (crates `hoocode-*`, binary `hoocode`, data in `~/.hoocode`, `HOOCODE_` env only). Don't
  add new `cortex` names (`scripts/ci/no_cortex.sh`).

Commands:

```bash
cargo nextest run --workspace                      # or cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
scripts/eval/subagent_evals.py --include-slow # subagent reliability evals (real binary + mock LLM)
python3 scripts/ci/check_dep_firewall.py           # only matters when deps change
python3 scripts/tui/goldens.py check all           # screen goldens, pre-release only (tmux, ~10 scenarios); reports in target/tui-goldens/
python3 scripts/tui/goldens.py update <scenario>   # accept a deliberate screen change
```

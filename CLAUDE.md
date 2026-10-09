# hoocode (Rust)

Rust port of the TypeScript coding agent hoocode-ts (https://github.com/kolisachint/hoocode-ts),
pinned to hoocode-ts v0.6.0 (`[workspace.metadata.hoocode.source]` in `Cargo.toml`).
`python3 migration/pin_drift.py status` shows whether upstream has released past the pin;
a bump needs the user's go-ahead and follows the `pin_drift.py delta` checklist.
Never modify hoocode-ts; it is the reference.

- "continue migration" / migration status → follow `.claude/skills/continue-migration/SKILL.md`.
- **Paused 2026-10-01:** every remaining task is `deferred` by user decision (plan §0.3).
  Resume only a task the user names, by moving it back to `todo` first.
- Plan and rationale: `docs/design/ts-to-rust-migration.md`.
- Task status (source of truth): `migration/ledger.json` via `python3 migration/ledger.py`.
- Handoff log: `migration/PROGRESS.md`.
- Build speed / agent loop design (profiles, nextest, test layout, hooks, CI):
  `docs/design/build-speed.md`. Its §4.3 rules join this file as they are implemented.
- Done = L1 (cargo fmt/clippy/nextest + `migration/check_dep_firewall.py`) + `python3 scripts/tui/goldens.py check all`
  (component goldens run in nextest; `UPDATE_GOLDENS=1` accepts them, `goldens.py update <scenario>` accepts screens).
  Visual changes get a review bundle (`scripts/tui/review_bundle.py`) reviewed by a Haiku subagent with the
  `tui-review` skill (advisory).

- **MIT only.** Never copy code, schemas or generated types from non-MIT projects (e.g.
  `../codex`, Apache-2.0) into this repo. Running such tools in tests is fine.
- Work outside the migration has no TypeScript reference (user, 2026-10-01).
- **What to build next:** `docs/design/README.md` (cards in build order, plus the user's
  design preferences). `docs/design/decisions-2026-10-07.md` and `decisions-2026-10-08.md` (concurrency
  card, scope cuts, crates to delete) win over any card. Start
  with step 0a of the plan in `docs/design/README.md`. A design session changes docs only.
- **Two plans side by side:** the core plan (`docs/design/README.md`) and the TUI plan
  (`docs/design/tui-activity.md`, 2026-10-09). When the user asks to proceed or implement,
  ask which plan and which step first.
- **TUI work is done by Haiku subagents** (user, 2026-10-09): review, implement, test and visual
  review. The main session orchestrates, briefs and merges. In the TUI plan, simplification
  (Phase S of `docs/design/tui-activity.md`) comes before the feature phases.
- **Maps:** `docs/maps/packages.md` (crates) and `docs/maps/ui.md` (screen, pickers, slash
  commands). Read them to find code; update them in the same commit as any crate, screen
  or command change.

- Naming (2026-10-08, `docs/design/naming-and-paths.md`): the Rust build is a drop-in
  replacement for hoocode-ts. Everything is being renamed to hoocode (crates `hoocode-*`,
  binary `hoocode`, data in `~/.hoocode` shared with hoocode-ts, `HOOCODE_` env only). The
  TS build is `hoocode-ts` via `scripts/shims/`. Don't add new `cortex` names (`scripts/ci/no_cortex.sh`).

Commands:

```bash
cargo nextest run --workspace                      # or cargo test --workspace
scripts/ci/fetch_hoocode_fixtures.sh               # fixtures some tests need (no build)
cargo clippy --workspace --all-targets -- -D warnings
scripts/eval/subagent_evals.py --include-slow # subagent reliability evals (real binary + mock LLM)
python3 migration/check_dep_firewall.py
python3 scripts/tui/goldens.py check all           # screen goldens (tmux, ~6 min); reports in target/tui-goldens/
python3 scripts/tui/goldens.py update <scenario>   # accept a deliberate screen change
python3 scripts/tui/review_bundle.py               # after a golden change: review bundles in target/tui-review/
```

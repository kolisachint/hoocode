---
name: continue-migration
description: Report the status of the hoocode-ts → hoocode (Rust) migration from the frozen ledger and resume a task only when the user names it. Use when the user says "continue migration", "continue", "keep migrating", "next migration task", or asks for migration status.
---

# Continue the hoocode-ts → hoocode (Rust) migration

**Status (2026-10-09).** Migration is paused (user decision, plan §0.3): every remaining task is
`deferred`. `migration/ledger.py` is read-only since TUI plan T0.6. It cannot start, note, block,
move or verify a task. The hoocode-ts parity gate (Level 2) is retired. The done bar is Level 1
plus `python3 scripts/tui/goldens.py check all`. New work is tracked in the phase tables of
`docs/design/tui-activity.md` and in the core plan's cards, not in the ledger.

So by default this skill reports status. It does not advance a task on its own.

## 0. Orient (read-only, about a minute)

```bash
cd <hoocode checkout>
git status --short && git log --oneline -5
python3 migration/ledger.py status
python3 migration/ledger.py next          # the next ready task, read-only
python3 migration/ledger.py check         # ledger structure
sed -n '1,60p' migration/PROGRESS.md      # latest handoff notes
```

- `docs/design/ts-to-rust-migration.md` explains why (§0 audit, §5.5 crate split, §9 phases).
  `migration/ledger.json` is the what and status. If they disagree, the ledger wins.
- Port only from the pinned hoocode at `target/hoocode-pin`, built by
  `migration/tui-parity/setup_hoocode.sh` or fetched for fixtures with
  `scripts/ci/fetch_hoocode_fixtures.sh`. Never port from another checkout or from memory.
  **Never modify hoocode.**

## 1. Resume a task (only when the user names it)

1. Resume only a task the user names. Move its status back to `todo` in
   `migration/ledger.json` by hand (CLAUDE.md), add a note at the top of `migration/PROGRESS.md`,
   and say so in the commit. `ledger.py` will not do it.
2. Read the task's hoocode sources and TS tests in full, at the pin.
3. Check for reuse before writing code: the ecosystem crates in the plan (§3.4) and the crates
   already in this workspace (§3.4.2). Volatile third-party crates go only in the adapter crate
   listed in `migration/dep-firewall.json`.
4. Port the TS tests as Rust tests (Level 1). Keep wire formats byte-compatible with hoocode's
   JSON (sessions, RPC, json mode, settings, auth, models).
5. If the task needs a screen check, write a scenario in `scripts/tui/scenarios/` (format in its
   README). Write it from what hoocode actually renders, never from what hoocode happens to do.
   Accept it with `python3 scripts/tui/goldens.py update <scenario>` and review the diff.
6. Gate, in this order:
   - Level 1: `cargo fmt --all -- --check`, `cargo clippy -D warnings`, `cargo nextest run` for
     the touched crates, and `python3 migration/check_dep_firewall.py`.
   - Done bar for a screen change: `python3 scripts/tui/goldens.py check all`, then
     `python3 scripts/tui/review_bundle.py` for a visual review (advisory).
7. Commit with `migrate(<id>): <title>`. Include the ledger and PROGRESS.md changes. Push only
   when the user asks.

## Tools

- `migration/tools/fix_struct_fields.py`: compiler-driven fixer for struct-shape changes. It
  inserts missing fields, deletes removed ones, and unwraps `Some`/`None` for fields that are no
  longer optional. Run it repeatedly, then review the diff.
- Recording hoocode data (sessions, etc.): `migration/tui-parity/harness.py run <scenario> --app hoocode --keep`,
  then look in the kept temp HOME (path in `target/tui-parity/<scenario>/hoocode/tmpdir.txt`).
  `harness.py record` regenerates the replay fixtures (`migration/tui-parity/replay.json`).

## Rules

- **Don't edit statuses** in the ledger except to undo a bookkeeping mistake or to resume a task
  the user named. Say so in the log.
- **Don't weaken the gates** to go green: no loosening normalization to hide a real difference,
  no deleting snapshots or assertions, no `#[ignore]` on a failing ported test. A normalization
  rule is allowed only for real nondeterminism (random ids, temp paths, durations) or branding,
  with a comment.
- Stuck, or you need a decision (a design choice, a go/no-go)? Record it in PROGRESS.md and ask
  the user.
- `.github/workflows/*` can't be pushed from these sessions (the token has no `workflow` scope).
  Stage workflow changes in `migration/ci/` and tell the user to apply them.
- Never commit secrets. Live-provider tests read keys from the environment and are `#[ignore]`d.
- Moving the hoocode pin is a separate, user-approved task (plan §0.1). Never do it as a side effect.
- Before the session ends or context gets long: commit, and make sure PROGRESS.md says exactly
  where to resume.

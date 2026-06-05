---
name: bun-migration
description: |
  Drive the gradual npm -> bun migration one safe, reversible step at a time.
  Reads docs/bun-migration.md as the source of truth, runs the next phase,
  verifies with npm run check, and updates the plan.
  Usage: /bun-migration [status | next | phase <n>]
argument-hint: status | next | phase <n>
---
Carry out the npm -> bun migration. `docs/bun-migration.md` is the source of
truth for the plan, phases, coexistence rules, and recovery procedure. Read it
in full first, then act on the argument: **$ARGUMENTS** (empty means `next`).

## Non-negotiable rules (do not break these)

- npm stays the supported default until the user explicitly signs off on Phase 4.
  `package-lock.json` is authoritative.
- bun must use the **hoisted** linker (pinned in `bunfig.toml`). Never run a bun
  install with the isolated linker, and never pass `--linker=isolated`.
- After ANY install (npm or bun), run `npm run check`. It must be green before
  you continue.
- If the tree breaks at any point, recover with `npm install` then `npm run check`,
  and stop to report what broke.
- Do NOT set the `packageManager` field or remove `engines.npm` / `package-lock.json`
  before Phase 4, and only then with explicit user approval.
- Keep both `bun.lock` and `package-lock.json` committed and in sync.
- Make ONE phase of progress per invocation. Stop at each phase boundary and
  report; do not silently run several phases at once.
- Never commit unless the user asks (follow the repo's parallel-agent git rules:
  stage only your own files, no `git add -A/.`, no reset/checkout/clean/stash).

## Argument handling

- `status` (or `phase` with no number): report the current phase from the
  checkboxes in `docs/bun-migration.md`, what is done, and the exact next step.
  Make no changes.
- `next` or empty: execute the next unchecked phase below, verify, and update the
  doc. Stop after that one phase.
- `phase <n>`: execute that specific phase if its prerequisites (earlier phases)
  are already checked; otherwise report what is missing and stop.

## Phase procedures

Determine the current phase from the `## Phases` checkboxes in
`docs/bun-migration.md`, then perform the next one:

1. **Phase 1 - run scripts via bun (opt-in).**
   - Verify the hoisted linker is pinned in `bunfig.toml`; if not, add it.
   - Run `bun install` (hoisted), then `npm run check` to confirm the tree is
     still npm-compatible.
   - Try the bun equivalents and compare against the npm path:
     `bun run check`, and the package test runner under bun where applicable.
   - Record any differences (failures, warnings, behavior gaps) in the
     "Known issues / divergences" section of `docs/bun-migration.md`.
   - Do not change the default toolchain yet.

2. **Phase 2 - CI dual-track.**
   - Add a bun job to CI alongside the existing npm job, marked non-blocking
     (allowed to fail) until proven stable.
   - Keep the npm job as the gating one.

3. **Phase 3 - flip default.**
   - Update README/docs to present bun as the default workflow, keeping npm as a
     documented fallback. Do not delete npm support.

4. **Phase 4 - drop npm (REQUIRES explicit user sign-off).**
   - Stop and confirm with the user before doing anything in this phase.
   - Only after approval: remove `package-lock.json`, relax/remove `engines.npm`,
     set `packageManager` to the bun version, make the bun CI job gating.

## After the step

- Re-run `npm run check` (and the bun equivalent if relevant) and confirm green.
- Update the phase checkboxes and "Known issues" in `docs/bun-migration.md`.
- Report: which phase ran, what changed, verification results, and the exact next
  step. Do not commit unless the user asks.

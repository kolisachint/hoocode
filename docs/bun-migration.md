# npm -> bun migration

Status: **complete.** bun is the toolchain. `package-lock.json` has been removed,
`engines.npm` dropped, `packageManager` is set to `bun@1.3.13`, and CI runs under
bun (gating). `bun.lock` is now the authoritative lockfile. npm is no longer
required for development; it is still invoked only for `npm publish` during
releases.

Recover a broken tree with `bun install` (the hoisted linker is pinned in
`bunfig.toml`).

## Why this is incremental

`bun install` defaults to the **isolated** linker, which builds a symlink store
under `node_modules/.bun/`. That layout is not npm-compatible and breaks the
per-package nested deps the npm toolchain relies on (this is what corrupted the
tree before: root ended up with chalk 4 while `packages/{tui,ai,coding-agent}`
lost their nested chalk 5). So we migrate in phases and keep npm as the fallback.

## Coexistence mechanism

`bunfig.toml` pins the **hoisted** (npm-compatible) linker:

```toml
[install]
linker = "hoisted"
```

With this, plain `bun install` produces a flat `node_modules` that the npm
toolchain can also use. Verified: after `bun install`, `npm run check` passes,
nested chalk 5.6.2 is preserved in `tui`/`ai`/`coding-agent`, and the workspace
symlinks (`@kolisachint/hoocode-*`) stay intact.

## Recovery (source of truth = bun.lock)

If the tree gets corrupted, restore it with bun:

```bash
bun install   # rebuilds node_modules from bun.lock (hoisted linker)
bun run check # confirm green
```

`bun.lock` is now the authoritative lockfile. `package-lock.json` has been
removed.

## Rules (post-migration)

- Never run `bun install` with the isolated linker. The hoisted linker is pinned
  in `bunfig.toml`, so plain `bun install` is safe; do not pass `--linker=isolated`.
- After any `bun install`, run `bun run check`.
- `packageManager` is set to `bun@1.3.13`; keep it and `bunfig.toml` in sync when
  bumping the bun version.
- `bun.lock` stays committed. CI uses `bun install --frozen-lockfile`, so commit
  an updated `bun.lock` whenever dependencies change.
- npm is only used for `npm publish` during releases; do not reintroduce
  `npm ci`/`package-lock.json` to the dev or CI install path.

## Known issues / divergences

- bun warns: `Bun currently does not support nested "overrides"` for the `gaxios`
  override in root `package.json`. Warning only; the override still applies under
  npm. Track whether bun honors it once it gains nested-override support.
- The root `package.json` depends on `@kolisachint/hoocode-agent: ^0.2.0`, which
  both npm and bun install from the registry (currently 0.2.x) as a real
  directory. This is identical under both managers, not a bun-specific issue.
- `bun run <script>` rewrites nested `npm run <x>` invocations inside a script to
  `bun run <x>` (observed with `check` -> `check:browser-smoke`). Behavior matches
  the npm path here; no action needed, but note scripts that hardcode `npm` will
  execute under bun when run via `bun run`.
- Package test runners are vitest (`ai`, `agent`, `coding-agent`) and `node --test`
  (`tui`), not bun's native `bun test`. Tests pass against the bun-installed
  (hoisted) tree via the existing runners. Do not switch packages to `bun test`
  as part of this migration; keep the established runners.

## Phase 1 verification log

Run on bun 1.3.13, npm 10.9.8 (node v24):

- `bun install` (hoisted): success, only the known gaxios nested-override warning.
- `npm run check`: green after `bun install` (tree stays npm-compatible).
- `bun run check`: green (biome + tsgo + browser-smoke).
- `node --test` (tui) and vitest (ai) sample tests: pass against the bun tree.
- `bun.lock` unchanged by the install -> still in sync with `package-lock.json`.
- Default toolchain unchanged; npm remains authoritative.

## Phases

- [x] **Phase 0 - coexistence.** Pin hoisted linker; verify `bun install` yields
  an npm-compatible tree that passes `npm run check`. (done)
- [x] **Phase 1 - run scripts via bun (opt-in).** Try `bun run check`, `bun test`,
  and `bun` for dev. Compare results against the npm path; record differences.
  (done - see "Phase 1 verification log" and "Known issues / divergences")
- [x] **Phase 2 - CI dual-track.** Run the pipeline under both npm and bun; treat
  bun as non-blocking until stable. (done - see "Phase 2 CI dual-track" below)
- [x] **Phase 3 - flip default.** Make bun the documented default in README/docs;
  keep npm as a working fallback. (done - README "Development" and
  CONTRIBUTING "Before Submitting a PR" now lead with bun, npm documented as
  fallback; npm support unchanged)
- [x] **Phase 4 - drop npm (done on user sign-off).** Removed `package-lock.json`;
  dropped `engines.npm`; set `packageManager` to `bun@1.3.13`; made the bun CI
  jobs gating and removed the npm CI jobs; switched the release workflows to
  `bun install --frozen-lockfile`, keeping npm only for `npm publish`.

## Phase 2 CI dual-track

`.github/workflows/ci.yml` now runs both toolchains:

- npm jobs (`check`, `test`, `build`) stay gating, unchanged.
- Mirrored bun jobs (`bun-check`, `bun-test`, `bun-build`) use
  `oven-sh/setup-bun@v2` (bun 1.3.13), `bun install --frozen-lockfile`, and the
  `bun run` equivalents. Each is marked `continue-on-error: true` so it is
  non-blocking until proven stable.
- The bun jobs rely on `bun.lock` being in sync with `package-lock.json`;
  `--frozen-lockfile` fails fast if it drifts. Verified locally that
  `bun install --frozen-lockfile` installs cleanly (only the known gaxios
  nested-override warning).

Phase 4 update: the bun jobs are now gating (`continue-on-error` removed) and the
npm CI jobs were deleted. Release workflows (`release.yml`, `merge-release.yml`)
install via `bun install --frozen-lockfile` and keep npm only for `npm publish`.

## Driving the migration

Run `/bun-migration` (defined in `.hoocode/commands/bun-migration.md`) to execute
the next phase safely:

- `/bun-migration status` - report the current phase and next step, no changes.
- `/bun-migration next` (or no argument) - execute the next unchecked phase,
  verify with `npm run check`, and update this file. One phase per run.
- `/bun-migration phase <n>` - run a specific phase if its prerequisites are met.

Phase 4 (dropping npm) always stops for explicit user sign-off.

## Continuing between sessions

1. Read this file and check the unchecked phase boxes above.
2. Make one small, reversible step toward the next phase.
3. Verify with `npm run check` (and, when relevant, the bun equivalent).
4. Update the checkboxes and "Known issues" here. Do not advance phases past what
   the user has approved.

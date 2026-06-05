# npm -> bun migration

Status: **in progress, dual-track.** npm is the supported default; bun is being
adopted gradually. Both must keep working until bun is proven out. Do not remove
npm support until the user signs off.

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

## Recovery (source of truth = package-lock.json)

If any tree gets corrupted, restore it with npm:

```bash
npm install   # rebuilds node_modules from package-lock.json
npm run check # confirm green
```

`package-lock.json` is authoritative during the migration. Keep both
`bun.lock` and `package-lock.json` committed and roughly in sync.

## Rules during migration

- Never run `bun install` with the isolated linker. The hoisted linker is pinned
  in `bunfig.toml`, so plain `bun install` is safe; do not pass `--linker=isolated`.
- After any `bun install`, run `npm run check`. If it fails, run `npm install` to
  recover, then investigate.
- Do not set the `packageManager` field yet (would force one manager via corepack).
- Keep the `engines.npm` constraint until npm is dropped.
- Both lockfiles stay committed. If they drift, regenerate the stale one and
  re-run `npm run check`.

## Known issues / divergences

- bun warns: `Bun currently does not support nested "overrides"` for the `gaxios`
  override in root `package.json`. Warning only; the override still applies under
  npm. Track whether bun honors it once it gains nested-override support.
- The root `package.json` depends on `@kolisachint/hoocode-agent: ^0.2.0`, which
  both npm and bun install from the registry (currently 0.2.x) as a real
  directory. This is identical under both managers, not a bun-specific issue.

## Phases

- [x] **Phase 0 - coexistence.** Pin hoisted linker; verify `bun install` yields
  an npm-compatible tree that passes `npm run check`. (done)
- [ ] **Phase 1 - run scripts via bun (opt-in).** Try `bun run check`, `bun test`,
  and `bun` for dev. Compare results against the npm path; record differences.
- [ ] **Phase 2 - CI dual-track.** Run the pipeline under both npm and bun; treat
  bun as non-blocking until stable.
- [ ] **Phase 3 - flip default.** Make bun the documented default in README/docs;
  keep npm as a working fallback.
- [ ] **Phase 4 - drop npm (only on user sign-off).** Remove `package-lock.json`,
  relax/remove `engines.npm`, set `packageManager` to bun.

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

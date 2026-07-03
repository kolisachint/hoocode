# Packages, Install, and Build

How the workspace installs, builds, and resolves cross-package imports. This is the doc to
read before running install/build/typecheck so you don't hit the traps below.

## Install

- Tooling is **bun** based: `packageManager` is `bun@1.3.13`, `engines` requires
  `node >=20`, and `bun.lock` is the single committed lockfile (`package-lock.json`
  has been removed). See [bun-migration.md](./bun-migration.md) for the completed
  npm -> bun migration.
- `bunfig.toml` pins the npm-compatible **hoisted** linker, so plain `bun install`
  produces a flat `node_modules` the toolchain relies on. npm is invoked only for
  `npm publish` during releases.

Install with:

```bash
bun install                    # rebuilds node_modules from bun.lock (hoisted linker)
bun install --frozen-lockfile  # CI-exact; fails fast if bun.lock drifts
```

### Never use the isolated linker

Do not run `bun install --linker=isolated`. The isolated linker builds a symlink store
under `node_modules/.bun/` that the workspace's relative `@kolisachint/*` symlinks and the
nested-dep layout cannot use, which breaks every cross-package import (tests, typecheck, and
the CLI). The hoisted linker is pinned in `bunfig.toml`, so a plain `bun install` is safe;
only an explicit `--linker=isolated` reintroduces the breakage. Recover a corrupted tree
with `bun install` (hoisted) then `bun run check`.

## Build

Each package builds with `tsgo -p tsconfig.build.json` into its own `dist/`. `dist/` is
**gitignored** and must never be committed (artifacts cause merge conflicts, bloat, and
silent staleness).

Build order is leaves-first because consumers resolve to a dependency's built `dist/`:

```
tui -> ai -> agent -> coding-agent
```

Commands:

```bash
bun run build                              # builds all four in the correct order
cd packages/agent && npx tsgo -p tsconfig.build.json   # build a single package
```

(`coding-agent`'s build also chmods the CLI bins and copies assets.)

## The src-vs-dist resolution split (read this)

There are two different ways imports resolve, and they behave differently with respect to
stale `dist/`:

1. **Root typecheck (`bun run check` at repo root).** The root `tsconfig.json` maps every
   `@kolisachint/*` import to that package's `src/` via `paths`. So a root typecheck reads
   **source**, not `dist`, and is immune to stale builds. This is the canonical way to
   typecheck the whole repo.

2. **Per-package build/typecheck (`tsgo -p packages/X/tsconfig.build.json`).** These extend
   `tsconfig.base.json`, which has **no** `paths`. Cross-package imports therefore resolve
   through `node_modules` to the dependency's **`dist/`**. If that `dist/` is stale or
   missing, you get spurious "has no exported member" / "property does not exist" errors.

3. **Runtime (the actual CLI and any test importing built output).** Also uses `dist/`.

Implications:

- To verify types, run `bun run check` (or `tsgo --noEmit`) **from the repo root**. Do not
  rely on a single package's `tsconfig.build.json` for cross-package typechecking - it will
  report stale-`dist` errors that the root check does not.
- To run the CLI or anything that imports built output, the dependency `dist/` must be
  current. After pulling, or after editing a dependency's `src/`, rebuild that dependency
  (and its dependents) before running.

### Trap: stale dist

Symptom: per-package typecheck or the running CLI reports missing exports/types that
clearly exist in `src/`. Cause: the dependency's `dist/` predates the source change. Fix:
rebuild the dependency in dependency order (or `bun run build`). The root `bun run check`
will not show this because it reads `src`.

## Generated files - regenerate, don't hand-edit

- Models: `cd packages/ai && bun run` the generate step (driven by
  `scripts/generate-models.ts`). Never edit `src/models.generated.ts` directly.
- Embedded agent/init templates: after editing `packages/coding-agent/templates/agents/*.md`
  (or other templates), regenerate the embedded copy:

  ```bash
  cd packages/coding-agent && node scripts/embed-templates.mjs
  ```

  This rewrites `src/init-templates.generated.ts`, which is what `agent-registry.ts` loads
  for built-in agents. Editing the template `.md` without regenerating has no effect.

## Tests

Tests run with vitest, **from the package root**:

```bash
cd packages/<pkg>
npx tsx ../../node_modules/vitest/dist/cli.js --run test/<file>.test.ts
```

Do not run the repo-wide `bun run test`. For `packages/coding-agent/test/suite/`, use the
faux provider and the suite harness - never real provider APIs or keys.

## Checks before committing

After code changes, run from the repo root:

```bash
bun run check     # biome (lint/format) + tsgo typecheck (src-based) + browser smoke
```

Fix all errors and warnings. `bun run check` does not run tests; run affected test files
separately.

## Quick reference

| Goal | Command (cwd) |
| --- | --- |
| Install | `bun install` (root) |
| Typecheck everything | `bun run check` (root) |
| Build everything | `bun run build` (root) |
| Build one package | `npx tsgo -p tsconfig.build.json` (package) |
| Regenerate embedded templates | `node scripts/embed-templates.mjs` (coding-agent) |
| Run a test file | `npx tsx ../../node_modules/vitest/dist/cli.js --run test/x.test.ts` (package) |

## Follow-up: kill the stale-dist class of bug

The repo already removes stale-dist pain for **typechecking** via root `paths`-to-`src`. It
does not solve it for **builds/runtime**: the root `build` script is a fixed sequential
chain with no incrementality, and nothing guarantees a dependent is rebuilt after its
dependency changes. A future improvement is TypeScript **project references**
(`composite: true` + `references` in each `tsconfig.build.json`, built with `tsgo -b`), so a
build of `coding-agent` automatically rebuilds `agent`/`ai`/`tui` first and only what
changed. Tracked as a follow-up, not yet implemented.

### Opt-in src resolution (prototyped on tui)

`packages/tui/package.json` carries a conditional `exports` map as a prototype:

```json
"exports": {
  ".": {
    "hoocode-dev": "./src/index.ts",
    "types": "./dist/index.d.ts",
    "default": "./dist/index.js"
  }
}
```

This is additive and inert by default: normal resolution (and the `node`-run built CLI in
`bin/hoocode.js`) still hits `dist`. Only a runner that opts into the `hoocode-dev`
condition resolves to `src`. Verified: `import.meta.resolve` returns `dist/index.js` by
default and `src/index.ts` with `--conditions=hoocode-dev`.

Note: pure src-as-`main` (the Turborepo "just-in-time" form) is **not** viable here because
`bin/hoocode.js` runs the built `dist` with plain `node`, which cannot import `.ts`. The
conditional form above avoids that.

To actually make dev/test read `src` (killing runtime stale-dist), wire the condition into
the runners and leave production untouched:

- vitest: `resolve.conditions: ["hoocode-dev", ...]`
- tsx / dev scripts: `--conditions=hoocode-dev`
- `bin/hoocode.js` / published runtime: leave as-is (uses `default` -> `dist`)

This is currently prototyped on `tui` only and the runner wiring is intentionally not done
yet (typecheck is already src-based via root `paths`, so the remaining benefit is dev/test
runtime). Roll out to `ai`, `agent`, and `coding-agent` only after the runner wiring is
decided.

# Changelog

## [0.2.5] - 2026-05-15

### Fixed

- Fixed `init.ts` seeding the global extensions directory at the wrong path. It created `~/.hoocode/extensions/` while the extension loader (`core/extensions/loader.ts` and `core/resource-loader.ts`) reads from `agentDir/extensions` = `~/.hoocode/agent/extensions/`. As a result, extensions placed in the freshly seeded directory were silently invisible. `initConfig()` now creates `~/.hoocode/agent/extensions/` directly, and the redundant `mkdir("agent")` call was dropped since the recursive mkdir on `agent/extensions` already covers it.

## [0.2.4] - 2026-05-15

### Fixed

- Fixed Windows standalone `.exe` (winget) and zip binary still failing to seed default `modes/` and `profiles/` on first run. The Bun-compiled entry point (`src/bun/cli.ts`) never invoked `initConfig()` — only the Node wrapper `bin/hoocode.js` did — so `~/.hoocode/{modes,profiles}` stayed empty for users installing via zip or winget. Added the `initConfig()` call to the Bun entry, and embedded the seed templates into the compiled binary itself (new `scripts/embed-templates.mjs` generates `src/init-templates.generated.ts` at build time, and `init.ts` now writes from these constants instead of reading the on-disk `templates/` folder). This also lets the standalone `.exe` self-seed without a sibling `templates/` directory.
- Dropped the redundant `cp -r templates …` step from `scripts/build-binaries.sh` now that seed content ships inside the binary; this slims every release archive.

## [0.2.3] - 2026-05-15

### Fixed

- Fixed `ERR_UNSUPPORTED_ESM_URL_SCHEME` when running `hoocode` / `hoo` on Windows after `bun add -g`. The bin shim passed raw `path.join(...)` results to dynamic `import()`; Node's ESM loader requires a `file://` URL specifier on Windows. Each `import()` argument is now wrapped with `pathToFileURL().href`.
- Fixed Windows binary missing default `modes/` and `profiles/` after extracting the release zip. Two causes: `init.ts` resolved templates from `__dirname`, which doesn't map to a real disk path inside Bun-compiled binaries; and `scripts/build-binaries.sh` never copied `templates/` next to the executable. Added `getTemplatesDir()` (mirrors `getThemesDir()` etc.), routed `init.ts` through it, replaced silent `try/catch` with a visible warning, and updated the build script to ship `templates/` in every platform archive.

## [0.2.2] - 2026-05-13

### Added

- Added external mode/profile search paths so modes and profiles can ship outside `~/.hoocode/`. Three sources feed the lookup: `HooConfig.mode_paths`/`profile_paths` (in `~/.hoocode/agent/hoo-config.json` or `./.hoocode/config.json`), repeatable `--mode-path <dir>` / `--profile-path <dir>` CLI flags, and new `pi.addModeSearchPath` / `pi.addProfileSearchPath` extension API methods (with matching getters). Resolution order: `./.hoocode/{modes,profiles}/{name}/...` → `~/.hoocode/{modes,profiles}/{name}/...` → external dirs in declared order → built-in defaults.

### Fixed

- Fixed `ERR_SUPPORTED_ESM_URL_SCHEME` error on Windows with Bun by normalizing file URLs to file paths in path resolution functions.

## [0.1.4] - 2026-05-11

### Added

- Added NVIDIA provider support with `NVIDIA_API_KEY` authentication and default model `meta/llama-3.3-70b-instruct`.
- Added mode-based tool filtering via `enabled_tools` configuration option (mode takes priority over profile).
- Added write path restrictions via `allowed_write_paths` configuration option for granular file access control.

### Fixed

- Fixed branding: changed default share viewer URL from `pi.dev` to `hoocode.dev`.
- Fixed branding: changed APP_TITLE from lowercase `hoocode` to `HooCode`.
- Fixed changelog showing old entries on every login by resetting changelog for new project.

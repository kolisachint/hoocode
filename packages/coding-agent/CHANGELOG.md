# Changelog

## [0.4.3] - 2026-05-29

### Changed

- Active subagent tasks are now rendered in a dedicated task panel just above the editor prompt instead of the footer. The panel shows only running tasks (pending / in_progress), uses LIFO ordering (newest closest to the prompt), and collapses to zero lines when idle.

### Added

- Clearer guidance on when the model should invoke the `subagent` tool (self-contained work, parallel investigation, discrete tasks, long-running commands).

## [0.4.2] - 2026-05-29

### Added

- Optional `subagent` tool: delegates a self-contained task to a fresh, isolated agent loop (clean minimal prompt, no parent conversation history) and returns only the subagent's final answer. Modes: `explore`, `edit`, `test`, `fix`, `review`. Opt in with the `--subagent` flag or the `enableSubagent` setting. Active subagent tasks are tracked in a task store and shown in the TUI footer. Replaces the former `examples/extensions/subagent` extension.

## [0.3.1] - 2026-05-29

### Added

- `ExtensionFactory` functions can now declare an optional `displayName` (e.g. `mode/build`). When present, the TUI and CLI show that name instead of the synthetic `<inline:N>` path in loaded resources and error messages.

## [0.3.0] - 2026-05-29

### Fixed

- Moved `AGENTS.md` / `CLAUDE.md` size warnings from `console.error` into the TUI startup screen so they are visible inside the app instead of being buried in shell stderr.
- Suppressed empty "What's New" sections on startup when the changelog has no new entries or only whitespace.
- Collapsed loaded resources into a single `[Resources]` line when the total count of context files, skills, prompts, extensions, and themes is 5 or fewer, saving vertical real-estate on sparse projects.

### Changed

- `ResourceLoader.getAgentsFiles()` now returns `{ agentsFiles, warnings }` where `warnings` contains any size/truncation notices. Implementations of `ResourceLoader` must update their return shape. `DefaultResourceLoaderOptions.agentsFilesOverride` and `loadProjectContextFiles()` have matching signature changes.

## [0.2.7] - 2026-05-29

## [0.2.6] - 2026-05-29

### Fixed

- Fixed startup changelog occasionally re-displaying entries the user had already seen. `getChangelogForDisplay()` recorded the app `VERSION` as the last-seen version even when that version overshot the latest entry actually in `CHANGELOG.md` (e.g. shipping `0.2.6` with an empty `[Unreleased]`). When entries for that version were appended later, they were filtered out as already-seen. Both code paths (fresh install and subsequent runs) now record the latest changelog entry's version instead, falling back to `VERSION` only if the changelog is empty.
- Stopped injecting the 7-line HooCode documentation block into the system prompt on every project. It is now gated on the cwd being inside the hoocode source repo (detected by walking up for a `package.json` with `name: "hoocode-monorepo"`). Saves ~150 tokens per turn on every unrelated project, across every provider.
- Added soft warning at 8 KB and hard truncation at 40 KB for context files (`AGENTS.md` / `CLAUDE.md`). Previously a pasted spec would silently bloat every request for the lifetime of the project. Truncation appends a marker so the agent knows content was elided.

### Removed

- Removed the `agent` mode (template, default auto-allow entry, `/mode agent` command target, and `KNOWN_MODES` registration). It overlapped with `build` in intent; users wanting more autonomy should add tools to `modes.build.auto_allow` in `~/.hoocode/config.json` or `.hoocode/config.json` instead. Existing configs that pin `active_mode: "agent"` will still run, but with no mode-prompt layer — switch to `/mode build` to restore a guided prompt.
- Deleted the stray `packages/ai/bedrock-provider.js` and `packages/ai/bedrock-provider.d.ts` root-level shims. They were one-line re-exports of `./dist/bedrock-provider.js` accidentally committed in the first release. The `package.json` `exports."./bedrock-provider"` already points directly at `dist/`, and the `files` field only ships `dist/` — so the shims were never published or imported.
- Slimmed the release pipeline to npm publish + a single Windows standalone zip (`hoocode-windows-x64.zip`). Dropped the macOS/Linux `tar.gz` binaries, the winget standalone `.exe`/manifest, the duplicate `build-binaries.yml` workflow, and the `submit-winget.yml` workflow. Install on macOS/Linux via `npm i -g @kolisachint/hoocode-agent`.

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

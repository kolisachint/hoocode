# Changelog

## [Unreleased]

## [0.4.47] - 2026-06-11

## [0.4.46] - 2026-06-11

## [0.4.45] - 2026-06-11

### Fixed

- Subagent pool now correctly marks tasks as failed when a child process writes a
  valid `result.json` with `status: "failed"` and exits non-zero. Previously the
  pool treated a well-formed but failed result as a clean completion, causing
  `result.ok` to be `true` and suppressing the concrete failure reason.

### Added

- The task panel now has three views over the same task list, cycled with
  shift+ctrl+t (`app.tasks.cycleView`) and shown as a `tasks · subagents · teams`
  switcher in the ledger header: flat (unchanged), subagents (tasks grouped by
  owning agent: ◆ main orchestrator + ⊕ workers), and teams (grouped by named
  role-agent: ▸, with lifecycle `[state]` tags and handoff arrows). Group headers
  carry each agent's own token/cost totals and done/total count; grouped rows sit
  on a faint indent guide and drop their per-row origin tag. Subagent dispatches
  register themselves in the new `TaskAgent` roster on the task store
  (`upsertAgent`/`patchAgent`/`addAgentStats`), which external orchestrators
  (e.g. hooteams) can also feed to drive the teams view.

## [0.4.44] - 2026-06-09

## [0.4.43] - 2026-06-05

### Fixed

- Built-in subagents now fall back to the parent's inherited model on a
  credits/auth/quota failure even when no provider was threaded through the
  dispatch. Previously the inherited-model retry required both a model and a
  provider from the parent; when the harness routed through a gateway without an
  explicit provider, a preferred-model failure (e.g. `CreditsError: Insufficient
  balance`) was reported as a hard failure instead of retrying on the inherited
  model. The retry now requires only the parent model and lets the child resolve
  the provider from its own default.

## [0.4.42] - 2026-06-05

### Fixed

- MCP tools failed with `MCP server "<name>" is not connected` once the server's
  connection was torn down (server process exit, host process churn between turns,
  or a racing teardown). The tool call gave up immediately with no recovery, and
  the server config was not retained so it could not reconnect. The loader now
  retains each server's config and a tool call lazily reconnects from it before
  failing. The client also now sends the spec-required `notifications/initialized`
  after `initialize`, the handshake (`initialize`/`tools/list`) is bounded by a
  timeout so a dead server can't hang startup, and spawned MCP servers are killed
  when the host process exits so they no longer linger as orphans.
- Quick-finishing subagents were reported as "stalled". A spawned subagent
  (`--mode json` with a task id) finished its work and wrote a valid
  `result.json`, but the child process did not exit on its own: lingering open
  handles in its runtime kept the event loop alive, so it sat idle until the
  parent lifeguard SIGKILLed it at the 60s heartbeat threshold (`exit_code: null`,
  `status: "stalled"`). Spawned subagents now force a clean, flushed exit as soon
  as their work is done, so they terminate in seconds instead of hanging until the
  reap. As a defense in depth, the pool now treats a child as successfully
  completed whenever it produced a verified `result.json`, regardless of exit
  code, so a kill that races a finished child still returns the real answer.

## [0.4.41] - 2026-06-05

### Added

- Slash commands are now discovered from `.agents/commands/` (project
  ancestor-walk up to the git root, plus user-level `~/.agents/commands/`), so
  commands written under the cross-vendor `.agents/` tree round-trip. Precedence
  is first-match-wins: project `.hoocode` > project `.claude` > project
  `.agents` (cwd-first) > user `.hoocode` > user `.agents` > user `.claude`.

### Fixed

- Background subagents that finished cleanly were sometimes reported as "stalled".
  A late lifeguard heartbeat-miss could fire SIGKILL just as a healthy child was
  already exiting; the kill was a no-op, the child still exited 0 and wrote a valid
  result.json, but the pool honored the stale stall verdict and discarded the real
  success. The exit handler now only honors a stalled/timeout kill when the child
  did not actually complete (non-zero exit or no verified result.json), so a
  genuine completion always wins over a racing stall verdict.

## [0.4.40] - 2026-06-05

## [0.4.39] - 2026-06-05

## [0.4.38] - 2026-06-05

### Fixed

- Subagents no longer get killed under heavy load. When many subagents (plus
  background MCP tools) run at once, CPU contention starved the parent event loop
  so it missed a healthy child's heartbeat and SIGKILLed it as "stalled" (or hit a
  wall-clock timeout that inflated faster than real work). The lifeguard now scales
  the heartbeat-miss and hard-timeout budgets by the number of concurrent
  subagents and forgives measured event-loop lag, so contention alone no longer
  reaps a working subagent. A genuinely stuck agent is still reaped at a hard
  ceiling.

### Changed

- Background tools now explain themselves verbosely and consistently in chat. Both
  background subagents and background MCP tools get the same shape: a verbose
  "started" line that names the subagent/MCP tool and summarizes its arguments, and
  a matching "finished/failed" follow-up message using the same label. Subagent and
  MCP background work already appear in the task pane (pending → in_progress →
  done/failed); the chat side is now in sync with it.
- MCP tool calls now render a clean, prefixed title in chat — `MCP [server › tool]
  <args>` — parallel to the subagent `Task [type] <desc>` line, instead of falling
  back to the raw `mcp_<server>_<tool>` tool name. The args summary reuses the same
  helper as the background start/finish messages so the title stays in sync with them.
- The task pane now shows a single-cell source glyph before the id — ⚙ for subagent
  rows, ⧉ for MCP rows (plain tasks reserve a blank cell) — so the two kinds of
  background work are distinguishable at a glance. The pane stays tag-free: the
  subagent *mode* tag (e.g. "[explore]") is still not shown.
- Task-pane ids are padded to a uniform column width, so titles stay aligned across
  rows regardless of id digit count (#1 vs #10 vs #100) instead of jagging right.

## [0.4.37] - 2026-06-05

## [0.4.36] - 2026-06-05

### Added

- MCP tools now run in background mode by default (non-blocking), since MCP servers are external processes that may have high latency. Configure per-server with `"background": false` in mcp.json.

### Fixed

- Slash commands in `.hoocode/commands/` now take precedence over prompt templates in `.hoocode/prompts/` when there is a name collision, instead of the reverse.

## [0.4.35] - 2026-06-05

## [0.4.34] - 2026-06-05

### Changed

- Built-in `explore`, `review`, `test`, and `doc` subagents now run in the background by default (their templates set `background: true`), so delegating to them no longer blocks the parent turn — it keeps reasoning while the subagent runs and the answer arrives as a follow-up message.

### Fixed

- Task pane no longer truncates task titles early or unevenly: rows now use the full available width (the previous budget subtracted the row prefix, so titles clipped ~5-7 columns short and differently per task-id width).
- Subagent fallbacks and exhaustion skips now surface as a compact ⚠ cue in the task pane (e.g. "ran on inherited model", "<provider> exhausted") instead of relying on a chat message.

## [0.4.33] - 2026-06-04

### Fixed

- Built-in subagents now retry with the inherited parent model when their preferred model fails due to model availability, auth, quota, or rate-limit errors.

## [0.4.32] - 2026-06-04

### Changed

- Background subagents (`Task` with an agent whose definition sets `background: true`)
  now run via the agent loop's non-blocking tool mechanism. The parent keeps reasoning
  while the subagent runs and its final answer is injected automatically as a follow-up
  message when it finishes — no `TaskOutput` polling required. The injected answer is a
  dedicated `backgroundTask` custom message (distinct styling) rather than a plain user
  message.

## [0.4.31] - 2026-06-03

## [0.4.30] - 2026-06-03

## [0.4.29] - 2026-06-03

## [0.4.28] - 2026-06-03

## [0.4.27] - 2026-06-03

## [0.4.25] - 2026-06-03

### Added

- Options pane (`ask_options`) options can now be flagged `recommended: true`, which renders a green `(recommended)` marker next to the option label to guide the user's choice.
- File-based slash commands: reusable command Markdown invoked with `/name`, auto-discovered from `~/.hoocode/commands/*.md` (global) and `.hoocode/commands/*.md` (project), plus the `slashCommands` settings array and the repeatable `--slash-command <path>` flag. A `type` frontmatter field controls how the command is injected: `user` (default, sent as a user message), `system` (appended to the system prompt), or `context` (added as a hidden context message). Disable discovery with `--no-slash-commands` (`-nsc`).
- Claude Code slash commands are now imported natively (D7) from `.claude/commands/*.md` (project) and `~/.claude/commands/*.md` (user), at lower precedence than `.hoocode/commands/`. Discovery respects `--no-slash-commands`.

### Changed

- Renamed the `--subagent` CLI flag to `--enable-subagents` for readability (the `enableSubagent` setting and `/subagent` command are unchanged).
- Release workflow no longer publishes binaries; releases now ship only GitHub's auto-generated source code archives.

### Fixed

- Removed the leading blank line inside the edit tool's output box (call render box paddingY changed from 1 to 0).

## [0.4.24] - 2026-06-03

### Added

- Added the `general-purpose` built-in subagent (the default dispatch target) with an explicit `read, bash, edit, write, grep, find, ls` tool allowlist so it can investigate and act end to end.
- Subagent dispatch now skips spawning when the inherited provider is flagged exhausted. When the main session's own turn fails with a persistent usage/quota/rate-limit error, the provider is recorded as exhausted for a short window (cleared on the next successful response, self-expiring after a TTL); the Task tool then returns a clear "provider appears exhausted" message and records a failed task instead of burning another doomed spawn (subagents inherit the parent's provider).

### Fixed

- Subagent failures now surface the real cause. A non-zero-exit subagent's `result.json` summary (which carries the provider/model error, e.g. "usage limit reached") is attached to the pool result, and the failed run's summary now embeds the provider error message, so the Task tool reports the concrete reason instead of a generic "subagent failed".

## [0.4.23] - 2026-06-02

### Added

- Added an options pane: the agent can call the new `ask_options` tool to ask you one or more decisions inline in the transcript. Move between options with up/down, confirm and advance to the next question with right (left to go back), quick-pick with number keys, or drop onto a custom row to type your own answer when none fit; esc skips. Answered steps stay on screen as a breadcrumb. Added `tui.select.next` (right) and `tui.select.back` (left) keybindings and an `askOptions()` method on the extension UI context.

### Changed

- Redesigned the interactive task panel to be more legible and informative. Added a state-colored left rail (working/reviewed/stopped), a deterministic block-glyph progress bar with a done/total count, total elapsed time in the header, an animated braille spinner on the active task, and `queued`/`running…` tags on unsettled rows.

## [0.4.21] - 2026-06-01

### Fixed

- Removed leading space from startup warning lines (`showWarning` paddingX changed from 1 to 0).

## [0.4.20] - 2026-06-01

## [0.4.19] - 2026-06-01

### Added

- Startup banner now renders the Hoo owl symbol as colored half-block ANSI art above the ASCII wordmark on truecolor terminals (wide layout only). Non-truecolor or narrow terminals keep the existing wordmark/app-name banner. The art is generated from `assets/symbol.svg` via `scripts/generate-wordmark-symbol.ts` into `src/core/wordmark-symbol.generated.ts`.

## [0.4.18] - 2026-06-01

### Changed

- Subagent dispatch dirs are now deleted on a clean, verified success. When a subagent exits 0 and its `result.json` passes verification, `SubagentPool` removes `.hoocode/dispatch/<task_id>/` entirely (session.jsonl, result.json, dispatch-log.json, budget.json); the in-memory result still carries `result_data`, so callers lose nothing. Dirs for failed/partial/stalled/timeout tasks are retained for debugging. Trade-off: `resume_task_id` now only works for non-successful tasks, since a completed task's persisted session is discarded.

## [0.4.17] - 2026-06-01

### Changed

- Consolidated subagent prompts to a single source of truth. The duplicate `templates/subagent/**` prompt set (and the generated `EMBEDDED_SUBAGENT_PROMPTS` map) is removed; `templates/agents/**` (the frontmatter registry) is now the only prompt source. The built-in agent set is the canonical five: `explore`, `edit`, `test`, `review`, `doc`. The unreachable `fix` mode (never exposed by the Task tool, `/subagent`, or routing) was dropped.
- Trimmed `DispatchEvaluator` to its only live responsibilities: the nested-delegation depth guard and a complexity estimate for the dispatch log. Delegation is fully description-driven (the parent agent chooses the agent), so the dead keyword-routing/auto-split surface was removed. Updated `docs/routing.md` to match.
- Moved built-in subagent tool allowlists into agent frontmatter (`tools:` in `templates/agents/*.md`), making the agent registry the single source of truth for each agent's prompt, tools, and model. The hardcoded `SubagentMode` enum and `MODE_TOOLS` map are gone; `SubagentPool` reads the allowlist solely from the resolved definition.

### Removed

- Removed unused exports: `isSubagentRecommended` (tools/subagent), `SubagentPool.dispatchBatch`, and the `DispatchEvaluator` routing helpers (`classifyWithConfidence`, `shouldSplit`, `canHandleInline`, `getReason`).
- Removed the `core/subagent.ts` module (`SubagentMode`, `SUBAGENT_MODES`, `MODE_TOOLS`); its role is now served by the frontmatter agent registry.

## [0.4.16] - 2026-06-01

### Fixed

- Fixed the available-agents list shown to the model (in the Task tool description and the main-session subagent prompt) collapsing to a repeated, useless header line. Every built-in agent description opens with the same `Use this subagent ONLY when:` line, and only that first line was surfaced. The list now condenses each description to a meaningful one-line summary (first "when to use" bullets, or first prose line), so agents are distinguishable.
- Fixed the footer and startup Resources list not reflecting an enabled subagent. The wiring checked for a tool named `subagent`, but the model-facing tool was renamed to `Task`, so `--subagent` (or the `enableSubagent` setting) never showed `mode + subagent` in the footer nor `subagent_system_prompt` under Resources.

## [0.4.15] - 2026-05-31

### Added

- Data-driven subagents: agents are now defined by frontmatter `.md` files (`name`, `description`, optional `tools`/`model`/`maxTurns`) loaded from a registry with precedence project > user > built-in. For drop-in Claude Code compatibility, `.claude/agents/` (project) and `~/.claude/agents/` (user) are also discovered, and Claude tool names are normalized to hoocode's tools (`Read`→`read`, `Glob`→`find`, `LS`→`ls`, etc.; unsupported tools are dropped with a diagnostic).
- Every spawned subagent now runs under a hard turn cap (`--max-turns`, default 50 when a definition sets no `maxTurns`). Near the cap the agent is asked to wrap up and return its findings; if it reaches the cap it is stopped and its partial findings are returned as a `partial` result instead of a failure.
- Background (non-blocking) subagents: an agent definition can set `background: true`. The `Task` tool then dispatches it detached and returns a `task_id` immediately instead of blocking. A new `TaskOutput` tool polls a background subagent by `task_id` and collects its final answer once finished.
- Resume capability: subagents now persist their session, and the `Task` tool accepts an optional `resume_task_id` to continue a previous subagent run with a follow-up `prompt` (full prior transcript intact). Partial results surface their resume handle so the parent can continue interrupted work.

### Changed

- Renamed the model-facing `subagent` tool to `Task`, mirroring Claude Code. Parameters are now `description`, `prompt`, and `subagent_type` (any registry agent name). Delegation is description-driven: the model chooses when and which agent to use, with no blocking dispatch gate. The `--subagent` flag, `enableSubagent` setting, and `/subagent` command are unchanged (the latter now validates against the registry).
- The subagent token budget is now advisory. It still emits `budget_warning` (80%) and `budget_exceeded` (100%) events for telemetry, but never kills or fails a subagent; the per-subagent turn cap is the guaranteed hard stop. Lifeguard stall/timeout kills are unchanged.
- Relocated subagent runtime dispatch state from `.hoocode/agents/<task_id>/` to `.hoocode/dispatch/<task_id>/`, freeing `.hoocode/agents/` to hold agent definitions.
- Subagent sessions are now persisted to `.hoocode/dispatch/<task_id>/session.jsonl` (previously ephemeral via `--no-session`) so a finished or interrupted subagent can be resumed.
- Nesting guard: the `Task`/`TaskOutput` tools are never registered inside a spawned subagent process (`--task-id` present), even when a project's `enableSubagent` setting is on, so subagents cannot recursively dispatch.

## [0.4.14] - 2026-05-31

### Changed

- Task panel restyled to the hoocode design system: the `#id` recedes (dim), completed task titles fade to muted while active/failed stay full foreground, each finished row's token count sits one step brighter than its elapsed time, and the header turn delta uses muted framing with bright numbers. The reviewed/deterministic stamp is now quiet (dim) rather than green; only the active "watching" state keeps a tint. Pure visual change, no logic changes.
- Subagent routing is now deterministic on ties: the dispatch evaluator uses an explicit agent-priority order instead of incidental object-iteration order, exposes a normalized `confidence` on each analysis, and defaults an ambiguous delegated task to `explore`. Tightened the parent's subagent guidance to prefer inline handling for small/quick tasks.
- Raised default subagent token budgets to stop hard-stopping agents mid-task: explore 35k, edit 60k, test 45k, fix 45k, review 35k, doc 30k (fallback 35k). Real per-event usage tracking is unchanged.
- Tool execution blocks (bash commands, diffs, file reads, etc.) are tighter: dropped the box vertical padding so consecutive tools are separated by a single blank line instead of three, saving vertical space in the TUI.

## [0.4.13] - 2026-05-30

## [0.4.12] - 2026-05-30

### Changed

- Subagents now run as isolated `hoocode` child processes through `SubagentPool` instead of in-process loops. The `subagent` tool and `/subagent` command dispatch through a shared pool with bounded concurrency, per-mode tool allowlists, token budgets, and lifeguard monitoring (heartbeat + hard timeout).
- Added a `--task-id` CLI flag (internal) and made `--mode json` subagents emit a `{"ping":true}` heartbeat and write a verified `result.json` on exit, so the parent pool can monitor liveness and validate output.

### Removed

- Removed the in-process `runSubagent` path; subagent execution is now exclusively pool/child-process based.

### Fixed

- Tool execution status dot (`●`) is now rendered inline with the tool/command on the first line instead of on its own line above it. Renderer-backed tools (e.g. bash) previously stacked the dot as a separate line.

## [0.4.11] - 2026-05-30

### Changed

- Task panel: subagent task titles are now limited to ~4–8 words so they stay legible in the pane.
- Task panel: finished tasks show combined token usage and elapsed time (`tokens · time`).
- Task panel: header shows a per-turn token + cost delta (`turn ↑in ↓out $cost`) summed across the turn's tasks.
- Task panel: the `[mode]` tag (e.g. `[explore]`) is no longer shown per row — the task title is the meaningful label.
- Finished subagent tasks now persist in the task panel until the next user message, instead of retiring when the main agent starts its next turn.

## [0.4.10] - 2026-05-30

## [0.4.8] - 2026-05-30

### Added

- TokenBudget class tracking per-agent-type token budgets with 80% warning and 100% hard-stop, persisting usage to disk.
- OutputVerifier validating subagent result.json after exit.
- SubagentLifeguard with heartbeat monitoring, hard timeouts, parent-exit cleanup, and startup sweep.
- Named `SUBAGENT_MAIN_PROMPT` system prompt appendix loaded when subagent tooling is enabled.

### Changed

- Footer displays active mode plus `+ subagent` when the subagent system prompt is active.
- Loading page `[Resources]` section always shows `mode/{activeMode}` and `subagent_system_prompt` when applicable.

### Fixed

- Editor wordmark first-line indentation preserved (array join instead of `String.raw` + `.trim()`).

## [0.4.7] - 2026-05-30

## [0.4.6] - 2026-05-30

### Changed

- Task panel now shows all subagent tasks with every status (pending, in_progress, done, failed) instead of only active ones. Finished tasks keep their final status icon and retire only when the main agent moves on to its next turn after a parallel subagent spawn.
- Manual and auto compaction now share a single `_applyCompaction` core, removing ~80 lines of duplicated extension-hook, persistence, and summary-extraction logic that could drift between the two paths.
- `/compact` no longer pre-checks message count in the UI; the session is the single source of truth and surfaces the precise reason ("Already compacted", "Nothing to compact") via the compaction event.

### Fixed

- Compaction now fails loudly instead of writing an empty summary when the summarizer model returns no usable text, preventing silent loss of conversation history.

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

# Changelog

All notable changes to HooCode will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Removed: hoocode-ts pin, shim and parity checks (2026-10-10)
- **hoocode fully replaces hoocode-ts.** The pinned TS checkout, the `hoocode-ts` shim (installer and release archives), the parity harness, the CI fixture fetch and the TS-comparison tests are gone.
- **Compaction tests use a generated session** in the old v1 format instead of the TS `large-session.jsonl`.
- **Moved:** `replay.json` to `crates/hoocode-code-main/tests/fixtures/`, and the dependency firewall check to `scripts/ci/`.

### Changed: model catalog comes from models.dev (2026-10-10)
- **New crate `hoocode-models-sync`** (`cargo run -p hoocode-models-sync --bin models-sync`) refreshes `data/models.json` from models.dev. Curated fields are kept and models are never removed automatically. Rules live in `data/overrides.json`.
- **A weekly workflow** (`models-sync.yml`) opens a PR when the data changes. It needs a `MODELS_SYNC_TOKEN` secret for that PR to run CI.
- **First sync:** 31 models added and 8 prices updated (DeepSeek, Fireworks, Anthropic cache read, Kimi K3 cache write).

### Changed: smaller target/ (2026-10-10)
- **Dev builds have `debug = false`.** A full build is 5.5 GB instead of 61 GB, and there are no `.o` debug files. Panic messages still show file:line.
- **`scripts/dev/prune_target.sh`** removes build output unused for 3 days, old `cortex*` names, and an incremental cache over 5 GB. It is a dry run by default; `--apply` deletes and `--if-over GB` skips small dirs. `CLAUDE.md` runs it at session start.

### Fixed: subagents are no longer stopped in the middle of a long tool (2026-10-10)
- **A running tool counts as progress.** The watchdog stopped any subagent that went 150s without a turn or tool event, so a long `cargo` build killed the agent mid-tool ("Ran out of time before completing."). Now it only stops a child that has no tool running. The heartbeat check and the 2-hour deadline are unchanged.
- **The footer no longer misses a branch switch made right after startup.** The git HEAD watcher took its baseline on a background task, so a HEAD change that landed first was never seen.

### Changed: footer has two modes (2026-10-10)
- **Alt+Z switches between full and compact.** `bare` is gone; a saved `bare` loads as compact, and `/chrome bare` gives an error.
- **The full footer has a fixed 22-cell left column**, so mode, context bar, model and tokens line up.
- **The compact footer is one row.**

### Changed: tool calls in peek and radar (2026-10-10)
- **Peek view:** every tool block has a 2-space left margin, and its output starts under the tool name. Read, Write and Edit sit on a light band (new optional theme token `toolBandBg`, which falls back to `toolPendingBg`).
- **Shell reads `● Shell  <cmd>`**, like the other tools, instead of `$ <cmd>`. No blank row after the call line.
- **Write shows a 5-line, line-numbered preview** of what it wrote.
- **Radar trims targets to 40 columns** with `…`, and shows only the first line of a command.

### Changed: less process between an idea and a release (2026-10-10)
- **Done = fmt, clippy and the tests of the crates you touched.** CI runs the whole workspace.
- **Screen goldens cut to 10 scenarios, run before a release.** Review bundles and the `tui-review` skill are removed.
- **CI:** a release no longer re-runs the gates (label `release:run-gates` forces them). macOS tests and subagent evals are off PRs. No test retries. Fuzzing uses a prebuilt `cargo-fuzz` and a cache.
- **The finished TS→Rust migration is archived** under `archive/`.

### Changed: subagents may run for 2 hours; AgentOutput can cancel (2026-10-10)
- **One 2-hour deadline for every subagent type.** The per-type 5-20 minute table is gone. Load scaling adds at most 30 minutes.
- **Poll and cancel with `AgentOutput`.** Poll by `task_id`; stop a run you no longer need with `AgentOutput(task_id, cancel: true)`.
- **Stall detection still reaps hung children early**, so a child stuck in a provider call does not hold the full deadline.

### Changed: header cursor blinks; wording cleanup (2026-10-10)
- **The header cursor blinks** every 530 ms, driven by the app clock (Ghostty ignores SGR 5). It stops
  after the first prompt, or while the view is scrolled back, and then stays solid.
- **Warnings use `⚠`** in the notification band, not `●`. One `WARNING_GLYPH` serves every warning line.
- **Ellipses are `…`** in the status and dialog labels: reloading, summarizing, auto-compacting,
  waiting for browser login.
- **Key names are lowercase** in hints: `esc`, `tab`, `ctrl+enter`.
- **Compaction cancelled** is a status line, not an error.
- **The header hint** no longer shows `ctrl+o more`. The header expands only under `verbose`.
- **The `ctrl+o` hint** on the tool-output row reads `to toggle radar and peek`.

### Added: scroll back while a picker is open, and a scrollbar (2026-10-10)
- **Scroll with a picker open.** PgUp/PgDn, Ctrl+Home/End and the wheel move the transcript while a
  picker or question has focus. Arrows, Enter, Esc and typing still go to the picker and never un-pin.
- **Scrollbar** in the last column while scrolled back: `┃` is the thumb, `░` the track. Click above
  the thumb to page up, below it to page down.

### Changed: footer is always two rows, layout B; the folder name is never dropped (2026-10-09)

### Changed: the tool output dial is radar/peek only (2026-10-09)
- **Two stops, not three.** The dial is `radar` and `peek`. The `full` stop is gone; a saved `full`
  view reads as `peek`.
- **Ctrl+O toggles** between `radar` and `peek`. Alt+O still cycles the dial.
- **The `ctrl+o to expand` hints are gone.** Truncated tool output now ends with `... (N more lines)`
  and no longer names a key.

### Added: app-server clients choose the effort and see the scope (2026-10-09)
- `thread/start`, `thread/resume` and `turn/start` take an optional `effort`. An explicit effort the model
  does not support is rejected, not silently changed.
- Switching to a scoped model applies its effort. Scoped models are listed first in `model/list`, with
  their `category` and `alias`, and each model reports the efforts it supports.
- Thread results include `reasoningEffort`. Names outside the model scope are rejected.

### Fixed: expired OAuth login says how to sign in again (2026-10-09)
- When an OAuth login has expired, the error now names the provider and the `/login` command to run,
  instead of the raw "No API key found" message. API-key providers are unchanged.

### Added: Claude Haiku 5.5 (2026-10-09)
- Added Claude Haiku 5.5 (`claude-haiku-5-5`) to the Anthropic model catalog: 1M context, 128K max output,
  reasoning, text and image input, $0.10 / $0.50 per MTok (cache read $0.01, cache write $0.125; the
  over-100K-token tier of $0.50 / $2.50 is not modelled). Hoocode-ts v0.6.0 does not ship it yet, so
  `scripts/models_overrides.json` adds it on each regeneration until the pin catches up.
- Haiku 5.5 uses adaptive thinking (no `budget_tokens`) and never sends `temperature`, which the API rejects.
  Haiku 4.5 is unchanged.

### Added: scoped models carry an effort and a category (2026-10-09)
- **`scopedModels` setting** in `settings.json`: an ordered list of `{ model, effort?, category?, alias? }`.
  It replaces `enabledModels` and `modelCategories`. A project `scopedModels` replaces the global list.
- **Migration:** on the first load, when `scopedModels` is missing, it is built once from `enabledModels`
  (a `:level` suffix becomes `effort`) and `modelCategories` (model to category), and written to disk.
  The old keys stay in the file and are no longer read. hoocode-ts does not see the new scope.
- **`/scoped-models` picker** has an effort column (`tab` cycles the supported levels) and a category
  column (`alt+j` cycles none, cheap, fast, standard, capable). Save writes the full entries, not ids only.
  `/settings` has a Models row that opens it.
- **Agent tool** takes `model` (a category or a scoped model) and `effort`. Tool results start with
  `[model: ...]`, naming the model that ran. `--models` still works for one run and is not saved.

### Removed: the Agent tool `complexity` parameter (2026-10-09)
- `complexity` is removed, not aliased. `model` is the only model parameter. The dispatch log line
  still prints a heuristic complexity estimate.

### Changed: tool names renamed; the old names are no longer accepted (2026-10-08)
- **New tool names:** `read` → `Read`, `bash` → `Shell`, `edit` → `Edit`, `write` → `Write`,
  `SearchCodebase` → `CodeSearch`, `SearchHooCode` → `DocSearch`, `ask_options` →
  `AskUserQuestion`, `webfetch` → `WebFetch`, `websearch` → `WebSearch`, `AgentOut` →
  `AgentOutput`. `Agent` and `TodoWrite` are unchanged.
- **No aliases for the old names.** They are unknown tools now. The legacy `Task` and
  `TaskOutput` aliases are deleted. Settings, permission rules, `enabled_tools`/`denied_tools`
  and agent `tools:` lists must use the new names.
- The model sees the built-in tools in this order: `Read`, `Shell`, `Edit`, `Write`,
  `CodeSearch`, `AskUserQuestion`, `DocSearch`, and the rest.

### Changed: the subagent tools are `Agent` and `AgentOut` (2026-10-06)
> Superseded 2026-10-08: `AgentOut` is now `AgentOutput`, and the `Task` and `TaskOutput`
> aliases described below were deleted.

- **`Task` → `Agent`, `TaskOutput` → `AgentOut`.** `Task` read as a to-do item while the tool starts
  a subagent run. The old names stay registered for one release as deprecated aliases that run the
  same executor, and the prompt's tool list marks them `deprecated alias for Agent; prefer Agent`.
- The transcript line names the tool: `Agent explore` (`Agent resume plan · background`) and
  `AgentOut explore#1 (wait)`. Tool results, notifications and error hints say `Agent`/`AgentOut`.
- Every place that checked for the subagent tool by name accepts both spellings: the agent list in
  the prompt, the footer indicator, the startup listing, the nested-agent `--tools` grant (pool and
  warm pool) and the tool-chain summary.

### Added: subagent evals and a dispatch ledger (2026-10-05)
- **Subagent reliability is now measured, not remembered.** `<cwd>/.hoocode/dispatch/ledger.jsonl`
  is an append-only line per dispatch **attempt** - agent, requested and resolved model, mode, depth,
  status, verifier verdict, confidence, wall clock, generated tokens, peak context, exit code and cause.
  Every terminal path in the pool writes one, including the ones that used to vanish: the inherited-model
  retry, a kill at the deadline, a cancelled task, a spawn failure. The file is bounded (rewritten past
  20k lines, newest 10k kept) and strictly best-effort - an unwritable ledger logs and drops, and never
  fails a dispatch. `result.json` still decides correctness; the ledger only records what happened.
- **`/subagent-stats [24h|7d|all]`** renders it in the TUI: attempts, usable rate, statuses,
  median/p90/max wall clock, tokens, fallback count, per-agent breakdown and the last five failures
  with their causes.
- **`scripts/eval/subagent_evals.py`** drives the real binary end to end - real parent session, real
  `Agent` tool (then `Task`), real pool, real child process - against a routable scripted mock provider
  (`scripts/eval/mock_provider.py`). Twelve scenarios cover the happy paths, the turn limit, an invalid
  tool call, a stream that dies mid-answer, the deadline wrap-up, the stall reaper, a region rejection
  that must fall back, a bad complexity tier and eight concurrent dispatches against five slots.
  Exits non-zero when a scenario deviates from its declared expectation; `--min-success` gates a build,
  `--repeat` measures flakiness, `--include-slow` adds the two minute-long probes. Reports land in
  `target/subagent-evals/`.
- **Fixed: concurrent ledger appends interleaved.** The record and its newline were two `write_all`
  calls, so eight simultaneous settles produced seven lines and one unparseable one. One write per
  record now, with a 400-line concurrency test. Found by the eval suite on the code the suite needed.
- **Fixed: a background dispatch was recorded as `blocking`.** The Task tool's background flag never
  reached the pool task, so `mode` in the ledger was wrong for every background run.


### Fixed: subagents stopped failing on almost every dispatch (2026-10-03)
- Subagents ran to completion in none of ten recorded dispatches. Seven fixes, detailed in
  `docs/design/subagents.md`.
- **Every subagent was killed after 5 minutes.** The per-agent timeout table was keyed on
  `edit`, `test` and `review` — agents this project does not ship — so no entry ever matched
  and every agent fell through to the 5-minute default. `code-review` died at exactly 300s
  three times holding up to 297s of finished work. Real per-agent deadlines now exist
  (`code-review`/`security-review` 15 min, `general-purpose` 20, `explore`/`plan` 10), and
  the warm worker pool's flat 3-minute limit follows the same table.
- **A subagent killed at its deadline lost everything it had finished.** `result.json` is
  written only after the run returns cleanly, so the kill discarded it. A child is now told
  its deadline up front and asked to wrap up shortly before it, so a run cut short reports
  its findings as a usable partial result instead of vanishing.
- **A subagent that died because its model was unreachable never retried on a reachable one.**
  Timed-out and stalled runs skipped the inherited-model fallback entirely, and the error
  classifier did not recognise region rejections (`requires Global regions`) or
  `finish_reason: error`. Both fixed; a cancelled run is still never retried.
- **The inherited-model fallback could silently do nothing.** When a caller passed a
  `complexity` tier, the fallback re-resolved that same tier and retried the identical model.
- **Reported token usage was wildly wrong.** The subagent budget summed each turn's whole
  context size, so a 15-turn run reported 1.6M tokens against a 35k budget. It now counts what
  the subagent generated; context size is reported separately as `peak_context`.
- A failed dispatch wrote a 256 KB `output.json` that duplicated the transcript already in
  `session.jsonl`. It now holds the outcome, the cause, and a short tail of stderr.

### Release: npm publish is now verified after it runs (2026-10-02)
- The npm publish loop moved from an inline shell loop in `binaries.yml` to
  `scripts/npm/publish_packages.py`, which publishes, then checks every package
  against the registry packument and fails the job if any is missing. A fresh
  publish is readable only after registry propagation — after v0.1.5, `darwin-x64`
  took ~11 minutes to appear — so a partial publish used to look exactly like a
  successful one. Re-runs still skip published versions, so a failed release is
  recovered by re-running `binaries.yml` for the same tag.
- `--verify-only` checks a published release without publishing, and reads the
  packument with cache busting (npm's CDN otherwise serves stale versions).

### Fixed: compaction and branch summaries on OpenCode Go (2026-10-02)
- Compaction (`/compact`, automatic context compaction, branch summaries) builds its own
  summarization request, which did not carry the session id, so OpenCode Go rejected it with
  `400 Request is missing x-opencode-session and cannot be routed efficiently`. Normal turns
  were unaffected — they already sent the header. Summarization now passes the session id
  through on every request (chat completions, responses, Anthropic Messages), and it is still
  omitted when there is no session. Two tests cover both directions.

### Pin bumped to hoocode v0.6.0 (2026-10-01)
- Model catalog regenerated from hoocode v0.6.0 (1228 models). It adds
  `anthropic/claude-sonnet-5-5`, `opencode-go/longcat-2.5-preview-free`, `opencode/gpt-6.1-sol`
  and others, and drops models upstream removed (e.g. `opencode-go/kimi-k2.6`).
- OpenCode Go: every request sends `user-agent: hoocode` and, when a session exists,
  `x-opencode-session` (Chat Completions, Responses and Anthropic Messages), even with prompt
  caching off. Go rejected requests without it: `400 Request is missing x-opencode-session`.
- Default models for `opencode-go`, `fireworks` and `together` are now Kimi K3. A test now
  fails when any provider default drops out of the catalog.
- Bundled theme JSONs match the pin (a test checks them against the pinned checkout).
- `migration/pin_drift.py` reports when upstream has released past the pin and prints the
  port checklist for a bump. `setup_hoocode.sh` installs with the pin's declared bun version.

### Migration paused, `hoocode` ready for use (2026-10-01)
- Every remaining ledger task is deferred by user decision: MCP (9.1, 10.11),
  webfetch/websearch (10.2e), the TS test ledger close-out (13.4) and phase 12. See the
  README's "Migration status" and §0.3 of the migration plan for what works and what doesn't.
- Interactive agent modes: `/mode`, `/plan`, `/grill`, `/approve` now act in the TUI. They
  notify, reload with the new mode's prompt and tool filter, and update the footer badge. alt+a
  cycles ask → plan → build → debug, and mode commands appear in autocomplete with
  argument completions (10.5e).
- Earlier in this cycle: phase 11 (the interactive TUI) is complete. That covers the chat
  view, selectors, input features, task panel, key bindings, thinking toggle and the
  external editor.

### Migration re-baseline (2026-09-24)
- Pinned the port to hoocode v0.5.89 (`a6cd96e7`). An audit found the earlier
  "100% complete" status overstated, and progress is now tracked in
  `migration/ledger.json` (see `docs/design/ts-to-rust-migration.md` §0).
- Added two-level done gates: Level 1 = cargo checks, Level 2 = rendered-TUI parity
  against hoocode via `migration/tui-parity/`.
- Removed the stale status reports (`MIGRATION_*.md`, `TEST_RECORD.md`,
  `E2E_TESTING_SUMMARY.md`, `OPENCODE_E2E_TEST_REPORT.md`, `QUICKSTART.md`).
- Live OpenCode checks moved to `scripts/live/` and
  `crates/hoocode-ai-provider-openai/tests/opencode_live.rs` (`#[ignore]`d). The API
  key now comes only from `OPENCODE_API_KEY`. A key previously committed in these
  files must be rotated.

### Added
- OpenCode API provider support (opencode, opencode-go)
- mimo-v2.5-free as default model for OpenCode provider
- Comprehensive E2E testing framework
- Vertex AI Application Default Credentials (ADC) support
- Service account JWT authentication for Google Cloud
- GCE/GKE metadata server authentication
- Windows virtual terminal input tweaks
- Paste-marker compression in editor
- Vim-style character jump (f/F) in editor
- Interactive OAuth login (Anthropic, GitHub Copilot)
- Permission gates for tool call approval
- Auto-migration from legacy ~/.hoocode settings
- Explicit --config CLI override
- Reasoning-item ID pairing in Azure provider

### Changed
- Improved provider configuration and model selection
- Enhanced error handling and user feedback
- Optimized streaming performance
- Updated documentation and test coverage

### Fixed
- Cargo fmt formatting issues
- Clippy warnings resolved
- Windows console input handling
- Context preservation in multi-turn conversations

## [0.1.0] - 2026-07-23

### Added
- Initial release of HooCode
- Rust migration from HooCode TypeScript framework
- 43 crates in workspace structure
- 640 tests passing
- Support for 6 LLM providers:
  - Anthropic (Claude)
  - OpenAI (GPT)
  - OpenCode (MiMo, DeepSeek, etc.)
  - Google (Gemini)
  - Azure OpenAI
  - Faux (testing)
- CLI with print and interactive modes
- JSON-RPC server mode
- Subagent pool and Task tool
- Core tools (read, write, edit, bash, grep, find, ls)
- MCP (Model Context Protocol) support
- WASM plugin extensions
- Session persistence and management
- Context compaction
- Cross-platform binary builds (Linux, macOS, Windows)
- GitHub Actions CI/CD pipeline
- crates.io publishing workflow

### Technical Details
- **Build System:** Cargo workspace with lockstep versioning
- **Minimum Rust Version:** 1.78
- **Test Coverage:** 640 tests across 43 crates
- **Clippy Warnings:** 0
- **CI Status:** All checks passing

### Supported Platforms
- Linux (x86_64)
- macOS (Intel, Apple Silicon)
- Windows (x86_64)

### Providers
| Provider | Models | Status |
|----------|--------|--------|
| Anthropic | Claude 3.5, Claude 4 | ✅ |
| OpenAI | GPT-4, GPT-4o | ✅ |
| OpenCode | MiMo, DeepSeek, Claude | ✅ |
| Google | Gemini 2.5 | ✅ |
| Azure | GPT-4, GPT-4o | ✅ |
| Faux | Test models | ✅ |

---

## Release Notes

### v0.1.0 - Initial Release

HooCode is a Rust migration of the HooCode TypeScript coding-agent framework. This release includes:

**Core Features:**
- Full LLM runtime with streaming support
- Tool execution and permission gates
- Session management and persistence
- MCP (Model Context Protocol) support
- Interactive TUI with vim-style editing

**Providers:**
- Anthropic (Claude 3.5, Claude 4)
- OpenAI (GPT-4, GPT-4o)
- OpenCode (MiMo, DeepSeek, Claude, Gemini)
- Google (Gemini 2.5)
- Azure OpenAI
- Faux (testing)

**CLI Modes:**
- Print mode (text/JSON output)
- Interactive TUI mode
- JSON-RPC server mode
- Subagent mode

**Development:**
- 43 crates in workspace structure
- 640 tests passing
- Zero clippy warnings
- CI/CD with GitHub Actions
- Cross-platform binary builds

**Installation:**
```bash
# From source
cargo install --path crates/hoocode-code-main --bin hoocode

# Pre-built binaries
# Download from GitHub Releases
```

**Usage:**
```bash
# Single-shot mode
hoocode -p "Explain this codebase"

# Interactive mode
hoocode

# With specific provider
hoocode --provider opencode --model mimo-v2.5-free -p "Hello"
```

---

## Migration Status

The migration from HooCode TypeScript to HooCode Rust is **98% complete**.

**Completed:**
- ✅ TUI Namespace (120 tests)
- ✅ AI Namespace (155 tests)
- ✅ Agent Namespace (57 tests)
- ✅ Code Namespace (88 tests)
- ✅ OpenCode API Provider (22+ tests)
- ✅ All 6 Providers Working
- ✅ 640 Tests Passing
- ✅ Zero Clippy Warnings
- ✅ CI/CD Pipeline Working

**Remaining (Deferred):**
- ⏳ Full parity testing with scripted hoocode scenarios
- ⏳ Release notes and changelog (this document)

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines on how to contribute to HooCode.

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

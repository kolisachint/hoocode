# Design docs

Start here. The migration from hoocode-ts is finished. hoocode fully replaces it (decided
2026-10-10), and no TS pin or parity check remains. Everything
still to build is planned in the cards below. Each card was agreed with the user on
2026-10-07 or later. The dated decision pages win if a card disagrees:
[decisions-2026-10-07.md](../../archive/docs/decisions-2026-10-07.md) and
[decisions-2026-10-08.md](../../archive/docs/decisions-2026-10-08.md) and
[decisions-2026-10-09.md](decisions-2026-10-09.md) and
[decisions-2026-10-09-subagents.md](decisions-2026-10-09-subagents.md) (each later
page adds to the earlier ones).

To find code, use the maps: [../maps/packages.md](../maps/packages.md) (crates) and
[../maps/ui.md](../maps/ui.md) (screen, pickers, slash commands).

## How we work on designs (user preferences)

- **Design first, code later.** A card is reviewed and agreed before any code. A
  session that is asked for design work changes docs only.
- **One page per card.** Sections in this order: Goal, Decisions, What we build,
  Not doing, Open questions, then a short Details appendix only if needed. Plain
  words, short sentences, tables over prose. No history or background essays; git
  has the long earlier drafts.
- **Decide with the user, recommendation first.** Put open questions to the user as
  choices with the recommended option first. Point out the trade-offs and the
  risks an answer creates. Record the answers in a dated `decisions-*.md`.
- **Standards first, no bloat.** Prefer open standards: AAIF's MCP and its
  extensions, AGENTS.md, Agent Skills, and Agent Plugins. Support a vendor format
  only when the standard can't cover it. Build for where the standards are going,
  not for legacy (example: drop deprecated MCP transports).
- **Reliability before features.**
- **No new always-on token cost.** Tools that add schemas are opt-in or appear only
  when used.
- **Licences:** copy code only from MIT sources (CLAUDE.md). Depending on a
  permissively licensed crate is fine. The user's own repos (`webtools`,
  `embeddingsearchtools`, `voicetools`, `hooteams`, `drawio-canvas`) are MIT and
  fair to use.

## Two plans, side by side

There are two plans: the **core plan** (below) and the **TUI plan**
([tui-activity.md](tui-activity.md), agreed 2026-10-09). Neither is ahead of the other.
**When the user asks to proceed or to start implementing, ask which plan and which step
first.** Don't pick one yourself.

| Plan | Card | Phases | First step |
|---|---|---|---|
| Core | this page, "The core plan" | milestones 0a–8 | per the status line below |
| TUI | [tui-activity.md](tui-activity.md) | T0 goldens · S simplification of every TUI component (tiers Now → Next → Later) · T2 subagent storage · T3 live attach · T4 panel tabs · T5 background shell. All work by Haiku subagents | T0 done 2026-10-09; next Phase S Now (N1) |

## The core plan, in priority order

Agreed on 2026-10-08. One milestone at a time; each lands as one or more PRs that
pass the done bar (CLAUDE.md). Sizes: **S** about a session, **M** a few,
**L** many.

| # | Milestone | Card | Size | Needs | Why here |
|---|---|---|---|---|---|
| 0a | **Done 2026-10-14.** Close the migration ledger: `moved` and `dropped` statuses; mark 9.1, 10.2e, 10.11, 12.1–12.7 with their cards; 13.4 dropped; one line each in plan §0.3 and `PROGRESS.md` | this page | S | — | Bookkeeping before code |
| 0b | **Done 2026-10-14.** Delete the unused crates, except `agent-mcp`, which is kept ([decision](decisions-2026-10-14.md)). Move `hoocode-ai`'s tests to `ai-registry` first; fix dep firewall, `generate_crates.sh`, CI, release scripts | [decisions-2026-10-08.md](../../archive/docs/decisions-2026-10-08.md) | S | 0a | Less to rename and build |
| 0c | **Done 2026-10-14.** Rename everything to hoocode, one mechanical commit, no other branch open | [naming-and-paths.md](../../archive/docs/naming-and-paths.md) §1 | M | 0b | Touches every file; must not race other work |
| 0d | **Done 2026-10-14, except CI wiring:** CI wiring is staged in [step1-workflows.patch](../../archive/migration/ci/step1-workflows.patch) (guards job, TUI parity workflow, macOS job on PRs); the user applies it with `git apply` and pushes. `scripts/maps/packages.py` (regenerates [../maps/packages.md](../maps/packages.md), `--check` in CI) and the `scripts/ci/no_cortex.sh` guard | [naming-and-paths.md](../../archive/docs/naming-and-paths.md) §1 | S | 0c | Keeps the maps and names honest from here on |
| 1 | **Done 2026-10-14.** Reliability, in this order: **1.1** rpc fails closed (security); **1.2** `~/.hoocode` paths, `HOOCODE_` env, one-time merge; **1.3** macOS test fixes and CI job; **1.4** `@file` without `fd`; **1.5** panic audit; **1.6** fuzzing | [reliability.md](reliability.md), [naming-and-paths.md](../../archive/docs/naming-and-paths.md) §2–4 | L | 0c | Reliability before features |
| 2 | Concurrency phases 0–1: `/perf` and the load test, then one runtime and the caps | [concurrency.md](concurrency.md) | M | 1 | Sets the runtime rules before MCP brings rmcp |
| 3 | `DocSearch`; close the 8 `l1_done` tasks | [semantic-search.md](semantic-search.md) part A | S | 2 | Small; finishes the migration's loose ends |
| 4 | MCP client on rmcp; then a short guide for the webtools MCP server | [mcp.md](mcp.md), [web-tools.md](web-tools.md) | L | 2 | The main missing capability |
| 5 | Concurrency phases 2–5: terminal-output thread, session writer, lanes and priority, watchdog and memory limits | [concurrency.md](concurrency.md) | L | 4 | Built against the real MCP and tool load |
| 6 | **Done 2026-10-15.** Plugins: load and install (`hoocode-code-plugins`, `/plugin`), then the opt-in model tools (`enablePluginTools`: SearchPlugins, InstallPlugin, UninstallPlugin, ListPlugins, ProposePlugin). Open: a model install or proposal reloads the plugin set, but the skills and MCP servers go live on `/reload` (no TUI reload from a tool result yet); the `--platform` help text still says Claude and Copilot layouts for ProposePlugin | [plugins.md](plugins.md) | L | 4 | Delivers skills, MCP servers and subagents |
| 7 | **Done 2026-10-14.** Scheduler and `/loop`: the `/loop` forms, `/loop auto`, the 10-minute late-fire rule | [scheduler-and-loop.md](scheduler-and-loop.md) | M | 1 | Independent; after the core |
| 8 | **Done 2026-10-14.** Version check (`hoocode-code-update`, `hoocode update`) and completion chime (`chime.rs`). Open: the live registry request is not in tests (no network there) | [extras.md](extras.md) | S | 0c | Small polish |
| 9 | Subagent orchestration: background by default, subagent brief, worktree protocol by instruction, child prompt fixes, evals | [subagent-orchestration.md](subagent-orchestration.md) | M | — | Parent stops waiting on subagents; writers stop sharing one tree |

**Status (2026-10-08):** milestone 2 (concurrency phases 0-1) is done. Milestone 3 (DocSearch;
the 8 `l1_done` tasks) is done. Milestones 4 (MCP client) and 5 (concurrency phases 2-5) are
built, not yet load-tested. Open items are listed in the 2026-10-08 (second session) entry in
[../../archive/migration/PROGRESS.md](../../archive/migration/PROGRESS.md).

**Status (2026-10-14):** steps 0a–0d are done. The 0d CI wiring (guards job, TUI parity workflow) is staged, not applied: the session token lacks workflow scope. The user applies `git apply archive/migration/ci/step1-workflows.patch` and pushes it ([tui-activity.md](tui-activity.md) T0.6). `agent-mcp` is kept as the home of the 9.1 MCP client ([decisions-2026-10-14.md](decisions-2026-10-14.md)). **Step 1 (reliability 1.1–1.6) is done in code** ([reliability.md](reliability.md)); the macOS job on PRs is staged in `step1-workflows.patch` and is not live until the user pushes it. **Step 6 parts 1 and 2 (plugin load, install) are done 2026-10-14** ([plugins.md](plugins.md)); part 3 (opt-in model tools) is next. Steps 7 and 8 are done 2026-10-14. 4 and 5 still need their load test.

**Only if the numbers or a need say so:** concurrency phase 6 (highlighting off the UI
thread), MCP Apps ([canvas-and-mcp-apps.md](canvas-and-mcp-apps.md)), the MCP Tasks
extension, background compaction (its own card first), thinking escalation.

**Dropped:** Copilot canvas, Skills over MCP, plugin update/package/publish tools,
writing Claude plugin formats, `/mcp import`, rpc approval dialogs
([rpc-approvals.md](rpc-approvals.md)), semantic search part B, crates.io, the WASM
crate, Azure, `fd`/`rg` and the external-tools pane, and the extras marked No in
[extras.md](extras.md).

## Designed, not scheduled

Agreed designs with no build step yet. They are not in the core order above. Ask the user
before scheduling one.

| Card | What | Status |
|---|---|---|
| [scoped-models.md](scoped-models.md) | `/scoped-models` gets an effort and a category per model; `scopedModels` replaces `enabledModels` and `modelCategories`; subagents ask by category or by model | Implemented 2026-10-09 (see its implementation notes). |

Older docs that stay as they are: [app-server.md](app-server.md), [subagents.md](subagents.md),
[subagent-evals.md](subagent-evals.md), [distribution.md](distribution.md),
[build-speed.md](build-speed.md), and the migration plan
[ts-to-rust-migration.md](../../archive/docs/ts-to-rust-migration.md).

## Standards checked (2026-10-07)

| Standard | State | Used in |
|---|---|---|
| MCP (AAIF) | Spec `2026-07-28`: stateless, `server/discover`, multi round-trip input, extensions. HTTP+SSE, Roots, Sampling, Logging and Dynamic Client Registration deprecated. | mcp.md |
| MCP Apps, Tasks, Skills extensions (AAIF) | Official | Not used for now: Apps and Tasks deferred, Skills dropped (2026-10-08) |
| Agent Plugins 1.0.0 (agent-plugins.org; Amazon, Cursor, Google, Microsoft, OpenAI, Vercel) | Published 2026-08-06: `plugin.json`, `skills/`, `mcp.json` | plugins.md |
| Agent Skills (agentskills.io) | `SKILL.md` frontmatter rules | plugins.md |
| AGENTS.md (AAIF) | Already supported | — |
| A2A (AAIF) | v1.0; no local or stdio story | not used |

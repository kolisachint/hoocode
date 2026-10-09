# Design docs

Start here. The migration from hoocode-ts is finished at the v0.6.0 pin. Everything
still to build is planned in the cards below. Each card was agreed with the user on
2026-10-07 or later. The dated decision pages win if a card disagrees:
[decisions-2026-10-07.md](decisions-2026-10-07.md) and
[decisions-2026-10-08.md](decisions-2026-10-08.md) and
[decisions-2026-10-09.md](decisions-2026-10-09.md) (each later page adds to the
earlier ones).

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
- **hoocode-ts is a reference, not a mandate.** Follow it where it is good. Say so
  where we deliberately differ.
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
| TUI | [tui-activity.md](tui-activity.md) | T0 goldens (replaces L1/L2 parity) · S simplification of every TUI component (tiers Now → Next → Later) · T2 subagent storage · T3 live attach · T4 panel tabs · T5 background shell. All work by Haiku subagents | T0 done 2026-10-09; next Phase S Now (N1) |

## The core plan, in priority order

Agreed on 2026-10-08. One milestone at a time; each lands as one or more PRs that
pass Level 1 and Level 2 (CLAUDE.md). Sizes: **S** about a session, **M** a few,
**L** many.

| # | Milestone | Card | Size | Needs | Why here |
|---|---|---|---|---|---|
| 0a | Close the migration ledger: `moved` status; mark 9.1, 10.2e, 10.11, 12.1–12.7, 13.4 with their cards; one line each in plan §0.3 and `PROGRESS.md` | this page | S | — | Bookkeeping before code |
| 0b | Delete the 11 unused crates (move `hoocode-ai`'s tests to `ai-registry` first; fix dep firewall, `generate_crates.sh`, CI, release scripts) | [decisions-2026-10-08.md](decisions-2026-10-08.md) | S | 0a | Less to rename and build |
| 0c | Rename everything to hoocode, one mechanical commit, no other branch open | [naming-and-paths.md](naming-and-paths.md) §1 | M | 0b | Touches every file; must not race other work |
| 0d | `scripts/maps/packages.py` (regenerates [../maps/packages.md](../maps/packages.md), `--check` in CI) and the `no_hoocode.sh` guard | [naming-and-paths.md](naming-and-paths.md) §1 | S | 0c | Keeps the maps and names honest from here on |
| 1 | Reliability, in this order: **1.1** rpc fails closed (security); **1.2** `~/.hoocode` paths, `HOOCODE_` env, one-time merge; **1.3** macOS test fixes and CI job; **1.4** `@file` without `fd`; **1.5** panic audit; **1.6** fuzzing | [reliability.md](reliability.md), [naming-and-paths.md](naming-and-paths.md) §2–4 | L | 0c | Reliability before features |
| 2 | Concurrency phases 0–1: `/perf` and the load test, then one runtime and the caps | [concurrency.md](concurrency.md) | M | 1 | Sets the runtime rules before MCP brings rmcp |
| 3 | `DocSearch`; close the 8 `l1_done` tasks | [semantic-search.md](semantic-search.md) part A | S | 2 | Small; finishes the migration's loose ends |
| 4 | MCP client on rmcp; then a short guide for the webtools MCP server | [mcp.md](mcp.md), [web-tools.md](web-tools.md) | L | 2 | The main missing capability |
| 5 | Concurrency phases 2–5: terminal-output thread, session writer, lanes and priority, watchdog and memory limits | [concurrency.md](concurrency.md) | L | 4 | Built against the real MCP and tool load |
| 6 | Plugins: load, install, opt-in model tools | [plugins.md](plugins.md) | L | 4 | Delivers skills, MCP servers and subagents |
| 7 | Scheduler and `/loop` | [scheduler-and-loop.md](scheduler-and-loop.md) | M | 1 | Independent; after the core |
| 8 | Version check and completion chime | [extras.md](extras.md) | S | 0c | Small polish |

**Status (2026-10-08):** milestone 2 (concurrency phases 0-1) is done. Milestone 3 (DocSearch;
the 8 `l1_done` tasks) is done. Milestones 4 (MCP client) and 5 (concurrency phases 2-5) are
built, not yet load-tested. Open items are listed in the 2026-10-08 (second session) entry in
[../../migration/PROGRESS.md](../../migration/PROGRESS.md).

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
| [scoped-models.md](scoped-models.md) | `/scoped-models` gets an effort and a category per model; `scopedModels` replaces `enabledModels` and `modelCategories`; subagents ask by category or by model | Locked 2026-10-09 (reviewed). Not implemented. |

Older docs that stay as they are: [app-server.md](app-server.md), [subagents.md](subagents.md),
[subagent-evals.md](subagent-evals.md), [distribution.md](distribution.md),
[build-speed.md](build-speed.md), and the migration plan
[ts-to-rust-migration.md](ts-to-rust-migration.md).

## Standards checked (2026-10-07)

| Standard | State | Used in |
|---|---|---|
| MCP (AAIF) | Spec `2026-07-28`: stateless, `server/discover`, multi round-trip input, extensions. HTTP+SSE, Roots, Sampling, Logging and Dynamic Client Registration deprecated. | mcp.md |
| MCP Apps, Tasks, Skills extensions (AAIF) | Official | Not used for now: Apps and Tasks deferred, Skills dropped (2026-10-08) |
| Agent Plugins 1.0.0 (agent-plugins.org; Amazon, Cursor, Google, Microsoft, OpenAI, Vercel) | Published 2026-08-06: `plugin.json`, `skills/`, `mcp.json` | plugins.md |
| Agent Skills (agentskills.io) | `SKILL.md` frontmatter rules | plugins.md |
| AGENTS.md (AAIF) | Already supported | — |
| A2A (AAIF) | v1.0; no local or stdio story | not used |

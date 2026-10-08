# Design docs

Start here. The migration from hoocode-ts is finished at the v0.6.0 pin. Everything
still to build is planned in the cards below. Each card was agreed with the user on
2026-10-07 or later. The dated decision pages win if a card disagrees:
[decisions-2026-10-07.md](decisions-2026-10-07.md) and
[decisions-2026-10-08.md](decisions-2026-10-08.md) (the later page adds to the
earlier one).

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

## Cards, in build order

| # | Card | Status | Replaces ledger task |
|---|---|---|---|
| 1 | [reliability.md](reliability.md) | Agreed; build next | — |
| 1b | [concurrency.md](concurrency.md) phases 0–1 (measure; one runtime and caps) | Agreed 2026-10-08 | — |
| 2 | [semantic-search.md](semantic-search.md) part A (`SearchHooCode`); part B dropped | Agreed | closes 10.2a/b/c/d/f/g, 10.4c, 10.5; part of 12.4 |
| 3 | [mcp.md](mcp.md) | Agreed; scope cut 2026-10-08 | 9.1, 10.11 |
| 3b | [concurrency.md](concurrency.md) phases 2–5 (terminal output, session writer, lanes, watchdog, memory limits) | Agreed 2026-10-08 | — |
| 4 | [plugins.md](plugins.md) | Agreed; scope cut 2026-10-08 | 12.1, 12.2 |
| 5 | [scheduler-and-loop.md](scheduler-and-loop.md) | Agreed | 12.5 |
| — | [canvas-and-mcp-apps.md](canvas-and-mcp-apps.md) | MCP Apps deferred; Copilot canvas dropped (2026-10-08) | 12.7 canvas |
| — | [extras.md](extras.md) | Version check and chime yes; rest no | 12.6, 12.7 rest |
| — | [web-tools.md](web-tools.md) | Deferred | 10.2e |
| — | [extension-runtime.md](extension-runtime.md) | Standards only; WASM crate deleted | 12.3 |

Older docs that stay as they are: [naming-and-paths.md](naming-and-paths.md)
(steps 1–2 are part of card 1), [rpc-approvals.md](rpc-approvals.md) (dropped
2026-10-08), [app-server.md](app-server.md), [subagents.md](subagents.md),
[subagent-evals.md](subagent-evals.md), [distribution.md](distribution.md),
[build-speed.md](build-speed.md), and the migration plan
[hoocode-to-cortexcode-migration.md](hoocode-to-cortexcode-migration.md).

## First step when coding starts

Close the migration ledger:

- add a `moved` status to `migration/ledger.py`;
- mark 9.1, 10.2e, 10.11, 12.1–12.7 and 13.4 `moved`, each with a note naming its
  card;
- add one line each to plan §0.3 and `migration/PROGRESS.md` pointing here;
- delete the 11 crates listed in [decisions-2026-10-08.md](decisions-2026-10-08.md)
  (move `cortexcode-ai`'s tests to `ai-registry` first), then update
  `migration/dep-firewall.json`, `scripts/generate_crates.sh`, CI and release
  scripts that name them;
- add `scripts/maps/packages.py` (regenerates [../maps/packages.md](../maps/packages.md)
  from `cargo metadata`; `--check` in CI).

This is tooling and bookkeeping, so it happens in the first coding session, not in a
design session.

## Standards checked (2026-10-07)

| Standard | State | Used in |
|---|---|---|
| MCP (AAIF) | Spec `2026-07-28`: stateless, `server/discover`, multi round-trip input, extensions. HTTP+SSE, Roots, Sampling, Logging and Dynamic Client Registration deprecated. | mcp.md |
| MCP Apps, Tasks, Skills extensions (AAIF) | Official | Not used for now: Apps and Tasks deferred, Skills dropped (2026-10-08) |
| Agent Plugins 1.0.0 (agent-plugins.org; Amazon, Cursor, Google, Microsoft, OpenAI, Vercel) | Published 2026-08-06: `plugin.json`, `skills/`, `mcp.json` | plugins.md |
| Agent Skills (agentskills.io) | `SKILL.md` frontmatter rules | plugins.md |
| AGENTS.md (AAIF) | Already supported | — |
| A2A (AAIF) | v1.0; no local or stdio story | not used |

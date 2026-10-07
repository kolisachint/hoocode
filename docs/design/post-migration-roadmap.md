# After the migration: where the deferred work goes

Status: **draft for review, 2026-10-07.** Design only, no code. This is the index for
the design docs that replace the migration ledger's deferred tasks. The user reviews
and finalises each doc before any code changes.

## Why this exists

The migration is closed at the pin (hoocode v0.6.0, `2223437c`; `pin_drift.py status`
shows the pin is upstream's latest). Phases 7, 8 and 11 are complete, and phase 10
is complete except for the eight `l1_done` tasks below. Everything still open was
`deferred` on 2026-10-01 (plan §0.3).

From here on, deferred work is **product work, not porting.** Three things change:

1. **No Level-2 parity gate.** The rendered side-by-side check against hoocode-ts
   applied to porting. New work has its own tests and its own "done" (see each doc).
   The TypeScript code and docs stay the reference *for behaviour we choose to
   keep*. Each doc says where it follows TS and where it deliberately does not.
2. **Open standards come first.** Where the Agentic AI Foundation (AAIF) or an
   AAIF project defines a format, we implement the standard and treat the TS
   behaviour as one consumer of it (§3).
3. **One design doc per area.** Each doc is reviewed before work starts. A doc's
   phases become the work list. The ledger isn't extended.

## 1. Ledger task → design doc

| Ledger task | Area | Doc |
|---|---|---|
| 9.1 MCP on rmcp | MCP client: transports, auth, protocol versions | [mcp.md](mcp.md) |
| 10.11 code-mcp | MCP discovery, deferred schemas, `/mcp` status | [mcp.md](mcp.md) |
| 10.2e webfetch + websearch | Web tools | [web-tools.md](web-tools.md) |
| 12.1 plugins + marketplace | Plugin consumption, marketplaces, hooks bridge | [plugins.md](plugins.md) |
| 12.2 package manager | `packages` setting, git/local sources | [plugins.md](plugins.md) §7 |
| 12.3 code extensions | Running third-party extension code | [extension-runtime.md](extension-runtime.md) |
| 12.4 semantic search + capability retrieval | `SearchCodebase` semantic/hybrid, capability index, `SearchHooCode` | [semantic-search.md](semantic-search.md) |
| 12.5 scheduler, `/loop`, Cron tools | Scheduled and autonomous prompts | [scheduler-and-loop.md](scheduler-and-loop.md) |
| 12.6 warm subagent pool | Already built by 10.9c | [extras.md](extras.md) §1 |
| 12.7 canvas | Rich surfaces: MCP Apps and Copilot canvases | [canvas-and-mcp-apps.md](canvas-and-mcp-apps.md) |
| 12.7 the rest | `/learn`, teams, voice, export-html/`/share`, telemetry, version check, thinking escalation | [extras.md](extras.md) |
| 13.4 TS test port ledger | Bookkeeping | §5 below |

Already designed elsewhere and not repeated here: [naming-and-paths.md](naming-and-paths.md)
(data directories), [rpc-approvals.md](rpc-approvals.md) (approval dialogs over
`--mode rpc`), [app-server.md](app-server.md) leftovers, [distribution.md](distribution.md)
leftovers, [build-speed.md](build-speed.md) D5–D12.

### The eight `l1_done` tasks

10.2a/b/c/d/f/g, 10.4c and 10.5 pass their ported tests. They miss Level 2 only
because the default-bundle system prompt at the pin contains the `SearchHooCode` tool
block (12.4). [semantic-search.md](semantic-search.md) §4 registers `SearchHooCode`
in its first phase, which removes that difference, and those tasks can then be closed
against the scenarios as written. **Decision R1** (§6) covers the alternative: close
them now and accept the one-block difference.

## 2. Principles for all eight docs

1. **Standards before vendor formats before hoocode formats.** Read every format we
   can. Write the standard one by default.
2. **Consume before produce.** Loading other people's MCP servers, skills, plugins
   and canvases comes first. Authoring and publishing come later, or not at all.
3. **Third-party code runs out of process.** MCP servers, hook commands, extension
   hosts and canvases are child processes, never loaded into `cortex`. The permission
   gate and the workspace trust gate are the controls, and docs never claim a safety
   property the runtime doesn't enforce.
4. **No new always-on token cost.** Every tool schema is re-sent on every request.
   A feature that adds tools says what it costs, and costs nothing when unused (the
   canvas tools are the model: zero schemas until something is open).
5. **Licences.** MIT only for copied code (CLAUDE.md). Depending on a crate under a
   permissive licence is not copying, and the workspace already depends on
   Apache-2.0-only crates (wasmtime/cranelift). New Apache-2.0-only dependencies
   (rmcp) are still named in their doc as a decision for the user.
6. **Your own crates are fair game.** `kolisachint/webtools`, `embeddingsearchtools`,
   `voicetools` and `hooteams` are MIT and owned by the user. Depending on them as
   libraries or binaries is a design choice, not a licence question.
7. **Shared state with hoocode-ts.** Both builds read `.agents/` (plugins,
   marketplaces, `scheduled_tasks.json`, `mcp.json`). Files there keep the TS shape,
   so the two tools interoperate, and writes are atomic (write then rename).
   Private state follows [naming-and-paths.md](naming-and-paths.md) (`~/.hoocode/rust/`).

## 3. Open standards baseline (checked 2026-10-07)

AAIF (the Linux Foundation's Agentic AI Foundation, aaif.io) hosts six projects.
What each means for us:

| Project | Version / state | Where it lands |
|---|---|---|
| **MCP** | Spec `2026-07-28`: stateless core (no `initialize`; per-request `_meta` carries version and capabilities), `server/discover`, `subscriptions/listen`, multi round-trip requests (`input_required`), required `resultType`, cacheable list results (`ttlMs`). HTTP+SSE, Roots, Sampling and Logging deprecated. DCR deprecated in favour of Client ID Metadata Documents. | [mcp.md](mcp.md) |
| MCP extensions | Official: **Tasks** (`io.modelcontextprotocol/tasks`), **MCP Apps** (UI in sandboxed iframes, `ui://` resources), **Skills** (`io.modelcontextprotocol/skills`, SEP-2640 Final 2026-09-13), Enterprise-managed auth, client credentials | [mcp.md](mcp.md) §6, [canvas-and-mcp-apps.md](canvas-and-mcp-apps.md), [plugins.md](plugins.md) §5 |
| MCP Registry | Preview. `server.json` (reverse-DNS names, packages for npm/pypi/oci/mcpb, remotes), OpenAPI for sub-registries. Hosts should consume sub-registries, not the central one. | [mcp.md](mcp.md) §8 (later) |
| **AGENTS.md** | Plain Markdown, nearest file wins, user prompt overrides. Already read (TS parity). | No new work; [extras.md](extras.md) notes the project-level `.agents/AGENTS.md` gap |
| **Agent Skills** (agentskills.io; referenced by the MCP Skills extension) | `SKILL.md` frontmatter: `name` (1–64 chars, `a-z0-9-`, must match its directory), `description` (≤1024), optional `license`, `compatibility`, `metadata`, `allowed-tools` (experimental) | Skills loading already exists; [plugins.md](plugins.md) §5 adds validation and MCP-served skills |
| **A2A** | v1.0.0: Agent Card at `/.well-known/agent-card.json`, JSON-RPC/gRPC/REST bindings, task states. No stdio/local guidance. | [extras.md](extras.md) §7 (teams). Not adopted now. |
| **goose** | Agent framework (Block). Extensions are MCP servers. | Reference only: confirms "extension = MCP server" ([extension-runtime.md](extension-runtime.md), "What the standards say") |
| agentgateway, Agent Router | Gateways between agents, models and MCP tools | No work. They work with us unchanged if we speak standard MCP over HTTP. |

## 4. Order (proposal)

Grouped by dependency. Each wave can start once the previous wave's docs are signed
off. Waves aren't timeboxed.

| Wave | Work | Why this order |
|---|---|---|
| **A** | MCP client (mcp.md, phases 1–3); web tools (web-tools.md) | The two biggest user-visible gaps against hoocode-ts. MCP is also the substrate for plugins, extensions and MCP Apps. |
| **B** | Capability index + `SearchHooCode` (semantic-search.md phase 1); closes the 8 `l1_done` tasks | Small. Unblocks deferred MCP schemas and `SearchPlugins` retrieval. |
| **C** | Plugin consumption + marketplaces + hooks bridge (plugins.md, phases 1–3) | Needs MCP (plugin `mcpServers`) and the capability index (`SearchPlugins`). |
| **D** | Scheduler and `/loop` (scheduler-and-loop.md) | Independent. Can run in parallel with C. |
| **E** | Copilot canvas host, then MCP Apps (canvas-and-mcp-apps.md, Decision C1); extension host if wanted (extension-runtime.md option B) | Both need a Node launcher; build it once. |
| **F** | Semantic `SearchCodebase` via embsearch (semantic-search.md §3); extras by individual go/no-go | Optional binaries; lowest demand |

## 5. Closing the ledger

- Each deferred task gets a final `note` pointing at its doc, and its status changes
  from `deferred` to a new terminal status, **`moved`**, meaning "left the migration,
  tracked in a design doc". `ledger.py check` and `status` need that status added.
  The alternative is to leave them `deferred` forever, which reads as unfinished
  migration work.
- **13.4** closes as `moved` too. `migration/ts-tests.json` keeps its 94 `pending`
  files. Each design doc lists the TS test files its area owns, as a reading list for
  behaviour, not as a port obligation. `ts_tests.py status` stays as it is.
- The eight `l1_done` tasks close as `done` once wave B makes their scenarios pass
  (or under Decision R1).
- Plan §0.3 and `PROGRESS.md` get one line each pointing here.

## 6. Decisions for the user (cross-cutting)

| # | Question | Recommendation |
|---|---|---|
| R1 | Close the 8 `l1_done` tasks now, accepting the `SearchHooCode` prompt difference, or after wave B? | After wave B. It is small and gives a real parity pass. |
| R2 | New ledger status `moved` for deferred tasks, or leave them `deferred`? | `moved` |
| R3 | Is the wave order in §4 right? In particular, MCP before web tools, or together? | Together: they don't share code. |
| R4 | Which 12.7 extras are wanted at all? | See [extras.md](extras.md) §0. Recommended: thinking escalation, version check and chime yes; `/learn`, teams and voice later; HTML export and `/share` no. |

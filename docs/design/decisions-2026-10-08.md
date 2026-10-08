# Decisions, 2026-10-08

Made with the user in three reviews: [concurrency.md](concurrency.md), a scope review
of the whole plan, and naming ([naming-and-paths.md](naming-and-paths.md)). Where a card disagrees with this page, this page wins.
[decisions-2026-10-07.md](decisions-2026-10-07.md) still stands except where the
"Replaces" column below says otherwise.

## Order of work (updated)

0. **First coding session (bookkeeping):** close the migration ledger (README),
   delete the 11 crates below, then rename everything to hoocode in one mechanical
   commit ([naming-and-paths.md](naming-and-paths.md) §1), then add the package-map
   generator.
1. **Reliability first** ([reliability.md](reliability.md)), unchanged.
2. **Concurrency phases 0–1**: measure, then one runtime and the caps.
3. **Close the 8 `l1_done` tasks** (`SearchHooCode`), unchanged.
4. **MCP client.**
5. **Concurrency phases 2–5**: terminal-output thread, session writer, lanes and
   priority, watchdog and memory limits.
6. **Plugins.**
7. **Scheduler.**

MCP Apps moved out of the order (deferred, below). Phase 6 of concurrency (moving
syntax highlighting off the UI thread) happens only if Phase 0's numbers show it is
needed.

## Concurrency

| Question | Decision |
|---|---|
| Thread model | **Lanes**, not a thread per subsystem. Dedicated threads for UI, input, terminal output, session writer and watchdog; one tokio runtime of 2–4 workers for I/O; a capped pool for tools; subagents stay processes. |
| Build order | **Split**: phases 0–1 before MCP, phases 2–5 after it. |
| "gcc separate" | Meant **compaction and cleanup**. Compaction runs on `cortex-io` and `cortex-tools`; cleanup runs in the Low lane. No GC thread (Rust has no GC). |
| Priority of the agent's `bash` commands | **Normal (nice 0)**, lowered with `performance.bashNice`. |
| Memory limits | **On by default**: soft at the lower of 2 GiB and 25% of RAM, hard at the lower of 4 GiB and 50% of RAM. |
| Settings | **Four keys** in a `performance` block: `maxParallelTools`, `memorySoftLimitMb`, `memoryHardLimitMb`, `bashNice`. Every other cap is fixed in code. |
| Background compaction while typing | **Later, in its own card.** Not part of concurrency. |
| Performance numbers | **`/perf`** command and **`--perf-log <file>`**. |

## Scope cuts

Rule: keep what the standards cover and what is used; drop vendor-only extras and
code nothing calls.

| Area | Decision | Replaces (2026-10-07) |
|---|---|---|
| Copilot canvas host | **Dropped.** `drawio-canvas` works through its MCP mode. | "Copilot canvases later" |
| MCP Apps | **Deferred.** Revisit when a tool you use needs a browser UI. | "MCP Apps first", step 5 of the order |
| Plugin authoring | **The model writes Agent Plugins packages only.** cortex still **reads** Agent Plugins and Claude plugins, and Claude's `marketplace.json`, so the official Anthropic directory installs. | "Authoring format" kept; marketplace "read and written" becomes **read only** |
| Plugin model tools | `SearchPlugins`, `InstallPlugin`, `UninstallPlugin`, `ListPlugins`, `ProposePlugin`. **Dropped:** `UpdatePlugin` (you run `/plugin update`), `PackagePlugin`, `PublishPlugin` (publishing writes Claude's marketplace format). | "Full lifecycle" |
| Skills over MCP (MCP Skills extension) | **Dropped.** | plugins.md item 4 |
| MCP Tasks extension | **Deferred.** Normal calls with progress and cancel cover today's servers. | mcp.md item 4 |
| `/mcp import` | **Dropped.** Copy entries into `~/.agents/mcp.json` by hand. | mcp.md M6 |
| WASM crate `code-extensions` | **Deleted**, not feature-flagged. | "off-by-default feature" |
| RPC approval dialogs | **Dropped.** hoobot uses the app-server; rpc fails closed (reliability.md). | rpc-approvals.md |
| Semantic search part B (`embsearch`) | **Dropped.** Lexical `SearchCodebase` and `SearchHooCode` stay. | semantic-search.md part B |
| crates.io publishing | **Dropped.** Users get release archives, npm and the curl installer. | distribution.md "Deferred" |
| Small crates | **Not merged.** The layout in build-speed.md stays. | — |

### Crates to delete (11: 76 → 65)

| Crate | Why |
|---|---|
| `cortexcode`, `cortexcode-agent`, `cortexcode-ai`, `cortexcode-code`, `cortexcode-tui` | Umbrellas, only for crates.io. `cortexcode-ai`'s cross-provider tests move to `ai-registry`. |
| `agent-mcp` | Not wired into the binary; mcp.md rebuilds MCP on rmcp |
| `agent-orchestrator`, `agent-tools` | SDK ports the product never calls |
| `ai-images` | No tool or command uses image generation |
| `code-extensions` | Unused WASM host |
| `ai-provider-azure` | Not used |

Kept after review: Gemini CLI and Antigravity logins, GitHub Copilot login, ChatGPT
(Codex) login, and `ai-provider-faux` (the test provider).

### Maps

[docs/maps/packages.md](../maps/packages.md) and [docs/maps/ui.md](../maps/ui.md)
are written now and kept current in the same commit as any crate, screen or command
change. The first coding session adds `scripts/maps/packages.py`, which regenerates
the package tables from `cargo metadata`, with a `--check` mode in CI.

## Naming and paths

hoocode (Rust) is a drop-in replacement for hoocode-ts. Card:
[naming-and-paths.md](naming-and-paths.md), which replaces its 2026-10-01 version.

| Question | Decision | Replaces |
|---|---|---|
| Data folder | **`~/.hoocode` and `<repo>/.hoocode/`, shared with hoocode-ts** | `~/.hoocode/rust/` (2026-10-01) |
| Existing `~/.cortexcode` data | **Full merge**, once, with backups | Copy into `rust/` with a marker |
| Conflicts in the merge | **`~/.cortexcode` wins** | — |
| Project `.cortexcode/` folders | **Auto-merged** into `.hoocode/` (not `dispatch/`) | Read as fallback, with a notice |
| What is renamed | **Everything**: crates, Rust paths, the cargo binary, `APP_NAME`, the prompt, scripts, CI | "Crates and the `cortex` binary keep their names" (CLAUDE.md, 2026-10-01) |
| Env variables | **`HOOCODE_` only**, no aliases | Deprecated aliases for a release |
| Docs | **Rewrite all**, history included; migration plan → `ts-to-rust-migration.md` | — |

Accepted risks: after the first merge hoocode-ts runs with the Rust settings and
logins (backups undo it); the project merge writes into git working trees; rewritten
docs hide the old names (git keeps them).

## fd and rg

| Question | Decision |
|---|---|
| `rg` | **Dropped.** Nothing runs it; search links ripgrep's libraries (`grep-*`, `ignore`). |
| `fd` | **Dropped.** `@file` autocomplete walks in process with the `ignore` crate (fd's own walker) using fd's rules, off the UI thread. Today it suggests nothing when `fd` is missing, and it blocks typing while `fd` runs. |
| The external-tools layer | **Removed**: the `external_tools` table and its `/settings` pane, `bin_dir()`, and `HOOCODE_RG_BINARY`, `HOOCODE_FD_BINARY`, `HOOCODE_NATIVE_SEARCH`. Its other rows are gone too: `embsearch` dropped, `webtools` comes through MCP, `voicetools` dropped. The parity harness masks the `/settings` difference. |

Built as item 6 of [reliability.md](reliability.md).

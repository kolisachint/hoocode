# Loop & Plugin System — Design

Status: **partial implementation landed** (minimum scope). This document is the
reference for what shipped and what is intentionally deferred, so the rest can be
picked up later without re-deriving the design.

Two independent subsystems are covered:

1. **Loop** — provider-agnostic agent iteration plus a user-facing `/loop` command.
2. **Plugin** — a packaging/discovery layer that loads capability bundles from a
   manifest, supporting both `.agents-plugin/plugin.json` (native) and
   `.claude-plugin/plugin.json` (Claude Code compatible).

Both are designed to be **general, not Claude-specific** — Claude is one supported
provider/format among others.

---

## 1. Loop

### 1.1 Background: the loop is already provider-agnostic

The model→tool→repeat cycle lives in the agent core (`packages/agent`). Every
provider normalizes its native stop signals into one `StopReason` union
(`stop` / `length` / `toolUse` / `aborted` / `error`) — e.g. Anthropic's
`end_turn`/`tool_use`/`max_tokens` are collapsed in the provider's `mapStopReason`.
The loop continues while `toolUse`. Claude, OpenAI, Google all feed the same loop;
nothing in the loop is Claude-aware. **Keep it that way:** loop logic must read only
the normalized `StopReason` + turn state, never raw provider payloads.

### 1.2 Shipped: cron scheduler + `/loop` + Cron* tools

The loop is backed by a real cron scheduler (`core/scheduler.ts`, `TaskScheduler`),
modeled on the harness `CronCreate`/`CronList`/`CronDelete` tools: 5-field cron in
local time, recurring vs one-shot, durable persistence to
`.agents/scheduled_tasks.json` (the primary, cross-vendor home; a legacy
`.hoocode/scheduled_tasks.json` is read once and migrates forward on the next
persist), and **idle-gated** firing (a due task never interrupts an in-flight
turn; it fires on a later tick within the same minute). Set up in `hoo-core.ts`
(`setupLoop`).

**Agent-callable tools** (the model can schedule its own follow-ups):

- `CronCreate({ cron, prompt, recurring? })`, `CronList()`, `CronDelete({ id })`.

**`/loop` command** (the user can too):

```
/loop "<cron>" <prompt>        schedule recurring (5-field cron, local time)
/loop <5m|2h|1d> <prompt>      schedule recurring at a simple interval
/loop once "<cron>" <prompt>   schedule a one-shot
/loop list | /loop delete <id> | /loop stop
/loop auto [--max-turns N] <task>   autonomous continuation (see 1.3)
```

The scheduler is created on `session_start` (bound to `sendUserMessage` +
`ctx.isIdle`), ticks every 30s, and is cleared on `session_shutdown`. Sub-minute
intervals are not expressible in cron (1-minute granularity); use `1m` minimum.

### 1.3 Shipped: autonomous continuation (`/loop auto`)

`/loop auto [--max-turns N] <task>` keeps the agent iterating instead of yielding:
the task is kicked off as a user message, and on each `agent_end` the loop
re-prompts ("continue…") until the model emits the `LOOP_DONE` token or the
turn budget (default 10) is exhausted. It yields if the user is steering
(`ctx.hasPendingMessages()`), and `/loop stop` cancels it.

Implemented at the **extension layer** (an `agent_end` re-prompt with a turn
budget) rather than as a harness-level `LoopPolicy`. A deeper controller in
`packages/agent` — a `shouldContinue(turnState)` predicate plus wall-clock/cost
budgets reading the normalized `StopReason` — remains a possible future
refinement, but the extension-level loop covers the autonomous-iteration need
without modifying the core turn loop.

---

## 2. Plugin system

### 2.1 What a plugin is

A directory that bundles agent capabilities behind a manifest. Discovered under
`plugins/` folders, **highest precedence first** (`discoverPlugins` is first-wins
by id, so `.agents/` wins over `.hoocode/` and project wins over global):

1. project `<cwd>/.agents/plugins/` — primary, cross-vendor home
2. project `<cwd>/.hoocode/plugins/` — legacy/fallback
3. global `~/.agents/plugins/` — primary, cross-vendor home
4. global `<agentDir>/plugins/` (i.e. `~/.hoocode/plugins/`) — legacy/fallback

See `defaultPluginDirs()` in `loader.ts`. `.agents/` is preferred everywhere per
the "`.agents/` first" policy (mapping doc: `plugin-format-mapping.md`); `.hoocode/`
stays supported so hand-placed plugins keep loading.

A directory is a plugin if it contains **either**:

- `.agents-plugin/plugin.json` — native format, and
- `.claude-plugin/plugin.json` — Claude Code compatible format.

When both are present, **native wins (no merge)**.

### 2.2 Architecture: normalizer + synthetic factory

```
plugins/manifest.ts      parse either format → NormalizedPlugin
plugins/hooks-bridge.ts  NormalizedPlugin.hooks → ExtensionAPI handlers (shell protocol)
plugins/index.ts         discoverPlugins() + buildPluginFactory() → ExtensionFactory
loader.ts                loadPlugins() loads each factory into a runtime/event bus
```

A plugin becomes a synthetic `ExtensionFactory`, so it rides the **existing** extension
loader/runner — plugins are "extensions assembled from a manifest instead of code."

`loadPlugins()` is invoked from **both** extension-loading entry points so plugins
load regardless of how the app was started:

- `ResourceLoader.reload()` — the path the interactive/print app uses.
- `discoverAndLoadExtensions()` — the standalone/SDK path.

Both load into the same runtime as ordinary extensions, after the regular
extensions and inline factories.

### 2.3 Capability mapping

| Manifest field / dir | hoocode target | Status |
|---|---|---|
| `name` / `version` / `description` / `author` | plugin identity metadata | ✅ shipped |
| `skills/` | `resources_discover` → `skillPaths` | ✅ shipped |
| `commands/` | `resources_discover` → `slashCommandPaths` (the `.agents/commands` surface) | ✅ shipped |
| `themes/` | `resources_discover` → `themePaths` | ✅ shipped |
| `agents/` | `resources_discover` → `agentPaths` (the `.agents/agents` subagent surface) | ✅ shipped |
| `providers` (native only) | `registerProvider` | ✅ shipped |
| `hooks` / `hooks/hooks.json` | shell-protocol bridge | ✅ shipped (events below) |
| `mcpServers` / `.mcp.json` | MCP server connect | ✅ shipped (registry below) |

The `resources_discover` event contract was extended with `slashCommandPaths` and
`agentPaths` (mirroring the existing `skillPaths`/`promptPaths`/`themePaths` fields)
so plugin `commands/` and `agents/` flow into the same loaders that handle hoocode's
native `.agents/commands` (slash commands) and `.agents/agents` (subagents). Agent
directories are expanded to their `.md` files for the agent registry's manifest-path
source; slash commands share the `/name` namespace with prompt templates.

**MCP wiring.** MCP connection happens in hoo-core's `setupMcpLoader` on
`session_start`, reading from fixed file locations. Plugins bridge to it via a
process-global registry (`core/extension-mcp-servers.ts`, mirroring
`agent-manifest-paths.ts`): the plugin factory registers its `mcpServers` during
load, and `setupMcpLoader` reads them (deduping by name) when connecting.
`loadPlugins` clears the registry before each (re)load so reloads rebuild the set
cleanly. `${CLAUDE_PLUGIN_ROOT}` / `${AGENTS_PLUGIN_ROOT}` in commands, args, and env
values are substituted with the plugin root.

### 2.4 Hooks bridge (true parity)

Claude Code hooks are shell commands wired to events and matched by tool name; hoocode
hooks are TS handlers on the `ExtensionEvent` union. The bridge registers handlers that
shell out per the hook protocol and translate the result back.

**Protocol (faithful to Claude Code):**

- Input: a JSON object on stdin describing the event.
- Exit `0`: success. stdout may carry a JSON decision; for prompt/session events plain
  stdout is treated as additional context.
- Exit `2`: blocking error. stderr (or JSON `reason`) is the block reason.
- Other non-zero: non-blocking error (kept off the model's path).
- Optional stdout JSON: `{ decision: "block"|"approve", reason, permissionDecision }`.
- `CLAUDE_PLUGIN_ROOT` and `AGENTS_PLUGIN_ROOT` env vars are set to the plugin root.

**Event mapping (shipped):**

| Claude Code event | hoocode event | Behavior |
|---|---|---|
| `PreToolUse` | `tool_call` | matcher vs tool name; exit 2 / `decision:block`/`deny` → blocks the tool |
| `PostToolUse` | `tool_result` | matcher vs tool name; exit 2 / `decision:block` → appends reason, marks error |
| `UserPromptSubmit` | `before_agent_start` | stdout / `reason` appended to the system prompt as context |
| `SessionStart` | `session_start` | side effects |
| `Stop` | `agent_end` | side effects |

Matcher semantics: empty / `"*"` matches all; otherwise an anchored regex on the tool
name (falls back to exact match on invalid regex).

**Deferred hook fidelity:** `Notification`, `SubagentStop`, `PreCompact`, and the newer
`hookSpecificOutput` / `continue` / `systemMessage` fields are not yet mapped.

### 2.5 Native manifest (`.agents-plugin/plugin.json`)

A strict superset of the Claude manifest, so any Claude plugin (minus native extras) is
a valid native plugin. Native-only additions:

- `providers: [{ name, config }]` — registered via `registerProvider` (config-only;
  uses a built-in API handler such as `anthropic-messages`).
- (Future) `extensions: string[]` — TS extension files loaded directly, for typed tools
  and arbitrary event handlers beyond the shell-hook protocol.

Example:

```json
{
  "name": "my-plugin",
  "version": "1.0.0",
  "description": "Example native plugin",
  "providers": [
    {
      "name": "my-proxy",
      "config": {
        "baseUrl": "https://proxy.example.com",
        "apiKey": "PROXY_API_KEY",
        "api": "anthropic-messages",
        "models": [
          { "id": "claude-sonnet-4-5", "name": "Claude Sonnet 4.5 (proxy)",
            "reasoning": false, "input": ["text"], "contextWindow": 200000, "maxTokens": 16384,
            "cost": { "input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0 } }
        ]
      }
    }
  ]
}
```

### 2.6 Shipped: marketplaces (install from a git index)

A **marketplace** is a git repo (or local dir) with an index manifest listing
installable plugins. Three formats are supported (native wins, then Claude, then
Copilot when more than one is present):

- Native: `.agents-plugin/marketplace.json` (preferred; the `.agents` surface)
- Claude: `.claude-plugin/marketplace.json`
- Copilot-style git index: `.github/marketplace.json`

All three use `{ name?, owner?, plugins: [{ name, source, description? }] }`. A plugin
`source` is a path relative to the marketplace root, a git URL, or `npm:<spec>`.

Parsing/resolution lives in `core/extensions/plugins/marketplace.ts`
(`parseMarketplaceDir`, `resolvePluginSource`, and a persisted registry at
`.hoocode/marketplaces.json`). Commands (`setupMarketplace` in `hoo-core.ts`):

```
/plugin marketplace add <git-url|path>   clone/read + register a marketplace
/plugin marketplace list
/plugin list                             available plugins across marketplaces
/plugin install <name>                   clone/copy into .hoocode/plugins/<name>, then reload
/plugin remove <name>                    remove + reload
```

Install copies a local plugin or `git clone`s a git source into
`.agents/plugins/<name>` (the primary, cross-vendor home) and reloads, so the
plugin loads through the same loader as any other plugin. The added-marketplace
registry lives at `.agents/marketplaces.json` (falling back to the legacy
`.hoocode/marketplaces.json` when the `.agents` one is absent), and clones are
cached under `.agents/marketplace-cache/`. `remove` deletes from `.agents/plugins/`
first and the legacy `.hoocode/plugins/` second. `npm:` sources are recognized but
not yet installed (deferred). Version/dependency resolution and update/pin are
future work.

### 2.7 Shipped: single-turn capability loop (search → install → use, live)

The model-facing lifecycle tools close the loop **within one turn**:

1. `SearchPlugins` queries all registered marketplaces. Curated **well-known
   marketplaces** (`WELL_KNOWN_MARKETPLACES` in
   `core/extensions/plugins/install.ts`, currently the official
   `anthropics/claude-plugins-official` directory, 250+ plugins) are cloned
   lazily into `.agents/marketplace-cache/` on first search — trusted out of the
   box (shipping an entry is a maintainer-level trust decision), never
   auto-updated, offline degrades gracefully.
2. `InstallPlugin` installs from a registered marketplace, then calls
   `AgentSession.activatePlugin(dest)` (exposed to tools as
   `ctx.activatePlugin`): passive capabilities — skills, slash commands,
   subagents, themes — register in the **live** session via
   `ResourceLoader.extendResources()` plus a system-prompt rebuild.
3. The rebuilt context reaches the model **mid-run**: `AgentSession` marks its
   runtime context dirty and serves the agent loop's `prepareNextTurn` hook, so
   the very next provider request in the same run carries the new system prompt
   and tool set. (This also makes `ResolveMcpTools`-resolved deferred MCP
   schemas callable in the same turn — previously the loop context was frozen
   at run start.)
4. Executable capabilities (hooks, MCP servers, providers) cannot be wired into
   a streaming run; when a plugin bundles them, an automatic full reload runs
   once the session goes idle (end of the current turn) — still autonomous.
   `UninstallPlugin` and `ProposePlugin` use the same machinery
   (`ctx.requestReloadWhenIdle`, live activation of authored plugins).

MCP tool schemas are **deferred by default** (`deferMcpSchemas: true`): names
only in context, full schema materialized on demand via `ResolveMcpTools`.

End-to-end coverage: `test/plugin-e2e-official.test.ts` clones the real
official marketplace, installs `skill-creator`, asserts the skill is in the
live system prompt and in the `prepareNextTurn` refresh, reads the skill body
(the "use" act), then uninstalls. `packages/agent`'s
`test/prepare-next-turn-refresh.test.ts` proves the mid-run context swap at the
loop level.

---

## File map

| File | Purpose |
|---|---|
| `src/core/extensions/plugins/manifest.ts` | manifest types + parse/normalize both formats |
| `src/core/extensions/plugins/hooks-bridge.ts` | shell-protocol hooks bridge |
| `src/core/extensions/plugins/index.ts` | discovery + synthetic factory builder (incl. MCP registration) |
| `src/core/extensions/loader.ts` | `loadPlugins` / `defaultPluginDirs`; plugin loading in `discoverAndLoadExtensions` |
| `src/core/resource-loader.ts` | loads plugins in `reload()` (the app path); routes agent/command paths |
| `src/core/extension-mcp-servers.ts` | process-global registry of extension/plugin MCP servers |
| `src/core/scheduler.ts` | cron matcher + `TaskScheduler` (durable, idle-gated) |
| `src/core/extensions/plugins/marketplace.ts` | marketplace manifest parsing, source resolution, registry |
| `src/core/extensions/runner.ts` | `resources_discover` aggregates `agentPaths` / `slashCommandPaths` |
| `src/extensions/core/hoo-core.ts` | `setupLoop` (scheduler + Cron* tools + `/loop` + auto); `setupMarketplace` (`/plugin`); `setupMcpLoader` consumes the MCP registry |
| `test/plugins.test.ts` `test/scheduler.test.ts` `test/marketplace.test.ts` | manifest/discovery/MCP, cron scheduler, and marketplace tests |
| `docs/plugin-format-mapping.md` | Claude ↔ GitHub ↔ native format mapping + `.agents/`-first storage/loading rules |

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

### 1.2 Shipped: `/loop` recurring runner (minimum scope)

Registered in `hoo-core.ts` (`setupLoop`). Re-submits a prompt on an interval.

```
/loop <30s|5m|1h> <prompt>   start a recurring run (default interval: 10m)
/loop <prompt>               start with the default interval
/loop status                 show the active loop
/loop stop                   stop the active loop
```

- The first tick fires immediately; subsequent ticks fire every interval via
  `setInterval`, delivering the prompt with `pi.sendUserMessage(..., { deliverAs: "followUp" })`
  so it queues correctly while the agent is mid-turn.
- A single active loop per session (starting a new one replaces it). Cleared on
  `session_shutdown`.

### 1.3 Deferred: autonomous continuation policy (the "loop controller")

Lift the hard-wired "continue while `toolUse`" rule into an explicit, overridable
policy on the harness:

```ts
interface LoopPolicy {
  shouldContinue(turnState): boolean;   // default: stopReason === "toolUse"
  maxTurns?: number;
  maxWallClockMs?: number;
  maxCost?: number;
  maxTokens?: number;
  onExhausted?: "stop" | "ask";
}
```

This enables a second `/loop` flavor — `/loop --max-turns 20 <task>` — that keeps the
agent iterating autonomously (instead of yielding to the user) until a stop condition.
It belongs in `packages/agent` (the harness), exposed to extensions as loop hooks.
Deferred because the recurring runner covers the immediate need and the policy is a
larger, harness-level change.

---

## 2. Plugin system

### 2.1 What a plugin is

A directory that bundles agent capabilities behind a manifest. Discovered under
`plugins/` folders:

- project: `<cwd>/.hoocode/plugins/`
- global: `<agentDir>/plugins/`

A directory is a plugin if it contains **either**:

- `.agents-plugin/plugin.json` — native format, and
- `.claude-plugin/plugin.json` — Claude Code compatible format.

When both are present, **native wins (no merge)**.

### 2.2 Architecture: normalizer + synthetic factory

```
plugins/manifest.ts      parse either format → NormalizedPlugin
plugins/hooks-bridge.ts  NormalizedPlugin.hooks → ExtensionAPI handlers (shell protocol)
plugins/index.ts         discoverPlugins() + buildPluginFactory() → ExtensionFactory
loader.ts                discoverAndLoadExtensions() loads each factory into the runtime
```

A plugin becomes a synthetic `ExtensionFactory`, so it rides the **existing** extension
loader/runner — plugins are "extensions assembled from a manifest instead of code."

### 2.3 Capability mapping

| Manifest field / dir | hoocode target | Status |
|---|---|---|
| `name` / `version` / `description` / `author` | plugin identity metadata | ✅ shipped |
| `skills/` | `resources_discover` → `skillPaths` | ✅ shipped |
| `commands/` | `resources_discover` → `promptPaths` | ✅ shipped (best-effort¹) |
| `themes/` | `resources_discover` → `themePaths` | ✅ shipped |
| `agents/` | `addModeSearchPath` | ⚠️ best-effort² |
| `providers` (native only) | `registerProvider` | ✅ shipped |
| `hooks` / `hooks/hooks.json` | shell-protocol bridge | ✅ shipped (events below) |
| `mcpServers` / `.mcp.json` | MCP server connect | ⏳ parsed, wiring deferred³ |

¹ Claude command `.md` files use frontmatter (`description`, `allowed-tools`,
`argument-hint`). hoocode prompt templates are broadly compatible markdown; a
format transform for full fidelity is a follow-up.

² hoocode mode lookup expects `{dir}/{modeName}/system.md`, while Claude `agents/`
are flat `*.md` with frontmatter. The path is registered but the structure differs —
a transform (agents `*.md` → mode dirs) is needed for real loading.

³ MCP loading today is file-based (`setupMcpLoader` reads standard `mcp.json`
locations). There is no ExtensionAPI to inject server configs at load time. Options
for wiring: (a) add `pi.registerMcpServer(...)`, or (b) have the plugin loader merge
plugin `mcpServers` into the set `setupMcpLoader` consumes. Until then, plugin
`mcpServers` are parsed into `NormalizedPlugin.mcpServers` but not connected.

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

### 2.6 Deferred: distribution / marketplace

True Claude Code parity includes marketplaces (git-repo registries + install/update).
hoocode currently requires manual placement in a `plugins/` folder. A marketplace layer
(`marketplace.json` listing plugins, an install command, version/dependency resolution)
is a phase-2 effort. The identity metadata captured in 2.3 (`id`/`version`) is the data a
registry would key on.

---

## File map

| File | Purpose |
|---|---|
| `src/core/extensions/plugins/manifest.ts` | manifest types + parse/normalize both formats |
| `src/core/extensions/plugins/hooks-bridge.ts` | shell-protocol hooks bridge |
| `src/core/extensions/plugins/index.ts` | discovery + synthetic factory builder |
| `src/core/extensions/loader.ts` | wires plugin discovery into `discoverAndLoadExtensions` |
| `src/extensions/core/hoo-core.ts` | `setupLoop` — the `/loop` command |
| `test/plugins.test.ts` | manifest/discovery/factory/integration tests |

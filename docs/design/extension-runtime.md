# Extension runtime: running third-party code

Status: **draft for review, 2026-10-07.** Design only. Replaces ledger task 12.3 (code
extensions: out-of-process protocol, wasm optional).
Index: [post-migration-roadmap.md](post-migration-roadmap.md).

## Problem

hoocode-ts extensions are TypeScript modules (an `ExtensionFactory` default export)
loaded in-process from `./.hoocode/extensions`, `~/.hoocode/extensions`,
`--extensions`, and packages ([plugins.md](plugins.md) §7). The `ExtensionAPI`
(`core/extensions/types.ts`, 1,646 lines) gives them:

- **about 30 events:** session lifecycle; `before_agent_start`, `context`,
  `before_provider_request`/`after_provider_response`; turn, message and
  tool-execution events; `tool_call`/`tool_result` (both can change the outcome);
  `input`; `user_bash`; `model_select`; `resources_discover`;
- **registration:** tools, slash commands, shortcuts, CLI flags, message renderers,
  model providers (including OAuth);
- **session control:** `sendMessage`, `sendUserMessage`, `exec`, active tools, model,
  thinking level, session name and labels;
- **UI:** `notify`/`select`/`confirm`/`input`, status, widgets, title, editor text,
  and **TUI component factories** (custom header, footer and editor, overlays).

The TS repo ships 72 examples. Many are TUI components (custom footer, modal editor,
overlays, even doom).

In Rust:

- hoocode-ts's built-in extensions (modes, permission gate, ask_options, cost) were
  ported as native Rust behind the `ExtensionHooks` trait
  (`code-agent-session/src/hooks.rs`).
- There is no way to load anyone else's code.
- `cortexcode-code-extensions` is a 412-line experimental wasmtime host with a
  three-function guest ABI. Nothing uses it, and it costs about 55s of CPU on every
  clean build ([build-speed.md](build-speed.md) D5).

Rust has no stable ABI, so third-party extensions can't be native Rust plugins.
Whatever we pick runs out of process or in a VM.

## What the standards say

The AAIF answer to "extend an agent" is **MCP servers for capabilities, plus hooks
for lifecycle**. goose's extensions *are* MCP servers. Claude Code and Copilot
plugins are skills, agents, commands, hooks and MCP servers, with no code API. Those
are exactly the surfaces [plugins.md](plugins.md) and [mcp.md](mcp.md) already
deliver. The cross-vendor Agent Plugins 1.0 package format (agent-plugins.org) makes
the same choice: its only portable components are skills and MCP servers, and hooks
and agents stay client-specific.

| Extension need | Standard surface |
|---|---|
| New tools | MCP server |
| Block, approve or rewrite a tool call; react to results | `PreToolUse`/`PostToolUse` hooks (exit code / JSON decision) |
| Add context to a prompt | `UserPromptSubmit` hook, skills, AGENTS.md |
| Session start/stop side effects | `SessionStart`/`Stop` hooks |
| New slash commands | `commands/*.md` (prompt templates) |
| New providers | `models.json`, native plugin `providers` |
| Rich UI | MCP Apps ([canvas-and-mcp-apps.md](canvas-and-mcp-apps.md)) |

What the standard surfaces **cannot** do: rewrite the message context before a
provider call (`context`, `before_provider_request`), drive the TUI (widgets, custom
editor), register shortcuts or flags, or run code-defined slash commands that call
back into the session (`sendUserMessage`, `setModel`). Those need a code API.

## Decision E1: how much code-extension support

| Option | What runs | Coverage | Cost |
|---|---|---|---|
| **(A) Standards only** | Plugins: hooks, MCP, skills, commands, providers | Most real-world plugin use; no TS `ExtensionFactory` modules | None beyond plugins.md |
| **(B) Node extension host** | A Node child loads TS extensions and proxies `ExtensionAPI` over JSON-RPC | Existing hoocode-ts extensions minus TUI component factories | One protocol, one small JS package, and Node ≥ 20.6 at runtime |
| (C) WASM | Guest modules against a WIT world | Nothing existing; new extensions in any language | Design a WIT API; wasmtime in the build; no users |

**Recommendation: (A) now and (B) as phase 2. Park (C).** Move wasmtime behind an
off-by-default `wasm` feature (build-speed D5), and decide later whether to delete
`code-extensions`. (B) should start only once someone needs a specific TS extension
that a hook or an MCP server can't replace. The protocol below is drafted so that
decision is cheap.

## (B) The extension host, if built

### Shape

```
cortex ──stdio JSON-RPC 2.0 (NDJSON)──▶ node hoocode-ext-host.mjs
                                         ├─ loads extensions (TS via jiti or Node type-stripping)
                                         ├─ fake ExtensionAPI whose calls become RPC to cortex
                                         └─ resolves "@kolisachint/hoocode-agent" imports
```

One host process per session, started lazily when the first extension is
discovered. It is killed with the session. A crash marks its extensions failed for
the session; the agent keeps running.

**Decision E2: own loader, or hoocode-ts's runtime as a library.** The published
`@kolisachint/hoocode-agent` package exports `discoverAndLoadExtensions`,
`ExtensionRunner` and `createExtensionRuntime`.

- **(B2) Reuse them:** the host is a thin bridge from the real runner to JSON-RPC.
  Every event's semantics (ordering, short-circuiting, error isolation) are TS's by
  construction, and extension imports of the agent package resolve naturally. It
  couples the host to hoocode-ts releases, which is acceptable: the host is versioned
  and pinned like the pin itself.
- (B1) A new loader in this repo: no dependency, but it re-implements runner
  semantics.

Recommend B2.

### Protocol (draft)

All messages are JSON-RPC 2.0, one per line. Names use `ext/` (cortex → host) and
`hoo/` (host → cortex).

| Direction | Method | Notes |
|---|---|---|
| → | `ext/initialize {protocolVersion, cwd, flags, extensionPaths}` | Returns registrations: tools (JSON Schema), commands, flags, providers (config only), the event types handled, load errors |
| → | `ext/event {type, payload}` (request) | Only for events with results: `tool_call`, `tool_result`, `context`, `before_agent_start`, `before_provider_request`, `input`, `message_end`, `user_bash`, `resources_discover`, `session_before_*`. Per-event timeout (default 5s; for `tool_call`, the tool timeout). On timeout the event resolves as "no change". |
| → | `ext/notify {type, payload}` | Every other event, sent only if some extension listens (`has_handlers`). Fire and forget. |
| → | `ext/tool.execute {name, callId, args}` | Streams `hoo/tool.update`; cancelled with `$/cancelRequest` |
| → | `ext/command.run {name, args}` | Slash command; argument completions via `ext/command.complete` |
| ← | `hoo/sendMessage`, `hoo/sendUserMessage`, `hoo/exec`, `hoo/setActiveTools`, `hoo/setModel`, `hoo/setThinkingLevel`, `hoo/setSessionName`, `hoo/setLabel`, `hoo/getContextUsage`, `hoo/getSystemPrompt` | Session control. `exec` goes through the permission gate like `bash`. |
| ← | `hoo/ui.notify`, `hoo/ui.select`, `hoo/ui.confirm`, `hoo/ui.input`, `hoo/ui.setStatus`, `hoo/ui.setWidget` (string lines only), `hoo/ui.setTitle`, `hoo/ui.setEditorText` | Mapped onto existing TUI surfaces. In rpc and app-server modes, forwarded to the client as `extension_ui_request` (rpc-approvals' wire shape). |

**Not supported:** TUI component factories (`setHeader`, `setFooter`,
`setEditorComponent`, overlays, custom message renderers returning components). The
host reports them as `unsupported_surfaces` at load time, and the extension still
loads with its other registrations. `registerShortcut` is supported only for key
combinations the TUI doesn't reserve.

### Rust side

- `ExtensionHooks` gains the event methods the protocol needs (`tool_call`,
  `tool_result`, `context`, `input`, …). Every one has a default no-op, so built-ins
  are unaffected.
- A `MultiHooks` composer runs, in order: built-ins, then the plugin hooks bridge,
  then the extension host. It keeps TS ordering rules (first block wins for
  `tool_call`; transforms chain).
- New crate `cortexcode-code-ext-host`: process supervision, the protocol client and
  the `ExtensionHooks` impl. The JS host lives in `npm/hoocode-ext-host/` and ships
  inside the `@kolisachint/hoocode` npm package. Curl installs fetch it on first use,
  or report "extensions need Node ≥ 20.6 and the ext host; run `cortex ext install-host`".
- The Node launcher (find Node, check the version, spawn with a clean env) is shared
  with canvases ([canvas-and-mcp-apps.md](canvas-and-mcp-apps.md), "Shared pieces").

### Trust

Extensions are arbitrary code with the user's privileges. User-scope extensions
(`~/.hoocode/extensions`, explicit `--extensions`) load. Project
`./.hoocode/extensions` loads only in a trusted workspace ([plugins.md](plugins.md)
§6). Docs and tool descriptions make no sandboxing claim.

## Tests (for B)

- Protocol conformance with a scripted fake host (Rust), covering every
  request/response pair, timeouts, crash recovery and cancellation.
- The real host against a few TS examples (`hello`, `permission-gate`,
  `confirm-destructive`, `dynamic-tools`, `input-transform`, `custom-compaction`),
  run in CI where Node is present.
- **Reading list (TS):** `extensions-runner`, `extensions-discovery`,
  `extensions-input-event`, `compaction-extensions`, `trigger-compact-extension`,
  `agent-session-dynamic-provider`, `plan-mode-utils`, `print-mode`,
  `suite/persisted-flags`, `suite/regressions/{2023,2835,2860,3592,3686,3688,3982}-*`.
  The ledger notes on 12.3 (`session_before_compact` cancel, `ReplacedSessionContext`,
  `hoo-config.json` default provider) carry over.

## Decisions for the user

| # | Question | Recommendation |
|---|---|---|
| E1 | (A) standards only now, (B) Node host later, (C) WASM parked? | Yes |
| E2 | If (B): reuse hoocode-ts's runner from its npm package (B2), or a new loader (B1)? | B2 |
| E3 | Move `code-extensions` (wasmtime) behind an off-by-default feature now (D5), or delete it? | Feature-gate now; delete if still unused after (B) is decided |

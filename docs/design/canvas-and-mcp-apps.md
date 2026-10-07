# Rich surfaces: Copilot canvases and MCP Apps

Status: **draft for review, 2026-10-07.** Design only. Replaces the canvas part of
ledger task 12.7. Index: [post-migration-roadmap.md](post-migration-roadmap.md).

hoocode-ts references: `docs/canvas-extensions-design.md` (1,277 lines; phases 1–3
shipped) and `core/canvas/` (16 files, 4,131 lines, including the `sdk-shim/`
child-side module).

## Problem

A terminal can't show a diagram editor, a diff you approve hunk by hunk, or a
dashboard. Two ecosystems solve this. Both end up as an HTML page the person opens
next to the agent:

| | **Copilot canvases** (GitHub, 2026) | **MCP Apps** (AAIF official MCP extension) |
|---|---|---|
| Unit | Directory with `extension.mjs` (zero dependencies except `@github/copilot-sdk/extension`), forked as a Node child | A tool with `_meta.ui.resourceUri` pointing at a `ui://` resource (`text/html;profile=mcp-app`) on an MCP server |
| Who serves the page | The extension: loopback HTTP server, per-instance token, its own CSP | The host renders the HTML in a sandboxed iframe, with CSP built from `_meta.ui.csp` |
| Agent ↔ surface | Canvas *actions* invoked by the agent; SSE to the page | Host pushes `ui/notifications/tool-input`/`tool-result`; the view calls `tools/call`, `ui/message`, `ui/update-model-context` over postMessage JSON-RPC |
| Who opens it | The person (`/canvas open`); the agent drives an open instance | Appears when its tool is called |
| Catalog | `github/awesome-copilot` (23 extensions); the user's own `kolisachint/drawio-canvas` | Excalidraw, maps, PDF and many more; supported by Claude, VS Code Copilot, goose, M365 Copilot |
| hoocode-ts | Shipped: discovery, trust, runner with module-resolution shim, registry with idle reaper, `/canvas list\|open\|close`, tools `list_canvas_capabilities`/`invoke_canvas_action`/`reload_canvas`, `/new-canvas`, talkback (the person asks the agent from the canvas) | Not implemented |

A finding that changes the order: **`drawio-canvas` also runs as a plain MCP server**
(`mcp.mjs`). Its `open_canvas` tool returns a URL, and the canvas actions become MCP
tools. As soon as [mcp.md](mcp.md) phase 1 lands, `cortex` can drive the user's own
canvas through MCP with no canvas host at all. It loses only what a host adds:
talkback, the busy/idle chip, and being opened by the person rather than the agent.

## Goals

1. The person can open a surface from the TUI (browser tab) and the agent can drive it.
2. Zero token cost while nothing is open (TS invariant, tested).
3. Repository-supplied surfaces never start in an untrusted workspace.
4. No safety claims the runtime doesn't enforce. A canvas is arbitrary Node code with
   the user's privileges.

## Shared pieces

- **Node launcher** (shared with [extension-runtime.md](extension-runtime.md)): find
  Node ≥ 20.6, refusing Bun (TS §11.1: Bun silently ignores `node:module` resolve
  hooks), and spawn with a clean environment (TS §11.2).
- **Open in browser:** `open`/`xdg-open`/`start`, or print the URL when there is no
  display (SSH, cloud). One TUI affordance, "surface ready: <title> · o to open", used
  by both kinds and by plain URLs that MCP tools like `open_canvas` return.
- **Trust:** the shared record ([plugins.md](plugins.md) §6). Canvases under
  `.agents/extensions/`, `.github/extensions/` or a project plugin home are listed but
  never forked in an untrusted workspace. A canvas is withheld whole: it has no
  passive half.
- **Lifecycle:** idle reaper (30 minutes since `cortex` last touched an instance),
  per-canvas instance cap, and teardown at session end. A person reading a tab is
  invisible to us; the reaper is advisory (TS §6).

## Part 1: Copilot canvas host (crate `cortexcode-code-canvas`)

A port of TS `core/canvas/`, because it is the only reference and the user's
canvas targets it:

- **Discovery:** `./.agents/extensions/<id>/extension.mjs`,
  `./.github/extensions/<id>/extension.mjs`, `~/.copilot/extensions/<id>/`, and
  canvases inside plugins (`extensions` dir override and the
  `com.github.copilot/extensions/` namespace). Detection keys off `extension.mjs`
  only.
- **Runner:** fork the entry with Node, inject the shim as the
  `@github/copilot-sdk/extension` resolver (`--import` of a resolve hook) so nothing
  is written into the extension directory, and run JSON-RPC over stdio. stdout is
  the protocol; stderr is the log.
- **Shim:** the JS in hoocode-ts `core/canvas/sdk-shim/` is MIT (the user's). It is
  copied into `npm/hoocode-canvas-shim/` here, shipped with the npm package, and
  fetched on first use for curl installs. Its protocol (`canvas.open`,
  `canvas.close`, `canvas.action.invoke`, the version handshake) is the TS tier-2
  contract and stays byte-compatible, so one shim serves both hoocodes.
- **Registry:** an instance table keyed by `instanceId` (UUID), the reaper, and an
  action inventory feeding `list_canvas_capabilities`.
- **Agent tools:** `list_canvas_capabilities` (~75 tokens),
  `invoke_canvas_action` (~144) and `reload_canvas` (~59), with the pin's texts. They
  are registered on the first successful open and answer honestly when nothing is
  open. There is no open tool: opening is a person's act (TS §11.5). A test asserts
  each schema stays under 250 tokens and that the description makes no safety claim.
  Per-method timeouts and the single abandon path on cancel follow TS §11.4/§11.6.
- **Commands:** `/canvas list | open <ext>[:<canvas>] | close <id>`.
- **Talkback** (the canvas sends the person's request into the session): TS
  `inbox.ts`/`events.ts`. Requests arrive as follow-up user messages tagged with
  the canvas, through the normal input path.
- `/new-canvas` scaffolding is phase 2, with [plugins.md](plugins.md) phase 4.

## Part 2: MCP Apps host

New; no TS reference. It follows the ext-apps spec (`io.modelcontextprotocol/ui`).

- **Negotiation:** declare the extension with
  `mimeTypes: ["text/html;profile=mcp-app"]` in per-request client capabilities
  (2026-07-28) or at `initialize` (older servers). Servers that don't support it are
  unchanged.
- **Detection:** a tool whose `_meta.ui.resourceUri` is a `ui://` URI. The text
  result still goes to the model as usual; the app is additive.
- **Local host page:** the browser stands in for the host's chat window.
  - `cortex` runs one loopback HTTP server per session on `127.0.0.1:0`, with a
    random per-session token.
  - It serves a small host page (our JS, MIT) that implements the spec's double
    iframe. The sandbox proxy runs on a **second loopback port**, so it is a
    different origin.
  - The view's HTML comes from `resources/read` and is served with a CSP built from
    `_meta.ui.csp` (restrictive default), with iframe `allow` from
    `_meta.ui.permissions` only after the person approves camera or microphone.
  - The host page relays postMessage JSON-RPC to `cortex` over a WebSocket on the
    same token.
- **Host methods:**
  - **`tools/call`** from the view: proxied to the originating server **through the
    permission gate** (same rules as a model call, [mcp.md](mcp.md) §5).
  - `resources/read`: same server only.
  - `ui/message`: queued as a user message, shown in the TUI as coming from the
    app, and needs the person's confirmation the first time per app.
  - `ui/update-model-context`: stored and sent as context on the next turn.
  - `ui/open-link`: opens the browser after confirmation.
  - `ui/request-display-mode`: answered with `fullscreen` only (it's a tab).
  - Notifications: `tool-input`, `tool-result`, `tool-cancelled`,
    `host-context-changed` (theme from the TUI theme).
- **Opening:** apps are offered, not auto-opened. "Surface ready · o to open",
  unless setting `mcpApps.autoOpen` is on. In print, json, rpc and app-server modes,
  apps are not rendered: rpc and app-server get the `ui://` URI and metadata in the
  tool item so their clients (hoobot, the Codex TUI) can decide.

## Order (Decision C1)

| Option | First | Then |
|---|---|---|
| **(a)** | MCP phase 1 (drawio-canvas works via its MCP mode), then the **Copilot canvas host** | MCP Apps |
| (b) | MCP Apps (standard, larger catalog) | Copilot canvas host |

**Recommendation: (a).** The user's own canvas targets the Copilot contract, the TS
port is complete and tested, and its talkback and host signals are what the MCP mode
lacks. MCP Apps follow, reusing the browser opener, trust, Node-free host server
and permission routing.

## Tests

- **Copilot host:** discovery and trust (TS `canvas-discovery`, `canvas-trust`),
  runner and protocol conformance with a fixture extension, registry reaper and cap,
  tools' token budget and no-safety-claim assertion, cancel/abandon, talkback; the
  `pr-artifact-explorer` acceptance test runs when Node and network are available.
  Reading list: the 17 `canvas-*.test.ts` files plus `new-canvas` and
  `plugin-canvases`.
- **MCP Apps:** an rmcp test server with a `ui://` tool; a headless Chromium
  (Playwright is in the cloud image) loads the host page and checks CSP and
  sandbox origins, that `tools/call` from the view hits the permission gate,
  `ui/message` confirmation, and that context updates reach the next request.

## Decisions for the user

| # | Question | Recommendation |
|---|---|---|
| C1 | Copilot canvas host first, or MCP Apps first? | Copilot canvases first (a) |
| C2 | Copy the TS sdk-shim JS into this repo as a shared npm package? | Yes (MIT, the user's) |
| C3 | MCP Apps: offer (`o to open`) or auto-open? | Offer; `mcpApps.autoOpen` setting |
| C4 | Forward `ui://` app metadata to rpc and app-server clients? | Yes, as tool-item metadata |

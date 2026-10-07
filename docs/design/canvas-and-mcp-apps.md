# Rich surfaces: MCP Apps (then Copilot canvases)

Status: **agreed 2026-10-07** (MCP Apps); Copilot canvases later. Design only.
Replaces the canvas part of ledger 12.7.

## Goal

When an MCP tool comes with an interactive UI (a chart, form or editor), you can
open it in your browser beside the terminal, and it can talk to the agent.

## Decisions

| Decision | Why |
|---|---|
| **MCP Apps first** (the official MCP UI extension) | It's the standard, needs no Node, and has the larger catalog (Claude, VS Code, goose, Microsoft 365 Copilot support it) |
| Shown as "app ready · o to open" in the TUI; `mcpApps.autoOpen` setting to change | No surprise browser tabs |
| rpc and app-server clients get the app's info and decide themselves | hoobot and other clients have their own UIs |
| Calls the app makes (tool calls) go through the same trust and permission rules as model calls | An app is not a back door |
| Copilot canvases come later. Your `drawio-canvas` works now through its MCP mode (`mcp.mjs`) once [mcp.md](mcp.md) lands. | Keeps scope small |

## What we build

1. Declare the MCP Apps extension to servers (`text/html;profile=mcp-app`).
2. Spot tools whose `_meta.ui.resourceUri` is a `ui://` resource. The normal text
   result still goes to the model.
3. **Local host page.**
   - `cortex` serves a small page on `127.0.0.1` with a random token per session.
   - The app runs in a sandboxed iframe on a second local port, so it is a
     different origin.
   - The page enforces the CSP and permissions from `_meta.ui`; camera and
     microphone only after you approve.
   - Messages between page and app use postMessage, relayed to `cortex` over a
     WebSocket.
4. **Host messages.**
   - The app's `tools/call` goes to its own server, under the usual rules.
   - `ui/message` becomes a user message; it asks you the first time per app.
   - `ui/update-model-context` is sent with the next turn.
   - `ui/open-link` opens only after you confirm.
   - Theme changes are passed on to the app.
5. "o to open" in the TUI uses the system browser opener, or prints the URL over
   SSH or in the cloud.

Done when: an rmcp test server with a `ui://` tool works end to end in headless
Chromium (Playwright is installed), with CSP, origin separation, and permission
checks covered by tests.

## Not doing (now)

- Copilot canvas host (forks `extension.mjs` under Node with a module shim, plus
  the canvas tools). hoocode-ts has it; port later.
- Showing apps inside the terminal.

## Open questions

- When to add the Copilot canvas host: after MCP Apps ships, or only if
  `drawio-canvas` needs features its MCP mode lacks (talkback, busy/idle state)?

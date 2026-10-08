# Web tools: deferred

Status: **deferred 2026-10-07**, design only. Replaces ledger 10.2e.

## Goal

Let the agent read web pages and search the web.

## Decisions

| Decision | Why |
|---|---|
| **No built-in `WebFetch`/`WebSearch` for now** | `Shell` + `curl` already fetches pages |
| Clean fetch and search come from the **webtools MCP server** once [mcp.md](mcp.md) lands | `kolisachint/webtools` already ships an MCP server (`webtools mcp`) with `fetch` and `search`. No code here. |
| Revisit **provider-native web search** (server-side search offered by Anthropic and OpenAI) later | Improves without our work |

## What we build

Nothing now. When MCP lands, add a short guide showing the `mcp.json` entry for
webtools.

## Not doing

- Porting hoocode-ts's web tool layer (`.webtoolsignore`, cache, notes).
- Linking the webtools crates into `hoocode`.

## Open questions

- If built-in tools come back: link the webtools library crates (same code as the
  binary, no download). That was the earlier recommendation.

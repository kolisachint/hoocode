# MCP client

Status: **agreed 2026-10-07**, scope cut 2026-10-08 ([decisions-2026-10-08.md](decisions-2026-10-08.md)).
Design only. Replaces ledger 9.1 and 10.11.

## Goal

`cortex` uses MCP servers (tools from other programs). It speaks the current spec,
is easy to keep current as the spec moves, and only starts servers the user has
trusted.

## Decisions

| Decision | Why |
|---|---|
| Build on **rmcp**, the official Rust SDK (Apache-2.0) | Already speaks MCP `2026-07-28` and older versions, OAuth and multi round-trip input. The spec changed its core twice in a year; the SDK tracks that, so we don't. |
| **Drop** legacy HTTP+SSE and the old hand-written `agent-mcp` code | Deprecated by the spec. Future-proof over legacy. `agent-mcp` is not wired into the binary today; it is deleted in the first coding session and this card starts a fresh crate. |
| **Folder trust, no per-tool prompts** | Servers from a repository folder or a plugin need a one-time trust grant. Your own `~/.agents/mcp.json` is trusted. |
| Trust **re-asks when the server list changes** | Trust binds to the approved servers (command, args, URL). A `git pull` that adds or changes one shows the diff and asks again. |
| Installs from a trusted marketplace count as trusted | See [plugins.md](plugins.md) |
| OAuth client identity: a Client ID Metadata Document at `https://kolisachint.github.io/hoocode/oauth-client.json` | It's the spec's new standard. Use dynamic registration until the file is published. |
| CI runs the official MCP conformance suite against our client | Catches the SDK or us falling behind the spec |

## What we build

1. **Core.**
   - New crate `agent-mcp` on rmcp; only this crate depends on rmcp. It runs on the
     one `cortex-io` runtime ([concurrency.md](concurrency.md)), never its own.
   - Transports: stdio and Streamable HTTP.
   - Tools named `mcp_<server>_<tool>`; too-long names are shortened with a hash
     suffix.
   - Tool progress shows in the TUI; aborting a turn cancels the call.
   - A dropped server reconnects once.
2. **Discovery and trust.**
   - New crate `code-mcp` reads `~/.agents/mcp.json` (user), `./.agents/mcp.json`
     (project) and plugin `mcp.json` files.
   - Trust records are kept outside the repository, shared with plugins.
   - `/mcp` lists servers with their state: connected, not trusted, auth needed,
     failed.
3. **OAuth.**
   - rmcp's OAuth support: PKCE, server metadata discovery, issuer (`iss`) check.
   - Tokens stored per issuer under `~/.hoocode/mcp-auth/` (owner-only file
     permissions).
   - The browser opens for login; tools appear when login finishes.
4. **Interaction.**
   - A server asking for input mid-call becomes a question in the TUI (or an
     approval request in rpc and app-server).
   - Tool-list changes apply at the next turn.

Subagents get MCP servers only if their tool list names an `mcp_` tool or inherits
all tools.

Done when: tests pass against in-repo rmcp test servers (stdio and HTTP, old and new
protocol versions), a fake OAuth server, and the conformance suite; plus one manual
run each against a real stdio server and a real OAuth server.

## Not doing

- Sampling, Roots, Logging (deprecated by the spec).
- Legacy HTTP+SSE servers.
- Being an MCP server (the app-server is our server side).
- Registry browsing (`server.json`); plugins and marketplaces cover install.
- The Tasks extension (deferred), the Skills extension (dropped), MCP Apps
  (deferred, [canvas-and-mcp-apps.md](canvas-and-mcp-apps.md)).
- `/mcp import` and reading other tools' MCP config files.

## Open questions

| # | Question | Recommendation |
|---|---|---|
| M2 | Send the model tool results as plain content (text, images) instead of hoocode-ts's JSON dump of the whole result? | Yes: fewer tokens |
| M7 | Large MCP tool sets cost tokens on every request. Defer their schemas? | Eager by default. Later, use the provider's native deferred loading where available (Anthropic's `defer_loading`, already in our types). |

# MCP servers

hoocode speaks the Model Context Protocol, so tools from an MCP server appear
alongside the built-in ones. Each server tool is registered as
`mcp_<server>_<tool>` — a `create_pr` tool on a server named `github` becomes
`mcp_github_create_pr`.

## Configuring servers

Config files are read in this order, **first wins by server name**:

1. `~/.agents/mcp.json` — user, standard format
2. `./.agents/mcp.json` — project, standard format
3. `~/.config/claude/mcp.json` — Claude Desktop, standard format
4. `~/.hoocode/mcp-servers/*.json` — user, one file per server
5. `./.hoocode/mcp-servers/*.json` — project, one file per server

Plugins can also declare servers in their manifest or a `.mcp.json` — see
[Plugins](plugins.md).

Standard format:

```json
{
  "mcpServers": {
    "my-server": {
      "command": "npx",
      "args": ["-y", "my-mcp-server"],
      "env": { "API_KEY": "..." }
    }
  }
}
```

Per-server format (`.hoocode/mcp-servers/<name>.json`) uses a top-level `name`
and `command` instead of the `mcpServers` wrapper.

## Transports

| Transport | Config | Notes |
|-----------|--------|-------|
| stdio | `"command"` (+ `args`, `env`) | Default when `command` is set |
| Streamable HTTP | `{ "type": "http", "url": "..." }` | Add `headers` for auth |
| SSE | `{ "type": "sse", "url": "..." }` | Legacy |

One of `command` or `url` is required. Remote transports support OAuth; hoocode
stores the tokens and runs the authorization callback for you when a server
requires it.

```json
{
  "mcpServers": {
    "remote": {
      "type": "http",
      "url": "https://example.com/mcp",
      "headers": { "Authorization": "Bearer ..." }
    }
  }
}
```

## Deferred schemas

A dozen MCP servers can contribute hundreds of tools, and their JSON schemas are
the largest thing in a system prompt that is otherwise a few thousand tokens. So
by default hoocode connects every server but **withholds the schemas**,
registering one resolver tool instead:

```
ResolveMcpTools  names: ["mcp_github_create_pr"]
ResolveMcpTools  query: "open a pull request"
```

Naming a tool resolves it; describing a capability finds it and resolves it, so
the model does not have to know the name in advance. Resolved tools become
callable in the same turn.

Retrieval is hybrid — BM25 over tool names and descriptions, fused with a dense
leg when the `embsearch` binary is present. The lexical leg always works and
never needs a download, so an exact name match is guaranteed; the dense leg only
adds. When only the lexical leg answered, the result says so.

Turn deferral off with the `deferMcpSchemas` setting (default `true`) to load
every schema eagerly. Subagent children never defer — they are spawned with a
narrow tool allowlist already.

## Checking status

Connection happens at session start. A server that fails to connect reports an
error notification and the session continues without it, rather than failing to
start.

## Related

- [Settings](settings.md) — `deferMcpSchemas` and tool policy
- [Plugins](plugins.md) — plugins that ship MCP servers, and the trust rule that governs them
- [Project-local resources](project-local-resources.md) — the full `.agents/` discovery table

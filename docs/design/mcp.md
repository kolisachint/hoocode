# MCP client

Status: **draft for review, 2026-10-07.** Design only. Replaces ledger tasks 9.1 (MCP
transport on rmcp) and 10.11 (`code-mcp`: discovery, deferred schemas, status).
Index: [post-migration-roadmap.md](post-migration-roadmap.md).

## Problem

`cortex` has no MCP. `cortexcode-agent-mcp` (1,760 LOC, hand-written) parses
`mcp.json`, speaks the 2024–2025 protocol over stdio, Streamable HTTP and legacy SSE,
and turns tools into `AgentTool`s named `mcp_<server>_<tool>`. However, only the
`cortexcode` umbrella crate depends on it, and nothing in `code-cli` loads it. OAuth
is a placeholder.

Meanwhile the protocol moved. MCP `2026-07-28` (released 2026-07-28) is the largest
revision since launch:

- **Stateless.** There is no `initialize` handshake and no `Mcp-Session-Id`. Every
  request carries `io.modelcontextprotocol/protocolVersion` and `clientCapabilities`
  in `_meta`. `server/discover` advertises versions and capabilities.
- **`subscriptions/listen`** replaces the HTTP GET stream and `resources/subscribe`.
- **Multi round-trip requests.** A server that needs input returns
  `resultType: "input_required"` with `inputRequests`, and the client retries the
  original request with `inputResponses`. This replaces server-initiated
  `elicitation/create`, `sampling/createMessage` and `roots/list`.
- Every result has `resultType`. List results carry `ttlMs`/`cacheScope`.
- `Mcp-Method`/`Mcp-Name` headers are required on HTTP POSTs.
- **Deprecated:** HTTP+SSE, Roots, Sampling, Logging, and Dynamic Client Registration
  (DCR), which gives way to Client ID Metadata Documents (CIMD). Each has a removal
  window of at least 12 months.
- **Auth hardening:** validate `iss` (RFC 9207); persisted credentials are keyed by
  issuer; DCR must send `application_type`.
- Tasks, MCP Apps and Skills are official **extensions**, negotiated per request.

Servers in the wild speak every revision from `2024-11-05` to `2026-07-28`. hoocode-ts
at the pin speaks the pre-2026 dialect only (`initialize`, `Mcp-Session-Id`, SSE
fallback).

## Goals

1. A user's existing `mcp.json` works in `cortex` as it does in hoocode-ts: same
   files, same precedence, same tool names.
2. Speak `2026-07-28` first and fall back to `2025-11-25` and earlier.
3. OAuth that works against real servers (GitHub, Linear, Notion, Atlassian…).
4. MCP tools go through the permission gate (§5). That is a deliberate change from TS.
5. No token cost for servers the user doesn't use. Deferral is an opt-in, measured
   policy (§7).

Non-goals: being an MCP *server* (the [app-server](app-server.md) is our server
surface), Sampling and Roots (deprecated), and hosting the MCP Registry.

## 1. Decision M1: rmcp, or keep the hand-written client

| | **(1) rmcp 3.5.1** (official Rust SDK) | (2) Extend the hand-written client |
|---|---|---|
| Protocol | `2026-07-28` stable, `2025-11-25` and earlier compatible; MRTR, Tasks, response caching, standard headers | Must add: stateless mode, `server/discover`, version fallback, MRTR, `resultType`, subscriptions, headers |
| Transports | child process (stdio), Streamable HTTP over reqwest, unix socket | Has stdio, HTTP and legacy SSE; needs reworking for stateless HTTP |
| Auth | `auth` feature: OAuth with PKCE and metadata discovery, Client ID Metadata Documents, `iss` validation (both present in `transport/auth.rs`); also client-credentials JWT and enterprise-managed | From scratch: PKCE, RFC 8414/9728 discovery, CIMD, DCR fallback, RFC 9207 `iss`, refresh. Security-sensitive. |
| Legacy HTTP+SSE client | **No** | Has one |
| Licence | **Apache-2.0** (the MCP project is relicensing from MIT) | Ours |
| Maintenance | Tracks the spec; conformance-tested upstream | Every future spec revision is ours to implement |

**Recommendation: (1), with the existing legacy-SSE path kept.** The 2026 revision
alone is more work than adopting the SDK, and OAuth is where a hand-written client is
most likely to be subtly wrong. The ledger's original objection (rmcp re-serializes
tool results from typed structs) no longer matters: there is no parity gate, and §4
renders results ourselves anyway.

The licence question is the user's call. rmcp is a dependency, not copied code, so
CLAUDE.md's MIT-only rule (which is about copying) is not broken. The workspace
already depends on Apache-2.0-only crates (wasmtime/cranelift via
`code-extensions`, and `debugid`). Apache-2.0 is compatible with distributing an MIT
binary, but release archives must then carry its licence text (and NOTICE file, if
any).

Legacy HTTP+SSE: hoocode-ts supports `"type": "sse"` and falls back to it on a 4xx.
The spec deprecates it, and rmcp has no client for it. Keep today's hand-written SSE
code behind the same transport trait, unchanged, and mark it deprecated in `/mcp`
output. Drop it when the spec removes it.

## 2. Crates

| Crate | Holds | Depends on |
|---|---|---|
| `cortexcode-agent-mcp` (rewrite) | Connection manager over rmcp: transports, protocol-version negotiation, OAuth client and token store, tool adapter (`AgentTool`), MRTR and Tasks loop, legacy SSE | rmcp (only this crate), reqwest, tokio |
| `cortexcode-code-mcp` (new, was 10.11) | Config discovery and merging, plugin-provided servers, permission policy, deferral and `ResolveMcpTools`, status model, `/mcp` command, subagent propagation | agent-mcp, code-settings, code-paths, code-permissions |

The dependency firewall (`migration/dep-firewall.json`) gets an entry that keeps rmcp
inside `agent-mcp`. A future SDK swap then touches one crate.

## 3. Configuration and discovery (TS parity)

Sources, merged first-wins by server name in this order. This is hoocode-ts
`mcp-loader.ts` order:

1. `~/.agents/mcp.json` (user), then `./.agents/mcp.json` (project)
2. `~/.config/claude/mcp.json` (Claude Desktop)
3. `~/.hoocode/mcp-servers/*.json`, `./.hoocode/mcp-servers/*.json` (one server per
   file). Under [naming-and-paths.md](naming-and-paths.md), the Rust private copy is
   `~/.hoocode/rust/mcp-servers/`. Both are read; only `rust/` is written.
4. Servers registered by plugins ([plugins.md](plugins.md) §3), with
   `${AGENTS_PLUGIN_ROOT}`/`${CLAUDE_PLUGIN_ROOT}` substituted

Server entry (standard `{ "mcpServers": { name: … } }` shape): `command`, `args`,
`env`; or `type: "http" | "sse"`, `url`, `headers` (the TS
`StandardMcpServerEntry` fields). Unknown fields are kept and ignored. TS runs MCP
tools in background mode by default (`background`, default true).

**Decision M4: project-scope servers need workspace trust.** `./.agents/mcp.json`
and `./.hoocode/mcp-servers/` start processes named by a repository. hoocode-ts runs
them without asking. Proposal: a project server connects only when the workspace is
trusted. That is the same trust record canvases and plugins use
([plugins.md](plugins.md) §6). An untrusted repo shows "2 project MCP servers not
started (untrusted workspace)" in `/mcp`. User-scope servers are unaffected.

## 4. Tools

- **Name:** `mcp_<server>_<tool>`, as in TS. TS doesn't shorten long names; a name
  over a provider's tool-name limit (64 characters for several providers) is cut and
  suffixed with a short hash, and the full name is kept for display.
- **Schema:** the server's `inputSchema` is passed through as JSON Schema 2020-12.
  The spec now allows any keyword, and `$ref` must resolve. The 10.11 ledger note
  records that TS rebuilds each schema with TypeBox (flattening types to
  string/number/boolean). We don't copy that lossy rebuild. We pass the schema
  through and validate arguments with the existing validator, and providers that
  can't take a keyword get it stripped at the provider boundary, as for built-in
  tools.
- **Result to model** (a change from TS, which sends `JSON.stringify(result)`):
  `text` blocks are joined; `image` blocks become image content; `resource_link` and
  embedded `resource` blocks become a one-line reference; `structuredContent` is sent
  as compact JSON only when there are no content blocks; `isError: true` becomes a
  tool error. This saves the JSON envelope's tokens on every call. The raw result is
  kept in `details` for the UI and the session file.
- **Progress:** `notifications/progress` on the request's response stream becomes
  `tool_execution_update` events (spinner text in the TUI, `item/updated` in the
  app-server).
- **Cancellation:** aborting a turn sends `notifications/cancelled`. Over HTTP it also
  drops the response stream (2026 rule: a broken stream loses the request).
- **Lazy reconnect:** a call to a dropped server reconnects once, then fails with a
  clear error (TS parity).
- **Tool list changes:** with `2026-07-28` servers, one `subscriptions/listen` per
  server for `toolsListChanged`; with older servers, `notifications/tools/list_changed`.
  The tool set is applied at the next turn boundary through the agent loop's existing
  `prepare_next_turn` hook. Because a tool change invalidates the prompt cache, the
  status line shows when it happened.

## 5. Permissions (Decision M3)

The MCP spec says hosts **must** obtain user consent before invoking a tool, and
treats tool annotations as untrusted unless the server is trusted. hoocode-ts doesn't
gate MCP tools (`GATED_TOOLS` is bash, write, edit, webfetch, websearch).

Proposal:

- MCP tools are **gated by default** in interactive mode. The prompt names server,
  tool and arguments, with the usual Yes / No / Always.
- "Always" is stored per `server/tool` in settings (`permissions.mcp.allow`), and a
  whole server can be allowed with `server/*`.
- `readOnlyHint: true` auto-allows **only** for servers the user marked `trusted` in
  settings. Otherwise annotations are ignored.
- Modes (ask/plan/build/debug) treat MCP tools as mutating unless allowed, so plan
  mode blocks them by default.
- RPC and app-server: the gate's UI is the client
  ([rpc-approvals.md](rpc-approvals.md), app-server approvals). Until rpc-approvals
  lands, `--mode rpc` with no UI **denies** gated MCP tools rather than allowing them.
  That fixes the fail-open default for MCP from day one.

## 6. Protocol features by phase

| Phase | Scope |
|---|---|
| **1. Core** | stdio + Streamable HTTP, version negotiation (`server/discover`, falling back to `initialize`), tools only, §3 discovery, §4, §5, `/mcp` status (servers, state, tool count, protocol version, deprecated transport), background connect, lazy reconnect, legacy SSE |
| **2. OAuth** | rmcp `auth`: RFC 9728 protected-resource metadata, RFC 8414, PKCE, loopback redirect `http://127.0.0.1:<port>/callback`. Prefer CIMD, fall back to DCR with `application_type: "native"`. Validate `iss`. Tokens keyed by issuer in `~/.hoocode/rust/mcp-auth/` (0600, atomic writes); TS uses `~/.hoocode/mcp-auth/<host>-<hash>.json`, which we read once to migrate. An "auth pending" state with the URL in `/mcp`; a connect that completes later registers tools mid-session (TS parity). |
| **3. Scale** | Deferred schemas (§7), subagent propagation (a child gets MCP only when its tool allowlist names an `mcp_` tool or inherits all tools; TS `toolAllowlistNeedsMcp`), list caching honouring `ttlMs` |
| **4. Interaction** | MRTR: `input_required` elicitations in form mode become the ask_options pane (interactive) or approval/input requests (rpc, app-server); URL mode opens the browser and retries. **Tasks extension:** a tool call that returns `resultType: "task"` polls `tasks/get` at `pollIntervalMs`, shows progress, supports `tasks/cancel`, and stores task ids in the session so a resumed session can pick up the result. |
| **5. Extensions** | Skills over MCP ([plugins.md](plugins.md) §5), MCP Apps ([canvas-and-mcp-apps.md](canvas-and-mcp-apps.md)), resources and prompts as `@` mentions and `/` commands (later), sub-registry search (§8) |

Not implemented: Sampling, Roots, Logging (deprecated). Older servers that send
`roots/list` get an empty list. Servers that need a directory get it through config
(`cwd`, args), as the spec advises.

## 7. Deferred schemas

hoocode-ts: with `HOOCODE_DEFER_MCP_SCHEMAS=1`, MCP tools are not registered. A
`ResolveMcpTools` tool lists their names in its description and materializes schemas
on demand, taking `names` (exact) or `query` (retrieval). It is off by default, and
the TS docs measured why (`plugin-system-architecture.md` §6.3, §8.6): resolving a
tool changes `tools`, which invalidates the whole prompt cache. Deferral therefore
loses money in long sessions.

Rust design:

- Default **eager**. Deferral is per server, policy-driven: `mcp.defer` = `never`
  (default) | `always` | `auto`. `auto` defers a server when its schemas exceed a
  token threshold, as reported by a `--print-token-surface`-style report.
- When deferring on a provider that supports native deferred loading (Anthropic
  `defer_loading` plus tool search; the ai types already carry the flag), use it.
  Loading is then append-only and doesn't invalidate the cache. Otherwise fall back
  to `ResolveMcpTools` (TS semantics: exact `names`, plus `query` backed by the
  capability index in [semantic-search.md](semantic-search.md) §2).
- The catalog in `ResolveMcpTools`'s description never truncates. Above 30 entries
  it switches to a per-server summary with counts (TS `CATALOG_EAGER_LIMIT`).

## 8. Later: registries

The MCP Registry (preview) publishes `server.json` (reverse-DNS names; npm, PyPI, OCI
or MCPB packages; remotes). Hosts are meant to consume sub-registries that implement
its OpenAPI, not the central registry. A later `/mcp add <registry-name>` could
resolve a `server.json` into an `mcp.json` entry. Anything that runs a package
(`npx`, `uvx`, `docker`) shows the exact command and asks first. Out of scope until
plugins land, because marketplaces cover the same need for curated sets.

## 9. Tests

- **In-repo servers:** rmcp's `server` feature builds small test servers (echo tool,
  slow tool, `input_required` tool, task-returning tool, list-changed). They run
  in-process over the worker transport, and as child processes for stdio. Both
  protocol eras are tested.
- **Legacy SSE:** the existing hand-written tests stay.
- **OAuth:** a fake authorization server (axum as a dev-dependency) covers PKCE, CIMD, DCR
  fallback, `iss` mismatch rejection, refresh, and issuer-keyed storage.
- **Conformance:** run the MCP conformance suite (`modelcontextprotocol/conformance`)
  against the client in CI as an external tool. Running a non-MIT tool in tests is
  fine per CLAUDE.md.
- **Permission gate:** an MCP tool prompts; "Always" persists per tool; plan mode
  blocks it; rpc mode with no UI denies.
- **Reading list:** TS behaviour for `packages/agent/test/tools/{mcp-tools,mcp-http,default-tools}.test.ts`
  and `coding-agent/test/mcp-deferred.test.ts`.

Done = the above green, `cargo clippy -D warnings`, and a manual run against one stdio
server (the official filesystem server), one OAuth HTTP server and one legacy SSE
server.

## 10. Decisions for the user

| # | Question | Recommendation |
|---|---|---|
| M1 | rmcp (Apache-2.0) or hand-written? | rmcp, keeping the legacy SSE code |
| M2 | Render tool results natively (§4) instead of TS's JSON dump? | Yes |
| M3 | Gate MCP tools by default (§5)? | Yes |
| M4 | Require workspace trust for project-scope servers (§3)? | Yes |
| M5 | CIMD needs a public HTTPS URL for our client metadata document. Host it at `https://kolisachint.github.io/hoocode/oauth-client.json`? | Yes, with DCR as fallback until it exists |

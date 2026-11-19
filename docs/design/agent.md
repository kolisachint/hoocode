# Agent

Status (2026-11-19): Built: agent tools, scoped models, scheduler and `/loop`, extras (version check, chime), DocSearch, MCP client, plugins (see plugins.md), app-server and rpc approvals. Partly built: web tools (flag only; replaced by the webtools plugin, plugins.md PL-12), semantic index (flag only), MCP schema deferral (setting read by nothing). Tool names follow option D (2026-11-19, plugins.md PL-17). Built in P2: origin, names, selectors in `--tools`. Built in P2b (2026-11-19): selectors in permission rules, the gate and the approval relays read the origin, malformed selectors refused. Open: P4 (TUI). Open items are at the end.

## Principles

- **Standards first.** MCP, Agent Skills, Agent Plugins 1.0, the Claude plugin and marketplace layouts (read only), and the Codex app-server wire format. A vendor format only where no standard covers it. Plugins: plugins.md.
- **MIT only.** Copy nothing from `../codex` (Apache-2.0): no code, schemas or generated types. Running it in tests is fine.
- **No always-on token cost.** Optional tools stay off until a flag or setting turns them on. Every tool schema is sent on every request.
- **Fail closed.** A gated tool does not run without asking. In a headless run with nobody to answer, it is denied unless `auto_allow` lists it.
- **Trust before start.** Only trusted MCP servers start. Trust binds to the approved command, args or URL. A change shows a diff and asks again. Trust records live outside the repo.
- **Subagents never get** plugin-management tools or Cron tools. Their names are passed in `--disallowed-tools` on both spawn paths. Cron tools are not registered outside an interactive run. Plugin-provided tools: see plugins.md.
- **Schedules fire only in an open interactive session.** No headless daemon: nobody is there to approve tools.
- **Tool names are final.** No aliases and no old names: `ext_*` and `mcp_*` are removed, not aliased (2026-11-19). Identity is structured (origin); the model-facing name is for the model only (plugins.md Naming and identity). The Tools pane sort keys use old TS names, but only for sorting.
- **Config and env names** are `hoocode` and `HOOCODE_` only. Writes go to a temp file, then rename.

Numbers a contributor needs:

| Item | Value | Source |
|---|---|---|
| MCP request deadline | 60 s | `hoocode-agent-mcp/src/client.rs` |
| MCP reconnect after a drop | once | `client.rs` |
| Scheduler tick | 30 s, idle only | `hoocode-code-scheduler/src/lib.rs` `TICK_INTERVAL` |
| Late-fire window | 10 min | `store.rs` `LATE_FIRE_MINUTES` |
| Turn chime | turn over 10 s; repeats within 5 s dropped | `hoocode-code-tui-app/src/chime.rs` |
| Version check | once a day | `hoocode-code-update` |
| `/loop auto` default | 10 turns | `auto_loop.rs` |

## How it works

### Tools

| Tool | Registered in | Default | Notes |
|---|---|---|---|
| `Read`, `Shell`, `Edit`, `Write` | `hoocode-code-tools-fs`, `hoocode-code-tool-bash` | on | Core four. The last core tool cannot be turned off. |
| `CodeSearch` | `hoocode-code-tool-search` | on | Lexical search. |
| `AskUserQuestion` | `hoocode-code-tools-optin` `ask_options.rs` | on | Dialog host is the TUI, or `NoUi` outside it. |
| `CronCreate`, `CronList`, `CronDelete` | `hoocode-code-scheduler` | on, interactive only | Never in print, json, rpc or subagent runs. |
| `Agent`, `AgentOutput` | `hoocode-code-subagents` | on | `Agent` starts a subagent. `AgentOutput` reads its result. |
| `ShellOutput`, `ShellKill` | `hoocode-code-tool-bash` `job_tools.rs` | on when `enableBackgroundShell` (default true) and interactive | Background shell jobs. |
| `TodoWrite` | `hoocode-code-tools-optin` `todo.rs` | on (`enableTodoWrite`, default true) | Main session only. |
| `SearchPlugins`, `ListPlugins`, `InstallPlugin`, `UninstallPlugin`, `ProposePlugin` | `hoocode-code-tools-optin` `plugins.rs` | off (`enablePluginTools`) | Main session only. Takes effect at the next session. |
| Plugin and MCP tools | `hoocode-code-wasm` (WASM, name as the plugin declares it); `hoocode-code-mcp` and `hoocode-agent-mcp` (MCP, `mcp__<server>__<tool>`) | per plugin or server | See plugins.md Naming and identity and MCP client. |

- Default active list (`DEFAULT_ACTIVE_TOOL_NAMES`, `hoocode-code-agent-session/src/session.rs`): `Read`, `Shell`, `Edit`, `Write`, `CodeSearch`, `AskUserQuestion`, `CronCreate`, `CronList`, `CronDelete`, `DocSearch`.
- Tool lookup has no alias table. The old TS names (`Task`, `TaskOutput`, `SearchCodebase`, `SearchHooCode`, `bash`) appear only in `tool_row_sort_key` (`settings_selector.rs`), which sets row order in the Tools pane.
- Plugin tool permissions (`hoocode-code-tools` `permissions.rs`): `SearchPlugins` and `ListPlugins` are read-only. `InstallPlugin` is granted with no prompt and refuses an untrusted marketplace. `UninstallPlugin` and `ProposePlugin` prompt. An unknown tool name is denied, so each new name needs a class.

### Models and scoped models

Settings key `scopedModels` in `~/.hoocode/settings.json` (or the project file). It is an ordered list:

```json
{ "model": "provider/id", "effort": "high", "category": "cheap", "alias": "short-name" }
```

- `effort` and `category` are optional. `category` is one of `fast`, `standard`, `capable`, `cheap`.
- `alias` is unique across the list and uses only `[a-z0-9-]`. It cannot be a category name or a glob.
- A project `scopedModels` replaces the global list. It never merges. `scopedModels: []` is an empty list, which is present.
- The old keys `enabledModels` and `modelCategories` are not read. There is no migration.

| Rule | Behaviour |
|---|---|
| Name match | Alias (exact), then exact id, then a unique substring of the id. Several matches is an error that lists the candidates. |
| Hard limit | Subagents use only scoped models. An empty scope falls back to all logged-in models. |
| Categories | One per entry, zero or one. Several entries may share one. Tie-break: list order. |
| Untagged entries | Usable by name or alias only. Never picked for a category. |
| Empty category | Nearest tier within scope. Order `cheap`, `fast`, `standard`, `capable`. If the lowest tier is empty, step up. The result names the model used. |
| Precedence | An explicit ask (model or category) beats the agent frontmatter `model:`. The pin is only the default. |
| Effort | A scoped effort is clamped to the nearest level the model supports. |
| Main model | Never gets categories. Categories choose subagent models only. |
| `--models` | Session-only override of the scope, as `id:effort`. Not written to disk. Passed on to nested subagents. |

- Agent tool params: `model` (a category or a scoped reference) and `effort` (overrides the scoped effort). `complexity` is removed. Each result starts with `[model: ...]`.
- Keys in `/scoped-models`: `tab` cycles effort. `c` cycles category (bare letter, `scoped_models_selector.rs`). `alt+m` and `shift+alt+m` cycle scoped models and apply their saved effort.
- Effort names accepted over app-server: `off`, `minimal`, `low`, `medium`, `high`, `xhigh`.

### Scheduler and `/loop`

- Store: `<cwd>/.agents/scheduled_tasks.json`, with a lock file. Fields include `ownerPid`, `ownerSessionId` (display only) and `fireCount`. Both owner fields are optional, so old files load.
- Cron: 5 fields, local time. `1-7` is Monday to Sunday. No seconds, `L`, `#` or nicknames.
- Ticks run every 30 s, only while the agent is idle (`Mode::agent_busy()` is false).
- A fire is claimed under the file lock. A task fires once per due minute. The store records the due minute. A task is never due before its creation minute.
- Ownership: a claim fires a task only for its owner pid. If the owner pid is dead (`kill(pid, 0)`), the claimer adopts the task and the TUI warns. Ownerless tasks are never adopted. On non-unix, an owner is assumed alive.
- Late fire: a due prompt runs at the next idle tick within 10 minutes. After that it is skipped with a warning. One-shot tasks are removed either way.
- Fire message: `⏰ Scheduled task <id> (<cron>, recurring|once)`, then the prompt. A prompt that starts with `/` is sent unchanged, with a status line instead of the message.

| Form | Meaning |
|---|---|
| `/loop "<cron>" <prompt>` | Recurring cron task. |
| `/loop <N>m\|<N>h\|<N>d <prompt>` | Becomes `*/N * * * *`, `0 */N * * *` or `0 0 */N * *`. Minutes and hours must divide evenly. |
| `/loop once <schedule> <prompt>` | One-shot. |
| `/loop list`, `/loop delete <id>`, `/loop stop` | Manage. `stop` ends `/loop auto` and this session's `/loop` prompts. |
| `/loop auto [--max-turns N] <task>` | Re-prompts after each turn. Ends on `LOOP_DONE`, the cap, an abort or an error. Yields while the agent works, while messages are queued or while the editor has text. `AskUserQuestion` takes the recommended option, or stops the loop if there is none. |

- Print, rpc and subagent processes never tick.
- The tick uses the session's current cwd. The tools use the cwd the session started with. `/cd` makes them differ (AG-15).

### Extras

| Feature | Rule | Setting or switch |
|---|---|---|
| Version check | Once a day. Never in print, json or rpc. Asks npm for `@kolisachint/hoocode`. State in `~/.hoocode/version-check.json`. Never replaces the binary. | `checkForUpdates` (default true), `HOOCODE_OFFLINE=1`, `--offline` |
| Update command | `hoocode update` prints it. Under `~/.hoocode/bin` it is curl. Inside `node_modules` it is npm or bun. | none |
| Completion chime | One terminal bell for a turn over 10 s, not on abort. Right away for an `AskUserQuestion` question. Not for MCP elicitation. | `terminal.chimeOnTurnComplete` (default **false**) |
| Install telemetry | Sends nothing. The key is accepted and kept. | `enableInstallTelemetry` (default true) |
| OpenRouter attribution | Env switch. | `HOOCODE_TELEMETRY` |

### Web tools

- Web access is the webtools plugin, not a built-in tool: its tools are `WebFetch` and `WebSearch`, the names the plugin declares (plugins.md Naming and identity, PL-12, PL-17). Built in P2 (2026-11-19); the webtools plugin's own release renames its tools (P3).
- hoocode has no web flag, web settings or web helpers. The `Claude` aliases of `WebFetch` and `WebSearch` in agent frontmatter go: the names are the tools' own (PL-17).
- The permission gate still names a plugin's network calls `WebFetch` (`hoocode-code-permissions`). That is a permission category, not a tool name. The TUI picks a renderer from the plugin's hint (`web-fetch`, `web-search`), not from the name (tui.md External tools).

### DocSearch and capability index

- Tool `DocSearch`: params `query` (required) and `limit` (optional).
- Searches live capabilities only: kinds `subagent`, `plugin`, `skill`. Not docs (AG-19).
- Index: crate `hoocode-code-capabilities`. BM25 with camelCase and snake_case splitting. In-process, no dependencies.
- Semantic index: `--enable-semantic-index` and `enableSemanticIndex` (default true, read only when set in a file). Today it prints "semantic indexing is not available in this build" when `embsearch` is found, and "embsearch binary not found" otherwise. No embsearch client exists. `CodeSearch` stays lexical.

### Extensions

- Extensions are MCP servers, Agent Skills, subagents, and WASM components shipped in plugins. Plugins: plugins.md.
- Skills: `SKILL.md` frontmatter follows the Agent Skills rules. Problems are warnings. Plugin skills are named `<plugin>:<skill>`.
- `allowed-tools` in a plugin skill is parsed, then dropped with a warning that it is ignored. hoocode never honours it, and records no approval (PL-5, 2026-11-18).
- Overloaded word: "extension" also names mode extensions and UI requests (`hoocode-code-modes/src/extension.rs`).

### MCP client

- Library: rmcp. `Cargo.lock` has 3.6.0; `hoocode-agent-mcp/Cargo.toml` requires 3.5.1. Only `hoocode-agent-mcp` depends on rmcp (`migration/dep-firewall.json`).
- Crates: `hoocode-agent-mcp` (client, transports, OAuth) and `hoocode-code-mcp` (config discovery, trust, expansion).
- Transports: stdio and Streamable HTTP. Legacy HTTP+SSE is dropped.
- Config files, in precedence order:

| File | Scope | Trust |
|---|---|---|
| `~/.agents/mcp.json` | user | trusted |
| `./.agents/mcp.json` | project | folder grant, re-asked on change |
| plugin `mcp.json` | plugin | trusted once the plugin loads (user plugins), or after folder grant (project plugins) |

- Server fields: `command`, `args`, `env`, `url`, `headers`. `${VAR}` and `${VAR:-default}` expand at connect time in all of them except the names. An unset variable with no default fails that server. The diagnostic names the variable, never its value. The trust fingerprint hashes the text as written.
- Trust store: `mcp-trust.json` in the agent dir (`hoocode_code_paths::agent_dir()`). Shared with plugins. Outside the repo.
- Tool names: `mcp__<server>__<tool>` (plugins.md Naming and identity). A name that is too long keeps 55 characters, then `_` and 8 hex FNV-1a digits (`naming.rs`). Built in P2 (2026-11-19).
- Runtime: 60 s request deadline. One reconnect after a drop. Tool progress shows in the TUI. Aborting a turn cancels the call. Tool-list changes apply at the next turn (not verified, AG-11).
- Elicitation: asked in the TUI. Print and rpc decline it (AG-11).
- `/mcp` lists servers with state: connected, not trusted, auth needed, failed.
- OAuth: PKCE, metadata discovery, `iss` check. Client identity is the Client ID Metadata Document `https://kolisachint.github.io/hoocode/oauth-client.json` (constant `CLIENT_METADATA_URL` in `oauth.rs`). Dynamic registration is the fallback. The JSON file is not in this repo (AG-9).
- Token files: `<data dir>/mcp-auth/`, one per issuer, owner-only (0600 in a 0700 directory). Each file records its issuer, which is checked on load.
- A server with its own `Authorization` header connects with that header only. No OAuth, no stored tokens.
- Spec version: the old target `2026-07-28` is not pinned in our code. rmcp negotiates it (AG-10).
- Subagents get MCP tools only when their tool list names them (a selector such as `mcp:<s>` or `mcp:*`) or inherits all tools.

### Subscription logins: client identity and OAuth state

Logins for Claude (`anthropic`), ChatGPT (`openai-codex`) and Gemini (`google-gemini-cli`, with Antigravity) send a pinned client identity. These are source constants:

| Constant or header | File | Value | Sent as |
|---|---|---|---|
| `CLAUDE_CODE_VERSION` | `crates/hoocode-ai-provider-anthropic/src/request.rs:20` | `2.1.322` | `user-agent: claude-cli/<version>` on OAuth requests |
| `DEFAULT_ANTIGRAVITY_VERSION` | `crates/hoocode-ai-provider-google-gemini-cli/src/lib.rs:41` | `1.18.4` | Antigravity `User-Agent`. Env `HOOCODE_ANTIGRAVITY_VERSION` overrides it (existing, not added) |
| Gemini CLI headers | `crates/hoocode-ai-provider-google-gemini-cli/src/lib.rs` (`provider_headers`) | `google-cloud-sdk vscode_cloudshelleditor/0.1`, `gl-node/22.17.0` | `User-Agent`, `X-Goog-Api-Client` |
| Google OAuth `User-Agent` | `crates/hoocode-ai-oauth-google/src/gemini_cli.rs:104`, `antigravity.rs:145` | `google-api-nodejs-client/9.15.1` | token and login requests |
| `CODEX_ORIGINATOR` | `crates/hoocode-ai-provider-openai-codex/src/lib.rs:62` | `pi` | `originator` header and the `user-agent` prefix |
| `DEFAULT_ORIGINATOR` | `crates/hoocode-ai-oauth-openai-codex/src/lib.rs:25` | `pi` | `originator` in the authorize URL. Must equal `CODEX_ORIGINATOR` |

Not one of the three, same rule: `crates/hoocode-ai-oauth-github-copilot/src/lib.rs:13,17-19` (`GitHubCopilotChat/0.35.0`, `vscode/1.107.0`, `copilot-chat/0.35.0`).

Refresh: `grep -rn "CLAUDE_CODE_VERSION\|DEFAULT_ANTIGRAVITY_VERSION\|CODEX_ORIGINATOR\|DEFAULT_ORIGINATOR\|google-api-nodejs-client\|gl-node\|vscode_cloudshelleditor\|GitHubCopilotChat" crates`. The list above is the whole set.

OAuth `state`, as built:

| Provider | `state` sent | Callback check | Pasted input check |
|---|---|---|---|
| Anthropic | the PKCE verifier (`crates/hoocode-ai-oauth-anthropic/src/lib.rs:51`) | the callback server rejects a mismatch before it takes the code (`crates/hoocode-ai-oauth/src/callback.rs:219`, `CodeAndState`) | `parse_pasted` rejects a mismatch (`lib.rs:196`). A bare code with no `state` is accepted and the verifier is used |
| Google | the PKCE verifier (`crates/hoocode-ai-oauth-google/src/lib.rs:142`) | the flow compares to the verifier and fails with `OAuth state mismatch - possible CSRF attack` (`lib.rs:221`, `CodeAndStateDeferred`) | the same check (`lib.rs:235-236`). A pasted URL with no `state` is accepted |
| ChatGPT | a random UUID, not the verifier (`crates/hoocode-ai-oauth-openai-codex/src/lib.rs:37`, `:251`) | the callback server rejects a mismatch, then requires the code (`callback.rs:171`, `StateThenCode`) | `parse_pasted` rejects a mismatch when one is present (`lib.rs:219`). A bare code is accepted |

All three compare the returned `state` to the one they sent. A pasted bare code skips the comparison; that is the user's own paste.

### Plugins

Moved to [plugins.md](plugins.md): formats, locations, install and trust, model tools, the two extension tiers.

### App-server and rpc

App-server (`hoocode app-server`):

- Crates: `hoocode-app-server-protocol` (serde types, no I/O), `hoocode-app-server` (connections, threads, approvals, transports). The server does not depend on `hoocode-code-cli`. The CLI passes a `SessionFactory` trait object.
- Listen: `--listen stdio://` (default), `unix://` (default path) or `unix://PATH`.
- Default socket `<agent dir>/app-server-control/app-server-control.sock`. Directory 0700, socket 0600. If the path answers, refuse with "already running". Otherwise remove the stale file and bind.
- Framing: stdio is one JSON object per LF line. Unix socket is one JSON object per WebSocket text frame.
- Wire: no `"jsonrpc"` field (accepted if sent). Request `{id, method, params?}`. Response `{id, result}` or `{id, error:{code,message,data?}}`. Notification `{method, params?}`. Server-to-client request ids are integers from one counter.
- A connection must send `initialize` first, or get `-32600`. Requests on one connection run in order. Connections run in parallel. Turns run in the background.

| Method | Notes |
|---|---|
| `initialize` | Result: `userAgent`, `codexHome`, `platformFamily`, `platformOs`. |
| `thread/start` | New session in the server cwd. Optional `model`, `effort`. Caller subscribes. |
| `thread/resume` | By `threadId` or `path`. Rebuilds turns. Replays pending approvals to the caller. |
| `thread/list`, `thread/read`, `thread/unsubscribe` | Newest first, `limit` and `cursor`. `read` does not subscribe. |
| `turn/start` | `input` (text, data-URL or local image). Optional `model` and `effort`. Error if a turn is running. |
| `turn/steer`, `turn/interrupt` | Steer needs `expectedTurnId`. Interrupt denies pending approvals and answers `{}` at once. |
| Neutral stubs | `account/read`, `model/list`, `configRequirements/read`, `skills/list` (empty), `thread/loaded/list`. |

Approvals:

| hoocode tool | Server request | Item |
|---|---|---|
| `Shell` | `item/commandExecution/requestApproval` | `commandExecution` |
| `Edit`, `Write` | `item/fileChange/requestApproval` | `fileChange` |
| `WebFetch`, `WebSearch` | `item/commandExecution/requestApproval` | `dynamicToolCall` |

Gates read the origin, not the name: a plugin tool with one of these names is not gated (2026-11-19, P2b). The `WebFetch` category is the built-in network permission, which a plugin's network request is asked as, once per site.

- Decisions: `accept` runs once. `acceptForSession` runs and skips re-asking that tool on this thread, in memory only. `decline` denies. `cancel` denies and interrupts. Anything else counts as `accept` for that call. "Always" (global config write) is never offered.
- Policy is `hoocode-code-permissions` `evaluate` with "has UI" set true.
- First answer wins. Every subscriber gets `serverRequest/resolved`. Late answers are ignored.
- Interrupt, unload and shutdown deny every pending approval. A dropped connection does not deny. The request stays pending and is replayed to the next subscriber on `thread/resume`.
- The gate runs `block_in_place`, so the runtime must be multi-thread.

Model scope and effort over app-server:

- Precedence, first match wins: (1) explicit `effort`, used as sent and must be supported. (2) Scoped effort, when the request names a scoped model and switches to it. It is clamped, with no error. (3) Unchanged. Every check runs before any change, so a rejected request leaves the thread as it was.
- `model/list` puts scoped models first, with `category` and `alias`. Others follow with `hidden: true`, shown only with `includeHidden`.

| Error (all `-32602`) | Message |
|---|---|
| `model` not in scope | `model {name} is not in your scoped models ({list})` |
| `model` matches several entries | `"{name}" matches more than one scoped model: {list}. Use an alias or the full provider/id.` |
| `model` unknown or without auth (no scope) | `unknown or unavailable model {name}` |
| Bad effort name | `unknown effort {name}; use one of off, minimal, low, medium, high, xhigh` |
| Effort not supported | `effort {name} is not supported by {id}; supported efforts: {list}` |

Mapping: session = thread. One agent run = turn. User prompt = `userMessage`. Shell = `commandExecution`. Edit and Write = `fileChange`. Other tools = `dynamicToolCall`. Denied = `declined` (or `failed` for dynamic tools). Neutral values: `approvalPolicy: "on-request"`, `approvalsReviewer: "user"`, `sandbox: {type: "dangerFullAccess"}` (hoocode has no sandbox).

Tests: protocol serde round trips; faux-provider server tests; `schema_conformance` in `crates/hoocode-app-server/tests/it/server.rs` (ignored; schemas from `scripts/codex-schema.sh` into `target/codex-schema/`, never checked in). That check is not in CI (AG-27).

rpc (`--mode rpc`):

- No permission dialog. A gated tool is denied unless `auto_allow` lists it. Print and json do not gate (no UI).
- Warm subagent workers run headless. The pool sets `HOOCODE_INTERNAL_WARM_WORKER` (not a CLI flag). `HOOCODE_INTERNAL_APPROVALS_FAIL_CLOSED` marks fail-closed runs.
- A subagent child relays its approvals to its parent (`relays_clearance`).
- Dialog rules that still apply: a cancelled or aborted dialog denies. Abort, stdin EOF and session replacement deny every pending approval. An unknown answer id is ignored. The stdin reader never blocks.

## Decisions

| Date | Decision |
|---|---|
| 2026-10-01 | `hoocode app-server` speaks the Codex app-server wire format, in both directions. MIT only. |
| 2026-10-07 | DocSearch searches live capabilities only, with an in-process BM25 index. `embsearch` is not linked. |
| 2026-10-07 | Scheduled prompts run only in an open interactive session. A task fires only in the process that created it, until that process exits. |
| 2026-10-07 | A prompt due while busy runs at the next idle tick within 10 minutes. After that it is skipped with a notice. |
| 2026-10-08 | MCP is built on rmcp. Transports are stdio and Streamable HTTP. HTTP+SSE is dropped. |
| 2026-10-08 | MCP folder trust, no per-tool prompts. Trust re-asks when a server's command, args or URL changes. |
| 2026-10-09 | `scopedModels` is the one model-scope key. Categories and effort are per entry. Project list replaces global. |
| 2026-10-09 | Subagents use only scoped models when a scope exists. Explicit asks beat the frontmatter pin. |
| 2026-10-14 | Version check and turn chime are the only extras built. Thinking escalation, `/learn`, teams, voice, `/share` and telemetry are out. |
| 2026-11-02 | No migration from `enabledModels` or `modelCategories`. The old keys are ignored. |
| 2026-11-06 | The category key is `c`, bare letter, in `/scoped-models`. |
| 2026-11-06 | rpc fails closed: a gated tool is denied unless `auto_allow` lists it. Approval dialogs over rpc are not built. |
| 2026-11-07 | `DocSearch` is removed (with single-view decision 7): discovery via the prompt skill/agent listing, `Read`, CLI help and user pages. AG-19 closed as No. Code removal with the robustness matrix is the next session. |
| 2026-11-11 | Pinned client identity (the table above) stays as source constants. A person refreshes them by hand in a normal PR, when a provider refuses the old value or at each minor release, whichever comes first. No runtime fetch. No new env override. (AU-6) |
| 2026-11-11 | OAuth `state` is kept as built. Anthropic and Google send the PKCE verifier as `state`; ChatGPT sends a random UUID. Not verified against each provider's own client: the Claude Code login could not be read for how it builds `state` (its authorize endpoints were checked). Code unchanged. Risk: the verifier is in the authorize URL, so it can show in browser history and logs, which weakens PKCE against a leaked code. The redirect goes to a loopback address, and the code is single-use and short-lived. Revisit if a provider rejects it, or when someone can compare with the provider's client. (AU-7) |
| 2026-11-19 | Tool names follow plugins.md Naming and identity (option D). The origin is structured. A plugin's WASM tools use the names the plugin declares (`WebFetch`, `WebSearch` for webtools). MCP tools are `mcp__<server>__<tool>`. Old `ext_*` and `mcp_*` names are removed with no aliases. Tool selectors are in plugins.md. PL-17. |

## Open

| ID | Item | Size | Trigger or gate | Design notes |
|---|---|---|---|---|
| AG-2 | Decide MCP schema deferral | M | Measured token cost of MCP schemas | `deferMcpSchemas` (default true) has no consumer. Either map it to the provider `defer_loading` field (exists in `hoocode-ai-types`) or delete the key. Plain-content tool results (M2) come first, measured. |
| AG-3 | Delete the dead `packages` setting | S | None | `settings/manager.rs` ~882-906 has accessors with no callers. Remove them and their tests. |
| AG-9 | Publish the OAuth Client ID Metadata document | S | An OAuth MCP server needs CIMD | `CLIENT_METADATA_URL` points at a file that is not in this repo. Publish `oauth-client.json` on the Pages site, or rely on dynamic registration and say so. |
| AG-10 | MCP conformance run and manual runs | M | Before MCP counts as done | Run the conformance suite in CI. Do one manual run each against a real stdio server and a real OAuth server. Pin and state the spec version we claim. |
| AG-11 | Verify the unchecked MCP, plugin and scope rules | S | None | Check: print and rpc wait 10 s for trusted servers; tool-list change at next turn; `Authorization` 401 text; git input hardening; npm refusal; elicitation decline in print and rpc; `hoo-config.json` mode rule (`auto_allow` is unioned); scoped-model alias, untagged and frontmatter-precedence rules. Add tests or mark not built. |
| AG-12 | Wire `/goal` to the auto loop | M | User asks for `/goal` | `StartAutoLoop` maps to `None` in `hoocode-code-modes/src/extension.rs` (~457). Add a listener that calls the `/loop auto` path. |
| AG-13 | `/grill` skips its questions in the loop | S | With AG-12 | Not done. |
| AG-14 | Stale-lock fix with `flock` | S | Two processes take over one stale lock | Use `File::try_lock`, which needs Rust 1.89. The workspace MSRV is 1.95 since PL-7, so this is no longer blocked. |
| AG-15 | Cron batch 2 | M | None | Keep a one-shot until delivered. Add busy guards (retry, session op, attached view). Split the `/cd` store: tick uses the current cwd, tools use the start cwd. Handle a corrupt store. Handle DST. |
| AG-16 | Off switch for the three Cron tools | S | Measured token cost | Add a setting. Interactive default stays on. |
| AG-17 | Decide `/learn` | S | None | The tip shows as unavailable (`tips.rs`). Remove the tip, or build the command. Decision says no. |
| AG-18 | Thinking escalation | S | A way to measure its effect | Config is merged at `hoocode-code-modes/src/config.rs` (~69-78) but never applied. Apply it or delete the read. |
| AG-19 | Remove `DocSearch` (was: index docs) | S | Decided 2026-11-07 | Remove the tool (with UI-21 decision 7) instead of indexing `docs/user/`. Code removal with the robustness matrix is the next session. |
| AG-20 | Semantic code search | L | User asks for semantic search | Build the `embsearch` daemon client behind the `EmbsearchService` seam (line JSON over stdio). Make `--enable-semantic-index` do something. Fall back to keyword search with a reason. Until then, remove the flag or keep the "not available" message. |
| AG-21 | Un-defer MCP Apps | L | A tool the user relies on ships a `ui://` resource | Declare `text/html;profile=mcp-app`. Read `_meta.ui.resourceUri`. Serve a host page on `127.0.0.1` with a session token. Use a sandboxed iframe on a second origin. Open links only after confirmation. Done when a headless Chromium test passes, with CSP and origin tests. App tool calls use the same rules as model calls. |
| AG-22 | app-server: WebSocket on TCP with a token | M | Remote clients | Bind to loopback by default. Require a token. |
| AG-23 | app-server: `item/tool/requestUserInput` | M | A client needs it | Not built. |
| AG-24 | app-server: thread fork, archive and name | M | Client need | Not built. |
| AG-25 | app-server: daemon start and stop, idle timeout, launchd | L | Headless use | No daemon subcommand exists. |
| AG-26 | app-server: profiles and worker processes | L | Multi-profile use | One worker per (profile, workspace). Not built. |
| AG-27 | app-server schema check in CI | S | Decision | Add a CI step that runs `scripts/codex-schema.sh` and the ignored `schema_conformance` test. Schemas stay generated, never checked in. |

## Not doing

- Legacy MCP HTTP+SSE servers; MCP sampling, roots and logging; MCP registry browsing (`server.json`); MCP Tasks and Skills extensions; `/mcp import` and reading other tools' MCP config files.
- Being an MCP server (app-server is our server side).
- The `packages` setting (AG-3 deletes it).
- `SearchSkills` (skills stay in the prompt); linking `embsearch` (ONNX, about 40 MB); indexing docs (AG-19 decided 2026-11-07: remove `DocSearch`).
- Porting the hoocode web layer (`.webtoolsignore`, cache, notes); linking webtools crates into the hoocode binary (they ship in the webtools plugin); provider-native web search for now.
- Copilot canvas host; `/share` and HTML export; voice; teams client; `/learn`.
- Install telemetry that sends anything; thinking escalation until AG-18 is decided.
- Headless schedules in a daemon; cron syntax beyond the ported matcher (seconds, `L`, `#`, nicknames).
- Category picks for the main model; a separate settings key for the picker.
- rpc approval dialogs (`RpcPermissionUi`); `confirm`, `input` and `editor` dialogs over rpc; an rpc host for `AskUserQuestion`; "Always" over server protocols.
- Chime for MCP elicitation.
- app-server methods `collaborationMode/list`, `hooks/list`, `config/read`, `plugin/list` (left unimplemented; the Codex TUI accepts `method not found` for them).

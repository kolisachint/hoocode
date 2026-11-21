# Plugins

Status (2026-11-20): Plugins and extensions. Built: Agent Plugins 1.0 load and install, `/plugin`, opt-in model tools, the WIT world `hoocode:extension@0.1.0` (`wit/`), the WASM host (`hoocode-code-wasm`), and v1: the webtools plugin (`hoo-web-plugin/plugin/`, listed in the pre-trusted `hoo-plugins` marketplace). Tool identity and names are option D (decided 2026-11-19). Built in P2 (2026-11-19): `ToolOrigin`, the names (WASM tools as declared, MCP tools `mcp__<server>__<tool>`), selectors in `--tools` and agent lists, collision diagnostics. Built in P2b (2026-11-19): selectors in permission rules, a gate and clearance relays that read the origin, malformed selectors refused everywhere. Built in P4 (2026-11-19): the TUI origin label and renderer hints. Built in PL-18 (2026-11-20, branch `agent/pl18-core`): native helpers (tier C). See Naming and identity. Open: PL-1 to PL-5, PL-13 to PL-17, PL-19 to PL-22. Still ahead: the browser plugin built on tier C (PL-19 to PL-22). See Tiers, Native tier contract and Browser plugin.

## Principles

- **Standards first.** Agent Plugins 1.0 (agent-plugins.org) is the package format. A plugin's components are `skills/` (Agent Skills) and `mcp.json`. Hooks, commands, prompts and agents are not plugin components. Revisit when Agent Plugins 1.1 (portable hooks, a working draft) is final (PL-14).
- **hoocode-only parts live in `dev.hoocode/`** (Agent Plugins spec §6.1). Other clients ignore it (§7).
- **Tiers** (2026-11-19). See Tiers.
  - A, static: a Rust crate compiled into the hoocode binary. Future only.
  - B, WASM: a WASM component run in-process by hoocode's wasmtime host. It reaches the outside only through host functions, so the permission gate is enforced.
  - C, native helper: one long-lived process per hoocode session, shipped as a per-platform binary, speaking MCP over Streamable HTTP on a local socket. Trusted, not enforced. Shared by the root agent and its subagents.
  - External: a plain MCP server (stdio or Streamable HTTP). Any language. Its own process. Trusted, not enforced.
- **Deployment rule.** A small stateless tool is tier B. A tool that keeps state, owns a process or a daemon, or needs a runtime is tier C (shared by subagents) or a plain MCP server (not shared).
- **Contract over runtime.** The internal contract mirrors the MCP tool shape (name, description, JSON input schema; JSON in, JSON out). A tool can move between tiers by changing transport, not logic.
- **Standard WASI where it exists.** hoocode interfaces only where none does.
- **One trust decision per plugin**, at install or first launch. It covers everything in the plugin (MCP and WASM). The permission gate still applies to every gated action. Only user or project settings bypass the gate. A plugin may suggest permissions; it never grants them. Plugins ship no settings.
- **Subagents** get tier B tools and the tools of tier C helpers that declare `shared: true`. They never get plain MCP plugin tools or plugin-management tools (narrowed 2026-11-19).
- **User beats project.** A user plugin wins a name over a project plugin. An untrusted project plugin is not loaded.
- **Stop rule.** If PL-7 shows that feature trimming does not cut the build and size cost, re-decide before host code. If two releases after PL-12 there is still one WASM plugin, record it as a decision (PL-15).

Numbers a contributor needs:

| Item | Value | Source |
|---|---|---|
| Plugin marketplace cache | refreshed after 1 day | `plugin-marketplace-cache/<name>` |
| WASM call limits | 30 s per call, 64 MB memory, Esc cancels (epoch interruption) | decided, PL-9 |
| wasmtime line | 48 LTS, supported to about 2028-11; next LTS 60 | docs.wasmtime.dev stability-release |
| Model-facing tool name cap | 64 chars. Longer names keep 55 characters, then `_` and 8 hex FNV-1a digits of the full name | `hoocode-agent-mcp/src/naming.rs` (MCP names, built 2026-11-19) |

## How it works

### Package format

```
my-plugin/
├── plugin.json            # Agent Plugins 1.0 manifest
├── README.md
├── skills/<name>/SKILL.md # Agent Skills (+ scripts/ references/ assets/)
├── mcp.json               # external tier: MCP servers, any language
├── servers/               # convention only: what mcp.json launches via ${PLUGIN_ROOT}
└── dev.hoocode/
    └── extension.wasm     # internal tier: WASI 0.2 component, world hoocode:extension
```

- The component path is fixed: `dev.hoocode/extension.wasm`.
- Keys under `extensions."dev.hoocode"` in `plugin.json` are additive and override the convention. Used today: `tools` (renderer hints, see Naming and identity). Later: suggested permissions.
- Convention to config is additive. Config to convention would break plugins, so we start with the convention.

### Formats read

| Format | Layout | Notes |
|---|---|---|
| Agent Plugins 1.0 | root `plugin.json`, `skills/`, `mcp.json` | Schemas are hand-coded from the 1.0.0 spec and bundled, never fetched. Only the 1.0.0 `$schema` ids are accepted. |
| Claude | `.claude-plugin/plugin.json`, `skills/`, `.mcp.json` or inline `mcpServers` | Read only. Claude variables such as `${CLAUDE_PLUGIN_ROOT}` are not expanded. Plugin `agents/` is no longer loaded (decided 2026-11-17, PL-6). |
| Claude marketplace | `.claude-plugin/marketplace.json` | Read only. Never written. |

Agent Plugins 1.0 rules:

- `plugin.json` fields: `$schema`, `name`, `version`, `description`, `author`, `homepage`, `repository`, `license`, `keywords`, `extensions`. An unknown field is a warning. Any other violation rejects the plugin.
- `name`: 1 to 64 characters of `[a-z0-9.-]`, alphanumeric at both ends, no `--` or `..`.
- `skills/`: each direct subdirectory with a `SKILL.md`. Not recursive.
- `mcp.json`: its `$schema` version must match `plugin.json`, or MCP is off for that plugin. `stdio` `command` is one token with no expansion. `args`, `env` and `cwd` expand only `${PLUGIN_ROOT}` and `${PLUGIN_DATA}`, and `env` may not set those names. `streamable-http` needs HTTPS unless the host is loopback. A bad server is skipped and reported.
- Every declared path must resolve inside the plugin root. Unknown `extensions` namespaces are ignored.
- Ignored contents: hooks, commands, agents, prompts, themes and providers. The plugin still loads, and `/plugin list` reports what was skipped.

Written: ProposePlugin writes one Agent Plugins package only. Never Claude or Copilot layouts, never `marketplace.json`.

### Locations and files

| Path | What |
|---|---|
| `~/.agents/plugins/` | User plugins. |
| `<cwd>/.agents/plugins/` | Project plugins (folder grant). |
| `~/.agents/plugin-data/<name>` | `PLUGIN_DATA`. Kept across updates. Created when the plugin has an Agent Plugins `mcp.json`. |
| `plugin-installs.json` (hoocode data dir) | Install records: source, marketplace, trust. Kept out of the plugin folder so a change does not re-ask for trust. |
| `plugin-marketplace-cache/<name>` (hoocode data dir) | Git checkout of a marketplace. |
| `~/.hoocode/cache/wasm/<sha256>-<wasmtime-version>.cwasm` | Precompiled components. Folder mode 0700. Safe to delete (PL-9). |

### Install and trust

- Commands: `/plugin marketplace add|list|refresh`, `/plugin install|remove [--scope user|project]`, `/plugin list`, `/plugin update [name]`, `/plugin trust`.
- Marketplaces are git repositories. Git runs with `GIT_TERMINAL_PROMPT=0`. Values never reach git as options. `ext::` and `fd::` are refused. A symlink that leaves the plugin folder refuses the install. `npm` sources are refused.
- Two built-in marketplaces are pre-trusted: the official directory (`claude-plugins-official`, URL in code an assumption: `https://github.com/anthropics/claude-plugins-official`, not stated in the docs) and `hoo-plugins` (`https://github.com/kolisachint/hoo-plugins`). Both are listed first; a user-added marketplace with a built-in name is not listed.
- `marketplace add <src> --trust` marks a marketplace trusted. Without it, an install still succeeds (the user typed it), but its MCP servers need a grant in `/mcp`.
- Project plugins need one folder grant. The fingerprint covers every file (SHA-256 of path and bytes). Any change asks again.
- If the install records cannot be read, no marketplace counts as trusted (fail closed).
- A plugin's manifest must parse and name the plugin the marketplace lists, before it moves into place.

### Model tools

Opt-in, see [agent.md](agent.md) Tools: `enablePluginTools` (default false) or `--enable-plugintools`.

- `InstallPlugin` installs without asking from a trusted marketplace only. Accepted risk: with the tools on, a prompt injection can trigger an install and run from a trusted marketplace.
- `UninstallPlugin` and `ProposePlugin` ask. A proposal is untrusted. Its MCP servers wait for a `/mcp` grant. A project proposal waits for `/plugin trust`.
- Adding a marketplace stays human-only.
- Subagents never get these tools.

### Tiers

| Tier | Runs as | Language | Reach | Enforced? | Subagents | Status |
|---|---|---|---|---|---|---|
| A, static | Rust crate compiled into hoocode behind a cargo feature, against a published contract crate | Rust | anything | no | yes | Future |
| B, WASM | WASM component in hoocode (wasmtime) | any that builds WASI 0.2 components | host functions only | yes, gate on every gated host call | yes, gated calls via clearance | Built |
| C, native helper | one process per hoocode session, started on first use; MCP over Streamable HTTP on a local socket | any | anything the process can do | no, trusted (one trust decision per plugin) | yes, if `shared: true` | Built (PL-18) |
| C+, WASM host | a tier C helper that hosts WIT components and keeps them alive | as tier B | host functions only | yes | yes | Later, on demand (PL-22) |
| External | plain MCP server process (stdio or Streamable HTTP) | any | anything the process can do | no, trusted | no | Built |

- Webtools fetch and search: internal (network via gated `wasi:http`).
- Notebook editing: internal (needs `workspace`, interface 0.2).
- Notebook execution: external or a skill (needs a kernel process).
- Browser automation: tier C (`hoo-browser`, PL-19).

### Native tier contract (tier C, built 2026-11-20)

Why: a browser driver needs state, its own processes and access from many subagents at once. Tier B cannot start processes (WASI 0.3, ratified 2026-06-11, has no process interface) and keeps no state between calls. A plain stdio MCP server serves one client process, so subagents would each start their own copy.

Package. Agent Plugins 1.0 `mcp.json` names the server, as for any MCP plugin. hoocode-only fields go in `extensions["dev.hoocode"].native`:

| Field | Meaning |
|---|---|
| `server` | names the plugin's `mcp.json` stdio server that the helper speaks. Required. That server does not start as a plain MCP server |
| `shared` | `true`: one helper per hoocode session, reachable by subagents. Default `false`: root only |
| `binaries` | per platform (`linux-x86_64`, `linux-aarch64`, `darwin-aarch64`, `darwin-x86_64`, `windows-x86_64`): download URL and `sha256`. Shape follows `.mcpb` `platform_overrides` |
| `tools` | the tool list (required, non-empty): name, description, input schema (an object schema), permission hint (`read`, `act` or `dangerous`; missing means `act`). The model sees them before the helper starts |
| `binaryPath` (user setting) | use an installed binary instead of downloading (offline and IT-approved installs). `nativeHelpers.<plugin>.binaryPath` in user settings only (a project settings value is ignored), absolute path only |

A bad `native` block turns the native part off for that plugin with an Error diagnostic. The rest of the plugin loads, and the named server is removed from the plugin's plain-MCP list: it does not start as a plain MCP server. Tool names follow the tier B rules (bare names, no `mcp__` prefix in any case, case-insensitive, first in load order wins); across tiers each plugin registers its WASM tools then its native tools before the next plugin loads, so first in load order wins there too. Tier C tools have origin `ToolOrigin::Plugin`, so `plugin:<p>` selectors match them. The permission hint gates the call: `read` runs without asking; `act` asks like an edit or write builtin, and is denied the same way where the mode denies Write or Edit; `dangerous` always asks (blocked where nobody can answer) unless `auto_allow` names it, even under `skip_permissions`.

Transport. MCP Streamable HTTP over a Unix socket, never TCP. The client builds reqwest with `ClientBuilder::unix_socket(path)` and passes it to rmcp's `StreamableHttpClientTransport::with_client`. The URL is `http://localhost/mcp`. No `interprocess` crate, no hand-written hyper client.

- Linux and macOS: a Unix socket in a `0700` per-session directory under `$XDG_RUNTIME_DIR`, else the temp directory. Path under 100 bytes (macOS limit 104, Linux 108). A longer path falls back to a hash name, else the helper fails with a clear error.
- Windows: not shipped (EN-11). A Unix-socket server off Unix is a clear config error.

Identity. Every process that spawns a subagent mints the child's token (32 random bytes, hex) and agent id (`<parent agentId>/<task-id>`; the root is `root`). The child gets `HOOCODE_AGENT_ID`, `HOOCODE_AGENT_TOKEN`, `HOOCODE_AGENT_PARENT` and `HOOCODE_AGENT_DEPTH` in its environment (subagents start with `env_clear`, so the parent adds them explicitly). hoocode's MCP client sends `Authorization: Bearer <token>`, which the model never sees and which is never logged. Every `tools/call` also carries `_meta["dev.hoocode/agent"]` (`agentId`, `parentId`, `depth`). The socket directory is mode 0700; the bearer token is the check. Shell gets `HOOCODE_AGENT_ID`, `HOOCODE_AGENT_TOKEN`, `HOOCODE_HELPER_SOCKET_<PLUGIN>` and `HOOCODE_HELPER_BIN_<PLUGIN>` for each live helper, for CLI fallbacks: the binary is the downloaded helper or the `binaryPath`, so it is not on PATH. The child's Shell env learns both through `EnsureHelper` on its first shared tool call.

Lifecycle.

- Started on first use (first tool call, or the first EnsureHelper from a subagent), never at session start, by the root process only. Ready means the socket exists and an MCP `initialize` probe succeeds; the supervisor waits up to 10 s, else the call fails with a clear error.
- Launch: `<exe> <mcp.json args...> --parent-pid <root pid>`. Env is the mcp.json env plus `HOOCODE_HELPER_SOCKET`, `HOOCODE_HELPER_TOKEN` and `HOOCODE_AGENT_ID=root`. Stdin is null; stdout and stderr append to `<runtime dir>/<plugin>.log`. The child is `kill_on_drop` in its own session.
- Executable order: the user `binaryPath` (`nativeHelpers.<plugin>.binaryPath` in user settings only, absolute only) first, then the downloaded `binaries` entry for this platform, then the mcp.json entry's own `command` (`./` paths resolve under the plugin root, bare names use PATH). Args, env and cwd come from the mcp.json entry.
- A subagent reaches the helper through its parent. On first use of a shared helper it asks up the parent chain (`EnsureHelper`); each parent ensures its own connection (the root starts the helper), sends `hoocode/agentStarted` for the child over its own connection, and replies with the socket path. The child then connects with its own token. One EnsureHelper round trip per plugin per process: the socket and the parent's connection are cached and re-asked only after a recoverable error. A child with no parent channel gets no identity and no shared tools: every call would fail. That covers warm rpc workers (rpc installs no clearance relay, so no identity is minted) and children of an rpc root (their headless relay always fails closed, so it never counts as a parent channel).
- Restart: a call that finds the helper gone restarts it and retries once. A call cut mid-call by a helper death is never replayed: it fails saying it may or may not have run. Only pre-send failures (a connect failure, an auth rejection before the send) retry with a fresh EnsureHelper. At most 5 restarts in any 3 minutes; after that every call returns `<plugin> helper is down: <reason>` until the session ends. After a restart the root re-sends `hoocode/agentStarted` for every live registered agent.
- Notifications: `hoocode/agentStarted` (`agentId`, `parentId`, `depth`, `token`), `hoocode/agentEnded` (`agentId`), `hoocode/sessionEnding` (`{}`, empty params). A child that ends is ended on each helper it used.
- Session end: `hoocode/sessionEnding`, up to 2 s to exit, then kill. The runtime dir is removed.
- Download: once per version into `~/.hoocode/plugin-bin/<plugin>/<version>/`, skipped when present with a matching sha256. Fetched with hoocode's TLS client (proxy and CA), checksum verified before rename, installed mode 0755. Archive URLs (`.tar.gz`, `.zip` and kin) are refused: plain binaries only.
- The app-server runs no tier C: no native tools registered, no supervisor, no host. Single-session paths (TUI, print, json, rpc) are unaffected. Tier C returns to the app-server once the holder, hints and host are per-session state.

Built 2026-11-20 (branch `agent/pl18-core`): manifest fields (`server`), supervisor (download, verify, start, watch, restart), Unix-socket Streamable HTTP in `hoocode-agent-mcp` (reqwest `unix_socket` + rmcp), per-agent tokens, declared tools, the narrowed subagent rule, the EnsureHelper relay, `docs/user` and `docs/maps`.

### Browser plugin (designed 2026-11-19)

Requirements (user, 2026-11-19):

- Fast, reliable, parallel: subagents at once, several tabs per agent, root and subagents together.
- One shared browser and one shared login. Each agent owns its tabs. No agent takes another's tab, focus, element refs, dialogs, popups or downloads.
- Backends, all configurable: Chromium headless (default), headed Chromium, the user's real Chrome, Lightpanda (opt-in).
- Reuse agent-browser rather than write a browser driver.

Architecture:

- Engine: Vercel agent-browser (Apache-2.0), run as an external program at a pinned release. Never vendored or copied. Fixes go upstream first (PL-20); the plugin pins a release of `kolisachint/agent-browser` until each fix lands upstream, then returns to upstream releases.
- Plugin repo: `kolisachint/hoo-browser-plugin` (MIT). The old chromiumoxide driver is deleted; its flow and replay ideas stay as notes for later.
- Helper: `hoo-browser`, a Rust tier C helper (MIT). It calls the agent-browser CLI with `--json` once per tool call. Revisit only if its own overhead goes over 20 ms at p95.
- Leases: agent `A` owns sessions `A-1`, `A-2`, and so on (one per tab it drives at once). Every session attaches to the one shared browser (`AGENT_BROWSER_CDP`, pinned tab) and opens its own window (`window new`). Limits: 4 tabs per agent, 24 per session, 10 minutes idle. On `agentEnded` its sessions close, unless the root agent took them with `adopt`.
- Backends: Chromium headless by default (real screenshots). Headed Chromium shows one window per agent. The real Chrome uses the Chrome 144+ setting at `chrome://inspect/#remote-debugging`; the helper holds one connection, so the user should see one approval. If a managed Chrome forbids remote debugging, the helper says so and offers a separate profile. Lightpanda is opt-in, run as an external program (AGPL-3.0), telemetry off (`LIGHTPANDA_DISABLE_TELEMETRY`), text snapshots first.
- Tools (declared in the manifest): `browser_open`, `browser_snapshot`, `browser_act` (click, fill, select, check, hover, press, by ref), `browser_screenshot` (returns an image), `browser_tabs`, `browser_wait`, `browser_nav` (back, forward, reload), `browser_eval` (permission hint `dangerous`).
- Skill: when to use the browser, refs before screenshots, and the CLI fallback.
- CLI fallback: `hoo-browser <agent-browser command>` through Shell, scoped to the caller's sessions by `HOOCODE_AGENT_ID`. For the long tail (cookies, network, recording).
- Install: the plugin downloads its pinned `hoo-browser` and agent-browser binaries, checksum verified.

Evidence (2026-11-19, Linux, Chrome for Testing 153, agent-browser 0.43.0 and HEAD 25b5d76, Lightpanda nightly 1.1.0-nightly.12282):

| Finding | Number or source |
|---|---|
| agent-browser serializes commands per session (one daemon, one lock) | a second command waited 1305 ms behind `wait 1500`; another session answered in 4 ms (`cli/src/native/daemon.rs:690`) |
| 8 pinned sessions on one Chrome | cold start 0.5 to 1.1 s each; warm command 3 to 16 ms; 80 snapshots 266 ms parallel, 912 ms serial |
| A pinned tab that is not in front | clicks 5007 to 5011 ms (visible tab 18 ms); in headed mode clicks were dropped |
| Focus emulation on a background tab | clicks 6 ms, screenshots time out at 30 s |
| One window per agent plus focus emulation | headed: 8 of 8 clicks, screenshots 41 to 63 ms. Headless with `window new`: clicks 7 to 17 ms, screenshots 18 to 36 ms |
| Popups | no opener ownership: another session took a popup |
| Downloads | browser-wide, last setting wins (`browser.rs:2823`): one client's file landed in another's folder |
| `tab switch` | calls `Page.bringToFront` (`browser.rs:2274`), which takes focus |
| `agent-browser mcp` | one request at a time across all sessions (`mcp.rs:593`); warm call 21 ms |
| Daemon killed | without `AGENT_BROWSER_CDP` the next command started a private Chrome and orphaned the pinned tab |
| Windows | agent-browser's own daemon uses loopback TCP on Windows (`connection.rs:106`) |
| Lightpanda | one CDP connection = one browser with one context; connections do not share tabs or cookies; screenshots are rough |
| Chrome 136+ | remote debugging flags ignored on the default profile (developer.chrome.com/blog/remote-debugging-port). Chrome 144+: user toggle with approval (developer.chrome.com/docs/devtools/agents/use-cases/auto-connect) |

Industry survey (2026-11-19). The de facto agent tool plugin is an MCP server the host starts, over stdio (Claude Code, Codex, Gemini CLI, Goose, Cline, Continue, Cursor, Zed context servers). Servers shared by many processes use a named pipe on Windows and a Unix socket elsewhere (vscode-languageclient, Neovim, Docker); go-plugin uses loopback TCP with mTLS on Windows. Claude Code subagents share the parent's MCP connection. Zed WASM extensions return a command to spawn rather than doing the work. Nobody surveyed uses gRPC for editor or agent tools. Sources: modelcontextprotocol.io spec 2026-07-28 transports; agent-plugins.org/specification; github.com/modelcontextprotocol/mcpb MANIFEST.md; zed.dev/docs/extensions; code.claude.com/docs/en/sub-agents; github.com/hashicorp/go-plugin; learn.microsoft.com named-pipe security.

Not verified: approval prompts per connection on Chrome Stable 144+; macOS, Windows and Wayland; behaviour above 8 agents; Windows firewall behaviour.

Implementation plan (locked 2026-11-19). Each phase ends with its tests passing.

| Phase | Repo | Work | Done when |
|---|---|---|---|
| B0 | `kolisachint/agent-browser` | PL-20 fixes AB-1 to AB-4 as upstream PRs; pinned fork release (in progress 2026-11-21: branches `agent/ab-focus-popups`, `agent/ab-downloads-reattach`) | 8 agents on one headed Chrome pass the no-steal checks |
| B1 | hoocode | PL-18 tier C core | Done 2026-11-20: an echo tier C fixture answers root and 3 subagents at once, identity correct, on Linux and macOS |
| B2 | `hoo-browser-plugin` | PL-19 helper, tools, leases, backends, skill, CLI fallback (built 2026-11-21, pending merge) | the no-steal suite and the latency targets pass headless and headed (Xvfb) |
| B3 | `hoo-browser-plugin`, `hoo-plugins` | packaging, pinned downloads, marketplace listing, `docs/user` page (built 2026-11-21, pending merge) | `hoocode install` then a subagent eval drives the browser |
| B4 | all | real Chrome (144+) manual check; Lightpanda backend; AB-5 and AB-6 | recorded results in this doc |

Checks: 8 parallel subagents on one browser; popups, dialogs and downloads stay with their owner; no focus change; helper overhead under 20 ms at p95; a killed helper or Chrome recovers or fails cleanly; a subagent's windows close when it ends.

### Contract: `hoocode:extension@0.1.0`

| Interface | Source | Gated |
|---|---|---|
| `wasi:http` outgoing | WASI 0.2 | yes: WebFetch category; host SSRF block (private, loopback, link-local) on every hop; host timeouts and TLS |
| `wasi:filesystem`, plugin-data folder only (`~/.agents/plugin-data/<name>`) | WASI 0.2 | no |
| `wasi:clocks`, `wasi:random` | WASI 0.2 | no |
| `hoocode:extension/tool`: `list-tools() -> list of {name, description, input-schema-json}`, `call(name, args-json) -> result-json` | ours | n/a |
| `hoocode:extension/host`: `config-get`, `log` | ours | no |

The `wit/` package holds only hoocode's interfaces; WASI 0.2 interfaces are provided by the host and included by each plugin's own world (no WASI WIT is vendored: MIT-only repo).

- API keys (for example Brave, Tavily) come through `config-get` from user settings.
- Not in 0.1:
  - Workspace read and write (0.2, with its first consumer). Writes will count as Write and Edit.
  - Process spawning. Never internal; use MCP.
  - Sockets.
- The workspace is never a WASI preopen, because a preopen would bypass the gate.
- Versioning: WIT semver. The host supports the current and the previous minor world.
- The WIT source of truth is `wit/` in the hoocode repo. Plugin repos take it by git tag.
- WASI 0.2 (p2) is pinned. 0.3 (async) will be a new world version.

### Host runtime

- One owner crate, `hoocode-code-wasm`, links wasmtime (listed under `owners` in `scripts/ci/dep-firewall.json`). Always on. No cargo feature (one configuration to test).
- wasmtime 48 LTS. Patch updates when advisories land. Move to the next LTS (60) once a year. Track wasmtime security advisories: the sandbox is our security boundary.
- Toolchain: wasmtime 48 needs rustc 1.95. Since PL-7, `rust-toolchain.toml` pins 1.100.0 and `rust-version` is 1.95 (EN-7 checks it).
- Features (PL-7): `wasmtime` with `default-features = false` and `std`, `runtime`, `cranelift`, `component-model`, `async`, `parallel-compilation`; `wasmtime-wasi` with `p2`; `wasmtime-wasi-http` with `p2`, `default-send-request`. Off: `wat`, `profiling`, `cache`, `coredump`, `debug-builtins`, `pooling-allocator`, `gc`, `threads`, `component-model-async`. No transitive dependency turns them back on (`cargo tree -e features`). In 48, `Config::async_support` is a deprecated no-op.
- Precompile at install or trust into the `.cwasm` cache. Key: file SHA-256, wasmtime version, engine config. Loading a `.cwasm` is native code via an unsafe deserialize, so the folder is 0700 and owned by the user.
- A fresh store and instance per call (`InstancePre`). No state between calls. Persistent state goes in plugin data.
- Subagent processes load from the same cache.
- `HOOCODE_INTERNAL_WASM_ALLOW_LOOPBACK=1` (internal, tests only, not in `--help`) lets loopback through the SSRF block. It is read from the environment only, never from plugin config.

### Naming and identity

Each tool has two names. The host keeps the origin as data and never reads it back from a name. The model sees only the model-facing name.

| Part | Set by | Used for |
|---|---|---|
| Origin | the host | registry, permission gate, selectors, transcript and events, TUI, subagent tool lists, JSON output |
| Model-facing name | the rules below | the tool schema and the model's calls |

Origins (`ToolOrigin`):

- `BuiltIn`.
- `Plugin { plugin, tool }`: a WASM tool. `tool` is the name the plugin declares.
- `Mcp { server, tool, plugin }`: an MCP tool. `server` is the key in `mcp.json`, `tool` is the server's own name for it, and `plugin` is the owning plugin, or none for user and project config.

Model-facing names:

| Origin | Name | Example | Rule |
|---|---|---|---|
| Built-in | fixed | `Read`, `Agent` | Unchanged |
| Plugin WASM | the name the plugin declares in `tool-info.name` | `WebFetch` | Matches `^[A-Za-z][A-Za-z0-9_-]{0,63}$` and does not start with `mcp__` (any case), which is the MCP prefix. PascalCase recommended. No match: not loaded, with a diagnostic |
| MCP, any source | `mcp__<server>__<tool>` | `mcp__github__create_issue` | Each part: a character outside `[A-Za-z0-9_-]` becomes `_`, then every run of `_` in the part becomes one `_`, and leading and trailing `_` are dropped, so `__` only separates parts. An empty part: not loaded, with a diagnostic |

- Length cap: 64 characters. A longer name keeps its first 55 characters, then `_` and 8 hex digits of FNV-1a over the full name, as `naming.rs` does today.
- Plugins are written for hoocode, so their tools need no prefix. MCP servers are not, so their tools keep one.
- The WIT contract stays `hoocode:extension@0.1.0`. Only the doc comment on `tool-info.name` changes.

Collisions. A collision is never silent.

| Case | Result | Diagnostic |
|---|---|---|
| A plugin or MCP name equals a built-in name | Not loaded. A built-in is never shadowed | Names the tool and the built-in, one line per plugin or server and holder |
| Two plugin or MCP tools have the same model-facing name | The first in load order is loaded. The other is not | Names both origins, for example `plugin webtools` and `mcp github (plugin docs)` |

- Load order is the existing one: user before project. The MCP path must not overwrite a tool silently, as it does today.
- Names are compared case-insensitively, as selectors match them. A plugin tool `read` is not loaded next to the built-in `Read`, and `Foo` and `foo` from two tools give one holder. The notice names each tool as it declares it.
- Diagnostics show in `/plugin list` and in plugin diagnostics.

Selectors. Permission rules, `auto_allow`, agent `tools` and `disallowedTools`, and the `--tools` and `--disallowed-tools` flags match a selector against the origin, not the name.

| Selector | Matches |
|---|---|
| `Name` | The tool whose model-facing name is `Name`. Case-insensitive, as built-in names are today |
| `plugin:<p>` | Every tool of plugin `p`, WASM and MCP |
| `plugin:<p>/<Tool>` | One tool of plugin `p`. `<Tool>` is the tool's own name: the declared name for WASM, the server's name for MCP |
| `plugin:*` | Every plugin tool, WASM and MCP |
| `mcp:<s>` | Every MCP tool of server `s`, from any source. `s` is the key in `mcp.json` |
| `mcp:<s>/<tool>` | One MCP tool, by the server's name for it |
| `mcp:*` | Every MCP tool |

- No other form is a selector. `ext_*`, `mcp_*` and other globs are removed, with no alias.
- A selector names a tool, not a permission category. `WebFetch` as a selector matches the tool of that name. The gate still uses `WebFetch` as the category for a plugin's network calls, so the prompt says what is asked. Network for every plugin, whatever its tool, is the `WebFetch` category, not `plugin:<p>`: `plugin:<p>` takes the plugin's calls, and a mode whose `enabled_tools` or `auto_allow` do not take `WebFetch` blocks the network request even when the call itself runs (see the 2026-11-19 row).
- A malformed selector in an agent definition rejects that definition, with a diagnostic. In a permission rule it is refused, with a diagnostic. Fail closed.

Renderer hint. A plugin says how the TUI draws its tools, in `plugin.json`:

```json
"extensions": {
  "dev.hoocode": {
    "tools": {
      "WebFetch": { "render": "web-fetch" },
      "WebSearch": { "render": "web-search" }
    }
  }
}
```

- The key is the plugin-declared tool name. `render` is an id from hoocode's closed list. An absent or unknown id gives the generic renderer. A key for a tool the component does not declare is ignored, with a warning.
- The hint is data. It runs no code and grants no permission. MCP tools have no hint in v1 and use the generic renderer.

Webtools. The plugin `webtools` (`hoo-web-plugin/plugin/`) declares `WebFetch` and `WebSearch`, renamed from `fetch` and `search`. The plugin name stays `webtools`. Its MCP server keeps its own tool names, and the MCP rule applies to them.

Built in P2 (2026-11-19): names, origin, selectors in `--tools`, collision diagnostics. Built in P2b (2026-11-19): selectors in permission rules and agent lists, the gate by origin, malformed selectors refused. Built in P4 (2026-11-19): the hint is read from the manifest (`manifest.rs`), carried on the tool definition and the start event as `render`, and drawn by the TUI. A hint for a tool the component does not declare is ignored with a warning. The webtools plugin's own release declares its hints: until that release is installed, its `WebFetch` and `WebSearch` rows draw generically.

### Distribution

- Official marketplace: repo `hoo-plugins` (planned name `hoocode-official-plugins`), a listing only (`marketplace.json` entries). Pre-trusted alongside the existing official directory (PL-12).
- `hoo-web-plugin` (the webtools repo, renamed): the webtools libraries, native CLI and MCP server, plus the hoocode plugin package in `plugin/` (`plugin.json`, the skill, the component crate in `plugin/component/` and the built `dev.hoocode/extension.wasm`). A marketplace entry points at the repo with path `plugin`.
- The webtools libraries stay hoocode-agnostic. They have a `Transport` trait (one hop, no redirects; native reqwest, and `wasi:http` behind the `wasi` feature) and build for `wasm32-wasip2`. The CLI reads its own `~/.config/webtools/config.json` (`WEBTOOLS_CONFIG`), not `~/.hoocode/settings.json`.
- Plain MCP server binaries are expected on PATH. Tier C plugins ship per-platform binaries, downloaded and checksum verified (reversed 2026-11-19, see Native tier contract).
- `hoo-browser-plugin` (MIT): the `hoo-browser` tier C helper and the plugin package. Listed in `hoo-plugins` (PL-19).

### Measurements (2026-11-17)

Single runs, 4 vCPU, 15 GB. As-is hoocode vs. a copy with wasmtime 51.0.0-rc.1 + wasmtime-wasi + wasmtime-wasi-http (untrimmed).

| Measure | As-is | + wasmtime | Change |
|---|---|---|---|
| Clean `cargo check -p hoocode-code-main` | 67.5 s | 117.9 s | +75% |
| Clean dev build | 105.8 s | 179.1 s | +69% |
| Clean release build | 285 s | 498 s | +75% |
| Incremental, touch `main.rs` | 1.5 s | 2.8 s | +1.3 s |
| Incremental, touch `hoocode-code-tools` | 4.8 s | 6.5 s | +1.7 s |
| Units compiled | 356 | 444 | +88 |
| Peak build memory, release | 1.15 GB | 2.58 GB | x2.2 |
| `target/` after dev + release | 3.3 GB | 5.3 GB | +2 GB |
| Release binary (stripped) | 35.2 MiB | 48.6 MiB | +13.5 MiB |
| `hoocode --version` | 3.56 ms | 3.69 ms | noise |

Runtime (webtools HTML to markdown, 197 KB page):

| Measure | Value |
|---|---|
| Native | 9.93 ms |
| WASM per call | 11.29 ms (+14%) |
| First compile | 590 ms |
| `.cwasm` load | 0.36 ms |
| Instantiate | 0.08 ms |
| Engine and linker setup | 0.15 ms |
| `.wasm` size | 2.5 MiB |
| `.cwasm` size | 4.9 MiB |

CI fit:

- Public repo. ubuntu-latest (4 vCPU, 16 GB) fits.
- macos-latest (3 vCPU, 7 GB) fits. Disk is the tight spot (two darwin release targets on one runner).
- ubuntu-24.04-arm specs unknown. Check in PL-9.
- musl targets untested with wasmtime. Check in PL-9.

Accepted by the user 2026-11-17.

### PL-7 measurements (2026-11-17)

Single runs, same machine (4 vCPU, 15 GB), rustc 1.100.0. As-is hoocode vs. a scratch copy with wasmtime, wasmtime-wasi and wasmtime-wasi-http 48.0.9, trimmed (features under Host runtime). A probe in `hoocode-code-main` builds an engine, a linker with `wasi:p2` and `wasi:http`, and instantiates a component, so the code is linked. Untrimmed is the table above (wasmtime 51.0.0-rc.1, rustc 1.94.1), so its column is a different version and toolchain.

| Measure | As-is | Untrimmed (above) | Trimmed 48 | Trimmed change | Share of untrimmed cost left |
|---|---|---|---|---|---|
| Clean `cargo check -p hoocode-code-main` | 67.8 s | +50.4 s (+75%) | 96.1 s | +28.3 s (+42%) | 56% |
| Clean dev build | 115.5 s | +73.3 s (+69%) | 149.4 s | +33.9 s (+29%) | 46% |
| Clean release build | 300.6 s | +213 s (+75%) | 408.1 s | +107.5 s (+36%) | 50% |
| Incremental, touch `main.rs` | 1.5 s | +1.3 s | 2.3 s | +0.8 s | 62% |
| Incremental, touch `hoocode-code-tools` | 5.1 s | +1.7 s | 5.5 s | +0.4 s | 24% |
| Units compiled | 356 | +88 | 421 | +65 | 74% |
| Peak build memory, release | 1.12 GiB | x2.2 | 2.24 GiB | x2.0 | about the same |
| `target/` after dev + release | 2.8 GB | +2 GB | 4.3 GB | +1.5 GB | 75% |
| Release binary (stripped) | 35.2 MiB | +13.5 MiB | 49.4 MiB | +14.2 MiB | not cut |
| `hoocode --version` | 3.9 ms | noise | 3.5 ms | noise | — |

Reading: trimming halves the added build time (clean check, dev and release) and cuts incremental and disk cost. It does not cut the binary size or the peak build memory: thin LTO already dropped the unused features from the binary, and cranelift is most of what remains.

## Decisions

| Date | Decision |
|---|---|
| 2026-10-08 | Plugin model tools are opt-in (`enablePluginTools`, default false). Subagents never get them. |
| 2026-10-08 | Installs from a trusted marketplace do not ask. Accepted risk, recorded. |
| 2026-10-08 | Plugins written by hoocode are Agent Plugins packages only. No Claude or Copilot layouts, no `marketplace.json`. |
| 2026-10-08 | No `UpdatePlugin`, `PackagePlugin` or `PublishPlugin` tools. Updates use `/plugin update`. |
| 2026-10-15 | Plugin model tools use a fixed permission class each: read-only, install (trusted only), or ask. |
| 2026-11-17 | Plugin format follows Agent Plugins 1.0: skills and `mcp.json`. Hooks, commands, prompts and agents are not plugin components. Supersedes 2026-10-07 "no code-extension runtime". |
| 2026-11-17 | Claude layout is read for skills and `.mcp.json` only. Plugin `agents/` is no longer loaded (PL-6). |
| 2026-11-17 | Two tiers: internal WASM components (in-process, gate enforced) and external MCP servers (any language, trusted). A tool needing its own process, daemon or runtime is external. Supersedes the 2026-10-08 WASM deletion. |
| 2026-11-17 | hoocode-only parts live in `dev.hoocode/`. The component path is fixed at `dev.hoocode/extension.wasm`; manifest keys may be added later. |
| 2026-11-17 | The internal contract is the WIT world `hoocode:extension@0.1.0`, shaped like an MCP tool. 0.1 has `wasi:http` (gated), plugin-data filesystem, clocks, random, `config-get` and `log`. Workspace access waits for 0.2. |
| 2026-11-17 | One trust decision per plugin covers MCP and WASM. The gate applies to every gated host call. Only user or project settings bypass it. Plugins suggest permissions, never grant them. |
| 2026-11-17 | Subagents get WASM plugin tools (gated calls via clearance). Never MCP plugin tools or plugin-management tools. |
| 2026-11-17 | Agent tool lists accept `ext_*` (all plugin WASM tools). The built-in general-purpose agent has it; read-only built-ins do not, since plugin tools may write in workspace 0.2. (superseded 2026-11-19: `ext_*` is `plugin:*`) |
| 2026-11-17 | Names: `ext_<plugin>_<tool>` and `mcp_<plugin>_<server>_<tool>`. Built-ins are never shadowed. (superseded 2026-11-19: option D names) |
| 2026-11-17 | wasmtime 48 LTS, owned by one crate, always on. Toolchain pin to current stable, `rust-version` at least 1.95. Build and size cost accepted (Measurements). |
| 2026-11-17 | v1 is the webtools plugin only: `web-plugins` builds the component on the webtools crates; `hoocode-official-plugins` lists it; hoocode deletes its web helpers and `webtools.*` settings. Supersedes the 2026-11-06 web tools row and AG-1 (now PL-12). |
| 2026-11-17 | Notebooks wait for demand. Editing is internal (own serde_json editor, byte-identical round trips) with workspace 0.2. Execution is a skill using `jupyter execute`; an MCP server only for stateful kernels. |
| 2026-11-17 | MCP server binaries are expected on PATH. |
| 2026-11-17 | A test-only setting may allow loopback through the SSRF block. It is never readable from plugin config. |
| 2026-11-17 | PL-7: toolchain pinned to 1.100.0, `rust-version` 1.95. Trimmed wasmtime 48.0.9 halves the added clean build time (release +36% instead of +75%) but not the binary (+14.2 MiB) or peak build memory (x2.0). Stop rule: build cost is noticeably cut, size is not; the user decides before host code (PL-9). |
| 2026-11-17 | `wit/` defines `hoocode:extension@0.1.0` with only `types`, `tool` and `host`; the host provides WASI 0.2 (`wasi:cli/imports`, `wasi:http/outgoing-handler`) and plugins include it in their own world. WASI WIT is not vendored (MIT only). |
| 2026-11-17 | Plugin settings live under `pluginConfig.<plugin>.<key>` (`extensions` is an existing settings key). |
| 2026-11-17 | A WASM component loads under the same trust as the plugin's MCP servers (`marketplace_trusted`); untrusted plugins' components are skipped with a diagnostic. |
| 2026-11-17 | Network calls from a WASM tool ask once per origin per tool call (changed 2026-11-18), as WebFetch. Origin is scheme, host and port. Later requests to the same origin in the call (redirect hops) do not ask again, but each passes the SSRF block. A request to another origin asks again, and a denial holds only for its own origin. A URL with no origin is refused without asking. |
| 2026-11-17 | The WASM host sends HTTP itself (hyper, rustls with webpki roots, HTTP/1.1 only) and connects only to the addresses the SSRF check resolved, so DNS rebinding cannot reach a private address. wasmtime-wasi-http's `default-send-request` feature is off. |
| 2026-11-17 | The 30 s call limit and Esc are wall-clock, not only epoch ticks, so a call blocked in host I/O stops too. Time spent waiting on the permission prompt does not count. Response bodies have a 600 s between-bytes timeout. |
| 2026-11-17 | Plugin WASM tools load in the main session and subagents, but not in light mode, with `--no-tools`, or when a `--tools` list names no `ext_` tool (same rule as MCP). (2026-11-19: `ext_` reads as a plugin selector) |
| 2026-11-17 | The webtools repo is renamed `hoo-web-plugin` and also holds the plugin package (`plugin/`); no separate `web-plugins` repo. The official listing is `hoo-plugins`. The webtools CLI reads `~/.config/webtools/config.json` and env vars, never hoocode settings. |
| 2026-11-17 | Built-in web helpers, `--enable-webtools`, `enableWebTools` and `webtools.*` settings are removed; web access is the webtools plugin (PL-12). |
| 2026-11-18 | Plugin skills' `allowed-tools` is parsed and warned about, never honoured; the approval record is removed (PL-5). |
| 2026-11-18 | Plugin tools are live without `/reload` (PL-4). The plugin set's generation counter (`hoocode-code-plugins::generation`) moves on every `set_current`. A session with a `PluginSource` compares it before each prompt, between the turns of a run and on `/reload`, and when it changes it rebuilds the `ext_` tools (same trust rule, gate and cache as at start, names taken by built-ins and its own tools skipped) and its plugin MCP hub (the old hub is shut down, and the new hub's servers keep their trust). `/plugin` refreshes at once and asks for MCP trust; a model install is followed at the next turn, and its MCP trust is asked when the turn ends. Skills still load on `/reload`. A build problem is a warning, shown like the other plugin notices. Chosen over a full `/reload` on every change, which would replay the transcript and reset the editor. |
| 2026-11-18 | pluginConfig is read from user settings only (a project cannot configure a plugin). The project `.hoocode/settings.json` comes from the working tree, which may be untrusted. In agent definitions, WebFetch/WebSearch name the webtools plugin tools (`ext_webtools_fetch`, `ext_webtools_search`); to keep an agent off the network entirely, deny `ext_*`. (WebFetch/WebSearch part superseded 2026-11-19: they are the tools' own names; deny `plugin:*` to keep an agent off the network) |
| 2026-11-18 | WASM HTTPS uses hoocode's TLS trust set (same as providers): bundled roots plus `--ca-cert`, `HOOCODE_CA_CERT`, `NODE_EXTRA_CA_CERTS` and `--use-system-ca`, verification always on. The host gets the set from `hoocode_ai_util::tls::installed_trust()`. |
| 2026-11-18 | `/reload` rebuilds the plugin tools even when the plugin set's generation did not move, because each `ext_` tool holds the `pluginConfig` values it was built with and `/reload` re-reads settings. The plugin MCP hub is still rebuilt only when its servers changed. Per-turn refreshes keep the generation check, so a settings edit reaches plugin tools on `/reload` or a new session, not mid-run. |
| 2026-11-19 | When a plugin's HTTP request is refused, the tool error adds the refusal's reason (the gate's answer, or the host's own SSRF block such as "10.1.2.3 is private") to any failed call that recorded a refusal, whatever the plugin's own text says. The reason is per call. The guest sees only its own refusal message, which may not name the code. |
| 2026-11-19 | Collisions are reported one line per requesting plugin or server and holder: `plugin echo-dup: 3 tools not loaded (echo, foo, bar): names already taken by plugin echo`. A plugin tool name is checked by one validator (`valid_plugin_tool_name`, `^[A-Za-z][A-Za-z0-9_-]{0,63}$`) in the component loader and the CLI. |
| 2026-11-19 | Tool identity is structured (option D). The origin (`BuiltIn`, `Plugin`, `Mcp`) is carried everywhere; the model-facing name is for the model only. Supersedes the 2026-11-17 naming row. |
| 2026-11-19 | Model-facing names: a plugin's WASM tools use the name the plugin declares (PascalCase, `^[A-Za-z][A-Za-z0-9_-]{0,63}$`). MCP tools are `mcp__<server>__<tool>`, sanitised, 64 cap with an FNV-1a suffix. Old `ext_*` and `mcp_*` names are removed, with no aliases. |
| 2026-11-19 | Collisions: a built-in is never shadowed. Between plugin and MCP tools the first in load order wins and the other is not loaded. Always a diagnostic that names both origins. The silent MCP overwrite is a bug. |
| 2026-11-19 | MCP name parts drop leading and trailing `_` as well as collapsing runs, so no name has `___`. `a_` + `b` and `a` + `_b` both name `mcp__a__b`; the collision check then keeps one. |
| 2026-11-19 | A plugin WASM tool name may not start with `mcp__` (any case). That prefix is the MCP tools' own, so such a name is not loaded, with a diagnostic. |
| 2026-11-19 | Plugin network is always the `WebFetch` permission category (`ToolNetGate` asks `("WebFetch", BuiltIn)`), for every plugin and tool. A selector for the plugin (`plugin:<p>`) does not cover it. To allow a plugin's network in a mode, `enabled_tools` and `auto_allow` must list `WebFetch`. `enabled_tools: ["Read", "plugin:webtools"]` runs the plugin's call but blocks its request as "not enabled". |
| 2026-11-19 | Name collisions are case-insensitive (`Read` and `read` are one name). Holders are keyed by lowercase name in the component loader and in the session's admit step. The notice keeps each tool's own spelling. |
| 2026-11-19 | Selectors: `Name`, `plugin:<p>`, `plugin:<p>/<Tool>`, `plugin:*`, `mcp:<s>`, `mcp:<s>/<tool>`, `mcp:*`. They match origin. A malformed selector fails closed. |
| 2026-11-19 | Webtools tools are `WebFetch` and `WebSearch`, renamed from `fetch` and `search`. The `Claude` aliases and `ext_webtools_*` names go. Supersedes the WebFetch/WebSearch part of the 2026-11-18 row. |
| 2026-11-19 | Renderer hint: `extensions."dev.hoocode".tools.<Name>.render` in `plugin.json`. The TUI picks a renderer from the hint, never from a plugin or MCP tool name. Absent or unknown: generic. |
| 2026-11-19 | Built in P2. `ToolOrigin` lives in `hoocode-agent-types` (`tool_origin.rs`), the lowest crate both the MCP and the plugin paths reach. `ToolDefinition` and `AgentTool` carry it. `AgentEvent::ToolExecutionStart` has `origin` (`null` for a name the session does not have), and the JSON `tool_execution_start` event gets `"origin": {"kind": "builtIn" \| "plugin" \| "mcp", ...}`. A bare name in `--tools` may be a plugin tool, so it loads plugin components; `mcp__` names and `mcp:` selectors start MCP servers. Closed in P2b: calls carry their origin (`AgentToolCall.origin`, from the registry), and the harness and the permission gate read it, not the `mcp__` shape. |
| 2026-11-19 | Built in P2b. A plugin's network request is asked as the built-in `WebFetch` permission category (origin `BuiltIn`), not as the plugin's tool. A plugin's WASM tool is never gated at call level, so a plugin tool named `WebFetch` gets one prompt per origin. `WebSearch` is no longer a gated built-in name. |
| 2026-11-19 | Built in P2b. Malformed selectors fail closed. `--tools` or `--disallowed-tools` with a bad entry is a CLI error. An agent `tools` or `disallowedTools` entry that is not a selector rejects the definition. A bad entry in `auto_allow`, `denied_tools` or `enabled_tools` refuses to start hoocode (every problem is named). Until it is fixed, a malformed `denied_tools` blocks every call in its mode and an unreadable `auto_allow` allows nothing. Rules match names case-insensitively, as `--tools` does. |
| 2026-11-19 | An agent's `tools` and `disallowedTools` keep a bare name with no Claude alias as a `Name` selector (`tools: Lookup`), instead of dropping it. It matches case-insensitively at start. A `--tools` name that matches no tool warns at start (subagents too) and is ignored; it does not reject the definition. Mapped Claude names (`Bash`, `Grep`) are unchanged. Skills' `allowed-tools` still drop a name with no alias. |
| 2026-11-19 | Plugin tiers: A static (future), B WASM (built), C native helper (designed), C+ WASM host on tier C (later), plus plain MCP. Real work that needs state or processes is tier C, as in the industry survey (see Browser plugin). |
| 2026-11-19 | Tier C transport: MCP over Streamable HTTP on a local socket (Unix socket; Windows named pipe with an explicit DACL). No loopback TCP, no gRPC, no custom protocol. |
| 2026-11-19 | Tier C scope: one helper per hoocode session, started on first use by the root process. Not shared across sessions. |
| 2026-11-19 | Tier C identity: host-minted bearer token per agent, sent as a header the model never sees, plus `_meta["dev.hoocode/agent"]`. The model never names its agent. |
| 2026-11-19 | Subagent rule narrowed: subagents get the tools of tier C helpers that declare `shared: true`. Plain MCP plugin tools stay root-only. |
| 2026-11-19 | Reversed: tier C plugins ship per-platform binaries (download URL and sha256 per platform, `.mcpb` shape), with a `binaryPath` override for offline and IT-approved installs. Trust stays one decision per plugin. |
| 2026-11-19 | Tier C tools are declared in the manifest, so discovery needs no process start. |
| 2026-11-19 | Browser plugin: reuse agent-browser (Apache-2.0) as an external program at a pinned release; fixes upstream first; no own CDP driver; the old chromiumoxide driver in `hoo-browser-plugin` is deleted. |
| 2026-11-19 | Browser isolation: one shared browser and login; each agent owns sessions pinned to its own tabs, one window per agent, never `bringToFront`. Several tabs per agent means several sessions. Limits 4 per agent, 24 per session, 10 min idle. |
| 2026-11-19 | Browser backends: Chromium headless default; headed; real Chrome via the Chrome 144+ toggle; Lightpanda opt-in (AGPL, external only, telemetry off). |
| 2026-11-19 | Browser tools: 8 declared tools plus a Shell CLI fallback (`hoo-browser`) for the long tail. |
| 2026-11-19 | Browser implementation plan B0 to B4 locked (see Browser plugin). |
| 2026-11-20 | Tier C transport as built (D1). MCP Streamable HTTP over a Unix socket: reqwest `ClientBuilder::unix_socket(path)` into rmcp's `StreamableHttpClientTransport::with_client`, URL `http://localhost/mcp`. No `interprocess` crate, no hand-written hyper client. Windows named pipes are not shipped (EN-11): a Unix-socket server off Unix is a clear config error. Supersedes the 2026-11-19 tier C transport row. |
| 2026-11-20 | Tier C manifest as built (D2). `extensions."dev.hoocode".native` holds `server` (required: the plugin's stdio server, which then does not start as plain MCP), `shared` (default `false`), `binaries` (per platform: https URL and 64-hex sha256) and `tools` (required, non-empty: name, description, object input schema, `read`/`act`/`dangerous`, missing is `act`). A bad block turns native off with an Error diagnostic; the rest of the plugin loads. Resolution order: `binaryPath`, download, mcp.json `command`. Tool names follow the tier B rules; origin is `ToolOrigin::Plugin`. Supersedes the 2026-11-19 Reversed and declared-manifest rows. |
| 2026-11-20 | Tier C user setting as built (D3). `nativeHelpers.<plugin>.binaryPath` in settings, absolute path only. A relative path is ignored with a diagnostic. |
| 2026-11-20 | Tier C supervisor as built (D4). Runtime dir `$XDG_RUNTIME_DIR/hoocode-<rand8>` else the temp dir, mode 0700, removed at session end. Socket `<dir>/<plugin>.sock`, under 100 bytes else a hash name. Started on first use only. Launch is `<exe> <args> --parent-pid <pid>` with `HOOCODE_HELPER_SOCKET`, `HOOCODE_HELPER_TOKEN` and `HOOCODE_AGENT_ID=root`; log `<dir>/<plugin>.log`. Ready is socket plus `initialize`, up to 10 s. At most 5 restarts in 3 minutes, then `<plugin> helper is down: <reason>`. Download goes to `~/.hoocode/plugin-bin/<plugin>/<version>/`, sha256 verified, mode 0755; archives refused. |
| 2026-11-20 | Tier C agents and tokens as built (D5). The spawner mints each child's token (32 random bytes, hex) and id (`<parent>/<task>`; root is `root`). The child env carries `HOOCODE_AGENT_ID`, `HOOCODE_AGENT_TOKEN`, `HOOCODE_AGENT_PARENT` and `HOOCODE_AGENT_DEPTH`. Calls carry `_meta["dev.hoocode/agent"]` (`agentId`, `parentId`, `depth`). A child reaches a shared helper through `EnsureHelper` up the parent chain; each parent sends `hoocode/agentStarted` (`agentId`, `parentId`, `depth`, `token`) over its own connection. `hoocode/agentEnded` carries `agentId`; `hoocode/sessionEnding` carries `{}`. Shell gets `HOOCODE_AGENT_ID`, `HOOCODE_AGENT_TOKEN` and `HOOCODE_HELPER_SOCKET_<PLUGIN>`. Supersedes the 2026-11-19 tier C identity row. |
| 2026-11-20 | Tier C subagent rule as built (D6). Subagents get tier C tools only from `shared: true` plugins; `shared: false` tools are root-only and never built into a child registry. Plain MCP plugin tools stay root-only. Pins the narrowed 2026-11-19 subagent row. |
| 2026-11-21 | Browser daemon isolation: every agent-browser spawn gets `AGENT_BROWSER_SOCKET_DIR` set to the helper's socket dir, so two helpers (or two test runs) never share sessions even when lease names match. |
| 2026-11-21 | Browser CLI fallback is a hidden `browser_cli {args}` tool: callable, never listed by `tools/list`. It runs agent-browser scoped to the caller's own sessions; a `--session` naming another agent's session is refused. |
| 2026-11-21 | Browser lease naming: `<sanitized agent id>-<n>`, other characters mapped to `_`, sanitized id capped at 32 characters; collisions across agents get a suffix. |
| 2026-11-21 | Browser tools carry one permission hint each: open, snapshot, screenshot, tabs-list and wait are `read`; act, nav and tabs are `act` (`browser_tabs` = `act`); eval is `dangerous`. |
| 2026-11-21 | Shell also gets the helper binary (tier C). `HOOCODE_HELPER_BIN_<PLUGIN>` sits next to `HOOCODE_HELPER_SOCKET_<PLUGIN>`: same name mangling, the resolved executable the helper was started from (`binaryPath`, download, or mcp.json command). The supervisor records it per started helper; `EnsureHelper` answers carry it as an optional `exe` field, so old parents (no field) and old children (socket only) keep working. A plugin's skill runs its CLI fallback as `"$HOOCODE_HELPER_BIN_<PLUGIN>" cli <args>`. |
| 2026-11-20 | Tier C `binaryPath` is user-level only (review 1). `sync` reads global settings; a project settings value is ignored. A cloned repo cannot point a trusted plugin's helper at its own binary. |
| 2026-11-20 | Tier C is off in the app-server (review 7). No native tools registered, no supervisor, no host. TUI, print, json and rpc paths keep working. TODO: turn it back on once the holder, hints and host are per-session instead of process-wide. |
| 2026-11-20 | No replay after a mid-call helper death (review 9). The child retries with a fresh EnsureHelper only for connect failures and auth rejections before the request was sent; a mid-call disconnect returns an error saying it may or may not have run. |
| 2026-11-20 | Tier C `dangerous` ignores `skip_permissions`; `act` follows Write/Edit denials (review 10). A `dangerous` tool asks (blocked where nobody can answer) unless `auto_allow` names it, even under `skip_permissions`. An `act` tool is denied where the mode denies Write or Edit, and asks elsewhere. |
| 2026-11-20 | No shared tier C tools without a relay (review 11). Warm workers get no identity and are offered no shared tools; a child with no parent channel mints none. `EnsureHelper` for a `shared: false` plugin is refused before anything starts. |

## Open

| ID | Item | Size | Trigger or gate | Design notes |
|---|---|---|---|---|
| PL-1 | Drop Copilot and `.agents-plugin` plugin-root markers | S | Done 2026-11-18 | `hoocode-code-resources/src/skills.rs` `is_plugin_root` still lists `.agents-plugin/plugin.json`, `.github/plugin/plugin.json` and `.github/copilot-plugin.json`. Keep the native and Claude markers. |
| PL-2 | Rewrite the plugin-authoring skill text | S | Done 2026-11-18 | `builtin_skills.rs:47` names `UpdatePlugin` and hooks. Its tests pin the text, so change both together. Align the text with the 2026-11-17 decisions. |
| PL-3 | Fix the TUI plugin tool list | S | Done 2026-11-18 | `hoocode-code-tui-widgets/src/tools/plugins.rs` names `SuggestPluginInstall` (not a tool) and omits `UninstallPlugin`. |
| PL-4 | Live reload of plugin tools | M | Done 2026-11-18 | Plugin tools (`ext_`) and plugin MCP servers follow the plugin set at the next turn, from `/plugin`, the model's plugin tools, or `/reload`. See the 2026-11-18 decisions. |
| PL-5 | Enforce `allowed-tools` or delete the approval | S | Done 2026-11-18 | Approval record and its trust-file field deleted. `allowed-tools` is parsed and warned about as ignored (`hoocode-code-plugins/src/skills.rs` `ignore_allowed_tools`). Old trust files still load. |
| PL-6 | Stop loading Claude plugin `agents/` | S | Done 2026-11-17 | `hoocode-code-plugins/src/load.rs` ~309 and `lib.rs` `agent_files`. Treat `agents/` like the other ignored folders: warn and skip. Update `/plugin list`. |
| PL-7 | M0: measure trimmed wasmtime 48 and the toolchain bump | S | Gates PL-8 to PL-12 | Done 2026-11-17: toolchain bumped, numbers in PL-7 measurements. Waits for the user's go on the stop rule. |
| PL-8 | M1: WIT world `hoocode:extension@0.1.0` in `wit/` | S | Done 2026-11-17 (tag `wit-v0.1.0` pending) | Interfaces as in Contract. Tag it so plugin repos can pin. |
| PL-9 | M2: WASM host | L | Done 2026-11-17 (x86_64 musl: tests pass, static release binary 50.0 MiB; ubuntu-24.04-arm at the next release rehearsal) | Owner crate. Loader for `dev.hoocode/extension.wasm`, `.cwasm` cache, per-call instances, limits, `wasi:http` with gate and SSRF block, `config-get`, names, registry origin tag, subagent clearance. Check musl and ubuntu-24.04-arm builds. |
| PL-10 | M3: webtools `Transport` trait and WASM features (`hoo-web-plugin` repo) | M | Done 2026-11-17 | One GET without redirects per hop (fetch already loops by hand). Five search call sites. DNS guard moves to the host. Native CLI and MCP unchanged. Stop reading `~/.hoocode/settings.json`. |
| PL-11 | M4: plugin component and package (`hoo-web-plugin/plugin/`) | M | Done 2026-11-17 (branches `claude/eager-pascal-o0xkwr` in hoocode, hoo-web-plugin, hoo-plugins; live install needs them merged) | Builds `extension.wasm` in CI; skill on fetch and search; `plugin.json`. |
| PL-12 | M5: end to end and cleanup | M | Done 2026-11-17 (branches `claude/eager-pascal-o0xkwr` in hoocode, hoo-web-plugin, hoo-plugins; live install needs them merged) | List in `hoocode-official-plugins` and pre-trust it. Tests: marketplace add, install, trust; the echo fixture (`ext_echo_echo`) works through a mock LLM in a subagent (`subagent_evals`, `subagent_plugin_tool_*`, which fail on a denial in the tool result); live `ext_webtools_fetch` and `ext_webtools_search` are covered by a manual live check, planned as `scripts/eval/webtools_live.py`; a gated call prompts; the SSRF block refuses a private IP (loopback allowed only by the test setting). Delete hoocode `webfetch`/`websearch` helpers, `--enable-webtools`, `enableWebTools`, `webtools.*` settings. Update `docs/user/` and `docs/maps/` in the same commit. Replaces AG-1. |
| PL-13 | Notebook plugin | L | A user asks for notebooks | Workspace interface 0.2. Own `serde_json::Value` editor with `preserve_order`, indent 1, no ASCII escaping, new cell ids via `wasi:random`. Execution: a skill using `jupyter execute`; MCP server only for stateful kernels. |
| PL-14 | Revisit hooks | S | Agent Plugins 1.1 final | Read the portable hooks contract. Decide whether to support it. |
| PL-15 | Second-consumer review | S | Two releases after PL-12 | If only one WASM plugin exists, record it and decide whether to keep the host. |
| PL-16 | Plugin HTTP through an explicit HTTPS_PROXY (CONNECT) | M | A user behind an explicit proxy | Needs care: the proxy resolves the host, so the SSRF address pin can't apply; opt-in only. |
| PL-17 | Tool identity separate from the model-facing name (option D) | M | Now (roadmap item 0). Phases: P1 design (2026-11-19); P2 core and P2b (built 2026-11-19, not merged); P3 plugin; P4 TUI; P5 live end-to-end; P6 review | See Naming and identity. P2: `ToolOrigin`, names, selectors, collision diagnostics, evals, `docs/user`, `docs/maps`, CHANGELOG. P3: webtools `WebFetch` and `WebSearch`, hint, wasm rebuild. P4: origin label, `toolExternal`, hint dispatch, goldens (tui.md). P5: Tavily rerun, incl. a subagent in json mode. P6: Opus review of the three branches before any PR. |
| PL-18 | Tier C core: native helpers | L | Done 2026-11-20 (branch `agent/pl18-core`) | Built: manifest fields (`server`), supervisor (download, verify, start, watch, restart), Unix-socket Streamable HTTP in `hoocode-agent-mcp` (reqwest `unix_socket` + rmcp), per-agent tokens, declared tools, the narrowed subagent rule, the EnsureHelper relay. Left: Windows (EN-11), archives in downloads, warm-pool grandchildren have no EnsureHelper. |
| PL-19 | Browser plugin `hoo-browser` (B2, B3) | L | After PL-18 and AB-1 to AB-4 | See Browser plugin. Repo `hoo-browser-plugin`. |
| PL-20 | agent-browser fixes upstream (B0, B4) | M | Now, independent of PL-18 | In `kolisachint/agent-browser`, as PRs to Vercel. AB-1 window and focus emulation for pinned sessions, never `bringToFront`. AB-2 popups owned by the opener's session (`openerId`). AB-3 per-session download folder, routed by frame. AB-4 a session bound to a CDP target reattaches to it after a crash, never launches a private browser. AB-5 `agent-browser mcp` serves sessions concurrently and takes its default session from the environment. AB-6 named pipe instead of loopback TCP on Windows. |
| PL-21 | Tier A: static plugin crates | M | A plugin is promoted into core | Published contract crate and cargo features; one static binary. |
| PL-22 | Tier C+: WASM host helper | M | A tier B plugin needs state across calls | A tier C helper that keeps WIT components alive and shares them across subagents. Not for the browser. |

## Not doing

- A Node extension host; native dynamic libraries as plugins.
- Hooks, commands, prompts and agents in plugins (until Agent Plugins defines them, PL-14).
- Plugin themes and providers; `.agents-plugin/` and Copilot layouts; writing Claude or Copilot layouts; writing `marketplace.json`.
- `UpdatePlugin`, `PackagePlugin` and `PublishPlugin` tools; npm plugin sources.
- Process spawning or raw sockets from WASM components.
- A cargo feature that turns the WASM host off.
- Per-tool approval prompts beyond the gate; settings shipped by plugins.
- Per-platform binaries in plain MCP or tier B plugins (tier C ships them, 2026-11-19).
- Loopback TCP or gRPC for plugin helpers.
- A hoocode-owned CDP browser driver; adopting `agent-browser mcp` as the browser plugin.
- Sharing a tier C helper across hoocode sessions.

# Plugins

Status (2026-11-19): Plugins and extensions. Built: Agent Plugins 1.0 load and install, `/plugin`, opt-in model tools, the WIT world `hoocode:extension@0.1.0` (`wit/`), the WASM host (`hoocode-code-wasm`), and v1: the webtools plugin (`hoo-web-plugin/plugin/`, listed in the pre-trusted `hoo-plugins` marketplace). Built names are `ext_<plugin>_<tool>` and `mcp_<server>_<tool>`. Decided 2026-11-19 (option D, not built): tool identity and names, see Naming and identity. Open: PL-1 to PL-5, PL-13 to PL-17.

## Principles

- **Standards first.** Agent Plugins 1.0 (agent-plugins.org) is the package format. A plugin's components are `skills/` (Agent Skills) and `mcp.json`. Hooks, commands, prompts and agents are not plugin components. Revisit when Agent Plugins 1.1 (portable hooks, a working draft) is final (PL-14).
- **hoocode-only parts live in `dev.hoocode/`** (Agent Plugins spec §6.1). Other clients ignore it (§7).
- **Two tiers.**
  - Internal: a WASM component run in-process by hoocode's wasmtime host. It reaches the outside only through host functions, so the permission gate is enforced.
  - External: an MCP server. Any language. Its own process. Trusted, not enforced.
- **Deployment rule.** A tool that needs its own process, a daemon, or a runtime (Python, Node, a kernel) is external. Everything else may be internal.
- **Contract over runtime.** The internal contract mirrors the MCP tool shape (name, description, JSON input schema; JSON in, JSON out). A tool can move between tiers by changing transport, not logic.
- **Standard WASI where it exists.** hoocode interfaces only where none does.
- **One trust decision per plugin**, at install or first launch. It covers everything in the plugin (MCP and WASM). The permission gate still applies to every gated action. Only user or project settings bypass the gate. A plugin may suggest permissions; it never grants them. Plugins ship no settings.
- **Subagents** get internal (WASM) plugin tools, never MCP plugin tools, never plugin-management tools.
- **User beats project.** A user plugin wins a name over a project plugin. An untrusted project plugin is not loaded.
- **Stop rule.** If PL-7 shows that feature trimming does not cut the build and size cost, re-decide before host code. If two releases after PL-12 there is still one WASM plugin, record it as a decision (PL-15).

Numbers a contributor needs:

| Item | Value | Source |
|---|---|---|
| Plugin marketplace cache | refreshed after 1 day | `plugin-marketplace-cache/<name>` |
| WASM call limits | 30 s per call, 64 MB memory, Esc cancels (epoch interruption) | decided, PL-9 |
| wasmtime line | 48 LTS, supported to about 2028-11; next LTS 60 | docs.wasmtime.dev stability-release |
| Model-facing tool name cap | 64 chars. Longer names keep 55 characters, then `_` and 8 hex FNV-1a digits of the full name | `hoocode-agent-mcp/src/naming.rs` (PL-17 changes the rule) |

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

### Two tiers

| Tier | Runs as | Language | Reach | Enforced? | Subagents |
|---|---|---|---|---|---|
| Internal | WASM component in hoocode (wasmtime) | any that builds WASI 0.2 components (Rust, also JS, Python, Go) | host functions only | yes, gate on every gated host call | yes, gated calls via clearance |
| External | MCP server process (stdio or Streamable HTTP) | any | anything the process can do | no, trusted | no |

- Webtools fetch and search: internal (network via gated `wasi:http`).
- Notebook editing: internal (needs `workspace`, interface 0.2).
- Notebook execution: external or a skill (needs a kernel process).

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
| Plugin WASM | the name the plugin declares in `tool-info.name` | `WebFetch` | Matches `^[A-Za-z][A-Za-z0-9_-]{0,63}$`. PascalCase recommended. No match: not loaded, with a diagnostic |
| MCP, any source | `mcp__<server>__<tool>` | `mcp__github__create_issue` | Each part: a character outside `[A-Za-z0-9_-]` becomes `_`, then every run of `_` in the part becomes one `_`, so `__` only separates parts. An empty part: not loaded, with a diagnostic |

- Length cap: 64 characters. A longer name keeps its first 55 characters, then `_` and 8 hex digits of FNV-1a over the full name, as `naming.rs` does today.
- Plugins are written for hoocode, so their tools need no prefix. MCP servers are not, so their tools keep one.
- The WIT contract stays `hoocode:extension@0.1.0`. Only the doc comment on `tool-info.name` changes.

Collisions. A collision is never silent.

| Case | Result | Diagnostic |
|---|---|---|
| A plugin or MCP name equals a built-in name | Not loaded. A built-in is never shadowed | Names the tool and the built-in |
| Two plugin or MCP tools have the same model-facing name | The first in load order is loaded. The other is not | Names both origins, for example `plugin webtools` and `mcp github (plugin docs)` |

- Load order is the existing one: user before project. The MCP path must not overwrite a tool silently, as it does today.
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
- A selector names a tool, not a permission category. `WebFetch` as a selector matches the tool of that name. The gate still uses `WebFetch` as the category for a plugin's network calls, so the prompt says what is asked. Network from other plugin tools is covered by `plugin:<p>`.
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

Built today: `ext_<plugin>_<tool>` and `mcp_<server>_<tool>`. This section is the target. It is built in PL-17 (P2 to P4).

### Distribution

- Official marketplace: repo `hoo-plugins` (planned name `hoocode-official-plugins`), a listing only (`marketplace.json` entries). Pre-trusted alongside the existing official directory (PL-12).
- `hoo-web-plugin` (the webtools repo, renamed): the webtools libraries, native CLI and MCP server, plus the hoocode plugin package in `plugin/` (`plugin.json`, the skill, the component crate in `plugin/component/` and the built `dev.hoocode/extension.wasm`). A marketplace entry points at the repo with path `plugin`.
- The webtools libraries stay hoocode-agnostic. They have a `Transport` trait (one hop, no redirects; native reqwest, and `wasi:http` behind the `wasi` feature) and build for `wasm32-wasip2`. The CLI reads its own `~/.config/webtools/config.json` (`WEBTOOLS_CONFIG`), not `~/.hoocode/settings.json`.
- MCP server binaries are expected on PATH. Plugins do not ship per-platform binaries.

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
| 2026-11-19 | When a plugin's HTTP request is refused, the tool error adds the refusal's reason (the gate's answer, or the host's own SSRF block such as "10.1.2.3 is private") after the guest's `HttpRequestDenied`. The reason is per call. The guest still sees only the code. |
| 2026-11-19 | Tool identity is structured (option D). The origin (`BuiltIn`, `Plugin`, `Mcp`) is carried everywhere; the model-facing name is for the model only. Supersedes the 2026-11-17 naming row. |
| 2026-11-19 | Model-facing names: a plugin's WASM tools use the name the plugin declares (PascalCase, `^[A-Za-z][A-Za-z0-9_-]{0,63}$`). MCP tools are `mcp__<server>__<tool>`, sanitised, 64 cap with an FNV-1a suffix. Old `ext_*` and `mcp_*` names are removed, with no aliases. |
| 2026-11-19 | Collisions: a built-in is never shadowed. Between plugin and MCP tools the first in load order wins and the other is not loaded. Always a diagnostic that names both origins. The silent MCP overwrite is a bug. |
| 2026-11-19 | Selectors: `Name`, `plugin:<p>`, `plugin:<p>/<Tool>`, `plugin:*`, `mcp:<s>`, `mcp:<s>/<tool>`, `mcp:*`. They match origin. A malformed selector fails closed. |
| 2026-11-19 | Webtools tools are `WebFetch` and `WebSearch`, renamed from `fetch` and `search`. The `Claude` aliases and `ext_webtools_*` names go. Supersedes the WebFetch/WebSearch part of the 2026-11-18 row. |
| 2026-11-19 | Renderer hint: `extensions."dev.hoocode".tools.<Name>.render` in `plugin.json`. The TUI picks a renderer from the hint, never from a plugin or MCP tool name. Absent or unknown: generic. |

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
| PL-17 | Tool identity separate from the model-facing name (option D) | M | Now (roadmap item 0). Phases: P1 design (2026-11-19, this change); P2 core; P3 plugin; P4 TUI; P5 live end-to-end; P6 review | See Naming and identity. P2: `ToolOrigin`, names, selectors, collision diagnostics, evals, `docs/user`, `docs/maps`, CHANGELOG. P3: webtools `WebFetch` and `WebSearch`, hint, wasm rebuild. P4: origin label, `toolExternal`, hint dispatch, goldens (tui.md). P5: Tavily rerun, incl. a subagent in json mode. P6: Opus review of the three branches before any PR. |

## Not doing

- A Node extension host; native dynamic libraries as plugins.
- Hooks, commands, prompts and agents in plugins (until Agent Plugins defines them, PL-14).
- Plugin themes and providers; `.agents-plugin/` and Copilot layouts; writing Claude or Copilot layouts; writing `marketplace.json`.
- `UpdatePlugin`, `PackagePlugin` and `PublishPlugin` tools; npm plugin sources.
- Process spawning or raw sockets from WASM components.
- A cargo feature that turns the WASM host off.
- Per-tool approval prompts beyond the gate; settings shipped by plugins.
- Per-platform binaries inside plugins.

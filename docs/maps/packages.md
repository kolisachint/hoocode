# Package map

What each crate does, who uses it, and where to make a change. Snapshot of 2026-10-08, after the deletions in
[decisions-2026-10-08.md](../../archive/docs/decisions-2026-10-08.md): 68 crates (65 after the deletions, plus `runtime`, added in Concurrency phase 1, and `agent-mcp` and `code-mcp`, the MCP client and its discovery and trust ([mcp.md](../design/mcp.md)); 76 before; see
[Deleted](#deleted-2026-10-08)). In step 0 the crates
become `hoocode-*` and the binary `hoocode`
([naming-and-paths.md](../../archive/docs/naming-and-paths.md)); this page is regenerated
then. The UI side is mapped in [ui.md](ui.md).

**Keep this current.** Add, delete or rename a crate → update this page in the same
commit. A generator script with a CI staleness check replaces the tables in the first
coding session (see Upkeep).

## The binary, top down

```
code-main (bin `hoocode`)
  └─ code-cli ── parses args, picks a mode, wires the session (runtime.rs)
       ├─ interactive → code-tui-app ── code-tui-widgets, code-tui-selectors, code-tui-theme,
       │                                code-tui-keybindings → tui-* library
       ├─ print / json → code-print
       ├─ rpc         → code-rpc (also how subagent children talk to the parent)
       └─ app-server  → app-server, app-server-protocol (hoobot, Codex TUI)
  all modes → code-agent-session (AgentSession: one agent's lifecycle)
                ├─ agent-core → agent-loop (turns, parallel tools) → ai-registry → ai-provider-*
                ├─ agent-session / code-session (session files, tree)
                ├─ agent-compaction
                ├─ code-tools (bundle) → code-tool-bash, code-tools-fs, code-tool-search,
                │                        code-tools-optin, code-tool-api
                ├─ code-subagents (pool, warm pool, child processes, ledger)
                ├─ code-resources (skills, prompts, slash commands, agent defs, AGENTS.md)
                ├─ code-modes, code-permissions (ask/plan/build, approvals)
                └─ code-settings, code-paths, code-auth, code-models
```

Layering: `ai-*` knows nothing of agents; `agent-*` knows nothing of the coding agent;
`code-*` is the product; `tui-*` is a UI library with no agent knowledge.
`scripts/ci/dep-firewall.json` keeps volatile third-party crates in their owner crate.

## Where to change things

| To change… | Start in |
|---|---|
| A CLI flag or mode | `code-cli` (`args.rs`, `runtime.rs`) |
| The home directory, and `~` in shown paths (one policy, decision 28) | `home` (`hoocode-home`): `home_dir`, `shorten_path` |
| The system prompt | `code-prompts` (`system_prompt.rs`) |
| A built-in tool | `code-tool-bash`, `code-tools-fs`, `code-tool-search`, `code-tools-optin`; registration in `code-tools` |
| The Cron tools (CronCreate, CronList, CronDelete), the schedule store, the cron matcher | `code-scheduler`; registered in `code-cli` `runtime.rs` `custom_tools`; fired by `code-tui-app` `mode/events.rs` `tick_scheduler`; the Schedule rows of the task panel: `rows.rs` ([scheduler-and-loop.md](../design/scheduler-and-loop.md)) |
| How a tool looks in the transcript | `code-tui-widgets/src/tools/` ([ui.md](ui.md)) |
| A slash command | list: `code-resources/src/slash_commands.rs`; handler: `code-tui-app/src/mode/commands.rs` `run_builtin_command` |
| Screen goldens: the scenarios, the mock LLM | `scripts/tui/` (`goldens.py`, `mockllm.py`, `scenarios/`); component goldens use the `tui-render` feature `golden` ([ui.md](ui.md)) |
| Transcript render cache: a `Container` keeps each child's lines under the child's `render_stamp` (a stamp changes with every `&mut` mutator; none while a clock or registry is read) and draws only changed children; debug builds redraw each hit and assert equality (`HOOCODE_RENDER_CACHE_CHECK=0` off). Theme changes bump `bump_render_epoch` | `tui-render` (`component.rs`: `Component::render_stamp`, `Container`, `Rendered`); stamps in `tui-components` (`Spacer`, `Text`) and `code-tui-widgets` (`AssistantMessageComponent`, `ToolChainComponent`, `ToolExecutionComponent`, `MissionHeader`, `LandingLine`, `PeekOnly`); epoch bump in `code-tui-theme` (`registry.rs`); a resumed history freezes all but the newest `LIVE_TOOL_WINDOW` blocks (`mode/transcript.rs`, `render_session_context`) |
| Terminal writes (the `hoocode-term-out` thread, one frame in flight), input batching, SIGWINCH, SIGTERM and SIGHUP | `tui-terminal` (`output.rs`, `interrupt.rs`), `tui-render` (`tui.rs`), `runtime` (`signals.rs`: `watch_sigwinch`, `watch_terminate`); [concurrency.md](../design/concurrency.md) Phase 2 |
| Ctrl+C fast path and emergency exit, the ESC timer (no thread on Unix), the UI heartbeat, the panic hook (restore, flush, log, chain) | `tui-terminal` (`interrupt.rs`, `stdin_hub.rs`, `panic_hook.rs`); the UI beats and sets the hook in `code-tui-app` `run`; the panic hook is installed in `code-tui-app` `mode/mod.rs` `install_panic_hook`; [concurrency.md](../design/concurrency.md) Phase 2 |
| Shell pipes read by async tasks on `hoocode-io` (no thread per pipe) | `code-tool-bash` (`operations.rs`) |
| Background shell jobs: `Shell` with `run_in_background`, the job registry and its logs, `ShellOutput`, `ShellKill` (opt-in, `enableBackgroundShell`) | `code-tool-bash` (`jobs.rs`, `job_tools.rs`; each job is a Shell-tab row, `set_task_rows`, T5.3; a job's stdout and stderr are its log file, with no pipe); the session's registry is built in `code-cli` `runtime.rs` (`background_job_registry`); completion notices reach the model in `code-agent-session` `services.rs` (`transform_context`) |
| Turn logic, parallel tool calls | `agent-loop` |
| Session file format | `agent-session` (entries, storage), `code-session` (manager) |
| Session file writes (queue caps, flush barriers, the `hoocode-session-io` thread) | `runtime` (`session_io.rs`); `code-session` `manager.rs` `persist` and `rewrite_file` queue the writes ([concurrency.md](../design/concurrency.md) Phase 3) |
| Compaction | `agent-compaction` |
| Subagents | `code-subagents` (see `docs/design/subagents.md`); a run's live activity for the UI (coalesced text deltas, the bounded tail, Agent call to run lookup) in `live.rs` ([tui-activity.md](../design/tui-activity.md) T3.1); a child with no screen asks its parent to clear a gated call over its pipes (`clearance_request` on stdout, `clearance_response` on stdin; the child's `ChildRelay`, the pool's relay hook, `instance::set_clearance_relay`) in `clearance.rs` |
| MCP client: connect to a server, list and call its tools, progress, caps, tool names, OAuth | `agent-mcp` (`hoocode-agent-mcp`; the only crate that depends on `rmcp`; `oauth.rs`; see [mcp.md](../design/mcp.md)) |
| MCP servers in a session: start the trusted ones in the background, `mcp_<server>_<tool>` tool definitions, shutdown | `code-agent-session` `mcp.rs` (`McpHub`); attached in `code-cli` `runtime.rs` (`attach_mcp`, `mcp_wanted`); synced at each turn in `session.rs`; the MCP rows of the task panel (server state, in-flight calls, login and abort): `mcp_rows.rs` |
| A model provider | `ai-provider-<name>`, registered in `ai-registry` |
| A login flow | `ai-oauth-<name>`, stored by `code-auth` |
| The model list | `ai-models-catalog` (`data/models.json`, generated from models.dev by `hoocode-models-sync`; hand rules in `data/overrides.json`), `code-models` (models.json) |
| Settings keys | `code-settings` |
| Config paths, env prefixes | `code-paths` |
| MCP server discovery, `mcp.json` precedence, folder and plugin trust, `/mcp` states | `code-mcp` (`config.rs`, `trust.rs`, `discover.rs`); design in [mcp.md](../design/mcp.md) |
| Permissions and modes | `code-permissions`, `code-modes` |
| Themes | `code-tui-theme` |
| Key bindings | `code-tui-keybindings` (app), `tui-keys` (parsing) |
| Threads, the io runtime, `run_blocking`, channel caps, parallel-tool cap, the ordered shutdown (`run_ordered`, timed steps) | `runtime` (`runtime.rs`, `threads.rs`, `limits.rs`, `shutdown.rs`; the exit path's steps are in `code-tui-app` `mode/shutdown.rs`); the only crate that builds runtimes, threads or channels ([concurrency.md](../design/concurrency.md)) |
| `performance.*` settings | `code-settings` (`manager.rs` getters), caps in `runtime` (`limits.rs`) |

## All crates

"Used by" counts workspace crates that depend on it (not dev-deps). Lines are
`src/` / `tests/`. The tables below are generated by `scripts/maps/packages.py`,
which rewrites only the part between the markers. The Status column is hand-edited
in those tables; the generator keeps each crate's Status by name.

<!-- BEGIN generated: packages -->
### Runtime and leaves (`runtime`, `home`)

| Crate | Does | Used by | src / tests lines | Status |
|---|---|---|---|---|
| `home` | The user's home directory and `~` display: one policy for every hoocode crate | 11 | 153 / 0 | keep |
| `runtime` | Threads, runtimes, blocking pool, channels, watchdog and memory limits for the hoocode process: the only crate that builds them | 17 | 2197 / 1279 | keep |

### AI: models, providers, logins (`ai-*`)

| Crate | Does | Used by | src / tests lines | Status |
|---|---|---|---|---|
| `ai-env` | Environment and API key handling for hoocode AI | 8 | 343 / 0 | keep |
| `ai-models` | LLM model registry and discovery for hoocode AI | 11 | 344 / 247 | keep |
| `ai-models-catalog` | Model catalog data (LLM and image models) for hoocode AI, generated from models.dev | 1 | 24 / 0 | keep |
| `ai-oauth` | OAuth core for hoocode AI: types, PKCE, callback server, provider registry | 7 | 1084 / 0 | keep |
| `ai-oauth-anthropic` | Anthropic (Claude Pro/Max) OAuth flow for hoocode AI | 1 | 544 / 0 | keep |
| `ai-oauth-github-copilot` | GitHub Copilot OAuth device flow for hoocode AI | 1 | 675 / 0 | keep |
| `ai-oauth-google` | Google Cloud Code Assist OAuth flows (Gemini CLI, Antigravity) for hoocode AI | 1 | 1448 / 0 | keep |
| `ai-oauth-openai-codex` | OpenAI Codex (ChatGPT Plus/Pro) OAuth flow for hoocode AI | 1 | 599 / 0 | keep |
| `ai-provider-anthropic` | Anthropic provider for hoocode AI | 1 | 2770 / 518 | keep |
| `ai-provider-faux` | Faux / test provider for hoocode AI | 0 | 931 / 816 | test provider (dev-dep only) |
| `ai-provider-google` | Google Gemini provider for hoocode AI | 2 | 3248 / 110 | keep |
| `ai-provider-google-gemini-cli` | Google Cloud Code Assist (Gemini CLI / Antigravity) provider for hoocode AI | 1 | 1720 / 0 | keep |
| `ai-provider-openai` | OpenAI provider for hoocode AI | 1 | 3404 / 808 | keep |
| `ai-provider-openai-codex` | OpenAI Codex (ChatGPT subscription) Responses provider for hoocode AI: SSE and WebSocket transports | 1 | 2556 / 0 | keep |
| `ai-provider-openai-responses` | OpenAI Responses API provider for hoocode AI | 2 | 2260 / 0 | keep |
| `ai-registry` | API provider registry for hoocode AI: dispatches streams on model.api | 4 | 308 / 3007 | keep |
| `ai-sse` | Server-Sent Events decoder shared by the hoocode AI providers | 2 | 216 / 12 | keep |
| `ai-stream` | Streaming response utilities for hoocode AI | 12 | 600 / 0 | keep |
| `ai-types` | Shared types for hoocode AI | 40 | 1261 / 0 | keep |
| `ai-util` | Shared utilities for hoocode AI: JSON repair, hash, headers, sanitization, overflow detection | 13 | 3844 / 163 | keep |

### Agent runtime (`agent-*`)

| Crate | Does | Used by | src / tests lines | Status |
|---|---|---|---|---|
| `agent-compaction` | Session compaction for hoocode agents | 3 | 1447 / 1315 | keep |
| `agent-core` | Core agent runtime for hoocode agents | 2 | 1977 / 0 | keep |
| `agent-harness` | Agent harness for hoocode agents | 6 | 3310 / 1042 | keep |
| `agent-loop` | Agent loop for hoocode agents | 1 | 3065 / 0 | keep |
| `agent-mcp` | MCP client for hoocode agents on rmcp: stdio and Streamable HTTP servers, tool calls with progress, caps and deadlines | 1 | 1864 / 1666 | keep |
| `agent-session` | Session trees for hoocode agents: entry format, storage, repositories | 2 | 2409 / 0 | keep |
| `agent-types` | Shared types for hoocode agents | 21 | 956 / 0 | keep |

### Coding agent (`code-*`, `app-server*`)

| Crate | Does | Used by | src / tests lines | Status |
|---|---|---|---|---|
| `app-server` | hoocode app-server: Codex app-server protocol over stdio and a Unix socket | 1 | 2758 / 1568 | keep |
| `app-server-protocol` | Wire types for hoocode's app-server (Codex app-server protocol compatible) | 1 | 1204 / 0 | keep |
| `code-agent-session` | AgentSession: the agent lifecycle shared by the hoocode run modes | 8 | 7820 / 6613 | keep |
| `code-auth` | Credential storage for the hoocode coding agent: auth.json API keys and OAuth tokens with locked refresh | 5 | 864 / 822 | keep |
| `code-capabilities` | Capability index for the hoocode coding agent: BM25 search over loaded skills, subagents and plugins (DocSearch) | 1 | 574 / 0 | keep |
| `code-cli` | CLI argument parsing and mode dispatch for the hoocode coding agent | 1 | 6345 / 0 | keep |
| `code-main` | Main entry point for the hoocode coding agent | 0 | 6 / 1640 | the `hoocode` binary |
| `code-mcp` | MCP server discovery and folder/plugin trust for the hoocode coding agent: mcp.json sources, precedence, trust store and /mcp states (no MCP client here) | 2 | 2059 / 0 | keep |
| `code-media` | Image handling for the hoocode coding agent: format sniffing, resize/re-encode for model input | 3 | 1582 / 608 | keep |
| `code-migrate` | One-time merge of the pre-1.2 coding-agent folders into ~/.hoocode | 1 | 758 / 334 | keep |
| `code-models` | Model registry for the hoocode coding agent: built-in catalog plus models.json custom providers and overrides | 5 | 2125 / 1040 | keep |
| `code-modes` | Modes for the hoocode coding agent: ask/plan/build/debug prompts, hoo-config.json, /mode /plan /grill /goal /approve | 3 | 1055 / 1099 | keep |
| `code-paths` | App identity, config directories and path helpers for the hoocode coding agent | 19 | 1146 / 475 | keep |
| `code-permissions` | Permission gate for the hoocode coding agent: per-mode tool policy from hoo-config.json and approval prompts | 4 | 980 / 860 | keep |
| `code-plugins` | Plugins for the hoocode coding agent: loading Agent Plugins 1.0 and Claude plugin layouts (discovery, precedence, folder trust, and the skills, subagents and MCP servers they contribute), and installing them from git marketplaces | 4 | 4398 / 2034 | keep |
| `code-print` | Output formatting for the hoocode coding agent | 1 | 297 / 281 | keep |
| `code-prompts` | Prompt templates for the hoocode coding agent | 2 | 829 / 0 | keep |
| `code-resources` | Resources for the hoocode coding agent: skills, prompt templates, slash commands, agent definitions, context files | 7 | 4471 / 3067 | keep |
| `code-rpc` | RPC mode for the hoocode coding agent | 2 | 1564 / 1046 | keep |
| `code-scheduler` | Cron scheduler for the hoocode coding agent: the CronCreate, CronList and CronDelete tools and the store that fires due prompts | 2 | 2290 / 0 | keep |
| `code-session` | Session handling for the hoocode coding agent | 6 | 2128 / 746 | keep |
| `code-settings` | Global and project settings.json for the hoocode coding agent | 9 | 2412 / 1829 | keep |
| `code-subagents` | Subagent orchestration for the hoocode coding agent | 3 | 10864 / 11187 | keep |
| `code-task-store` | In-process task store for the hoocode coding agent (TodoWrite plan items, subagent runs) | 8 | 1112 / 0 | keep |
| `code-tool-api` | Shared tool plumbing for the hoocode coding agent: tool definitions, output truncation, path resolution | 11 | 1072 / 125 | keep |
| `code-tool-bash` | The Shell tool for the hoocode coding agent: shell resolution, process-tree kill, streamed and truncated output, background jobs (ShellOutput, ShellKill) | 5 | 3033 / 1463 | keep |
| `code-tool-search` | CodeSearch for the hoocode coding agent: ranked lexical code search (ripgrep libraries), fusion and reranking | 1 | 1956 / 1185 | keep |
| `code-tools` | Coding tools for the hoocode coding agent | 4 | 1701 / 0 | keep |
| `code-tools-fs` | File tools for the hoocode coding agent: read (with read-dedup) | 4 | 3352 / 2478 | keep |
| `code-tools-optin` | Opt-in tools for the hoocode coding agent: TodoWrite, AskUserQuestion and the plugin tools | 3 | 963 / 1000 | keep |
| `code-update` | Startup version check and the `hoocode update` command for the hoocode coding agent | 2 | 505 / 0 | keep |

### Coding agent UI (`code-tui-*`)

| Crate | Does | Used by | src / tests lines | Status |
|---|---|---|---|---|
| `code-tui-app` | The coding agent's interactive mode on the hoocode TUI | 1 | 24412 / 7398 | keep |
| `code-tui-keybindings` | The coding agent's keyboard map: app keybindings, keybindings.json loading and hint text | 3 | 786 / 888 | keep |
| `code-tui-selectors` | The coding agent's pickers and dialogs on the hoocode TUI | 1 | 9033 / 4789 | keep |
| `code-tui-theme` | Color themes for the hoocode coding agent's interactive mode | 6 | 2524 / 2142 | keep |
| `code-tui-widgets` | The coding agent's chat transcript widgets on the hoocode TUI | 2 | 14344 / 9826 | keep |

### TUI library (`tui-*`)

| Crate | Does | Used by | src / tests lines | Status |
|---|---|---|---|---|
| `tui-components` | UI components for the hoocode TUI | 4 | 10466 / 7579 | keep |
| `tui-editing` | Text editing primitives for the hoocode TUI | 1 | 303 / 0 | keep |
| `tui-fuzzy` | Fuzzy matching for the hoocode TUI | 4 | 365 / 0 | keep |
| `tui-glyphs` | The TUI status glyphs and their animation frames: one set for every surface | 3 | 98 / 0 | keep |
| `tui-highlight` | Syntax highlighting for the hoocode TUI, using the highlight.js 10.7.3 engine and grammars | 1 | 2327 / 291 | keep |
| `tui-images` | Terminal image rendering for the hoocode TUI | 4 | 1048 / 377 | keep |
| `tui-keys` | Keyboard handling for the hoocode TUI | 7 | 1956 / 583 | keep |
| `tui-ratatui` | Generic ratatui adapter for the hoocode TUI: renders ratatui widgets into the line-based Component model (Mission Control frame chrome, palette, RatatuiComponent) | 3 | 1510 / 172 | keep |
| `tui-render` | Differential rendering for the hoocode TUI | 6 | 4922 / 2800 | keep |
| `tui-terminal` | Terminal abstraction for the hoocode TUI | 2 | 3273 / 343 | keep |
| `tui-util` | Shared utilities for the hoocode TUI | 15 | 2495 / 682 | keep |

### Other

| Crate | Does | Used by | src / tests lines | Status |
|---|---|---|---|---|
| `models-sync` | Regenerates the model catalog data from models.dev (the models-sync tool) | 0 | 848 / 323 | keep |

<!-- END generated -->

## Deleted 2026-10-08

Ten crates were removed, as agreed in
[decisions-2026-10-08.md](../../archive/docs/decisions-2026-10-08.md). Azure was dropped
later the same day (65 crates), so `ai-provider-azure` is listed below too.

| Crate | Why | What moved or is left |
|---|---|---|
| 5 umbrellas: `hoocode`, `agent`, `ai`, `code`, `tui` | Only for crates.io, which is dropped | `ai`'s tests moved to `ai-registry` (below); `ai`'s `oauth_providers` test was dropped, as it only tested the umbrella's own install wrapper |
| `agent-mcp` | **Kept 2026-10-14, not deleted.** It is the home of the MCP client that [mcp.md](../design/mcp.md) (card 9.1) rebuilds on rmcp | See [decisions-2026-10-14.md](../design/decisions-2026-10-14.md) |
| `agent-orchestrator`, `agent-tools` | hoocode SDK ports the product never calls; `code-agent-session` does the job | Nothing |
| `ai-images` | No tool or command generates images | Not done: `ai-models-catalog`'s `IMAGE_MODELS_JSON` and `data/image-models.json` still exist and are maintained by hand (models.dev lists no image models). A follow-up. |
| `code-extensions` | Unused WASM host; costs ~55 s per clean build | `build-speed.md` D5 no longer applies |

**Moved to `ai-registry`** (`crates/hoocode-ai-registry/tests/it/`): `cache_retention`,
`cross_provider_handoff`, `github_copilot`, `live_matrix`, `routing`, `stream_hooks`, with
their `tests/data/red-circle.png`. Imports now name the leaf crates directly.

**Azure removed (`ai-provider-azure`, 65 crates).** The decision said "Not used", and the
provider was wired into `ai-registry` only, so it was deleted with its `azure-openai-responses`
API, the `AZURE_OPENAI_*` env vars, the `/login` display name, the help text and the 47
`azure-openai-responses` entries in `ai-models-catalog`. `ai-provider-openai-responses` keeps
the shared Responses plumbing; only its doc comments mentioned Azure. The catalog now has
1182 entries at the time. The models.dev sync (`hoocode-models-sync`) filters the Azure
provider out (`DROPPED_PROVIDERS`), so a regeneration does not bring it back. Its
`data/overrides.json` adds `claude-haiku-5-5` (anthropic) when models.dev lacks it.

## Upkeep

Refresh the tables with:

```bash
python3 scripts/maps/packages.py          # rewrite the generated tables
python3 scripts/maps/packages.py --check  # exit 1 with a diff if the page is stale
```

The generator reads `cargo metadata` and writes only the part between the markers:
"Does" from each crate's `Cargo.toml` `description`, "Used by" and the line counts
from the tree, and Status kept by name. A new crate gets Status `keep`; set it by
hand. `--check` runs in CI after the rename (step 0c). Keep crate descriptions
accurate, since they are the "Does" column.

# Package map

What each crate does, who uses it, and where to make a change. Snapshot of 2026-10-08, after the deletions in
[decisions-2026-10-08.md](../design/decisions-2026-10-08.md): 66 crates (76 before; see
[Deleted](#deleted-2026-10-08)). In step 0 the crates
become `hoocode-*` and the binary `hoocode`
([naming-and-paths.md](../design/naming-and-paths.md)); this page is regenerated
then. The UI side is mapped in [ui.md](ui.md).

**Keep this current.** Add, delete or rename a crate → update this page in the same
commit. A generator script with a CI staleness check replaces the tables in the first
coding session (see Upkeep).

## The binary, top down

```
code-main (bin `cortex`)
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
`migration/dep-firewall.json` keeps volatile third-party crates in their owner crate.

## Where to change things

| To change… | Start in |
|---|---|
| A CLI flag or mode | `code-cli` (`args.rs`, `runtime.rs`) |
| The system prompt | `code-prompts` (`system_prompt.rs`) |
| A built-in tool | `code-tool-bash`, `code-tools-fs`, `code-tool-search`, `code-tools-optin`; registration in `code-tools` |
| How a tool looks in the transcript | `code-tui-widgets/src/tools/` ([ui.md](ui.md)) |
| A slash command | list: `code-resources/src/slash_commands.rs`; handler: `code-tui-app/src/interactive_mode.rs` `run_builtin_command` |
| Turn logic, parallel tool calls | `agent-loop` |
| Session file format | `agent-session` (entries, storage), `code-session` (manager) |
| Compaction | `agent-compaction` |
| Subagents | `code-subagents` (see `docs/design/subagents.md`) |
| A model provider | `ai-provider-<name>`, registered in `ai-registry` |
| A login flow | `ai-oauth-<name>`, stored by `code-auth` |
| The model list | `ai-models-catalog` (generated), `code-models` (models.json) |
| Settings keys | `code-settings` |
| Config paths, env prefixes | `code-paths` |
| Permissions and modes | `code-permissions`, `code-modes` |
| Themes | `code-tui-theme` |
| Key bindings | `code-tui-keybindings` (app), `tui-keys` (parsing) |

## All crates

"Used by" counts workspace crates that depend on it (not dev-deps). Lines are
`src/` / `tests/`.

### AI: models, providers, logins (`ai-*`)

| Crate | Does | Used by | src / tests lines | Status |
|---|---|---|---|---|
| `ai-env` | Environment and API key handling for cortex AI | 7 | 347 / 0 | keep |
| `ai-models` | LLM model registry and discovery for cortex AI | 10 | 348 / 234 | keep |
| `ai-models-catalog` | Model catalog data (LLM and image models) for cortex AI, generated from the pinned hoocode | 1 | 43 / 0 | keep |
| `ai-oauth` | OAuth core for cortex AI: types, PKCE, callback server, provider registry | 7 | 1084 / 0 | keep |
| `ai-oauth-anthropic` | Anthropic (Claude Pro/Max) OAuth flow for cortex AI | 1 | 517 / 0 | keep |
| `ai-oauth-github-copilot` | GitHub Copilot OAuth device flow for cortex AI | 1 | 675 / 0 | keep |
| `ai-oauth-google` | Google Cloud Code Assist OAuth flows (Gemini CLI, Antigravity) for cortex AI | 1 | 1456 / 0 | keep |
| `ai-oauth-openai-codex` | OpenAI Codex (ChatGPT Plus/Pro) OAuth flow for cortex AI | 1 | 602 / 0 | keep |
| `ai-provider-anthropic` | Anthropic provider for cortex AI | 1 | 2678 / 516 | keep |
| `ai-provider-azure` | Azure OpenAI provider for cortex AI | 1 | 648 / 0 | keep: `ai-registry` registers its `stream` for `azure-openai-responses` |
| `ai-provider-faux` | Faux / test provider for cortex AI | 0 | 930 / 759 | test provider (dev-dep only) |
| `ai-provider-google` | Google Gemini provider for cortex AI | 2 | 3157 / 111 | keep |
| `ai-provider-google-gemini-cli` | Google Cloud Code Assist (Gemini CLI / Antigravity) provider for cortex AI | 1 | 1724 / 0 | keep |
| `ai-provider-openai` | OpenAI provider for cortex AI | 1 | 3415 / 826 | keep |
| `ai-provider-openai-codex` | OpenAI Codex (ChatGPT subscription) Responses provider for cortex AI: SSE and WebSocket transports | 1 | 2555 / 0 | keep |
| `ai-provider-openai-responses` | OpenAI Responses API provider for cortex AI, and the Responses plumbing shared with Azure | 3 | 2260 / 0 | keep |
| `ai-registry` | API provider registry for cortex AI: dispatches streams on model.api | 4 | 314 / 3068 | keep |
| `ai-sse` | Server-Sent Events decoder shared by the cortex AI providers | 2 | 218 / 0 | keep |
| `ai-stream` | Streaming response utilities for cortex AI | 13 | 611 / 0 | keep |
| `ai-types` | Shared types for cortex AI | 40 | 1090 / 0 | keep |
| `ai-util` | Shared utilities for cortex AI: JSON repair, hash, headers, sanitization, overflow detection | 12 | 3866 / 163 | keep |

### Agent runtime (`agent-*`)

| Crate | Does | Used by | src / tests lines | Status |
|---|---|---|---|---|
| `agent-compaction` | Session compaction for cortex agents | 3 | 1451 / 1227 | keep |
| `agent-core` | Core agent runtime for cortex agents | 2 | 1972 / 0 | keep |
| `agent-harness` | Agent harness for cortex agents | 6 | 3325 / 1040 | keep |
| `agent-loop` | Agent loop for cortex agents | 1 | 2379 / 0 | keep |
| `agent-session` | Session trees for cortex agents: entry format, storage, repositories | 2 | 2355 / 0 | keep |
| `agent-types` | Shared types for cortex agents | 20 | 892 / 0 | keep |

### Coding agent (`code-*`, `app-server*`)

| Crate | Does | Used by | src / tests lines | Status |
|---|---|---|---|---|
| `app-server` | hoocode app-server: Codex app-server protocol over stdio and a Unix socket | 1 | 2522 / 1012 | keep |
| `app-server-protocol` | Wire types for hoocode's app-server (Codex app-server protocol compatible) | 1 | 1187 / 0 | keep |
| `code-agent-session` | AgentSession: the agent lifecycle shared by the cortex run modes | 7 | 5868 / 5436 | keep |
| `code-auth` | Credential storage for the cortex coding agent: auth.json API keys and OAuth tokens with locked refresh | 4 | 890 / 706 | keep |
| `code-cli` | CLI argument parsing and mode dispatch for the cortex coding agent (port of hoocode cli/args.ts + main.ts) | 1 | 4985 / 0 | keep |
| `code-main` | Main entry point for the cortex coding agent | 0 | 6 / 1102 | the `cortex` binary |
| `code-media` | Image handling for the cortex coding agent: format sniffing, resize/re-encode for model input | 3 | 1586 / 610 | keep |
| `code-models` | Model registry for the cortex coding agent: built-in catalog plus models.json custom providers and overrides | 4 | 1996 / 670 | keep |
| `code-modes` | Modes for the cortex coding agent: ask/plan/build/debug prompts, hoo-config.json, /mode /plan /grill /goal /approve | 3 | 1057 / 1050 | keep |
| `code-paths` | App identity, config directories and path helpers for the cortex coding agent | 16 | 1029 / 415 | keep |
| `code-permissions` | Permission gate for the cortex coding agent: per-mode tool policy from hoo-config.json and approval prompts | 3 | 290 / 210 | keep |
| `code-print` | Output formatting for the cortex coding agent | 1 | 298 / 280 | keep |
| `code-prompts` | Prompt templates for the cortex coding agent | 2 | 780 / 0 | keep |
| `code-resources` | Resources for the cortex coding agent: skills, prompt templates, slash commands, agent definitions, context files | 5 | 4454 / 2995 | keep |
| `code-rpc` | RPC mode for the cortex coding agent | 2 | 1547 / 763 | keep |
| `code-session` | Session handling for the cortex coding agent | 6 | 1797 / 262 | keep |
| `code-settings` | Global and project settings.json for the cortex coding agent | 8 | 2043 / 1043 | keep |
| `code-subagents` | Subagent orchestration for the cortex coding agent | 3 | 7829 / 6683 | keep |
| `code-task-store` | In-process task store for the cortex coding agent (TodoWrite plan items, subagent runs) | 5 | 588 / 0 | keep |
| `code-tool-api` | Shared tool plumbing for the cortex coding agent: tool definitions, output truncation, path resolution | 10 | 1058 / 120 | keep |
| `code-tool-bash` | The bash tool for the cortex coding agent: shell resolution, process-tree kill, streamed and truncated output | 5 | 1389 / 508 | keep |
| `code-tool-search` | SearchCodebase for the cortex coding agent: ranked lexical code search (ripgrep libraries), fusion and reranking | 1 | 1962 / 1185 | keep |
| `code-tools` | Coding tools for the cortex coding agent | 4 | 1279 / 0 | keep |
| `code-tools-fs` | File tools for the cortex coding agent: read (with read-dedup) | 4 | 3286 / 2384 | keep |
| `code-tools-optin` | Opt-in tools for the cortex coding agent: TodoWrite and ask_options | 3 | 545 / 540 | keep |

### Coding agent UI (`code-tui-*`)

| Crate | Does | Used by | src / tests lines | Status |
|---|---|---|---|---|
| `code-tui-app` | The coding agent's interactive mode on the cortex TUI | 1 | 11375 / 4571 | keep |
| `code-tui-keybindings` | The coding agent's keyboard map: app keybindings, keybindings.json loading and hint text | 3 | 744 / 836 | keep |
| `code-tui-selectors` | The coding agent's pickers and dialogs on the cortex TUI | 1 | 8000 / 3305 | keep |
| `code-tui-theme` | Color themes for the cortex coding agent's interactive mode | 4 | 2406 / 2502 | keep |
| `code-tui-widgets` | The coding agent's chat transcript widgets on the cortex TUI | 2 | 6706 / 4228 | keep |

### TUI library (`tui-*`)

| Crate | Does | Used by | src / tests lines | Status |
|---|---|---|---|---|
| `tui-components` | UI components for the cortex TUI | 4 | 9805 / 7265 | keep |
| `tui-editing` | Text editing primitives for the cortex TUI | 1 | 307 / 0 | keep |
| `tui-fuzzy` | Fuzzy matching for the cortex TUI | 4 | 374 / 0 | keep |
| `tui-highlight` | Syntax highlighting for the cortex TUI: a port of highlight.js 10.7.3 over its own grammars | 1 | 1963 / 72 | keep |
| `tui-images` | Terminal image rendering for the cortex TUI | 4 | 1237 / 376 | keep |
| `tui-keys` | Keyboard handling for the cortex TUI | 6 | 1968 / 570 | keep |
| `tui-render` | Differential rendering for the cortex TUI | 4 | 2766 / 2615 | keep |
| `tui-terminal` | Terminal abstraction for the cortex TUI | 2 | 1592 / 100 | keep |
| `tui-util` | Shared utilities for the cortex TUI | 8 | 2062 / 561 | keep |

## Deleted 2026-10-08

Ten crates were removed, as agreed in
[decisions-2026-10-08.md](../design/decisions-2026-10-08.md).

| Crate | Why | What moved or is left |
|---|---|---|
| 5 umbrellas: `cortexcode`, `agent`, `ai`, `code`, `tui` | Only for crates.io, which is dropped | `ai`'s tests moved to `ai-registry` (below); `ai`'s `oauth_providers` test was dropped, as it only tested the umbrella's own install wrapper |
| `agent-mcp` | Not wired into the binary; [mcp.md](../design/mcp.md) rebuilds MCP on rmcp | Nothing; its stub server idea returns as rmcp test servers |
| `agent-orchestrator`, `agent-tools` | hoocode SDK ports the product never calls; `code-agent-session` does the job | Nothing |
| `ai-images` | No tool or command generates images | Not done: `ai-models-catalog`'s `IMAGE_MODELS_JSON` and `data/image-models.json` still exist, and `scripts/convert_models_to_json.py` still writes them. A follow-up. |
| `code-extensions` | Unused WASM host; costs ~55 s per clean build | `build-speed.md` D5 no longer applies |

**Moved to `ai-registry`** (`crates/cortexcode-ai-registry/tests/it/`): `cache_retention`,
`cross_provider_handoff`, `github_copilot`, `live_matrix`, `routing`, `stream_hooks`, with
their `tests/data/red-circle.png`. Imports now name the leaf crates directly.

**Kept: `ai-provider-azure`.** The decision said "Not used", but `ai-registry` depends on
it and registers it for `azure-openai-responses`, so it stays. The Azure cleanup listed in
the decision is not done.

## Upkeep

Until the generator lands, refresh the tables with:

```bash
cargo metadata --format-version 1 > /tmp/meta.json   # then rebuild "Used by" and line counts
```

The generator (first coding session): `scripts/maps/packages.py` writes the "All
crates" section from `cargo metadata` plus each crate's `description`, and
`--check` fails CI when the page is stale. Crate descriptions in `Cargo.toml` are
the source of the "Does" column, so keep them accurate.

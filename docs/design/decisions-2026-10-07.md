# Decisions, 2026-10-07

> Parts of this page were replaced on 2026-10-08 (MCP Apps, Copilot canvases, plugin
> tools, marketplace writing, the WASM crate, the order of work). See
> [decisions-2026-10-08.md](decisions-2026-10-08.md).

Made with the user in a review of the post-migration design docs. Where a design card
disagrees with this page, this page wins. The cards are indexed in
[README.md](README.md).

## Order of work

1. **Reliability first:**
   - make `--mode rpc` deny tools that need approval when nobody can answer
     (today they run without asking);
   - fix the `~/.hoocode` collision with hoocode-ts ([naming-and-paths.md](naming-and-paths.md)
     steps 1–2, with a backup);
   - fix the 4 tests that fail on macOS on a clean checkout;
   - audit panics (multibyte slicing, `unwrap` on input) and fuzz the parsers.
2. **Close the 8 `l1_done` tasks:** add `SearchHooCode` (live capabilities only, no
   docs), re-run their side-by-side checks, mark them done.
3. **MCP client.**
4. **Plugins.**
5. **MCP Apps** and the **scheduler**.

## Decisions

| Area | Decision |
|---|---|
| Ledger | New status `moved`. Each deferred task points at its design doc. |
| Docs | Rewrite each design doc as a one-page decision card: goal, decisions, what we build, not doing, open questions. |
| MCP library | **rmcp** (official SDK; Apache-2.0), chosen for the future. Drop legacy HTTP+SSE and the old hand-written client. CI runs the MCP conformance suite so SDK lag shows up early. |
| MCP permission | **Folder trust instead of per-tool prompts.** Servers from a repository folder or a plugin need a one-time trust grant. User config (`~/.agents/mcp.json`) is trusted. Trust binds to the approved server list (commands and URLs), so a change after a `git pull` asks again and shows the diff. |
| MCP OAuth | Client ID Metadata Document hosted at `kolisachint.github.io` (`/hoocode/oauth-client.json`). Dynamic registration as the fallback until it is published. |
| Web tools | **Deferred.** `bash` plus `curl` today; the webtools MCP server once MCP lands. Revisit provider-native web search later. |
| Code extensions | **Standards only:** MCP servers, skills and subagents. No code-extension runtime. Make the WASM crate optional (off by default). |
| Plugin formats | Read **Agent Plugins 1.0** (the standard) and **Claude** plugins. Drop `.agents-plugin` and the Copilot-only layouts. |
| Plugin contents | Skills, MCP servers, **subagents**. Hooks, slash commands, themes and providers are not supported. A plugin that ships them still loads, and `/plugin list` warns. |
| Install | **Plugins only.** Drop the `packages` setting and npm sources. |
| Marketplace | Claude's `.claude-plugin/marketplace.json`, read and written. Anthropic's official directory ships **pre-trusted** (TS behaviour). |
| Model plugin tools | **Full lifecycle:** search, install, author, update, remove, package, publish. Behind an **opt-in setting** (off by default, so no token cost). Installs from a trusted marketplace don't ask. Plugins the model writes itself ask before anything runnable starts. |
| Authoring format | Agent Plugins, with hoocode-only extras under our own namespace directory. |
| Rich surfaces | **MCP Apps first.** Shown as "app ready · o to open" (browser), with `mcpApps.autoOpen` to change. rpc and app-server clients get the app info. Copilot canvases later; `drawio-canvas` works through its MCP mode meanwhile. |
| Scheduler | A prompt that comes due while busy runs at the next idle moment, up to 10 minutes late; after that it's skipped with a notice. No headless runs in the app-server daemon in v1. |
| Extras | **Yes:** version check, completion chime. **No for now:** thinking escalation, `/learn`, teams, voice, HTML export, `/share`, telemetry. 12.6 (warm pool) is already built (10.9c). |

## Accepted risk, stated once

Pre-trusting the official directory and letting trusted-marketplace installs skip the
prompt means this: **with plugin tools enabled, the model can install and run any of
its 250+ plugins, including MCP servers, without asking.** A prompt injection in
fetched content could trigger that. The opt-in setting is the only gate. hoocode-ts
behaves the same way.

## Still to decide (later, not blocking)

- ~~Semantic search via the `embsearch` daemon: who downloads it, and when (wave F).~~ Dropped 2026-10-08.
- ~~Copilot canvas host: when, after MCP Apps.~~ Dropped 2026-10-08.
- Whether thinking escalation comes back (hoocode-ts has it on by default).

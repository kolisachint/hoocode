# Plugins

Status: **agreed 2026-10-07**, scope cut 2026-10-08 ([decisions-2026-10-08.md](decisions-2026-10-08.md)).
Design only. Replaces ledger 12.1 and 12.2.

## Goal

Load plugins written to the open standard (Agent Plugins) and Claude Code's format.
Install them from marketplaces. Optionally let the model find, install and write
plugins itself. Write only the standard; keep only what standards cover.

## Decisions

| Decision | Why |
|---|---|
| Read **Agent Plugins 1.0** and **Claude** plugins only | Agent Plugins is the cross-vendor standard (Copilot, Cursor, Codex, VS Code, Kiro). Claude Code doesn't support it yet, so its format stays. hoocode's own `.agents-plugin/` and the Copilot-only layouts are dropped. |
| Supported contents: **skills, MCP servers, subagents** | Skills and MCP are the standard. Subagents are used by both Claude and Copilot. |
| Hooks, slash commands, themes, providers: **not supported** | A plugin that ships them still loads; `/plugin list` warns what was skipped. |
| **Plugins are the only install system** | The `packages` setting and npm sources are dropped |
| Marketplace index: Claude's `.claude-plugin/marketplace.json`, **read only** | The most used; the official Anthropic directory uses it. We don't publish, so we never write it. |
| Anthropic's official directory is **pre-trusted** | hoocode-ts behaviour |
| Model tools: **search, install, list, remove, write** (2026-10-08). No update, package or publish tools. | Updating is `/plugin update`; packaging and publishing target Claude's marketplace format, which we don't write |
| Those tools are **opt-in** (`enablePluginTools`, off by default) | No token cost unless wanted |
| Installs from a trusted marketplace **don't ask**. Plugins the model writes itself ask before anything runnable starts (MCP server, or a subagent with write or shell tools). | User choice. **Accepted risk:** with the tools on, the model can install and run any plugin from a trusted marketplace without asking, and a prompt injection could trigger that. |
| Plugins we write are **Agent Plugins packages only**, never Claude or Copilot layouts; hoocode extras go in our own namespace directory | Other clients can load what we produce, and there is one writer to keep correct |
| Folder trust as in [mcp.md](mcp.md): repository-supplied plugins need it, and it re-asks on change | Same rule everywhere |

## What we build

1. **Load.** Crate `code-plugins`.
   - **Agent Plugins:** root `plugin.json` with an `agent-plugins.org` `$schema`;
     `skills/`; `mcp.json`. Follow the spec exactly (Details).
   - **Claude:** `.claude-plugin/plugin.json`; `skills/`, `agents/`, `.mcp.json` or
     inline `mcpServers`.
   - **Where:** `<cwd>/.agents/plugins/` (project) and `~/.agents/plugins/` (user),
     plus `~/.claude/skills/` so Claude-installed skills load. First found wins by
     plugin name.
   - Skills are named `<plugin>:<skill>`, so two plugins can't collide.
   - Skill frontmatter is checked against the Agent Skills rules; problems are
     warnings.
   - `allowed-tools` in a skill takes effect only after the user approves that
     skill once.
2. **Install.**
   - `/plugin marketplace add|list|refresh`;
   - `/plugin install|remove [--scope user|project]`;
   - `/plugin list`; `/plugin update [name]`; `/plugin trust`.
   - Fetch with the `git` command (uses your git credentials), with a cache and
     refresh interval.
   - A new plugin's skills and subagents are live from the next turn; MCP servers
     start once the turn ends.
3. **Model tools** (opt-in): `SearchPlugins`, `InstallPlugin`, `UninstallPlugin`,
   `ListPlugins`, `ProposePlugin` (writes an Agent Plugins package).
   - Adding a marketplace stays human-only.
   - Subagents never get these tools, so a subagent can't bootstrap more power.

Done when: format conformance tests pass (Agent Plugins normative cases, Claude
fixtures), install and remove work against local test marketplaces, the lifecycle
tools pass their gate tests, and a plugin installed by hoocode-ts loads in `cortex`.

## Not doing

- Hooks, slash commands, themes and providers in plugins.
- `.agents-plugin/`, Copilot-only layouts and Copilot marketplace files.
- Writing Claude or Copilot plugin layouts, or writing `marketplace.json`.
- `UpdatePlugin`, `PackagePlugin`, `PublishPlugin` model tools.
- Skills over MCP (the MCP Skills extension).
- npm sources, and the `packages` setting.
- Validating against other vendors' tools (`claude plugin validate`, smoke runs)
  for now.

## Open questions

- None blocking. Revisit hooks if Agent Plugins adds them to the standard.

## Details: Agent Plugins 1.0 rules we follow

- **`plugin.json`:** allowed fields are `$schema`, `name`, `version`, `description`,
  `author`, `homepage`, `repository`, `license`, `keywords`, `extensions`.
  - Unknown fields: warning.
  - Any other violation rejects the plugin. `name` is 1–64 characters of
    `[a-z0-9.-]`, alphanumeric at both ends, with no `--` or `..`.
  - Schemas are bundled in the binary, never fetched.
- **`skills/`:** each direct subdirectory with a `SKILL.md` file; not recursive.
- **`mcp.json`:** its `$schema` version must match `plugin.json`'s, or MCP is off
  for that plugin.
  - `stdio`: `command` is one token, with no variable expansion in it.
    `args`/`env`/`cwd` expand only `${PLUGIN_ROOT}` and `${PLUGIN_DATA}`. `cwd`
    defaults to the plugin root. `env` must not set those two names.
  - `streamable-http`: an HTTPS URL unless loopback; fixed headers.
  - A bad server is skipped and reported; the others still load.
- **Processes** get `PLUGIN_ROOT` and `PLUGIN_DATA` (`~/.agents/plugin-data/<id>/`,
  kept across updates).
- **Paths:** every declared path must resolve inside the plugin root.
- **Namespaces:** unknown `extensions` namespaces are ignored.

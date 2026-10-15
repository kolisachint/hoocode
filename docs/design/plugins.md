# Plugins

Status: **agreed 2026-10-07**, scope cut 2026-10-08 ([decisions-2026-10-08.md](../../archive/docs/decisions-2026-10-08.md)).
Replaces ledger 12.1 and 12.2.

**Built so far (2026-10-15): parts 1, 2 and 3, Load, Install and Model tools.** Crate `hoocode-code-plugins`: Agent
Plugins and Claude layouts, discovery and precedence, project folder trust, `<plugin>:<skill>`
skills, subagent files, and MCP servers in the MCP config. The loader also runs at session start
(`hoocode-code-cli` `runtime.rs`). Part 2 adds `install.rs` and `marketplace.rs` and the `/plugin`
command (`hoocode-code-tui-app` `mode/plugin.rs`). Part 3 is below.

Part 3 choices (2026-10-15):

- **Where.** The five tools are in `hoocode-code-tools-optin` (`plugins.rs`), registered in the
  CLI's `custom_tools` only when `enablePluginTools` is on (or `--enable-plugintools`). They
  arrive at the next session, as the settings pane says. Their logic is `hoocode-code-plugins`
  `model.rs` (search, trusted-only install, reload) and `propose.rs` (the writer).
- **Subagents.** A child process has a task id, so it does not register them. Every child is also
  started with the five names in `--disallowed-tools` (`hoocode-code-subagents` `pool.rs`,
  `subagent_disallowed_tools`, used by both spawn paths), whatever its definition lists.
- **Permissions** (`hoocode-code-tools` `permissions.rs`). SearchPlugins and ListPlugins are
  read-only. InstallPlugin is granted without a prompt, and refuses any marketplace the user has
  not trusted. UninstallPlugin and ProposePlugin are dangerous, so the Ask policy prompts for them.
  An unknown tool is denied by the policy, so every new name is classed there.
- **Installs** search every listed marketplace (a stale one is fetched, as `/plugin` does). Only
  the official directory is trusted, so the model installs only from it. A plugin that is in
  another marketplace is refused with the `/plugin install name@marketplace` the user can run.
  The same name in two marketplaces needs `name@marketplace`.
- **ProposePlugin** writes one Agent Plugins package: `plugin.json` with the 1.0 `$schema` and
  `extensions.hoocode.origin`; `skills/<name>/SKILL.md`; `mcp.json` only when there are servers.
  Never a Claude or Copilot layout. The package is checked with the loader's own parsers in a
  staging folder, and only then moves into place. Its record names `ProposePlugin` as the
  marketplace and is untrusted.
- **Ask before anything runnable.** Skills are passive and load at once. An MCP server is held by
  the untrusted record until the user grants it in `/mcp` (the existing grant path, which the
  startup trust prompt also uses). The policy's prompt for ProposePlugin is a second gate. A
  project proposal waits for `/plugin trust`, as every project plugin does. Agent Plugins 1.0 has
  no subagents, and the loader reads no `agents/` folder in that layout, so a proposal cannot add
  a subagent; the "subagent with write or shell tools" rule has nothing to apply to.
- **Review 2026-10-15 (step 6 part 3).** Checked against the `--trust` change (6fe94ac):
  InstallPlugin installs without asking from any marketplace whose record is trusted (the official
  one, or one added with `--trust`), and refuses the rest; a test now covers a `--trust` install.
  ProposePlugin rejects traversal in the plugin and skill names, and refuses a folder already
  there with no record. Subagents: the deny list is in both spawn paths, and a warm worker (no
  task id) also skips registration because it runs under a subagent depth. Fixed: the permission
  gate's plugin list named `SuggestPluginInstall` (no such tool) and left out `UninstallPlugin`,
  so fail-closed rpc mode did not gate uninstall; `/plugin update` now reports a proposed plugin
  as `Skipped` with the reason, not as no longer listed; the `--platform` help text describes the
  Agent Plugins layout and says the flag is not implemented yet.
- **Open, not done.** A tool result does not reload the TUI, so skills and MCP servers of a change
  go live on `/reload` (or the next session). The `plugin-authoring` skill still names
  `UpdatePlugin` and hooks: its tests pin that text (the content hash and the "second hook"
  guidance), so it is left as it is until someone decides to change it. The TUI widget list for
  plugin tools (`hoocode-code-tui-widgets` `tools/plugins.rs`) still names `SuggestPluginInstall`
  and does not list `UninstallPlugin`; it is display-only and was not changed in this review.

Part 2 choices (2026-10-14):

- **Marketplaces** are git repositories, fetched with the `git` command (`GIT_TERMINAL_PROMPT=0`, so
  a credential prompt cannot hang the TUI; the user's credential helper still answers). The cache is
  `plugin-marketplace-cache/<name>` in the hoocode data directory, refreshed when older than a day.
  A failed refresh uses the old cache with a note; `/plugin marketplace refresh` fails instead.
  `owner/repo` means GitHub. The official directory's URL is **assumed** to be
  `https://github.com/anthropics/claude-plugins-official`, not stated in the design docs.
- **Trust.** Only the official directory is pre-trusted. `/plugin marketplace add <src> --trust`
  marks the marketplace the user adds as trusted; without `--trust` it is untrusted. The listing
  shows `trusted` or `not trusted` for each marketplace. An install from an untrusted marketplace
  still succeeds, because the user typed the command, and its success notice says the plugin's MCP
  servers need a grant in `/mcp` (its record says `trusted: false`, and the loader sets
  `marketplace_trusted` from it). Skills and subagents load without a grant, as before.
- **Git input.** Marketplace values (URLs, refs, subfolders, names) never reach git as options:
  a value that starts with `-` is refused, `name::` transport helpers (`ext::`, `fd::`) are
  refused, and clones use `--`. A plugin folder never gets a file from outside its marketplace:
  a symbolic link in the checkout that resolves outside it, or does not resolve, refuses the
  install, and a `git-subdir` path that leaves the repository is refused. `/plugin remove` and
  `update` act only on the plugin's own folder; a record that names another folder is left alone.
  Records and the registry are written through a temporary file with a per-write name, then renamed.
- **Records.** Each install's source, marketplace and trust go in `plugin-installs.json` in the
  hoocode data directory, not in the plugin folder: a folder change would re-ask for project trust.
  A plugin with no record is hand-placed and keeps the old trust. If the record file cannot be read,
  no plugin's marketplace is trusted (fail closed), and `/plugin list` says why.
- **Installs** check the manifest (it must parse and name the plugin the marketplace lists) before
  they move into the plugin folder; staging is outside the plugin folders so a half copy never loads.
  `npm` and other sources are refused. Project installs need `/plugin trust` after the folder changes.
- **Update** compares the folder fingerprint, so an unchanged plugin is left alone. A plugin that the
  marketplace dropped fails its own update and is kept.
- **Remove** keeps `PLUGIN_DATA` and says where it is.
- **Live.** A change reloads the plugin set (`hoocode_code_plugins::set_current`), the MCP servers of
  the new set start (only when the session runs MCP at all), and then the `/reload` path runs. During
  a turn the change waits for the turn to end. Review 2026-10-14 of part 1: user
plugins now win names over project plugins; Claude `.mcp.json` stdio paths are held to the plugin
root; retargeting a symlink in a project plugin asks again; capabilities lists loaded plugins.
Choices made while building part 1:

- **Precedence:** a user plugin wins a name over a project plugin, so a repository cannot shadow
  the user's plugins ([mcp.md](mcp.md)). The repository's copy is skipped with a warning. An
  untrusted project plugin is not loaded at all, so it cannot take a name either. Within one folder
  the first name in sorted order wins. Plugin MCP config is ordered after `mcp.json` (user, project).
- **Folder trust** for project plugins is one grant per folder, kept in the MCP trust file. Its
  fingerprint covers every file in the project plugin folders (SHA-256 of each file's path and
  bytes; a symbolic link counts by its path and target text, and a linked folder is not followed).
  Any change asks again. Only the grant API exists (`trust_project_plugins`); the prompt is not
  wired into the UI yet, so an untrusted folder shows a warning line at start. Cost: with no plugin
  folders, a session pays only for a few directory checks; with 200 files (about 800 KB) the
  fingerprint costs about 25 ms in the dev profile, most of it SHA-256 at opt-level 0. A release
  build is much faster (not measured). If dev startup matters, optimize `sha2` in the profile, as
  `regex` is.
- **Claude `.mcp.json` paths** follow the Agent Plugins containment: a stdio `command` is a bare
  name or a `./` path inside the plugin (run from there), and `cwd` is a `./` path inside it.
  Anything else skips that server with an error. `${CLAUDE_PLUGIN_ROOT}` is refused for the same
  reason as `${PLUGIN_ROOT}` in `command`: it is not expanded here.
- **Plugin MCP servers** are trusted once their plugin loads (user plugins are the user's own;
  project plugins passed folder trust), so they need no second grant. Their values are not
  expanded: only `${PLUGIN_ROOT}` and `${PLUGIN_DATA}` are, and only for Agent Plugins.
- **`PLUGIN_DATA`** is `~/.agents/plugin-data/<name>`, created when the plugin has an Agent Plugins
  `mcp.json`.
- **Claude layout:** `.mcp.json`, or the manifest's inline `mcpServers`. Claude's own variables
  (for example `${CLAUDE_PLUGIN_ROOT}`) are not expanded, since this repo has no spec for them.
  The Claude manifest's name follows the Agent Plugins name rules.
- **Skill `allowed-tools`:** a plugin skill's list is kept only when the user approved that exact
  list (stored in the MCP trust file, keyed by `<plugin>:<skill>`). Nothing in hoocode reads
  `allowed-tools` at run time yet, so the gate has no effect on tools until something does.
- **Schemas** are not bundled as JSON. The Agent Plugins rules are hand-coded from the 1.0.0
  specification, and the only `$schema` values accepted are the 1.0.0 identifiers.
- **Capabilities:** `hoocode-code-capabilities` lists each loaded plugin as a `kind: plugin` entry
  (its manifest description, its folder as the source). Untrusted project plugins are not listed.
- **`/plugin list`** shows the marketplaces, the loaded plugins (scope, where each came from, whether
  its MCP servers need trust), records that did not load, the project plugins waiting for trust, and
  every skipped or rejected part with its reason.

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
tools pass their gate tests, and a plugin installed by hoocode-ts loads in `hoocode`.

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

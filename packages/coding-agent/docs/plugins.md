# Plugins

A plugin is a directory that bundles capabilities — skills, slash commands,
subagents, hooks, MCP servers, themes, and canvas extensions — behind one
installable name. Plugins come from marketplaces, which are git repositories or
local directories that index them.

Plugins overlap with [hoocode packages](packages.md); the difference is where
they come from and who installs them. A package is an npm or git dependency you
list in settings. A plugin is installed by name from a marketplace, can be
installed by the model mid-task, and is portable across agent tools —
hoocode reads the Claude Code and GitHub Copilot plugin formats as well as its
own.

## Using plugins

```
/plugin marketplace add <git-url|path>   # register a marketplace (a human act)
/plugin marketplace list                 # what is registered
/plugin marketplace refresh              # re-fetch marketplace indices
/plugin list                             # what is installed
/plugin install <name> [--scope user|project]
/plugin remove <name>
```

`--scope user` (the default) installs under your agent dir, for every project.
`--scope project` installs into the current repository, where it travels with
the clone.

## Trust

Two different decisions, deliberately kept apart:

**Adding a marketplace is a human act.** It says you accept code from that
source. Nothing installs a marketplace for you.

**Installing from a marketplace you already added is the model's discretion** —
the package-manager model. hoocode announces what it installed and the install
is reversible with `/plugin remove`.

Separately, a plugin committed to a repository is code that runs for whoever
clones it next. Its skills and commands are only text the model reads, no worse
than reading the repository itself, but **its hooks and MCP servers are
processes that start when the session loads**. So hoocode keeps a per-machine
record of which working directories you have agreed to run repository-supplied
plugin code from:

```
/plugin trust [list]   # inspect, or grant trust for this workspace
/plugin untrust        # revoke it
```

The record lives in your agent dir, keyed by absolute path — outside the
repository, so repository content cannot forge it. Until a workspace is trusted,
a repo-supplied plugin loads with its hooks and MCP servers held back; hoocode
reports this as a warning at startup rather than failing the session.

Granting trust is always a human act. The autonomous install path never grants
it. As with VS Code's trusted folders, trust is a statement about a place you
work, not about a specific commit — code pulled into a trusted directory later
is trusted too.

## What a plugin can contain

A directory is recognized as a plugin if it has a `plugin.json` manifest or any
of these:

| Path | Provides |
|------|----------|
| `skills/` | Agent Skills ([Skills](skills.md)) |
| `commands/` | Slash commands |
| `agents/` | Subagent definitions ([Subagent delegation](routing.md)) |
| `hooks/`, `hooks/hooks.json` | Lifecycle hooks |
| `.mcp.json` | MCP servers ([MCP](mcp.md)) |
| `SKILL.md` | A single-skill plugin |

Read from the manifest but not sufficient on their own to mark a directory as a
plugin: `themes/` and `extensions/` (canvas extensions — see [Canvas](canvas.md)).

## Formats

hoocode reads three on-disk layouts, so a plugin written for another agent tool
generally works unchanged:

| Format | Marker | Notes |
|--------|--------|-------|
| hoocode (native) | `plugin.json` | The only format with `providers` |
| Claude Code | `.claude-plugin/` | Same component layout, no `providers` |
| GitHub Copilot | `.github/plugin/` | Also read from the plugin root, `.plugin/`, and `.claude-plugin/`; written to `.github/plugin/` |

## Authoring

Scaffold the pieces:

```
/new-skill <name>
/new-agent <name>
/new-command <name>
/new-canvas <name>
```

Then package for distribution:

```
/plugin publish <name> [--to <dir>]
```

## Model-facing tools

When plugin tooling is enabled (`--enable-plugintools`, or the
`enablePluginTools` setting), the model gets its own lifecycle tools rather than
going through the slash command: `SearchPlugins`, `ListPlugins`,
`SuggestPluginInstall`, `InstallPlugin`, `UninstallPlugin`, `UpdatePlugin`,
`ProposePlugin`, `RemovePluginCapability`, and `PackagePlugin`.

`SearchPlugins` matches on capability, not just name — a plugin described as
"compose and send mail" is findable by a search for "email" — so the model can
close a capability gap mid-task instead of hand-rolling a solution.

## Related

- [HooCode packages](packages.md) — npm/git-distributed bundles you list in settings
- [Skills](skills.md), [Extensions](extensions.md), [MCP](mcp.md), [Canvas](canvas.md)
- [Project-local resources](project-local-resources.md) — the `.agents/` conventions plugins build on

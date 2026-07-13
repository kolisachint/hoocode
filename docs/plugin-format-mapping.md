# Plugin Format Mapping — native `.agents` ↔ Claude ↔ GitHub

Status: **reference**. This document is the single source of truth for how hoocode
supports plugins authored for **Claude Code** (`.claude-plugin/`) and **GitHub /
Copilot** (`.github/plugin/`, with `.github/marketplace.json` as a legacy index
location) alongside hoocode's **native** format (`.agents-plugin/`), and where
each thing is **packaged, installed, stored, and loaded**. It exists so that both
humans and agents can reason about the layout without re-reading the loader.

Companion design doc: [`loop-and-plugin-system.md`](./loop-and-plugin-system.md)
(what shipped and what is deferred). This doc focuses on the **cross-vendor
mapping** and the **`.agents/`-first policy**.

---

## 0. The one rule: `.agents/` and `.agents-plugin/` come first

hoocode reads resources from several vendors' conventions (`.agents/`, `.claude/`,
`.github/`, `.hoocode/`). The policy is:

> **`.agents/` (authored, cross-vendor resources) and `.agents-plugin/` (native
> plugin/marketplace manifests) are the primary surface. hoocode reads them first
> and writes/creates into them by default. The other conventions
> (`.claude/`, `.claude-plugin/`, `.github/`, `.hoocode/`) remain supported for
> compatibility and as fallbacks.**

Two homes, two jobs:

| Home | Role | Examples |
|---|---|---|
| **`.agents/`** | Authored, shareable, **cross-vendor** resources plus user-meaningful state — read first, created by default | `skills/`, `commands/`, `agents/`, `mcp.json`, **`plugins/`**, **`marketplaces.json`**, **`scheduled_tasks.json`** (`/loop`) |
| **`.hoocode/`** | hoocode-**private runtime state** & config — not meant to be portable | `sessions/`, `dispatch/`, `settings.json`, `mcp-servers/` |

`.claude/`, `.claude-plugin/`, and `.github/` are **compatibility inputs**: hoocode
reads plugins/marketplaces written in those formats, but installs and new files
land in `.agents/`.

---

## 1. Two things called "plugin": the manifest vs. the marketplace index

It is easy to conflate these. They are separate layers.

| Layer | What it is | Formats hoocode accepts |
|---|---|---|
| **Plugin manifest** | Describes **one** capability bundle (a single plugin directory). | `.agents-plugin/plugin.json` (native), `.claude-plugin/plugin.json` (Claude), `.github/plugin/plugin.json` (Copilot; legacy `.github/copilot-plugin.json` still read) |
| **Marketplace index** | Lists **many** installable plugins in a repo/dir. | `.agents-plugin/marketplace.json` (native), `.claude-plugin/marketplace.json` (Claude), `.github/plugin/marketplace.json` (Copilot; legacy `.github/marketplace.json` still read) |

**Real-world Copilot convention** (established by
[github/copilot-plugins](https://github.com/github/copilot-plugins) and the
plugins it indexes, e.g. microsoft/work-iq): the manifest lives at
`.github/plugin/plugin.json` and the **capability tree mirrors the Claude
layout** (top-level `skills/`, `commands/`, `agents/`, `hooks/hooks.json`,
`.mcp.json`). Manifests may carry dir-path overrides (`"skills": "./skills/"`),
which hoocode honors. Marketplace entries may use the shorthand source
`{ "source": "github", "repo": "owner/name", "path": "subdir" }`, which hoocode
normalizes to the equivalent git / git-subdir source. Plugins in the wild often
ship *both* `.claude-plugin/` and `.github/plugin/` manifests over one shared
capability tree — hoocode's authored output follows the same pattern (one tree,
one marker manifest per platform).

---

## 2. Plugin manifest mapping (`.agents-plugin` ↔ `.claude-plugin`)

Both parse into one `NormalizedPlugin` (`plugins/manifest.ts`). The native format
is a **strict superset** of the Claude format: any Claude plugin (minus native
extras) is a valid native plugin. **When a directory carries both, native wins —
no merge.**

| Manifest field / dir | Claude (`.claude-plugin`) | Native (`.agents-plugin`) | hoocode target |
|---|---|---|---|
| `name` / `version` / `description` / `author` | ✅ | ✅ | plugin identity metadata |
| `skills/` | ✅ | ✅ | `resources_discover → skillPaths` |
| `commands/` | ✅ | ✅ | `resources_discover → slashCommandPaths` (the `.agents/commands` surface) |
| `agents/` | ✅ | ✅ | `resources_discover → agentPaths` (the `.agents/agents` subagent surface) |
| `themes/` | ✅ | ✅ | `resources_discover → themePaths` |
| `hooks` / `hooks/hooks.json` | ✅ | ✅ | shell-protocol hooks bridge |
| `mcpServers` / `.mcp.json` | ✅ | ✅ | extension-MCP registry → connected on `session_start` |
| `providers: [{ name, config }]` | ❌ (ignored) | ✅ **native-only** | `registerProvider` |
| `extensions: string[]` (typed TS) | ❌ | 🔜 native-only (deferred) | direct extension load |

### Environment-variable parity

Inside `commands`, `args`, `env`, and hook commands, hoocode substitutes **both**
root placeholders with the plugin's root directory, and exports **both** as env
vars to hook processes:

- `${CLAUDE_PLUGIN_ROOT}` — Claude Code's placeholder (accepted for compatibility)
- `${AGENTS_PLUGIN_ROOT}` — native placeholder (preferred in new plugins)

They are interchangeable. A Claude plugin using `${CLAUDE_PLUGIN_ROOT}` works
unchanged; new native plugins should prefer `${AGENTS_PLUGIN_ROOT}`.

### Hooks event mapping (Claude Code → hoocode)

| Claude Code event | hoocode event | Behavior |
|---|---|---|
| `PreToolUse` | `tool_call` | matcher vs tool name; exit `2` / `decision:block\|deny` → blocks the tool |
| `PostToolUse` | `tool_result` | matcher vs tool name; exit `2` / `decision:block` → appends reason, marks error |
| `UserPromptSubmit` | `before_agent_start` | stdout / `reason` appended to the system prompt as context |
| `SessionStart` | `session_start` | side effects |
| `Stop` | `agent_end` | side effects |

Deferred (not yet mapped): `Notification`, `SubagentStop`, `PreCompact`, and the
newer `hookSpecificOutput` / `continue` / `systemMessage` fields.

---

## 3. Marketplace index mapping (`.agents-plugin` ↔ `.claude-plugin` ↔ `.github`)

All three parse into one `NormalizedMarketplace` (`plugins/marketplace.ts`) with
the **same shape**:

```json
{ "name": "...", "owner": "...", "plugins": [ { "name": "...", "source": "...", "description": "..." } ] }
```

Precedence when more than one file is present in the same repo/dir:
**`.agents-plugin` (native) → `.claude-plugin` (Claude) → `.github` (Copilot)**.
The `format` field is the precedence **winner**; no plugin lists are merged.

| Marketplace file | `format` | `supportPlatform` token | Origin | Notes |
|---|---|---|---|---|
| `.agents-plugin/marketplace.json` | `"agents"` | `agents` | native | preferred; the `.agents` surface |
| `.claude-plugin/marketplace.json` | `"claude"` | `claude` | Claude Code | |
| `.github/plugin/marketplace.json` (or legacy `.github/marketplace.json`) | `"copilot"` | `github` | GitHub / Copilot plugin directory index | plugins may also carry a `.github/plugin/plugin.json` manifest |

### `supportPlatform` — conflict is recorded, not hidden

When a repo carries **more than one** index format (a "conflict"), precedence
still picks a single `format` to read, but the parse result also exposes every
platform present so nothing is silently dropped:

- `NormalizedMarketplace.supportPlatform: MarketplacePlatform[]` — the platform
  token of **every index file present**, plus any authored top-level hint, deduped
  and never empty. Example: a repo with both `.github/marketplace.json` and
  `.claude-plugin/marketplace.json` resolves to `format: "claude"` **and**
  `supportPlatform: ["claude", "github"]`. `/plugin marketplace list` shows the
  extra platforms when there is more than one.
- **Optional authored field** `supportPlatform` (string or array) is accepted at
  the **top level** of a manifest and **per plugin entry**
  (`plugins: [{ …, supportPlatform }]`). It is folded into the normalized list;
  aliases `copilot` / `gh` → `github`, `native` → `agents`. Unknown tokens are
  dropped. **Omitting it changes nothing** — the field is purely additive and
  informational today (no entry is filtered out by it).

Platform tokens are `agents` | `claude` | `github`. Note the friendly `github`
token maps to the internal `format: "copilot"` (the `.github/` file); the two
names refer to the same thing.

A plugin `source` inside any index is classified by `resolvePluginSource`:

| `source` value | `kind` | Resolution |
|---|---|---|
| `./path/inside/repo` or absolute | `local` | copied from the marketplace root |
| `https://…`, `git@…`, or `…​.git` | `git` | `git clone --depth 1` |
| `npm:<spec>` | `npm` | recognized, **not yet installed** (deferred) |

---

## 4. Lifecycle: packaged → installed → stored → loaded

### 4.1 Where plugins are packaged (authoring)

A plugin is just a directory with a manifest at its root:

```
my-plugin/
  .agents-plugin/plugin.json     # or .claude-plugin/plugin.json
  skills/  commands/  agents/  themes/
  hooks/hooks.json               # or inline "hooks"
  .mcp.json                      # or inline "mcpServers"
```

A marketplace is a repo/dir with an index at its root plus the plugin dirs it
points at (or git/npm sources):

```
my-marketplace/
  .agents-plugin/marketplace.json    # or .claude-plugin/… or .github/marketplace.json
  plugins/foo/.agents-plugin/plugin.json
  plugins/bar/.claude-plugin/plugin.json
```

### 4.2 How they are installed — `/plugin`

`setupMarketplace` (`extensions/core/marketplace.ts`) registers `/plugin`:

```
/plugin marketplace add <git-url|path>   clone/read + register a marketplace
/plugin marketplace list
/plugin list                             available plugins across marketplaces
/plugin install <name>                   fetch into .agents/plugins/<name>, then reload
/plugin remove <name>                    remove (.agents first, then .hoocode) + reload
```

`install` copies a `local` source or `git clone`s a `git` source.

### 4.3 Where things are stored (the `.agents/`-first map)

| Thing | Location (primary) | Fallback / legacy |
|---|---|---|
| Installed plugin | `.agents/plugins/<name>/` | reads `.hoocode/plugins/<name>/`; `remove` deletes both |
| Added-marketplace registry | `.agents/marketplaces.json` | reads `.hoocode/marketplaces.json` if `.agents` one is absent |
| Marketplace clone cache | `.agents/marketplace-cache/` | — |
| Scheduled `/loop` tasks | `.agents/scheduled_tasks.json` | reads `.hoocode/scheduled_tasks.json` once, then migrates forward on next persist |

### 4.4 How they are loaded

Discovery scans these `plugins/` dirs, **highest precedence first**
(`defaultPluginDirs`, first-wins by plugin id):

1. project `<cwd>/.agents/plugins/`
2. project `<cwd>/.hoocode/plugins/`
3. global `~/.agents/plugins/`
4. global `~/.hoocode/plugins/` (`<agentDir>/plugins/`)

Each discovered plugin is normalized (`parsePluginDir`) and turned into a
**synthetic `ExtensionFactory`** (`buildPluginFactory`) that registers its
capabilities through the ordinary `ExtensionAPI`. So **a plugin is just an
extension assembled from a manifest** — it rides the existing extension
loader/runner, no special-case runtime.

`loadPlugins()` runs from **both** extension-loading entry points so plugins load
regardless of how the app started:

- `ResourceLoader.reload()` — the interactive / print app path.
- `discoverAndLoadExtensions()` — the standalone / SDK path.

MCP servers declared by plugins are pushed into a process-global registry
(`extension-mcp-servers.ts`) during load, then connected by `setupMcpLoader` on
`session_start` (deduped by name). `loadPlugins` clears that registry before each
(re)load so reloads rebuild cleanly.

**Net effect:** once `/plugin install` finishes and the reload completes, the
plugin's skills, commands, agents, themes, providers, hooks, and MCP tools are all
live in the session — **loaded means ready to use**, no extra step.

---

## 5. Discrepancies & how hoocode resolves them (quick reference)

| # | Discrepancy | Resolution in hoocode |
|---|---|---|
| 1 | Claude has `.claude-plugin/`; hoocode wants a native home | Native `.agents-plugin/plugin.json` is a strict superset; **native wins** when both present |
| 2 | GitHub/Copilot plugins ship a `.github/plugin/plugin.json` manifest over a Claude-mirror capability tree (often alongside a `.claude-plugin/` manifest); some indexed plugins are bare capability trees with **no manifest at all** | Copilot manifests parse natively (dir overrides honored); manifest-less installs get a synthesized `.agents-plugin/plugin.json` from the marketplace entry |
| 3 | Claude marketplace had no native equivalent | Added native `.agents-plugin/marketplace.json` (preferred over Claude/Copilot) |
| 4 | Claude uses `${CLAUDE_PLUGIN_ROOT}` | Both `${CLAUDE_PLUGIN_ROOT}` and `${AGENTS_PLUGIN_ROOT}` are substituted/exported |
| 5 | Installs historically hardcoded `.hoocode/plugins/` | Installs now default to `.agents/plugins/`; `.hoocode/plugins/` still discovered + removable |
| 6 | Providers / typed TS `extensions` are richer than Claude allows | Native-only fields; ignored on the Claude-compat path (`extensions` deferred) |
| 7 | Some Claude hook events aren't mapped | `Notification` / `SubagentStop` / `PreCompact` / `hookSpecificOutput` deferred — see §2 |
| 8 | A repo can carry conflicting index formats (e.g. `.github/` + `.claude-plugin/`) | Precedence still picks one `format`; the optional `supportPlatform` records **all** platforms present so the conflict is visible — see §3 |

---

## 6. Authoring guidance (for humans and agents)

- **Prefer native.** New plugins/marketplaces should use `.agents-plugin/` and be
  installed under `.agents/plugins/`.
- **Claude plugins work as-is.** Drop a `.claude-plugin/` plugin into
  `.agents/plugins/` (or install it via a marketplace) and it loads; use
  `${CLAUDE_PLUGIN_ROOT}` or `${AGENTS_PLUGIN_ROOT}` — either resolves.
- **GitHub/Copilot = marketplace, not manifest.** To publish via GitHub, ship a
  `.github/marketplace.json` index whose entries point at plugin dirs / git URLs;
  don't expect a `.github/plugin.json` to be recognized.
- **User-meaningful state lives in `.agents/`; private runtime state stays in
  `.hoocode/`.** Plugins, marketplaces, and `/loop` scheduled tasks are
  `.agents/` (portable, user-visible). Sessions and dispatch state remain
  `.hoocode/` (private, machine-local).

# Plugin Format Mapping — native `.agents` ↔ Claude ↔ GitHub

Status: **reference**. This document is the single source of truth for how hoocode
supports plugins authored for **Claude Code** (`.claude-plugin/`) and **GitHub
Copilot** (`.github/plugin/` preferred, with the Copilot CLI's other locations —
root `plugin.json`, `.plugin/` — and legacy `.github/` spots also read)
alongside hoocode's **native** format (`.agents-plugin/`), and where each thing
is **packaged, installed, stored, and loaded**. It exists so that both humans
and agents can reason about the layout without re-reading the loader.

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
| **Plugin manifest** | Describes **one** capability bundle (a single plugin directory). | `.agents-plugin/plugin.json` (native), `.claude-plugin/plugin.json` (Claude), Copilot probe order: `.github/plugin/plugin.json` (preferred) → root `plugin.json` → `.plugin/plugin.json` → legacy `.github/copilot-plugin.json` |
| **Marketplace index** | Lists **many** installable plugins in a repo/dir. | `.agents-plugin/marketplace.json` (native), `.claude-plugin/marketplace.json` (Claude), Copilot probe order: `.github/plugin/marketplace.json` (preferred) → legacy `.github/marketplace.json` → root `marketplace.json` → `.plugin/marketplace.json` |

**Copilot conventions.** hoocode's preferred Copilot home is the
**`.github/plugin/` marker directory** — the convention used by the real-world
plugins indexed by
[github/copilot-plugins](https://github.com/github/copilot-plugins) (e.g.
microsoft/work-iq, which ships `.github/plugin/plugin.json` alongside a
`.claude-plugin/` mirror) and the documented location for GitHub-hosted
marketplaces. The official Copilot CLI plugin reference (docs.github.com,
`copilot/reference/copilot-cli-reference/cli-plugin-reference`, "File
locations"; verified 2026-07) accepts several locations for both files
(`.plugin/`, plugin root, `.github/plugin/`, `.claude-plugin/`), and hoocode
reads them all — `.github/plugin/` first — while **emitting**
`.github/plugin/plugin.json` for authored plugins. The **capability tree
mirrors the Claude layout** (top-level `skills/`, `agents/`, `commands/`, hooks
at root `hooks.json` or `hooks/hooks.json`, `.mcp.json`). Manifests may carry
dir-path overrides (`"skills": "./skills/"`), which hoocode honors, and
`author` is an object (`{ "name": ... }`). Marketplace entries may use the
shorthand source
`{ "source": "github", "repo": "owner/name", "path": "subdir" }`, which hoocode
normalizes to the equivalent git / git-subdir source, and both vendors'
`metadata.pluginRoot` (a base dir prepended to relative plugin sources) is
applied at parse time.

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
| `.github/plugin/marketplace.json` (preferred), legacy `.github/marketplace.json`, root `marketplace.json`, `.plugin/marketplace.json` — probed in that order | `"copilot"` | `github` | GitHub Copilot CLI plugin marketplace | plugins may carry a `.github/plugin/plugin.json` (preferred) or root `plugin.json` manifest |

### `--support-platform` — targeting what hoocode *writes*

The same platform vocabulary drives the write side. The `--support-platform`
CLI flag (or the `supportPlatform` setting; flag wins) sets the session-wide
target layout(s) for everything hoocode produces:

```
hoocode --support-platform copilot            # Copilot layouts only
hoocode --support-platform claude,copilot     # both (comma or repeated flag)
```

Tokens are the aliases above (`copilot`/`gh` → `github`, `native` → `agents`);
unknown tokens warn and are skipped. The targets apply to:

- **Authored plugins** (`ProposePlugin` / `UpdatePlugin`): replaces the default
  portable `agents` (native) target. Authored artifacts are meant to be
  reusable, so with no flag they are written as one vendor-neutral native
  layout; `--support-platform` is the only way to also emit vendor layouts (an
  opt-in interop choice). The authoring tools expose no per-call platform
  parameter — the session flag governs.
- **Workspace scaffolds** (`/new-skill`, `/new-agent`, `/new-command`): instead
  of `.hoocode/`, each target platform's *workspace* conventions are written
  (verified against the vendors' docs, 2026-07):

| Artifact | `claude` | `github` (Copilot) | `agents` (native) |
|---|---|---|---|
| skill | `.claude/skills/<name>/SKILL.md` | `.github/skills/<name>/SKILL.md` | `.agents/skills/<name>/SKILL.md` |
| subagent | `.claude/agents/<name>.md` (`tools` comma string) | `.github/agents/<name>.agent.md` (`tools` YAML list) | `.agents/agents/<name>.md` |
| command | `.claude/commands/<name>.md` | `.github/prompts/<name>.prompt.md` | `.agents/commands/<name>.md` |

Each adapter (`formats/claude.ts`, `formats/copilot.ts`, `formats/agents.ts`)
carries its own `WorkspaceLayout`, and the session state lives in
`formats/platform-targets.ts` — supporting a new vendor's write conventions is
still a one-adapter change.

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
  .agents-plugin/plugin.json     # or .claude-plugin/plugin.json, or .github/plugin/plugin.json (Copilot;
                                 #   root plugin.json and .plugin/plugin.json also read)
  skills/  commands/  agents/  themes/
  hooks/hooks.json               # or inline "hooks" (Copilot also reads root hooks.json)
  .mcp.json                      # or inline "mcpServers"
```

A marketplace is a repo/dir with an index at its root plus the plugin dirs it
points at (or git/npm sources):

```
my-marketplace/
  .agents-plugin/marketplace.json    # or .claude-plugin/…, .github/plugin/… (Copilot preferred),
                                     #   legacy .github/…, root marketplace.json, .plugin/…
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
| 2 | Copilot CLI accepts several manifest locations (`.plugin/`, root `plugin.json`, `.github/plugin/`, `.claude-plugin/`); real-world plugins (github/copilot-plugins index) ship `.github/plugin/plugin.json` + `.claude-plugin/` mirrors; some indexed plugins are bare capability trees with **no manifest at all** | Copilot adapter reads every location (`.github/plugin/` first — hoocode's preferred home — then root, `.plugin/`, legacy) and emits `.github/plugin/plugin.json`; manifest-less installs get a synthesized `.agents-plugin/plugin.json` from the marketplace entry |
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
- **GitHub/Copilot: `.github/plugin/` for both files.** Ship the plugin
  manifest at `.github/plugin/plugin.json` (the Copilot CLI also accepts a
  root `plugin.json` or `.plugin/plugin.json`), and make a repo a marketplace
  with `.github/plugin/marketplace.json` whose entries point at plugin dirs /
  git URLs / `{ "source": "github", "repo": ... }` shorthands.
  `metadata.pluginRoot` prefixes relative sources.
- **User-meaningful state lives in `.agents/`; private runtime state stays in
  `.hoocode/`.** Plugins, marketplaces, and `/loop` scheduled tasks are
  `.agents/` (portable, user-visible). Sessions and dispatch state remain
  `.hoocode/` (private, machine-local).

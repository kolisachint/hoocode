# Plugins, marketplaces and packages

Status: **draft for review, 2026-10-07.** Design only. Replaces ledger tasks 12.1
(declarative plugins and marketplace) and 12.2 (package manager).
Index: [post-migration-roadmap.md](post-migration-roadmap.md).

hoocode-ts references, all MIT and at the pin: `docs/loop-and-plugin-system.md` §2,
`docs/plugin-system-spec.md`, `docs/plugin-format-mapping.md`,
`docs/plugin-system-architecture.md`, `docs/agent-spec-tree-map.md`. Source is
`core/extensions/plugins/` (23 files, 5,801 lines) and `core/package-manager.ts`
(1,952 lines).

## Problem

A plugin is a directory with a manifest that bundles skills, slash commands,
subagents, themes, hooks, MCP servers and (native only) model providers. hoocode-ts
reads three manifest formats (native `.agents-plugin/`, Claude `.claude-plugin/`,
Copilot `.github/plugin/` and its other probe locations). It installs plugins from
git-hosted marketplaces, lets the model search, install and remove them in a single
turn, and can author and publish plugins behind four validation gates.

`cortex` loads none of it. The Rust resource loader already does the local half of
discovery: skills, prompts, themes and agents from `.agents/`, `.claude/` and the
agent dir, with `PathMetadata.namespace` ready for `plugin:skill` names. What is
missing:

- reading plugin manifests;
- the hooks bridge;
- plugin MCP servers (needs [mcp.md](mcp.md));
- marketplaces and install;
- the model-facing lifecycle tools;
- the `packages` setting's git and npm sources (12.2).

## Goals

1. A plugin that works in hoocode-ts or Claude Code works in `cortex`, from the same
   directories. The two hoocodes share `~/.agents/plugins/`,
   `~/.agents/marketplaces.json`, `~/.agents/marketplace-cache/`,
   `~/.agents/plugin-data/` and `~/.agents/trusted-workspaces.json`, and keep their
   file formats byte-compatible.
2. Executable content (hooks, MCP servers) never starts from a repository the user
   hasn't trusted.
3. Passive capabilities stay lazy. Skills, commands and agents keep their two-tier
   catalogs, and nothing is injected eagerly that TS doesn't inject.

Non-goal for now: authoring and publishing (§8 phase 4). Running typed TS extension
code is [extension-runtime.md](extension-runtime.md).

## 1. Model

```
PluginManifest (native | claude | copilot)      ── parse ──▶  NormalizedPlugin
NormalizedPlugin { id, version, root, scope, format,
                   skills_dir, commands_dir, agents_dir, themes_dir,
                   hooks: HookTable, mcp_servers: Map, providers: [..],
                   canvases: [..], unknown_fields, unsupported_surfaces }
```

- **Precedence when a directory has several manifests:** native, then Claude, then
  Copilot. There is no merging (TS rule).
- **Copilot probe order:** `.github/plugin/plugin.json`, then root `plugin.json`,
  then `.plugin/plugin.json`, then legacy `.github/copilot-plugin.json`. Copilot
  agents are read from `agents/<name>.agent.md` (YAML-list `tools`) and from bare
  `.md`. Copilot commands come from `.github/prompts/*.prompt.md`. The vendor
  namespace dir `com.github.copilot/agents/` is read.
- `unknown_fields` and `unsupported_surfaces` are kept so drift is visible in
  `/plugin list` and not silently dropped (TS step 6).

Parsing lives in a new crate, `cortexcode-code-plugins` (pure: no IO beyond reading
the plugin dir, no process spawning), which is easy to fuzz and snapshot.

## 2. Discovery

Plugin directories, highest precedence first, first-wins by id. This is TS
`defaultPluginDirs` at the pin:

1. `<cwd>/.agents/plugins/` — project scope
2. `<cwd>/.hoocode/plugins/` — legacy project
3. `<cwd>/.claude/skills/` — skills-directory plugins (plugin roots inside are not
   also scanned as plain skills)
4. `~/.agents/plugins/` — user scope (marketplace installs)
5. `~/.agents/publish/github/` — authored GitHub-format plugins
6. `~/.claude/skills/`
7. `<agentDir>/plugins/` — legacy user. For Rust this is
   `~/.hoocode/rust/plugins/` under [naming-and-paths.md](naming-and-paths.md); the
   TS `~/.hoocode/plugins/` is read but never written.

## 3. Capability mapping

| Manifest field / dir | Rust target |
|---|---|
| `skills/` | Resource loader skill paths, named `<plugin>:<skill>` (TS D6, Claude parity). Collisions between plugins are impossible by construction. |
| `commands/`, Copilot `prompts/` | Prompt-template / slash-command paths, namespaced the same way |
| `agents/` | Agent registry manifest paths. Catalog eager, body on dispatch (unchanged). |
| `themes/` | Theme paths |
| `providers` (native only) | Model registry `register_provider` (the same path `models.json` uses) |
| `mcpServers`, `.mcp.json` | Plugin MCP registry, which [mcp.md](mcp.md) §3 consumes. `${AGENTS_PLUGIN_ROOT}`/`${CLAUDE_PLUGIN_ROOT}` and `${AGENTS_PLUGIN_DATA}`/`${CLAUDE_PLUGIN_DATA}` are substituted. |
| `hooks`, `hooks/hooks.json`, Copilot root `hooks.json` | Hooks bridge (§4) |
| canvas `extensions/<id>/extension.mjs` | Canvas discovery only ([canvas-and-mcp-apps.md](canvas-and-mcp-apps.md)). Listed, never loaded. |

Plugin data dir: `~/.agents/plugin-data/<id>/`. It's outside every plugin home, so a
reinstall keeps it (TS D9).

## 4. Hooks bridge

This is the main way third-party behaviour hooks into `cortex`, and the reason
[extension-runtime.md](extension-runtime.md) can stay small.

**Protocol (Claude Code's, as in TS):**

- A JSON event goes to the hook command on stdin.
- Exit `0` means success; stdout may be a JSON decision, and for prompt/session
  events plain stdout is extra context.
- Exit `2` blocks, with stderr or JSON `reason` as the reason.
- Any other exit is a non-blocking error.
- `CLAUDE_PLUGIN_ROOT`, `AGENTS_PLUGIN_ROOT` and the data-dir variables are set.
- Matchers: empty or `*` matches all tools; otherwise an anchored regex on the tool
  name, falling back to an exact match.

**Events:**

| Claude Code event | Rust hook point | TS at pin |
|---|---|---|
| `PreToolUse` | `tool_call`, before the permission gate (a hook `deny` skips the prompt) | yes |
| `PostToolUse` | `tool_result` | yes |
| `UserPromptSubmit` | `before_agent_start`, stdout appended as context | yes |
| `SessionStart` | `session_start` | yes |
| `Stop` | `agent_end` | yes |
| `PreCompact` | `session_before_compact` | **no**: proposed (P5) |
| `SubagentStop` | dispatch settle in `code-subagents` | **no**: proposed (P5) |
| `Notification` | permission prompt shown / turn idle | **no**: proposed (P5) |

Most extension points already exist: `ExtensionHooks` has `before_agent_start` and
`emit_session_event`, and `agent-core` has `before_tool_call`/`after_tool_call` hooks
(the permission gate uses the first). `ExtensionHooks` needs `tool_call` and `tool_result` methods wired to the
agent-core hooks; they also serve [extension-runtime.md](extension-runtime.md).

Hooks run with a timeout (default 60s, configurable per hook), in the session's cwd,
with output capped. A hung hook can't stall a turn: it is killed and reported.

## 5. Skills: standards on top of what exists

- **Agent Skills spec validation** (agentskills.io). `name` is 1–64 characters of
  `a-z0-9-`, no leading, trailing or double hyphen, and equal to its directory name;
  `description` is 1–1024 characters. Violations are **warnings** in `/skills` and
  `/plugin list`, not load failures, because existing skills in the wild break the
  rules. Scaffolding (§8) always writes valid ones.
- **`allowed-tools`** (experimental in the spec): it never grants permissions on its
  own. A skill's `allowed-tools` takes effect only after the user approves that skill
  once (stored per skill path and content hash). Until then the normal permission
  gate applies. This is stricter than TS and matches the MCP Skills extension's
  rule.
- **Skills over MCP** (`io.modelcontextprotocol/skills`, Final). When a connected
  server declares the extension:
  - its `skills/list` entries join the catalog under `<server>:<name>`, with the
    server as origin;
  - loading verifies size, SHA-256 and frontmatter against the retained manifest;
  - file reads are restricted to manifest URIs;
  - approval binds to the manifest digests, so a changed manifest re-asks;
  - verified content is cached under `~/.hoocode/rust/mcp-skill-cache/`, outside
    every filesystem skill path, and re-verified on read.
  This is [mcp.md](mcp.md) phase 5.

## 6. Trust

The record is shared with TS: `~/.agents/trusted-workspaces.json`
(`{ "workspaces": [{ "path", "at" }] }`). It lives outside every repository, so
repository content can't forge it.

- **User-scope plugins** (`~/.agents/plugins/` etc.): load fully.
- **Repository-supplied plugins** (anything under the cwd) in an untrusted workspace:
  passive capabilities load; hooks, MCP servers and providers are withheld, and
  `/plugin list` says so (TS D7, `plugin-workspace-trust`).
- Granting trust is a human act: `/plugin trust` (or `/trust`), or an explicit
  `/plugin install --scope project`. No tool can grant it.
- The same record gates project MCP servers ([mcp.md](mcp.md) M4) and canvases.

## 7. Packages (12.2)

The `packages` setting (user or project settings) lists sources that contribute
resources. Each source has a `package.json` `hoocode` key (`skills`, `prompts`,
`themes`, `agents`, `extensions`) or uses the conventional directories. Rust already
resolves the **local** half (`code-resources/src/package_resolve.rs`). Missing:

| Source | TS | Rust proposal |
|---|---|---|
| local path | yes | done |
| `git:` / git URL (`host/path[@ref]`) | `git clone` into `<agentDir>/git/<host>/<path>`, or `.hoocode/git/…` for project scope | Same layout under `~/.hoocode/rust/git/` and `<cwd>/.hoocode/git/`. Uses the `git` CLI, so the user's credentials and SSH config just work. Clone and update run with a timeout; `HOOCODE_OFFLINE=1` skips the network. |
| `npm:<spec>` | `npm install --prefix …` | **Phase 2, no Node required:** fetch the tarball from the registry over HTTPS, verify `integrity` (SHA-512), extract to `~/.hoocode/rust/npm/<name>@<version>/`. **Never run install scripts.** Packages whose `hoocode.extensions` entries are TS code need the extension host. |
| CLI | `hoocode install/remove/update/list [--local]` | Same subcommands on `cortex` (they exist in TS's `package-manager-cli.ts`) |

Plugins and packages share the git fetcher and the resource-loading path. Plugins
are the user-facing story; `packages` stays for settings-driven setups and hoocode-ts
users.

## 8. Phases

| Phase | Scope |
|---|---|
| **1. Load** | Crate `code-plugins` (parse, normalize), discovery (§2), capability mapping (§3) except MCP, hooks bridge (§4), trust (§6), skills validation (§5), `/plugin list`, `/plugin trust`. MCP servers flow in once [mcp.md](mcp.md) phase 1 lands. |
| **2. Install** | Marketplace parsing (native, Claude and Copilot indexes; `{ source: "github", repo, path }` shorthand; `metadata.pluginRoot`; precedence with `supportPlatform` recorded), registry `~/.agents/marketplaces.json`, cache with TTL and `.fetched.json`, ref pinning, `/plugin marketplace add|list|refresh`, `/plugin install|remove [--scope user|project]`, synthesized manifest for manifest-less installs. Reload: passive capabilities live at the next turn via `prepare_next_turn`; executable ones after the turn (reload when idle). Packages: git sources (§7). |
| **3. Model tools** | `SearchPlugins` (substring match plus capability-index hits appended, per [semantic-search.md](semantic-search.md) §2), `InstallPlugin`, `UninstallPlugin`, `ListPlugins`. Trust rules from `plugin-system-spec.md`: adding a marketplace is human-only; installing from a trusted marketplace is the model's call, announced and reversible; an install prompted by untrusted content (fetched web text, a PR comment) asks first; plugin tools are never given to subagents. Tool schemas cost tokens on every request, so they're registered only when at least one marketplace is configured. Packages: npm sources. |
| **4. Produce** (decision P2) | `/new-skill`, `/new-agent`, `/new-command` scaffolds (cheap and useful). `ProposePlugin`/`UpdatePlugin`/`RemovePluginCapability`, gates G1–G4, packaging and publish only if wanted. |

## 9. Tests

- **Parsing snapshots** (insta) for every manifest location and format, copied from
  the TS test fixtures (MIT). Fuzz the manifest parsers.
- **Hooks bridge:** exit codes, JSON decisions, matcher semantics, timeout kill,
  env variables, `deny` before the permission prompt.
- **Trust:** a repository plugin's hook doesn't run untrusted and runs once trusted;
  the trust file round-trips with the TS format.
- **Marketplace:** local git repos made in a temp dir (no network); TTL; ref pinning;
  install and remove at both scopes.
- **Cross-tool:** a plugin installed by hoocode-ts loads in `cortex`, and the reverse.
  A fixture tree is shared by both test suites.
- **Reading list (TS):** `plugins`, `marketplace`, `marketplace-ttl`,
  `marketplace-ref-pin`, `plugin-lifecycle`, `plugin-hooks-scripts`,
  `plugin-workspace-trust`, `plugin-containment`, `plugin-marketplace-command`,
  `plugin-e2e-official` and `plugin-e2e-copilot` (network; manual), and for packages
  `package-command-paths`, `package-manager-ssh`, `git-update`, `config` and
  `regressions/2781-skill-collision-precedence`.

## 10. Decisions for the user

| # | Question | Recommendation |
|---|---|---|
| P1 | Ship the well-known marketplace (`anthropics/claude-plugins-official`) pre-trusted, as TS does? | Yes, read-only and never auto-updated, same as TS |
| P2 | Build plugin authoring and publishing (phase 4 beyond scaffolds)? | Not now. Scaffolds yes. |
| P3 | Gate `allowed-tools` behind a one-time per-skill approval (§5), unlike TS? | Yes |
| P4 | npm package sources without Node (tarball fetch, no scripts)? | Yes, in phase 3 |
| P5 | Map `PreCompact`, `SubagentStop` and `Notification` hooks, which TS doesn't? | Yes. They're cheap, and Claude plugins use them. |
| P6 | Copilot plugins' MCP servers win over user config (Copilot's last-wins) or stay first-wins like everything else (TS open item §8.6.1)? | First-wins |

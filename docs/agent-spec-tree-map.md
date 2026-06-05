# Agent Spec Tree Map

> **Audience: humans and LLMs.** This doc is read by people and by coding
> agents. Keep it deterministic: stable headings/anchors, literal path strings
> in `code`, one fact per row, and a `file:line` citation for every "hoocode
> does X" claim. When code changes, update the matching row and its citation in
> the same change. Do not state behavior you have not verified in source.

Tracks the cross-vendor "agent spec" surfaces that exist on disk, their
standardization status, and **how hoocode supports (or does not support) each
one today**. Use this as the single source of truth when adding loaders or
deciding whether to adopt a competing convention.

- Reference tree below = the broad ecosystem proposal (`~/.agents/` "Protocol").
- "What hoocode actually scans" = the real surface area hoocode loads from disk.
- Coverage matrix maps one to the other with file:line citations.

Last verified against source: see citations in the [Coverage matrix](#coverage-matrix).

### How to read this doc

- **Human:** start at the [Reference tree](#reference-tree-ecosystem-proposal),
  then [What hoocode actually scans](#what-hoocode-actually-scans), then the
  [Gap analysis](#gap-analysis--suggestions).
- **LLM / agent:** the [Quick reference](#quick-reference-machine-readable) block
  below is the authoritative, parseable summary. Treat the
  [Coverage matrix](#coverage-matrix) as the detail source and the cited
  `file:line` as ground truth. If prose and code disagree, the code wins —
  re-verify and fix the row.

### Definitions

- **Surface** = a named on-disk artifact hoocode may load (a file or directory
  convention), e.g. slash commands or skills.
- **Global** = user home scope, e.g. `~/.hoocode/`. **Project** = repo scope,
  e.g. `./.hoocode/`.
- **Support levels:** `Full` = read and honored; `Partial` = read but limited or
  the format differs from the standard; `None` = not scanned.
- **Std tag:** standardization status, see
  [Legend & competing standards](#legend--competing-standards).

### Quick reference (machine-readable)

Stable, parseable summary. `support` ∈ {full, partial, none}. Paths are literal.

```yaml
scopes:
  global: ~/.hoocode/
  project: ./.hoocode/
surfaces:
  - id: instructions
    support: full
    files: [AGENTS.md, AGENTS.MD, CLAUDE.md, CLAUDE.MD]
    paths: [~/.hoocode/, "walk cwd -> filesystem root"]
    src: core/resource-loader.ts:76-128
  - id: slash-commands
    support: full
    files: ["*.md"]
    paths: [./.hoocode/commands, ./.claude/commands, ~/.hoocode/commands, ~/.claude/commands]
    keys: [name, description, type, argument-hint]
    src: core/resource-loader.ts:340-360
  - id: skills
    support: full
    files: [SKILL.md]
    paths: [~/.hoocode/skills, ~/.claude/skills, ~/.agents/skills, ./.hoocode/skills, ./.claude/skills, "<git-root..cwd>/.agents/skills"]
    keys: [name, description, allowed-tools, disable-model-invocation]
    src: core/skills.ts
  - id: agents
    support: partial   # single-file .md frontmatter, NOT paired agent.md+config.json
    files: ["*.md"]
    paths: [./.hoocode/agents, ./.claude/agents, "<git-root..cwd>/.agents/agents", ~/.hoocode/agents, ~/.claude/agents]
    keys: [name, description, tools, model, maxTurns, background]
    src: core/agent-registry.ts:9-16
  - id: mcp
    support: full     # reads standard mcp.json + per-server JSON
    files: ["*.json"]
    paths: [~/.agents/mcp.json, ./.agents/mcp.json, ~/.config/claude/mcp.json, ~/.hoocode/mcp-servers, ./.hoocode/mcp-servers]
    keys: [name, command, args, env] (per-server) or [mcpServers] (standard)
    src: extensions/core/hoo-core.ts:385-600
  - id: models
    support: full      # hoocode-specific schema
    files: [models.json]
    paths: [~/.hoocode/models.json]
    src: core/model-registry.ts:345
  - id: settings
    support: full      # equivalent of speakmcp-settings.json
    files: [settings.json]
    paths: [~/.hoocode/settings.json, ./.hoocode/settings.json]
    src: core/settings-manager.ts:74-201
  - id: extensions
    support: full
    files: [index.ts, index.js, "*.ts", "*.js"]
    paths: [./.hoocode/extensions, ~/.hoocode/extensions]
    src: core/extensions/loader.ts:420-500
  - id: tasks
    support: none      # only background:true frontmatter + runtime dispatch state
    src: core/agent-frontmatter.ts:41-45
  - id: memories
    support: none
  - id: layouts
    support: none
  - id: backups
    support: none      # sessions auto-saved at ~/.hoocode/sessions/<id>/session.jsonl
    src: config.ts:385-390
```

---

## Reference tree (ecosystem proposal)

The aspirational `~/.agents/` layout, annotated by standardization status. Tags
are defined in [Legend & competing standards](#legend--competing-standards).

```text
~/.agents/
├── AGENTS.md                    [A]  OpenAI / Linux Foundation — Accepted
├── system-prompt.md             [D]  Claude Code de facto — ~/.claude/CLAUDE.md
├── mcp.json                     [A]  Anthropic / Linux Foundation MCP — Accepted
├── models.json                  [P1] SpeakMCP / .agents Protocol — Proposed
├── speakmcp-settings.json       [P1] SpeakMCP-specific — Proposed
│
├── commands/                    [D]  Claude Code convention — ~/.claude/commands/
│   ├── review.md
│   └── deploy.md
│
├── skills/                      [A]  Anthropic Skills — Accepted (SKILL.md)
│   ├── code-review/SKILL.md
│   ├── deploy-pipeline/SKILL.md
│   └── api-design/SKILL.md
│
├── agents/                      [P1] SpeakMCP / .agents Protocol — Proposed
│   ├── code-reviewer/           [C1] Competes: Claude Code (~/.claude/agents/*.md)
│   │   ├── agent.md             [C2] Competes: OpenCode (~/.config/opencode/agents/*.md)
│   │   └── config.json
│   ├── security-scanner/{agent.md,config.json}
│   └── test-writer/{agent.md,config.json}
│
├── tasks/                       [P1] SpeakMCP / .agents Protocol — Proposed
│   └── daily-code-review/task.md   [C3] Closest: Claude Code ScheduleWakeup / Background
│
├── memories/                    [P1] SpeakMCP / .agents Protocol — Proposed
│   ├── arch-decisions.md        [C4] Competes: Claude Code ~/.claude/agent-memory/
│   ├── user-prefs.md
│   └── project-context.md
│
├── layouts/                     [P1] SpeakMCP / .agents Protocol — Proposed
│   └── ui.json
│
└── .backups/                    [P1] SpeakMCP / .agents Protocol — Proposed
    ├── skills/
    └── memories/
```

---

## What hoocode actually scans

hoocode's native home is `~/.hoocode/` (global) and `./.hoocode/` (project). It
also reads selected Claude Code (`.claude/`) and `.agents/` locations for
compatibility. Filenames and formats below are the ones hoocode resolves.

```text
~/.hoocode/                         GLOBAL                       ./.hoocode/        PROJECT
├── settings.json   [S]  ~40 keys                               ├── settings.json  [S]  (deep-merged over global)
├── models.json     [S]  custom providers/models                ├── commands/*.md  [S]  slash commands
├── commands/*.md   [S]  slash commands                         ├── skills/<n>/SKILL.md
├── skills/<n>/SKILL.md                                         ├── agents/*.md    [S]  single-file frontmatter
├── agents/*.md     [S]  single-file frontmatter                ├── extensions/    [S]
├── extensions/     [S]                                         ├── mcp-servers/*.json
├── mcp-servers/*.json  [S]  per-server JSON (fallback)         └── dispatch/<id>/  runtime task state (not a spec)
└── sessions/<id>/session.jsonl  runtime, not a spec

Compatibility locations also scanned:
  ~/.claude/commands/*.md, ./.claude/commands/*.md      (slash commands)
  ~/.claude/skills/,        ./.claude/skills/           (skills)
  ~/.claude/agents/*.md,    ./.claude/agents/*.md       (subagents)
  ~/.agents/skills/                                     (skills)
  <git-root..cwd>/.agents/skills/                       (skills, ancestor-walk)
  <git-root..cwd>/.agents/agents/*.md                   (subagents, ancestor-walk)

Project instructions resolved by walking up from cwd to filesystem root, plus
the global agent dir, matching: AGENTS.md / AGENTS.MD / CLAUDE.md / CLAUDE.MD.

Declared via package.json "hoocode" (or legacy "pi") manifest:
  agents[], skills[], extensions[], prompts[], themes[]
```

Legend: `[S]` = hoocode-supported surface.

---

## Directory routing policy (read + write)

**Read: broad.** Scan `.hoocode/` (native), vendor dirs (`.claude/`), and
`.agents/`, merged last-wins. See [Precedence](#precedence-project-beats-global-last-write-wins-on-name).

**Write: everything to `.agents/`.** When hoocode creates or edits a spec
(`/init`, saving an agent, etc.), write it under `.agents/` so other tools can
consume it. Be a good `.agents` citizen.

- **Standard surfaces** (AGENTS.md, `mcp.json`, SKILL.md) -> write in the
  standard shape. Readable by everyone, no extra annotation needed.
- **Non-standard surfaces** (commands, subagents, etc.) -> write under
  `.agents/`, and document them in a single top-level **`.agents/AGENTS.md`** so
  other tools know how to read them. That file describes each non-standard
  surface: its location, file shape, and which keys hoocode honors.
- **Never** write to a vendor dir (`.claude/`). Never overwrite a foreign file
  (merge by name / create-if-absent).

### `.agents/` support matrix

Per-surface status against the policy above. **Std?** = is it a cross-vendor
standard (read by everyone) or non-standard (needs `.agents/AGENTS.md`).
**Read .agents** / **Write .agents** = does hoocode do it *today*.

| Surface | Std? | Write target (policy) | Read `.agents` today | Write `.agents` today | Gap |
|---|---|---|---|---|---|
| Instructions (`AGENTS.md`) | Standard | `.agents/AGENTS.md` | No (reads `~/.hoocode`, cwd-walk) | No (writes `.hoocode`) | Read+write `.agents/AGENTS.md` |
| Skills (`SKILL.md`) | Standard | `.agents/skills/<n>/SKILL.md` | Yes (`~/.agents/skills`, ancestor) | No (writes `.hoocode`) | Write to `.agents/skills` |
| MCP (`mcp.json`) | Standard | `.agents/mcp.json` | No (per-server `mcp-servers/*.json`) | No | Read+write standard `.agents/mcp.json` |
| Commands (`*.md`) | Non-standard | `.agents/commands/` + doc in `.agents/AGENTS.md` | No (`.hoocode`/`.claude` only) | No | Read+write `.agents/commands`; document |
| Subagents (`*.md`) | Non-standard | `.agents/agents/` + doc in `.agents/AGENTS.md` | Yes (`.agents/agents` ancestor) | No | Write `.agents/agents`; document shape |
| Settings | Non-standard | `.agents/` (hoocode shape) + doc | n/a | No (`.hoocode/settings.json`) | Optional: relocate + document |
| Models | Non-standard | `.agents/models.json` + doc | No | No (`~/.hoocode/models.json`) | Optional: relocate + document |
| Extensions | Non-standard | `.agents/extensions/` + doc | No | No (`.hoocode/extensions`) | Optional: relocate + document |
| Tasks | Non-standard | `.agents/tasks/` + doc | No | No | Not implemented |
| Memories | Non-standard | `.agents/memories/` + doc | No | No | Not implemented |

Legend: **Standard** = self-describing, no annotation needed. **Non-standard** =
must be listed in `.agents/AGENTS.md` so other tools can read it.

### Quick wins (toward the policy)

Ordered by value-to-effort. Each is small and standards-aligned.

1. **Write specs to `.agents/` (the core of the policy).** Point the spec
   writers (`/init`, save-agent, save-command) at `.agents/` instead of
   `.hoocode/`. Start with skills + subagents (loaders already read `.agents`).
2. **Generate `.agents/AGENTS.md` on write.** When writing a non-standard
   surface, append/update its entry (location, file shape, honored keys) in the
   top-level `.agents/AGENTS.md`. Make it auto-generated ("do not edit").
3. ~~Read + write standard `.agents/mcp.json`.~~ **Done.** Added readers for
   `~/.agents/mcp.json`, `.agents/mcp.json`, and `~/.config/claude/mcp.json`.
   Standard format desugared into existing per-server config. Closes the [A]
   standard mismatch on format.
4. **Read `~/.agents/AGENTS.md` (and `.agents/AGENTS.md`) as instructions.**
   Trivial addition to the instruction scanner; honors the standard location.
5. **Read `.agents/commands/`.** Add `.agents/commands` (global + ancestor) to
   the command search path so commands written there round-trip.

---

## Coverage matrix

Status: **Full** = read and honored; **Partial** = read but limited/format
differs; **None** = not scanned.

| Surface (proposal) | Std tag | hoocode | Actual path(s) in hoocode | Format / keys | Source (file:line) |
|---|---|---|---|---|---|
| `AGENTS.md` instructions | [A] | **Full** (relocated) | Global `~/.hoocode/`; walk cwd→root | `AGENTS.md`/`CLAUDE.md` (+`.MD`); plain markdown | `core/resource-loader.ts:76-128` |
| `system-prompt.md` (`~/.claude/CLAUDE.md`) | [D] | **Partial** | Same as above; `--system-prompt` / `--append-system-prompt` flags | `CLAUDE.md` read as instructions; no `system-prompt.md` filename | `core/resource-loader.ts:76-128`, `core/system-prompt.ts` |
| `mcp.json` | [A] | **Full** | `~/.agents/mcp.json`, `.agents/mcp.json`, `~/.config/claude/mcp.json`, `~/.hoocode/mcp-servers/*.json`, `./.hoocode/mcp-servers/*.json` | Standard `{"mcpServers":{...}}` + per-server JSON fallback | `extensions/core/hoo-core.ts:385-600` |
| `models.json` | [P1] | **Full** (own format) | `~/.hoocode/models.json` | hoocode custom providers/models; not the [P1] schema | `core/model-registry.ts:345`, `core/sdk.ts:202` |
| `speakmcp-settings.json` | [P1] | **None (equivalent)** | `~/.hoocode/settings.json`, `./.hoocode/settings.json` | hoocode `settings.json` (~40 keys), deep-merged | `core/settings-manager.ts:74-201` |
| `commands/*.md` | [D] | **Full** | `./.hoocode/commands`, `./.claude/commands`, `~/.hoocode/commands`, `~/.claude/commands` | `*.md` frontmatter: `name`, `description`, `type`, `argument-hint` | `core/resource-loader.ts:340-360`, `core/prompt-templates.ts` |
| `skills/<n>/SKILL.md` | [A] | **Full** | `~/.hoocode/skills`, `~/.claude/skills`, `~/.agents/skills`, project `.hoocode`/`.claude`/`.agents` skills, ancestor-walk | `SKILL.md` frontmatter: `name`, `description`, `allowed-tools`, `disable-model-invocation` | `core/skills.ts`, `core/package-manager.ts:442,2179,2233` |
| `agents/<n>/{agent.md,config.json}` | [P1]/[C1]/[C2] | **Partial (different shape)** | `.hoocode/agents`, `.claude/agents`, `.agents/agents` (ancestor), `~/.hoocode`/`~/.claude`, package manifest, `--agent` | **Single `.md` + YAML frontmatter** (Claude/[C1] shape), NOT paired `agent.md`+`config.json`. Keys: `name`, `description`, `tools`, `model`, `maxTurns`, `background` | `core/agent-registry.ts:9-16,129-192`, `core/agent-frontmatter.ts` |
| `tasks/<n>/task.md` | [P1]/[C3] | **None (spec)** | — | No file-based task specs. Only `background: true` agent frontmatter + runtime `./.hoocode/dispatch/<id>/` state | `core/agent-frontmatter.ts:41-45`, `config.ts:337-342` |
| `memories/*.md` | [P1]/[C4] | **None** | — | No on-disk memory loader (`~/.claude/agent-memory/` not read) | (absent) |
| `layouts/ui.json` | [P1] | **None** | — | UI compiled in; no on-disk layout specs | (absent) |
| `.backups/` | [P1] | **None (equivalent)** | `~/.hoocode/sessions/<id>/session.jsonl` | Sessions auto-saved (not a `.backups` spec) | `config.ts:385-390` |
| Extensions | — (hoocode) | **Full** | `./.hoocode/extensions`, `~/.hoocode/extensions`, `--extensions`, manifest | `index.ts/js` or `*.ts/js`, default `ExtensionFactory` export | `core/extensions/loader.ts:420-500` |
| package.json manifest | — (hoocode) | **Full** | `hoocode` (or legacy `pi`) key | `agents[]`, `skills[]`, `extensions[]`, `prompts[]`, `themes[]` | `core/package-manager.ts:534-550` |

### Precedence (project beats global, last write wins on name)

- Instructions: global agent dir first, then cwd→root; project-closest wins.
- Slash commands: `./.hoocode` > `./.claude` > `~/.hoocode` > `~/.claude` (first-match-wins by name).
- Skills: claude-user < user < ancestor-walk < claude-project < project (last wins).
- Agents: builtin < package-manifest < `~/.claude` < `~/.hoocode` < `.agents` ancestor < `./.claude` < `./.hoocode` < `--agent` CLI.
- Settings: global loaded, then project deep-merged on top.

---

## Legend & competing standards

| Tag | Status | Source / Proponent | Notes |
|-----|--------|-------------------|-------|
| **[A]** | Accepted | Linux Foundation Agentic AI Foundation | AGENTS.md (OpenAI), MCP + Skills (Anthropic) |
| **[D]** | De facto | Claude Code / Anthropic | Widely used; not ratified cross-vendor. OpenCode reads these as fallbacks |
| **[P1]** | Proposed | aj47 / SpeakMCP / `.agents` Protocol | Draft from Feb 2026; not adopted by other frameworks |
| **[C1]** | Competes | Claude Code subagents | Single `.md` with YAML frontmatter at `~/.claude/agents/<name>.md` |
| **[C2]** | Competes | OpenCode agents | Single `.md` at `~/.config/opencode/agents/<name>.md`; filename is agent name |
| **[C3]** | Competes | Claude Code background | `background: true` in frontmatter; no directory structure |
| **[C4]** | Competes | Claude Code memory | `~/.claude/agent-memory/<agent>/MEMORY.md` via `memory:` frontmatter key |

---

## Claude subagent frontmatter (direct copy-paste)

hoocode reads single-file agents with YAML frontmatter, so Claude agent files
drop in directly. Source format:

```yaml
---
name: code-reviewer
description: Code review agent
tools: Read, Edit, Bash, Glob
model: sonnet
skills: [typescript-review]
memory: project
color: cyan
---
# System Prompt
...
```

Full Claude key set (16): `name`, `description`, `tools`, `disallowedTools`,
`model`, `permissionMode`, `maxTurns`, `skills`, `mcpServers`, `hooks`,
`memory`, `background`, `effort`, `isolation`, `initialPrompt`, `color`.

**hoocode honors today:** `name`, `description`, `tools`, `model`, `maxTurns`,
`background`. Tool names are aliased to hoocode's lowercase tools
(`Read`→`read`, `Write`→`write`, `Edit`→`edit`, `Bash`→`bash`, `Grep`→`grep`,
`Glob`/`Find`→`find`, `LS`→`ls`); unsupported Claude tools (WebSearch,
WebFetch, Task, MultiEdit, ...) are dropped. The remaining keys are ignored.

---

## Gap analysis & suggestions

Ordered by leverage. Each is a candidate, not a commitment.

1. **Agents: paired `agent.md` + `config.json` vs single-file frontmatter.**
   hoocode already supports the Claude/[C1] single-file shape, which is the
   widest-adopted format. The [P1] paired shape is not. Recommendation: do
   **not** add a second on-disk shape. If interop is needed, write a one-way
   importer that reads `agent.md` + `config.json` and emits a single
   frontmatter `.md` (the loader you proposed). Keep the canonical store
   single-file.

2. ~~**MCP: align on a single `mcp.json`.~~** **Done.** Added readers for standard
   `mcp.json` format at `~/.agents/mcp.json`, `.agents/mcp.json`, and
   `~/.config/claude/mcp.json`. Desugars into existing per-server config with
   deduplication (first-wins).

3. **Memories ([C4]) — currently None.** If we want persistent agent memory,
   the lowest-friction path is reading `memory:` frontmatter + a
   `memories/<agent>/MEMORY.md` (or Claude's `~/.claude/agent-memory/`). Scope
   first: session-only vs durable, and who writes it.

4. **Tasks ([P1]/[C3]) — currently None on disk.** hoocode has runtime dispatch
   + `background: true`, but no declarative `tasks/<n>/task.md`. If scheduled or
   reusable tasks are desired, a `task.md` spec (prompt + schedule + target
   agent) maps cleanly onto the existing dispatch machinery.

5. **`~/.agents/` instructions/commands.** hoocode reads `~/.agents/skills` and
   `.agents/agents` but **not** `~/.agents/AGENTS.md`, `~/.agents/commands/`, etc.
   If we want to be a good `.agents` citizen, extend the instruction and command
   scanners to include `~/.agents/` and the ancestor-walk `.agents/` (commands),
   mirroring the skills/agents treatment. Note: `~/.agents/mcp.json` is now read.

6. **Layouts / `.backups` ([P1]).** Low priority. hoocode covers the need via
   compiled UI + `sessions/`. Skip unless an explicit interop requirement
   appears.

### On your loader question

Yes — a one-way importer that reads Claude's single-file `.md` frontmatter is
worth building, but invert the direction from your phrasing: hoocode's
**canonical** agent format is already the single-file frontmatter `.md`, so the
importer should convert the [P1] paired `agent.md` + `config.json` **into** a
single frontmatter `.md`, not the reverse. That avoids a second on-disk shape in
the loader and keeps precedence/dedup logic simple.

---

## Maintenance

- When a loader changes, update the [Coverage matrix](#coverage-matrix) row and
  its `file:line` citation.
- Promote a [P1]/[D] row to **Full** only after the loader lands and is tested.
- Keep the proposal tree and "What hoocode actually scans" in sync; divergence
  between them is the gap list.

### Test coverage by surface

| Surface | Dedicated test file | Coverage |
|---|---|---|
| Agent frontmatter | `test/agent-frontmatter.test.ts` | 15 tests: parsing, validation, tool normalization |
| Agent registry | `test/agent-registry.test.ts` | 5 tests: loading, precedence, dedup |
| Skills | No dedicated file | Covered indirectly via integration tests |
| Commands | No dedicated file | Covered indirectly via integration tests |
| MCP servers | No dedicated file | Covered indirectly via extension tests |

Validation rules live inline in source (`agent-frontmatter.ts`, `skills.ts`).
No formal schemas (JSON Schema, Zod) exist yet.

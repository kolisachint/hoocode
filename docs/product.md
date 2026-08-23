<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="../assets/hoocode.svg">
    <img alt="HooCode" src="../assets/hoocode-light.svg" height="64">
  </picture>
</p>

<p align="center">Deterministic terminal coding agent.</p>

# Product

HooCode is a terminal coding agent built around a single idea: **you stay in control**. Where most agents act first and report later, HooCode is *deterministic* — every edit and every shell command passes through a permission gate you control, and the agent is scoped by an explicit mode instead of one do-everything prompt.

## Why HooCode?

| | HooCode | Typical AI editor |
|---|---|---|
| **Approval gates** | `Yes (once) / No (block) / Always` on every edit and command | Edits and commands apply on their own |
| **Mode-driven focus** | Ask · Plan · Build · Debug — each with its own prompt and tool set | One chat does everything |
| **Provider flexibility** | 25+ providers; switch with `--provider` / `--model` | Locked to one vendor |
| **Extensibility** | MCP servers, TypeScript extensions, per-project profiles | Closed plugin system |
| **Binary distribution** | Single self-contained binary, no Node.js at runtime | Requires an IDE or cloud account |

## Modes

Four modes, switched any time with `/mode <name>`:

1. **ask** — read-only Q&A. The agent explains, never writes.
2. **plan** — explores the repo and writes `.hoocode/plan.md` for you to review.
3. **build** — executes the approved plan, gating each edit and command.
4. **debug** — root-causes a failure without touching files.

```bash
hoocode               # start in build mode
hoocode /mode plan    # or draft a plan first
hoocode /approve      # review .hoocode/plan.md, then execute it
```

### From plan to execution

A plan does not have to go straight to `/approve`. Three commands sit on that path:

| Command | What it does |
|---|---|
| `/grill [me\|plan]` | Stress-tests the current plan. `/grill me` interrogates the *request* — it surfaces assumptions nobody confirmed and puts the consequential ones to you as questions. `/grill plan` attacks the *plan*, verifying its claims against the codebase and revising it where it is weak. Bare `/grill` runs both, asking first: underspecification sits upstream of plan weakness, so critiquing a plan built on a misread request only yields a well-reviewed plan for the wrong job. |
| `/approve` | Switches to build mode and executes the plan step by step, gating each edit and command as usual. |
| `/goal [--max-turns N] [objective]` | Works toward the goal autonomously, iterating until it reports completion or spends its turn budget. With no argument it takes the objective from the plan's **Goal** section; the plan's **Verification** section becomes the completion condition. Stop a run at any time with `/loop stop`. |

`/goal` is deliberately linear — one thread of execution, no task graph and no
subtask fan-out. It runs under whatever the active mode already permits
(`enabled_tools`, `allowed_bash_commands`, `allowed_write_paths`, `auto_allow`),
so how much rope an unattended run gets is a mode-config decision. Note that it
asks the agent to run the verification and trusts its report of completion;
nothing checks an exit code on your behalf.

## Tools

The agent works through a small, deterministic tool set. Available by default:

| Tool | What it does |
|---|---|
| `read` · `write` · `edit` | Read files, create new ones, and make exact-text edits. One `edit` call can apply several replacements at once, and an edit can set `replaceAll` to replace every occurrence instead of requiring a unique match. |
| `bash` | Run shell commands — each one gated by the `Yes / No / Always` permission prompt. |
| `search` | Ranked "find where code lives" — fuses exact-text and semantic (local embedding index) retrieval, returning `file:line-range` hits. Always available: it degrades to grep-backed lexical retrieval when no semantic index is present, so `--enable-embsearchtools` only controls whether the semantic index is built and fused in, not whether the tool exists. Use `search` to locate a concept or behavior; use `grep` for exact matching lines. |
| `grep` · `find` · `ls` | Search file contents (ripgrep), find files by glob pattern (fd — one or more patterns, optional type/depth/exclude filters), and list directories. `grep`/`find` respect `.gitignore`; `ls` lists a single directory and takes an optional `ignore` list to skip noise like `node_modules`. |
| **Task** (subagents) · **TodoWrite** | Delegate a self-contained task to a specialized agent that runs in its own isolated context and returns only its final answer, and maintain a live todo list shown in the task panel. Both are **on by default** — disable with `"enableSubagent": false` / `"enableTodoWrite": false`. |

When running interactively, the agent can also ask you to make a decision through a multiple-choice prompt when it genuinely needs your input to proceed. In non-interactive (`-p`) runs it falls back to proceeding on its own.

Four tool groups are **off by default** — turn them on per session with a flag, or persistently in settings:

| Tool group | Enable | What it does |
|---|---|---|
| **Web** (`webfetch` · `websearch`) | `--enable-webtools` or `"enableWebTools": true` | Fetch a URL as text and run web searches. |
| **Plugins** (`SearchPlugins` · `InstallPlugin` · `ProposePlugin` · ...) | `--enable-plugintools` or `"enablePluginTools": true` | The autonomous plugin lifecycle system — discover, install, and propose plugins, plus a runtime reuse nudge. |

## External binaries

hoocode is self-sufficient on its own: nothing below is required, and every one
of them is optional. They are an expansion layer — five small Rust binaries that
either make an existing tool faster or add a capability hoocode otherwise does
not have. hoocode resolves each from `PATH` or its own bin directory, and
downloads a published release when it needs one.

Because hoocode degrades quietly without them, they used to be invisible.
`/settings` → **External tools** now lists all five with their live status, what
each one enables, and what happens without it. Any settings row that is inert
without its binary is marked `needs <binary>` there.

| Binary | Adds | Without it |
|---|---|---|
| `rg` (ripgrep) | The fast path for `grep` and for the lexical half of `search`. Fetched at startup. | A pure-JS scanner with identical match output — materially slower on large trees. |
| `fd` | The fast path for `find`. Fetched at startup. | A JS directory walker with the same result shape, slower and with approximated glob/ignore handling. |
| `embsearch` | The local embedding index: semantic hits fused into `search`, and meaning-ranked MCP/capability lookup. Fetched on first use. Requires the ONNX build; the mock build is rejected. | `search` runs lexical-only and capability lookup ranks lexically. Nothing errors; intent-phrased queries just rank worse. |
| `webtools` | The `webfetch` and `websearch` tools — hoocode's only network path. Fetched on first use. | Both tools error when called. The web tool group is off by default, so this stays invisible until you enable it. |
| `voicetools` | Push-to-talk voice input in the TUI. Fetched on first use. | Voice capture reports an error and never starts. |

Resolution order for each is: `HOOCODE_<TOOL>_BINARY` (an explicit path, for a
local build) → hoocode's bin directory → `PATH` → download. Set `HOOCODE_OFFLINE=1`
to never download; set `HOOCODE_NATIVE_SEARCH=1` to force the JS paths even when
`rg`/`fd` are present.

## Extensibility

- **MCP servers** — connect external tools and data sources through the Model Context Protocol.
- **TypeScript extensions** — hook into the agent runtime to add commands, tools, UI, and behavior. See [`packages/coding-agent/examples`](../packages/coding-agent/examples) for working examples.
- **Per-project profiles** — scope settings, enabled tools, and providers to each project.
- **Claude compatibility** — reads Claude `.claude/agents` subagents and `SKILL.md` skills natively, normalizing `allowed-tools` to HooCode tool names.

## Packages

| Package | Description |
|---------|-------------|
| **[@kolisachint/hoocode-agent](../packages/coding-agent)** | Interactive coding agent CLI (`hoocode` / `hoo`) |
| **[@kolisachint/hoocode-agent-core](../packages/agent)** | Agent runtime with tool calling and state management |
| **[@kolisachint/hoocode-ai](../packages/ai)** | Unified multi-provider LLM API (OpenAI, Anthropic, Google, …) |
| **[@kolisachint/hoocode-tui](../packages/tui)** | Terminal UI library with differential rendering |

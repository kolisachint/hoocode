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
| `SearchCodebase` | The one code-discovery tool: ranked "find where code lives", fusing exact-text and semantic (local embedding index) retrieval and returning `file:line-range` hits. Respects `.gitignore`. Always available: it degrades to exact-text lexical retrieval when no semantic index is present, so `--enable-semantic-index` only controls whether the semantic index is built and fused in, not whether the tool exists. Use `SearchCodebase` to locate a concept or behavior; shell out through `bash` (`rg`, `find`, `ls`) when you need exact matching lines, counts, or a raw directory listing. |
| **Task** (subagents) · **TodoWrite** | Delegate a self-contained task to a specialized agent that runs in its own isolated context and returns only its final answer, and maintain a live todo list shown in the task panel. Both are **on by default** — disable with `"enableSubagent": false` / `"enableTodoWrite": false`. |

When running interactively, the agent can also ask you to make a decision through a multiple-choice prompt when it genuinely needs your input to proceed. In non-interactive (`-p`) runs it falls back to proceeding on its own.

Four tool groups are **off by default** — turn them on per session with a flag, or persistently in settings:

| Tool group | Enable | What it does |
|---|---|---|
| **Web** (`webfetch` · `websearch`) | `--enable-webtools` or `"enableWebTools": true` | Fetch a URL as text and run web searches. Search defaults to keyless DuckDuckGo; with no Brave/Tavily/SearXNG credential configured the TUI says so once per session — see [Web search providers](../packages/coding-agent/docs/settings.md#web-search-providers). |
| **Plugins** (`SearchPlugins` · `InstallPlugin` · `ProposePlugin` · ...) | `--enable-plugintools` or `"enablePluginTools": true` | The autonomous plugin lifecycle system — discover, install, and propose plugins, plus a runtime reuse nudge. |

### How much of a tool call you see

One dial, `Alt+O`, with three stops from least to most (`Shift+Alt+O` goes back). The footer shows where it sits.

| View | What a tool call looks like |
|---|---|
| **radar** | One line per *chain* — a run of consecutive tool calls. While it works the line shows its shape in order, `◐ search › read › bash✗ › edit › bash…`; once the agent moves on it becomes what the run amounted to, `● Edited packages/tui/src/keys.ts`. Thinking traces are hidden here regardless of the `Ctrl+T` setting — a view that folds a whole run into a row cannot then spend forty lines on the reasoning behind it. |
| **glance** | The tool's own call line, one per call, body folded away. **The default.** |
| **full** | Call line plus the result body. |

**A failure always shows why**, in every view. That is the one thing none of them fold away: a red dot with no explanation is worse than no folding at all.

A chain ends when the agent next speaks, or when the turn ends. A run that was interrupted keeps its working shape plus a marker rather than a tidy summary — the settled line is a claim about what the run amounted to, and a run cut off partway through has no such claim to make.

Two keys open things without moving the dial. `Ctrl+O` expands everything at once — tool bodies, the header, summaries. `Alt+U` opens one thing, newest first, repeating to peel backwards (`Shift+Alt+U` re-folds): in radar that turns a chain back into its calls, elsewhere it opens a single body. It works from the newest backwards because the transcript is bottom-anchored — older output lives in your terminal's own scrollback, which the agent can neither scroll nor put a cursor into.

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
| `rg` (ripgrep) | The fast path for the lexical half of `SearchCodebase`. Fetched at startup. | A pure-JS scanner with identical match output — materially slower on large trees. |
| `fd` | The fast path for `@`-file autocomplete in the TUI. Fetched at startup. | A JS directory walker with the same result shape, slower and with approximated glob/ignore handling. |
| `embsearch` | The local embedding index: semantic hits fused into `SearchCodebase`, and meaning-ranked MCP/capability lookup. Fetched on first use. Requires the ONNX build; the mock build is rejected. | `SearchCodebase` runs lexical-only and capability lookup ranks lexically. Nothing errors; intent-phrased queries just rank worse. |
| `webtools` | The `webfetch` and `websearch` tools — hoocode's only network path. Fetched on first use. | Both tools error when called. The web tool group is off by default, so this stays invisible until you enable it. |
| `voicetools` | Push-to-talk voice input in the TUI. Fetched on first use. | Voice capture reports an error and never starts. |

Resolution order for each is: `HOOCODE_<TOOL>_BINARY` (an explicit path, for a
local build) → hoocode's bin directory → `PATH` → download. Set `HOOCODE_OFFLINE=1`
to never download; set `HOOCODE_NATIVE_SEARCH=1` to force the JS paths even when
`rg`/`fd` are present.

## Working across repositories

`/cd <path>` moves the session to another directory without leaving the process, so provider auth, the warmed model list, and the terminal all survive the move. Everything scoped to the directory is rebuilt for the new root: tools, context files, project settings, extensions, skills, agents, and MCP servers. Sessions are stored per project, so a fresh session starts in the target; the one you left is still on disk and `/resume` in the old directory reopens it.

Bare `/cd` goes home, `/cd -` returns to where you came from, and the argument completes against real subdirectories. `Alt+W` opens it.

## Keys

Three rings, and a key's modifier says which ring it is in:

- **`Ctrl` — the view.** What is on screen right now: expand everything (`Ctrl+O`), thinking blocks (`Ctrl+T`), the task panel (`Ctrl+N`), model cycling (`Ctrl+P`).
- **`Alt` — the cockpit.** What the agent is and where it works: mode (`Alt+G`), model (`Alt+M`), directory (`Alt+W`), the view dial (`Alt+O`), unfold one block (`Alt+U`), settings (`Alt+S`), session tree (`Alt+T`), history (`Alt+H`), shortcuts (`Alt+K`).
- **Inside a picker**, every `Ctrl` key still belongs to the query you are typing — `Ctrl+A`, `Ctrl+U`, `Ctrl+W` edit text, as they do everywhere else. A picker's own verbs are all on `Alt`, and its hint line names them.

`Shift` reverses whatever the unshifted key does. Everything is rebindable in `~/.hoocode/keybindings.json`; `/hotkeys` lists the set currently in force.

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

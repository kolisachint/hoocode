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
| **Browser** (`browser_run` · `browser_continue`) | `--enable-browsertools` or `"enableBrowserTools": true` | Drive a real browser to load pages, interact, and capture results. |
| **Documents** (`DocRead` · `DocEdit` · `DocWrite` · `DocScan` · `DocGrep` · `DocPeek`) | `--enable-filetools` or `"enableFileTools": true` | Read, search, and edit structured documents — OOXML (docx/xlsx/pptx), PDF, XML, drawio. |
| **Plugins** (`SearchPlugins` · `InstallPlugin` · `ProposePlugin` · ...) | `--enable-plugintools` or `"enablePluginTools": true` | The autonomous plugin lifecycle system — discover, install, and propose plugins, plus a runtime reuse nudge. |

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

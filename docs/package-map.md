# Package Map

Where things live in this monorepo. Kept high-level on purpose; for exact APIs read the
source. For build/install mechanics see [npm-packages.md](./npm-packages.md). For UI
internals see [ui-map.md](./ui-map.md).

## Workspaces

npm workspaces rooted at `packages/*` (plus the example extensions under
`packages/coding-agent/examples/extensions/*`). All four library packages share one
version number (lockstep releases).

| Package | Dir | npm name | Responsibility |
| --- | --- | --- | --- |
| ai | `packages/ai` | `@kolisachint/hoocode-ai` | Unified LLM API: providers, streaming, model discovery, images, OAuth/env auth |
| agent | `packages/agent` | `@kolisachint/hoocode-agent-core` | Provider-agnostic agent loop, tool execution, state, transport abstraction |
| tui | `packages/tui` | `@kolisachint/hoocode-tui` | Terminal UI library: differential renderer, components, editor, keybindings |
| coding-agent | `packages/coding-agent` | `@kolisachint/hoocode-agent` | The `hoocode` CLI: tools, sessions, modes (interactive/print/rpc), config |

## Dependency graph

```
ai        (no internal deps)
tui       (no internal deps)
agent     -> ai
coding-agent -> agent, ai, tui
```

Build order (leaves first): `tui`, `ai`, `agent`, `coding-agent`. This is exactly what the
root `build` script does.

## packages/ai

Unified LLM layer. Key files:

- `src/index.ts` - public entry; re-exports types and the provider registry.
- `src/stream.ts`, `src/types.ts` - core streaming API and the `Api`/options type unions.
- `src/providers/` - one file per provider (`anthropic.ts`, `openai-completions.ts`,
  `openai-responses.ts`, `google.ts`, `amazon-bedrock.ts`, `mistral.ts`, `cloudflare.ts`,
  `faux.ts` for tests, etc.). `register-builtins.ts` lazily registers them.
- `src/models.generated.ts` - generated; never edit by hand. Update
  `scripts/generate-models.ts` instead.
- `src/env-api-keys.ts` - credential detection per provider.
- `src/oauth.ts`, `src/images.ts` / `images-api-registry.ts` - auth and image generation.

Adding a provider touches many files; the checklist lives in the root `AGENTS.md`.

## packages/agent

The reusable agent loop, independent of any specific UI or provider.

- `src/agent.ts` - the `Agent` class / orchestration and `AgentOptions`.
- `src/agent-loop.ts` - the turn loop, tool dispatch, and the background-tool mechanism
  (non-blocking tools whose results are injected later).
- `src/types.ts` - shared types (`AgentTool`, `BackgroundToolResult`, message types).
- `src/harness/` - the embeddable agent harness (used by external consumers such as
  hooteams) and the **canonical home of logic shared with the CLI**:
  - `compaction/` - context compaction and branch summarization (coding-agent imports
    these; there is no separate copy).
  - `messages.ts` - custom message types (`bashExecution`, `custom`, summaries) and
    `convertToLlm`; session entry types live in `harness/types.ts`.
  - `utils/output-compression.ts` - lossless tool-output compression (used by the CLI's
    bash/grep/read tools and compaction).
  - `session/`, `skills.ts`, `system-prompt.ts`, `prompt-templates.ts` - harness-side
    session storage and resources. These are `ExecutionEnv`-abstracted designs, distinct
    from coding-agent's fs-based `core/` equivalents (not diverged copies).
- `src/proxy.ts` - transport proxy.

## packages/tui

See [ui-map.md](./ui-map.md) for detail.

- `src/index.ts` - public exports.
- `src/tui.ts`, `src/terminal.ts` - the app/runtime and differential renderer.
- `src/editor-component.ts`, `src/components/` - reusable widgets.
- `src/keybindings.ts`, `src/keys.ts` - configurable key handling.

## packages/coding-agent

The shipped CLI. Largest package.

- `src/cli.ts`, `src/main.ts`, `bin/hoocode.js` - entry points.
- `src/cli/` - arg parsing, model/session pickers, file processing.
- `src/config.ts` - config + paths (`~/.hoocode/...`, dispatch dirs).
- `src/core/` - the engine:
  - `agent-session.ts`, `sdk.ts` - wiring the agent loop into a session.
  - `tools/` - built-in tools (`read`, `bash`, `edit`, `write`, `subagent.ts` = the `Task`
    tool, etc.). `tools/index.ts` holds the single `TOOL_FACTORIES` registry table that
    everything (name union, option lookups, bundles) derives from. Optional feature tools
    live in subdirectories: `tools/browser/` (browser_run/browser_continue) and
    `tools/doc/` (DocRead/DocEdit/…).
  - `subagent-pool.ts` - spawns subagents as child processes (concurrency, retries,
    inherited-model fallback).
  - `agent-registry.ts` - loads agent definitions; built-ins come from
    `init-templates.generated.ts` (embedded from `templates/agents/*.md`).
  - `task-store.ts` - in-memory task list shown in the task panel.
  - `export-html/`, `extensions/` - HTML export and the extension system. Context
    compaction, custom message types, and session entry types come from
    `@kolisachint/hoocode-agent-core` (see packages/agent above);
    `session-manager.ts` re-exports the entry types under their historical names.
- `src/modes/` - run modes: `interactive/` (the TUI app), `print-mode.ts`, `rpc/`.
  Voice input (panel + `voicetools` transcription) is grouped under
  `interactive/voice/`.
- `templates/agents/*.md` - built-in subagent definitions (frontmatter + prompt). Edit
  these, then regenerate the embedded copy (see npm-packages.md).

## Generated files (do not edit by hand)

- `packages/ai/src/models.generated.ts`, `image-models.generated.ts` - via
  `packages/ai/scripts/generate-models.ts`.
- `packages/coding-agent/src/init-templates.generated.ts` - via
  `packages/coding-agent/scripts/embed-templates.mjs`.

## Common "where is X" answers

- A built-in tool: `packages/coding-agent/src/core/tools/`.
- The `Task`/subagent tool: `core/tools/subagent.ts` + `core/subagent-pool.ts`.
- A provider: `packages/ai/src/providers/`.
- The agent turn/tool loop: `packages/agent/src/agent-loop.ts`.
- A TUI widget: `packages/tui/src/components/` (generic) or
  `packages/coding-agent/src/modes/interactive/components/` (app-specific).
- The task panel: `.../interactive/components/task-panel.ts`.

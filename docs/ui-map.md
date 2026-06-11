# UI Map

Where the terminal UI lives. Two layers: the generic **tui** library
(`packages/tui`) and the coding-agent **interactive mode** that consumes it
(`packages/coding-agent/src/modes/interactive`). Kept at file granularity, not line
granularity, to stay accurate as the UI evolves.

## Layer 1: tui library (`packages/tui/src`)

Generic, app-agnostic terminal UI toolkit with a differential renderer.

Core runtime:

- `tui.ts` - the app/runtime: holds the component tree, drives render/update cycles, routes
  input.
- `terminal.ts` - low-level terminal control and the differential renderer (only redraws
  changed cells).
- `index.ts` - public exports (including `visibleWidth`, truncation helpers in `utils.ts`).
- `utils.ts` - width/ANSI-aware string helpers (`visibleWidth`, `truncateToWidth`, ...).

Input / editing:

- `editor-component.ts` - the multiline text editor used for the prompt.
- `keybindings.ts`, `keys.ts` - configurable keybindings and key parsing. Never hardcode a
  key check; add defaults to the keybinding maps.
- `kill-ring.ts`, `undo-stack.ts` - editor kill-ring and undo history.
- `stdin-buffer.ts` - raw stdin handling.
- `autocomplete.ts`, `fuzzy.ts` - autocomplete and fuzzy matching.
- `terminal-image.ts` - inline image rendering support.

Reusable widgets (`src/components/`): `box`, `text`, `truncated-text`, `spacer`, `input`,
`editor`, `loader`, `cancellable-loader`, `markdown`, `select-list`, `settings-list`,
`image`.

A component generally exposes a `render(width)` method returning an array of styled lines;
the renderer diffs successive frames.

## Layer 2: coding-agent interactive mode

Path: `packages/coding-agent/src/modes/interactive`.

- `components/` - app-specific UI built on the tui library (grouped below).
- `theme/` - color theme and `theme.fg("...", text)` styling used across components;
  drives `theme-selector`.

### Chat transcript rows (`components/`)

Rendered in order as the conversation scrolls:

- `user-message.ts`, `assistant-message.ts` - the two primary message rows.
- `custom-message.ts` - custom/system-injected messages (e.g. background-task results).
- `bash-execution.ts`, `tool-execution.ts` - tool call + result rendering.
- `diff.ts` - unified diff rendering for edits.
- `skill-invocation-message.ts` - skill invocations.
- `branch-summary-message.ts`, `compaction-summary-message.ts` - summaries produced by
  branch summarization and context compaction.

### Status / chrome

- `task-panel.ts` - the task ledger shown above the prompt (status icons, usage
  stamps, and the warning cue). Owns `formatTaskLine`. Has three views cycled with
  `app.tasks.cycleView` (shift+ctrl+t): flat, subagents (grouped by owning agent),
  and teams (grouped by named role-agent with handoffs). Grouping is driven by
  `task.agent` + the `TaskAgent` roster in `core/task-store.ts`, which subagent
  dispatches populate and external orchestrators (hooteams) can feed.
- `footer.ts` - the bottom status/footer line.
- `keybinding-hints.ts` - the hint strip.
- `countdown-timer.ts`, `bordered-loader.ts`, `dynamic-border.ts` - timers, loaders, and
  animated borders.

### Inputs / editors

- `custom-editor.ts` - the prompt editor wrapper around the tui editor.
- `extension-input.ts`, `extension-editor.ts` - inputs for the extension system.

### Modal selectors / dialogs

Pickers presented over the main view:

- `ask-options.ts` - the options pane (`ask_options` tool); supports `recommended`.
- `model-selector.ts`, `scoped-models-selector.ts` - model pickers.
- `login-dialog.ts`, `oauth-selector.ts` - auth / `/login`.
- `session-selector.ts`, `session-selector-search.ts`, `user-message-selector.ts` -
  session and history navigation.
- `settings-selector.ts`, `config-selector.ts`, `theme-selector.ts`,
  `thinking-selector.ts`, `show-images-selector.ts` - settings and toggles.
- `extension-selector.ts`, `tree-selector.ts` - extensions and tree/file selection.

### Misc

- `visual-truncate.ts` - app-level truncation helper.
- `index.ts` - barrel exports for the components.
- `armin.ts`, `daxnuts.ts` - easter eggs.

## Common "where is X" answers

- The task pane / subagent list, status icons, warning cue: `components/task-panel.ts`.
- How a tool call is shown: `components/tool-execution.ts` (and `bash-execution.ts`,
  `diff.ts`).
- Colors / styling: `theme/` and `theme.fg(...)`.
- The prompt editor and keybindings: `tui/src/editor-component.ts`,
  `tui/src/keybindings.ts`.
- A generic widget (box, list, markdown): `tui/src/components/`.
- The render/diff loop: `tui/src/tui.ts` + `tui/src/terminal.ts`.
- Width/truncation math: `tui/src/utils.ts` (`visibleWidth`, `truncateToWidth`).

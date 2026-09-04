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
- `tool-signal.ts` - a single call's radar row (tool, subject, size), shown when a chain is opened.
- `tool-chain.ts`, `tool-chain-summary.ts` - the radar view's per-chain line: a run
  of consecutive tool calls as one line, working shape vs settled phrase.
- `diff.ts` - unified diff rendering for edits.
- `skill-invocation-message.ts` - skill invocations.
- `branch-summary-message.ts`, `compaction-summary-message.ts` - summaries produced by
  branch summarization and context compaction.

### Status / chrome

- `../team-focus.ts` (`TeamFocusController`) - the `--team` feature: role roster focus,
  the nudge input, the attach side panel, and approval gates. Extracted from
  `interactive-mode.ts` behind a narrow `TeamFocusDeps` interface.
- `../extension-dialogs.ts` (`ExtensionDialogs`) - the selector / options pane /
  confirm / input / editor / custom-component dialogs behind the ExtensionUIContext.
- `../extension-chrome.ts` (`ExtensionChrome`) - extension widget slots and custom
  footer/header overrides.
- `../bash-execution-controller.ts` (`BashExecutionController`) - the `!cmd` prompt mode:
  runs a bash command through the session (extensions can intercept via the `user_bash`
  event), renders a `BashExecutionComponent`, and streams output into it. Commands started
  while the agent is streaming park in the pending area and move into the transcript when
  the turn ends. Extracted from `interactive-mode.ts` behind a narrow
  `BashExecutionControllerDeps` interface. (Editor bash-mode toggling stays in
  `interactive-mode.ts`.)
- `../message-queue-controller.ts` (`MessageQueueController`) - message queueing: the
  compaction queue (messages typed while a compaction runs) and the pending-messages
  display above the editor. The session owns the live steering / follow-up queues; this
  controller merges them for display, restores everything to the editor on demand, and
  flushes the compaction queue once compaction finishes. Extracted from
  `interactive-mode.ts` behind a narrow `MessageQueueControllerDeps` interface.
- `../model-controller.ts` (`ModelController`) - model selection: the `/model` single
  picker, the `/models` scoped-models (enable set) picker, model cycling (the cycle
  keys), exact-match lookup for slash-command arguments, the footer's available-provider
  count, and the Anthropic subscription-auth warning. Extracted from `interactive-mode.ts`
  behind a narrow `ModelControllerDeps` interface. The billing warning fires once per
  session through `showNotice` (a `warningBg`-filled box, not the flat line `showWarning`
  paints) and is never awaited - resolving the auth type can hit the keychain.
- `../login-controller.ts` (`LoginController`) - the `/login` and `/logout` flows:
  provider auth-type selector, OAuth and API-key login dialogs, the Bedrock setup
  notice, and post-login default-model selection. Extracted from `interactive-mode.ts`
  behind a narrow `LoginControllerDeps` interface; `showOAuthSelector` is the entry point.
- `../voice/voice-controller.ts` (`VoiceController`) - voice-to-text capture
  (daemon + legacy paths) and the voice panel lifecycle.
- `../resource-display.ts` - the startup/reload resource listing and diagnostics
  formatting. Owns the counted capability grid (glyphs from `brand.ts`), the
  context row with its inline size note, and the collapsible details. The details
  block carries no expand hint of its own - the banner has the only one, and the
  same key opens both. Model and thinking level are the footer's job, not the
  splash's. Live MCP servers come from `core/mcp-status.ts`, which
  the hoo-core `mcp-loader` fills on connect. `../startup-checks.ts` -
  update/tmux/changelog startup probes.
- **One surface for startup, a session swap, and `/reload`.** All three rebuild the
  same things from disk, so all three repaint from the same list in
  `interactive-mode.ts`: `applyRuntimeSettings` (keybindings, footer, editor,
  cursor) before extensions bind, `applySessionTheme` (registered themes, the
  settings theme, the banner) after they bind, and `finishRuntimeSettings`
  (provider count, editor border colour, session identity) last. The transcript
  side is `resetTranscriptView` plus one `showLoadedResources` call per path -
  never two. Two hand-kept copies of that list is what let `/reload` keep a
  footer promising auto-compaction after the setting was turned off, and let the
  banner keep a retired theme's colours (a `Text` holds its string with the
  escapes already in it, so invalidating is not repainting). The theme is applied
  after binding on purpose: an extension's `resources_discover` handler can
  contribute the directory the theme name resolves in. Guarded by
  `test/suite/session-surface-sync.test.ts`, which drives every chain of `/new`
  and `/reload` through a real mode against a capturing terminal
  (`test/suite/interactive-surface-harness.ts`) and compares rendered frames.
- `task-panel.ts` - the task ledger shown above the prompt (status icons, usage
  stamps, and the warning cue). Owns `formatTaskLine`. Has three views cycled with
  `app.tasks.cycleView` (ctrl+n; the cycle and the header switcher skip lenses
  with no content): flat, subagents (grouped by owning agent),
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

## Vertical rhythm

Screen rows are the scarcest thing in the UI: everything the transcript and the
overlays spend is taken from the editor and from how much conversation stays
visible. One blank line is the separator between two blocks. Three rules keep
that from compounding:

- **A block's separator is the `Spacer(1)` before it, not its own padding.**
  A `Text`/`Markdown` added after a leading `Spacer(1)` takes `paddingY: 0` —
  the pair `Spacer(1)` + `paddingY: 1` renders *two* blank rows above the block
  and one below. `components/tool-execution.ts` is the reference case.
- **A blank line next to a rule is a blank line wasted.** `DynamicBorder`
  already separates; title text sits flush under the top rule and the last row
  sits flush above the bottom one. (`components/login-dialog.ts` is the
  reference case.)
- **Nothing pads its own bottom edge.** The footer, the editor and a closing
  rule are their own bands, so the last child of a selector or a transcript
  block never ends with a `Spacer(1)`.

The exception is a `Box` with a background: its `paddingY` rows are *painted*
band, not empty space, and they are what makes a user message or a warning read
as a sheet (`components/user-message.ts`, `showBlock` in `interactive-mode.ts`).

## Common "where is X" answers

- The task pane / subagent list, status icons, warning cue: `components/task-panel.ts`.
- How a tool call is shown: `components/tool-execution.ts` (and `bash-execution.ts`,
  `diff.ts`). How much of it is shown: the view dial in `core/tool-output-view.ts`
  (radar / glance / full); radar groups calls into `components/tool-chain.ts`.
- Colors / styling: `theme/` and `theme.fg(...)`.
- The prompt editor and keybindings: `tui/src/editor-component.ts`,
  `tui/src/keybindings.ts`. The app's own bindings and the three-ring layout they
  follow (`ctrl` = view, `alt` = cockpit, pickers never take a `ctrl+<letter>`):
  `coding-agent/src/core/keybindings.ts`, guarded by
  `coding-agent/test/keybinding-layout.test.ts`.
- A generic widget (box, list, markdown): `tui/src/components/`.
- The render/diff loop: `tui/src/tui.ts` + `tui/src/terminal.ts`.
- Width/truncation math: `tui/src/utils.ts` (`visibleWidth`, `truncateToWidth`).

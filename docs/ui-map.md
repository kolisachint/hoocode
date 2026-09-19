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
- `mouse.ts` - mouse reporting: the enable/disable sequences, and reading SGR
  (`?1006`) and X10 reports back. Only the wheel is acted on; clicks are
  swallowed so a report can never be typed into a field.
- `stdin-buffer.ts` - raw stdin handling.
- `autocomplete.ts`, `fuzzy.ts` - autocomplete and fuzzy matching.
- `terminal-image.ts` - inline image rendering support.

Reusable widgets (`src/components/`): `box`, `frame`, `text`, `truncated-text`, `spacer`,
`input`, `editor`, `loader`, `cancellable-loader`, `markdown`, `select-list`,
`settings-list`, `image`.

- `components/spacer.ts` - `Spacer` is a fixed run of blank rows. `FlexSpacer`
  is the one whose height the *renderer* sets, and it is what makes the app
  full-screen; see "Filling the screen" below.

- `components/frame.ts` - the border, and the only thing that draws one.
  `renderFrameEdge` renders one horizontal edge: the rule, the `↑ N more` scroll
  indicator that eats into it, the label that rides what is left, and the
  corners in box mode. `Frame` is a `Container` that wraps its children in that
  border at a chosen style (`box`, `rule`, `none`), insets them by a gutter, and
  pads every row out to the width it was handed; it also answers `labelFits`, so
  an owner can put a label somewhere else rather than have it dropped. The
  scroll indicator is `renderFrameEdge`'s, used by the editor - a `Frame` pane
  that scrolls prints its own `(3/12)` row, as the pickers already do.
  `editor.ts` draws its own border through the same function, and
  `EditorBorderStyle` / `EditorBorderChars` / `EditorTopBorderLabel` are now
  aliases of the frame's types.

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
- `notification-panel.ts` - the transient band directly above the prompt: dial
  steps, settings glimpses and warnings, each gone in a few seconds. See
  "What ghosts and what stays" below.
- `keybinding-hints.ts` - the hint strip.
- `countdown-timer.ts`, `bordered-loader.ts`, `dynamic-border.ts` - timers, loaders, and
  animated borders.

### Inputs / editors

- `custom-editor.ts` - the prompt editor wrapper around the tui editor.
- `extension-input.ts`, `extension-editor.ts` - inputs for the extension system.
- `input-frame.ts` (`InputFrame`) - **the chrome every surface that asks the user
  for something wears.** See "One frame for every user input" below.

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

## One frame for every user input

Every surface that asks the user for something replaces the prompt editor in
`editorContainer`: the pickers (`/model`, `/models`, `/settings`, `/theme`,
`/thinking`, `/sessions`, `/tree`, `/color`, `/login`, the fork-from-message
list), the `ask_options` pane, the extension selector / input / editor, and the
login dialog. They all draw `InputFrame`, which is the *prompt's own* frame:

- **One border renderer.** `InputFrame` extends the tui `Frame`, which draws its
  edges with `renderFrameEdge` - the same function `Editor` draws its border
  with. There is no second way to draw a border around an input.
- **One border style.** `InputFrame` reads the `editorBorder` setting (`box` or
  `rule`) on every frame, so `/settings` moves the prompt and everything that
  stands in for it together, including a pane that is already open.
  `interactive-mode.ts` pushes it with `setInputFrameBorder`, next to
  `editor.setBorder`, in `applyRuntimeSettings` and in the settings callback.
- **One place the name goes:** into the top border, flush right - the slot the
  session chip rides on the prompt. `setTitle`, which collapses the name to one
  line (a newline in a rendered row splits the border open and throws the
  renderer's row count out with it - `ExtensionDialogs.confirm` passes its
  question and its detail newline-joined). A name with no room for a run of
  border beside it drops to the frame's first row instead of being dropped
  altogether: `renderFrameEdge` discarding a label is right for the session chip
  and wrong for a question the user is about to answer yes or no to.
- **One place the hints go:** the last row inside, flush above the bottom edge.
  `setHint`, which keeps that row last however the pane was built.
- **One gutter.** The frame insets content one column, so a picker's rows line
  up with the prompt's `❯` instead of starting at column 0. A child that
  overruns is cut rather than allowed to wrap the frame.
- **The border colour is the plain `border` token**, not the prompt's: the
  prompt's border carries the thinking level and bash mode, and a picker has
  neither to report.

A pane embedded in a surface that already frames it draws no frame of its own -
`AskOptionsComponent` takes `{ framed: false }` for the `--team` attach panel,
and a nested `Editor` takes `border: "none"`. Two boxes one column apart is a
picture frame. The *host* then owns the rules it is banded off by:
`team-attach-panel.ts` draws an accent rule above and below the gate and closes
itself. It used to lean on the gate's own bottom rule to close the panel, which
is the kind of thing that breaks silently when the inner surface changes shape -
`test/team-attach-panel.test.ts` holds it now.

`DynamicBorder` is still the right thing for a *divider* inside a pane
(`tree-selector.ts` uses one between its key hints and the tree) and for a rule
around a transcript block (`bordered-loader.ts`, `/reload`, `/hotkeys`). It is
not the thing to frame an input with.

A frame with nothing in it draws nothing, so a pane that has not yet loaded its
content needs a row to hold it open (`login-dialog.ts` draws "Starting…" until
the flow speaks).

Guarded by `coding-agent/test/input-surface-frame.test.ts`, which builds every
input surface and asserts the corners, the gutter, the title's place, the
`rule`-mode fallback, and that every row is exactly as wide as the terminal -
at every width from 160 columns down to 2. `tui/test/frame.test.ts` holds the
frame's own geometry and that `renderFrameEdge` still produces the editor's
border byte for byte.

## Two arrows, and what each one means

There are exactly two arrow glyphs on screen, and they are told apart by role,
not by location:

- **`❯` is a caret: a line is waiting for you.** The prompt editor
  (`interactive-mode.ts`, `Editor.promptPrefix`) and *every* other place you can
  type - a picker's query line, the login dialog, an extension's prompt, a
  session rename, the `ask_options` custom-answer row. `DEFAULT_INPUT_PROMPT`
  in `tui/src/components/input.ts` is the glyph; `Input` appends the trailing
  space itself, the same contract `Editor` has.
- **`›` is a cursor: this is the row you are on.** `SELECT_CURSOR` in
  `theme/theme.ts`, every picker's selected row, and the library's unthemed
  `DEFAULT_SELECT_CURSOR` fallback.

There was a third for a long time. `Input` hardcoded `"> "` - unstyleable, so it
also sat in the default foreground next to a themed cursor - which meant a
picker drew an ASCII caret two rows under an accent `›` for the role the prompt
spells `❯`. Nobody chose that glyph; it was the one `Input` shipped with,
because the widget never got the prefix knob `Editor` has had all along. Three
glyphs for two signals.

Rules that follow:

- **A new input surface sets no prefix.** `Input` defaults to the caret. What it
  *does* set is the colour: `styleInput(input)` in `theme/theme.ts`, which paints
  the caret `muted` so it never out-shouts the accent `›` a row or two above.
  A widget that owns its own `Input` wires `promptColor` to its own theme
  instead (`SettingsList` uses its `hint`).
- **`→` is not in this family.** It means "maps to" - the `ask_options`
  breadcrumb's question → answer - and is left alone.

Guarded by `coding-agent/test/input-surface-frame.test.ts` ("every place you can
type wears the prompt's caret"), which renders every input surface and asserts
no ASCII caret survives anywhere, and `tui/test/input.test.ts`, which holds the
default, the colouring, and that a coloured prefix is still measured by its
visible width.

## Scrolling the transcript

The transcript is not scrolled by the terminal. It used to be, and that was the
bug: the renderer keeps the whole session in one line buffer on the normal
screen, and any change to a row *above* the viewport drops it to
`\x1b[2J\x1b[H\x1b[3J` + a full reprint — which throws away the scrollback the
reader was sitting in. Output arriving, a pane closing, `alt+o`, a resize: all of
them took you back to the bottom for no visible reason.

So the view is the app's. `TUI.scrollOffset` is the transcript row drawn at the
top of the screen, and `null` means "follow the tail", which is the live path
everything else in `tui.ts` is written for. The moment it is a number the TUI
takes the **alternate screen** and paints a window of the buffer itself: a fixed
grid, addressed row by row, no scrollback for anything to fight over. Three rules:

- **A pinned view does not move.** New output lands in the buffer and the window
  stays where it was put; only the indicator's total changes. This is the whole
  feature and `tui/test/scroll-viewport.test.ts` asserts it frame for frame.
- **Live mode is untouched.** Normal rendering, the normal screen and the
  terminal's own scrollback, selection and search all work exactly as before.
  `?1049l` restores them byte for byte, and the exit is an ordinary differential
  frame against a snapshot taken on the way in — never a clear-and-replay.
- **Reaching the bottom releases the pin.** A pinned view of the tail looks
  exactly like a live one and silently stops following, which is the confusion
  this exists to remove.

Search lives on the same window. `TUI.setScrollSearch` measures matching rows
once per query (re-measured only when the buffer grows), the paint highlights
them across the screenful actually on show, and the indicator becomes the query
line. It runs backwards from where you are, like `ctrl+r` in a shell, because
what you are looking for in a session is behind you. Jumping by turn reads the
same render memos rather than re-rendering: a message's offset inside the chat
container plus that container's offset at the root is its absolute row
(`Container.childRowOffsets`).

**A picture is drawn in the window, not named.** A kitty or iTerm image is
placed by the cursor, so it can only be drawn where the cursor can go, and
`Image` renders an n-row picture as n-1 blank lines and one line that moves back
up and draws. Two things follow. (1) The window draws the image when *all* of it
fits — the drawing line's row has to be at least `imageRowOffset` from the top,
because a terminal asked to draw above row 1 clamps and paints over rows that
are not the picture's. A block hanging off the top is named `[image]` until the
whole of it is on screen. (2) The window transmits its **own copy** under its
own kitty id. It has to delete last frame's placement before painting this one —
`CSI 2 K` clears text and a placement is not text — and deleting a kitty image
deletes *every* placement of it, so deleting the live id would wipe the picture
from the normal screen too, which the differential frame on the way back out has
no reason to repaint. `scrollImageIds` maps live id to window id and
`releaseScrollImages` frees the copies on the way off the alternate screen.
`tui/test/scroll-images.test.ts` holds both.

The app's half is `interactive/scroll-view.ts`: the key scopes (the prompt keys,
the fuller set once pinned) and the themed indicator. Anything that is
not a scroll key un-pins and then does its usual job, so the mode is left by
doing something rather than by remembering to escape first. Keys and the reason
each sits where it does: `core/keybindings.ts` -> "Scrolling the transcript".

**Clicking a link is the app's too, and for the same reason.** Capturing the
mouse for the wheel (`tui/src/mouse.ts`, `?1000h` + `?1006h`) takes clicks away
from the terminal, so every OSC 8 hyperlink in the transcript quietly stopped
being clickable the day the wheel started working. The TUI now answers the
click itself: `hyperlinkAt` walks the buffer row under the pointer counting
*display cells*, and the URL it finds goes to `TUI.onHyperlink` — which the
interactive mode points at `utils/open-url.ts`. Press and release must land on
the same cell, or it is a drag and someone is selecting text. Opening a URL is
spawning a process, so the TUI resolves which link and stops there; the opener
is no-shell and scheme-allowlisted, because a transcript is full of text a model
wrote. `tui/test/hyperlink-click.test.ts` holds it.

## Filling the screen

The app is the height of the terminal, always: banner on the first row, prompt
and footer on the last, the conversation directly above them, and whatever
room is left over between the banner and the conversation. It did
not used to be. This renderer *appends* — a frame is the component tree
flattened into a line buffer and written from wherever the cursor is — so a
frame was exactly as tall as its content. On a fresh session that meant the
banner a few rows down with the prompt under it and the user's shell history
above, and the prompt then walking down the screen over the next few turns until
the session was finally long enough to scroll. Two layouts for one app, and the
one you meet first is the one that does not look like an app.

`FlexSpacer` is the fix and it sits directly below the banner: header, then
fill, then everything else.
`TUI.setFlexSpacer` nominates it; `doRender` measures the frame it just built,
hands the leftover rows to it, and flattens once more. Three things follow:

- **Nothing below the fill ever has room held back from it.** In
  `interactive-mode.ts` that is everything below the banner: the transcript, the
  queued messages, the status rows, the widget containers, the task ledger, the
  notification band, the prompt and the footer. They are one run against the
  floor, which is what keeps the last thing the agent said touching the box you
  answer it in. The fill sat between the transcript and the chrome once, and a
  short session — a fresh one, or a long one folded by the `radar` view dial —
  showed as a page of blank between the two.
- **A picker is bottom-anchored for free.** Every input surface replaces the
  prompt inside `editorContainer` (see "One frame for every user input"), so the
  fill shrinks to make room for it and grows back when it closes.
- **This is still the normal screen.** No alternate screen, so the terminal's
  own scrollback, selection and search keep working, and the session is still
  there after you quit. Only the pinned view takes the alternate screen, and it
  empties the fill first — blank rows in the buffer would be rows of transcript
  the reader has to scroll past, and `transcriptLength` excludes the fill for
  the same reason.

Past a screenful the fill is 0 and the *terminal's* scroll is what puts the
prompt on the bottom row, which leaves one frame the append-only path cannot
draw: the one where the buffer gets **shorter**. A picker closing, a
notification fading, the ledger emptying, a view dial folding a run of tool
calls — the window over the buffer has to move *back*, showing rows the reader
scrolled past, and nothing can scroll a terminal's own content down to bring
them in. Appending clears the rows that came off the end and strands the prompt
mid-screen with blanks under it.

So `doRender` repaints the visible window in place: one screenful, absolutely
addressed, which the app may do precisely because a buffer taller than the
screen owns every row of it. It costs a screen of writes and no clear, where the
honest alternative is `\x1b[3J` and the whole transcript. The version before it
banked the rows the chrome gave up into the fill — which held the floor, but
paid for it with a band of blank between the conversation and the prompt, up to
a screenful of it after a fold. A shadow of a layout is not better than the
layout.

Guarded by `tui/test/screen-fill.test.ts` (the mechanism, including the fold)
and `coding-agent/test/screen-anchor.test.ts` (the real mode, real chrome,
banner on the first row and prompt on the floor at every session length).

## What ghosts and what stays

Moving a dial used to write a line into the transcript: `Model: opus-5`,
`Chrome: compact`, `Tool output: peek`. Each is true for about a second and
litter forever after — a session where someone found their thinking level by
stepping through it carried five rows of dead settings chatter, interleaved with
the conversation the transcript is supposed to be a record of. Warnings had the
same problem from the other end: a filled block, kept for the life of the
session, for something ("No previous directory to return to") worth one glance.

So there are two destinations now, and one rule for choosing:

> **If missing it costs you nothing, it goes on the band. If missing it costs
> you the information, it goes in the transcript.**

The band is `components/notification-panel.ts`, directly above the prompt.
Through it: every dial step (`showDialStep` — all six dials, now that saying
where one landed no longer costs a permanent row), the settings glimpses
(`InteractiveMode.notify` — model, session name, and the like), every
`showWarning`, and `showStatus` — which is *everything a command has to say for
itself*, extensions' `ctx.ui.notify(…, "info")` included, listings and all. The
band takes a third of the screen for a body and earns reading time by the row
(`BODY_ROW_MS`), so a listing arrives whole and stays long enough to be read.

Still in the transcript: errors, `showNotice` (the ones the user pays for if
they miss them), and `showRecord` — the handful of statuses carrying a value the
screen cannot answer for a minute later: a share URL, an export or import path,
where credentials were saved. `showRecord` is the old `showStatus` body,
coalescing row and all; `showStatus` now goes to the band. The test for which
one to call is not importance, it is whether the screen can still answer the
question once the band clears.

The band is **filled**: the block fill a message sheet takes (`warningBg` for a
warning, `customMessageBg` for the rest), painted to the full width. A line of
text above the prompt is one more line of text on a screen already full of them;
a filled band has edges, and the eye finds it without anything being drawn
around it.

One more rule, inside the band: **a glimpse replaces, a warning queues.** A
glimpse reports state, so only the newest one is true and holding a dial key
down should flash the value it ended on, not five stale ones in sequence. A
warning reports an event, so every one is still true when the next arrives, and
collapsing them would let the last of three startup warnings erase the two
before it. Neither overwrites the notification already on screen: a message that
can be erased before it is read is not one.

The exception is a glimpse that names a **topic**, which is how every dial comes
through `showDialStep` (the keybinding id with its direction taken off, so
forward and back are one knob). Two glimpses of one topic are two readings of
one thing, so the later one replaces the earlier wherever it is sitting, the
head included, and restarts the clock under it. Without that, the second press
of `alt+z` had nowhere to go but the back of a queue of one and the band went on
showing the stop you had already left for its full three seconds — a dial that
lags a press behind is a dial you cannot step twice. Guarded by
`coding-agent/test/notification-panel.test.ts`.

### Tips are the band's one uninvited guest

Everything above arrives because the user did something. A tip
(`interactive/tips.ts`, scheduled by `interactive/tips-controller.ts`) does not,
which is why it obeys one extra rule on top of all of the above:

> **A tip is posted only when the band is empty, and a refused tip is dropped,
> never retried.**

Both halves matter. Posting into an occupied band would let a tip queue behind —
and therefore delay — something the user caused, and the whole justification for
a tip is that it costs nothing. Retrying a refused one would be worse: it turns
"the band is busy" into a tip that lands the instant the user's own notification
fades, which is the interruption the first half exists to prevent. The next
moment arms its own timer; there is always another one.

Tips carry `topic: "tip"`, so a second tip replaces a first rather than stacking,
and take a longer TTL than a glimpse — a glimpse confirms something the user just
did and only has to be recognised, while a tip says something new and has to be
read. They stay inside `MAX_BODY_ROWS`: the band drops what does not fit, so a
four-row tip is a tip with an invisible last line, and `tips.test.ts` asserts the
content never grows past it.

The scheduler is otherwise all reasons to stay quiet: a startup grace period
(the banner and changelog are still being read), a cooldown between tips, and the
`tips.enabled` setting, read fresh on every offer so switching it off silences
the tip already scheduled. Every timer is `unref`'d — a pending tip must never be
why the process is still alive.

## Screen columns

Rows are scarce (see below) and so are columns: a margin held back "for safety"
is a column of every row, forever. Two rules:

- **A widget fills the width it was handed.** Truncate to it, pad to it, and
  spend nothing on a margin the layout does not need. `SelectList` and
  `SettingsList` each held two columns back with no reason recorded (one of them
  commented `-2 for safety`); that was two columns off every picker row and
  every settings row. `tui/test/list-right-margin.test.ts` holds the recovery.
- **A sheet's gutter is one column.** `PAPER_INSET` is 1: the shadow's column is
  `▏`, a *left one-eighth* block, so it paints the sheet's edge as a hairline
  at the left of that one cell and leaves the rest as page. A wider gutter shows
  nothing a reader can use. `coding-agent/test/message-block-fill.test.ts` ("a
  sheet's reach") asserts the sheet runs to the terminal's last cell.
- **A shadow is an eighth of a cell, not half of one.** The bottom run is `▔`
  and the column is `▏` (`tui/src/components/box.ts`). `▀` and `▌` are half a
  cell of solid ink, which at a terminal's resolution is not a shadow but a
  second band of colour wrapped around two sides of every message — heavy
  enough to pull the eye off the text it sits behind.
- **Nothing draws the corner; the two legs already meet.** `▔` fills its cell
  edge to edge, so the bottom run ends at exactly the boundary the column paints
  its hairline on, and the gutter cell on the run's row stays empty. A glyph
  there can only overshoot — `▔` by seven eighths of a cell to the right, `▏`
  (full cell height, against the run's top eighth) by a whole row downwards, a
  tick hanging under the sheet. `tui/test/paper-sheet.test.ts` holds both sides.

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
- Why the prompt is on the bottom row: `tui/src/components/spacer.ts`
  (`FlexSpacer`) and `TUI.setFlexSpacer` — see "Filling the screen".
- Where a "Model: …" or a warning goes now:
  `components/notification-panel.ts` — see "What ghosts and what stays".
- How a tool call is shown: `components/tool-execution.ts` (and `bash-execution.ts`,
  `diff.ts`). How much of it is shown: the view dial in `core/tool-output-view.ts`
  (radar / peek / full); radar groups calls into `components/tool-chain.ts`.
- Colors / styling: `theme/` and `theme.fg(...)`.
- Copying a conversation out of the session: `utils/markdown-to-html.ts` and
  `utils/rich-clipboard.ts`, driven by `CommandExecutor.handleCopy` (`/copy`,
  `/copy all`, `/copy <turns>`). What is on screen is markdown *rendered* —
  box-drawing tables, bordered code panels, wrapping frozen at the window's
  width — so a copy goes back to the source (`sessionToMarkdown`) and offers it
  as text and as HTML at once. macOS and Windows carry both flavours; a Linux
  clipboard advertises one type, so it gets the markdown.
- The prompt editor and keybindings: `tui/src/editor-component.ts`,
  `tui/src/keybindings.ts`. The app's own bindings and the three-ring layout they
  follow (`ctrl` = view, `alt` = cockpit, pickers never take a `ctrl+<letter>`):
  `coding-agent/src/core/keybindings.ts`, guarded by
  `coding-agent/test/keybinding-layout.test.ts`.
- A generic widget (box, frame, list, markdown): `tui/src/components/`.
- The border around anything: `tui/src/components/frame.ts`. The chrome around a
  surface that takes user input: `interactive/components/input-frame.ts`.
- The render/diff loop: `tui/src/tui.ts` + `tui/src/terminal.ts`.
- Width/truncation math: `tui/src/utils.ts` (`visibleWidth`, `truncateToWidth`).

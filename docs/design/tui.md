# TUI

Status (2026-11-11): built and consolidated. The transcript has one view: one row per tool call, opened in place (built 2026-11-07). Full-screen is the default. Themes and keybindings are built. The task panel has tabs and its own hints. Colour rules have a machine-readable file (`crates/hoocode-code-tui-theme/color-semantics.json`). Screen anatomy, part by part, is in [docs/maps/ui.md](../maps/ui.md). Open work is in the Open table below and in [roadmap.md](roadmap.md). The session handover is [docs/backlog.md](../backlog.md). This page holds the rules as built.

## Principles

| Rule | What it means |
|---|---|
| Plain words | Say what happened, in plain English. No space metaphor in help, tips, slash descriptions, settings, notices or status text. `scripts/ci/plain_words.sh` bans these in TUI string literals: SHIP, FUEL, CR, LANDED, HIT, M1, CLEARANCE, TRANSMISSION, HANGAR, DOCKING, MISSION, SYSTEMS, RESOURCES, "flight path", "end shift", "satellite", "jettison", "comms", "hull", "mission control". |
| One fact, one place | Status line: session, model, context %, cost, live state or last turn result. Transcript: per-call facts (`+2 −2`, `exit 0 · 8.2s`). Prompt: input only. |
| Status line is the only status surface | No HUD, logo, toast, comms frame, session chip, footer rows or footer dial. |
| Everything is a block | Every item in the transcript and every dialog is a block with one grammar (below). |
| Blocks are separated by one blank row | One blank row between blocks. None inside a block. A tool row with a body is followed by one blank row. Consecutive body-less tool rows stay tight. |
| Separation by colour, not borders | Panes and the prompt use background steps and text weight. The only borders are in monochrome (see Accessibility). |
| Glyph set from code | `crates/hoocode-tui-glyphs/src/lib.rs` (`status`): `●` done or call, `◉` working, `○` todo or queued, `⊗` failed, stopped or denied, `›` marker. Nothing else. No spinner: running is still and only the elapsed time moves. The one spinner is the compaction loader. `◐` is not used (missing from JetBrainsMono Nerd Font Mono). |
| Shape carries state | Each state has its own shape, so state reads under `NO_COLOR`. Colour is added on top. Never colour alone. |
| The blocking thing is the only yellow | A permission or question title is yellow bold, the safe default is selected, and nothing else on screen is yellow. |
| Block grammar | Mark, one space, bold name, target, two spaces, dim facts. Body at column 4, dim. Fold line `    … N more lines`. |
| Hint grammar | `key action · key action`. Keys in normal ink, actions dim. Keys lowercase (`esc`, `ctrl+o`, `f1 more`). At most 4 items, then `f1 more`. Hints appear only where the key acts. Never a sentence (`esc back`, not "Escape goes back"). Hints come from the keymap, never typed literally. |
| Width | Labels are cut first, at a word boundary, with `…`. Every mark is one cell wide. Tool rows follow their own cut order (see Transcript: single view), so their secondary facts drop before the target. Other rows keep their stats. |

## Layout

### Rows, top to bottom (full-screen)

1. Start header (two lines, start screen) or one blank row.
2. Attach header, only while attached to a subagent run.
3. Body: the transcript. Live follows the tail. Pinned holds still. A short live transcript is bottom-anchored.
4. Task panel: a pane above the prompt, rows only. See Task panel.
5. Notice rows (max 3). When the task panel is drawn, they join its pane.
6. Completion menu (slash, `@file`, argument choices), above the prompt.
7. Transient rows: leader hint, warnings, retry notice, extension status, startup progress.
8. Queued messages: dim `○` rows directly above the prompt.
9. Prompt row: ` › ` on the surface colour, no frame, grows with input. A dialog takes this row.
10. Status line: the last row.

Start screen: ` hoocode v<ver>` (bold) and ` <cwd> · N skills · M agents` (dim). Nothing else. Idle prompt placeholder is empty.

Full-screen is the default (`fullscreen` setting, default on). `HOOCODE_FULLSCREEN=0` gives the inline screen for one run (see Open UI-14).

### Turn result and status line

| State | Left slot of status line | Right side (always, dim) |
|---|---|---|
| Working | `◉ working · 4s · esc stop` | `amber-harbor · claude-haiku-5-5 · 42% context · $1.23` |
| Idle after success | `● done · 11s · 23k tokens` | same; `plan` first in plan mode |
| Idle after failure | `⊗ failed · 5.0s · 14k tokens` (`⊗` red) | same |
| Done with a failed call | `● done · 8.5s · 7.1k tokens · 1 failed call` | same |
| Not signed in | `⊗ not signed in  /login` (outranks the last turn, yields to working) | session and model only |
| Permission or question open | empty | same |
| Pinned, no block focused | `scrolled 43–61 of 78 · ↓ 3 new  ctrl+x u focus · esc live` | same |
| Pinned, block focused | `enter open · y copy · { } move · esc clear` | same |
| Search open | `search  query  1 match` | same |

At 100 columns the pinned hints drop cost, then the session name, from the right side to make room.

Narrow terminal: the right side drops, in order, cost, session name, the word `context` (keep `42%`), model effort. The left side keeps its text and is cut last.

Failed and stopped turns also end the transcript with a line: ` ⊗ failed · 5.0s` (`⊗` red, `failed` dim) or dim ` ⊗ stopped · 2.0s`. A successful turn writes nothing to the transcript.

## Transcript: single view

Built 2026-11-07 (UI-21 and UI-22, phases P0–P6 on branch `feat/u21-u22`). Decisions are dated rows in the Decisions table. The mockup `docs/assets/mockups/tui-single-view.html` is historical. The goldens are the source.

### Interaction model

- One transcript mode. Each tool call is one row, collapsed. A click on the header, or Enter on a focused row, shows the full output in place. A second click or Enter hides it. The status line shows no mode name.
- Each block has an `expanded` flag. It starts at its default (below), lives in memory and is not saved, so a resumed session opens collapsed. It survives resize, theme change and rewrap.
- Click and Enter read and write the same flag (`set_block_expanded`, `crates/hoocode-code-tui-app/src/mode/transcript/focus.rs`).
- Focusing a block does not expand it. `{` and `}` change no expansion state.
- Live: a click pins the view first, with the header on the same screen row, then toggles. `↓ N new` counts rows that arrive below.
- `ctrl+x o` (`app.scroll.toggleAll`) opens every tool row, or closes them all when any is open. It writes the same `expanded` flag. It reaches every block that can open: tool rows (frozen ones too, which keep their open body), thinking, summaries, `!` blocks, the skill block, the background notice and the startup listing. A block that cannot open is left alone. It is silent.

#### Keys

The rule: at the prompt a bare printable key always types. Transcript letters act only while a transcript block is focused. The `›` gutter shows on the focused tool row.

- Pinned view, no block focused: every typed character goes back to live and types. This includes `j k q [ ] { } / y`. Non-printable keys keep working: arrows (one line over an empty prompt), PgUp and PgDn, Home and End (top, and live), Esc (back to live), `ctrl+x /` (search).
- `ctrl+x u` focuses the newest block and pins the view.
- With a block focused and the prompt empty:

| Key | Action |
|---|---|
| `{` `}` | Focus the previous or next block |
| `enter` | Toggle the focused block |
| `y` | Copy the raw output (never the rendered rows) |
| `j` `k` | One line down or up |
| `[` `]` | Previous or next user message |
| `/` | Search the transcript |
| `q` | Clear the focus and go live |
| `esc` | Clear the focus only. Never stops a turn |
| End | Clear the focus and go live |
| Home | Top |

- Any other typed character clears the focus, goes live and types.
- While a transcript search is open it keeps its own keys: `enter` next, `n` and `p`, `esc` close.
- Mouse: a press and release on the same header row toggles it, anywhere on the line up to the scrollbar column. A press on one row and a release on another does nothing. Body rows do nothing, except the fold line (`… N more lines`), which opens the block. A link opens. The scrollbar column pages. Capture stays `?1000h ?1006h`.
- The mouse never takes the keyboard from the prompt. With no block focused, a click creates no focus. With a block focused, a click also moves the focus to the clicked block.

### Default expansion

| Starts collapsed | Starts expanded |
|---|---|
| Tool calls, thinking, compaction and branch summaries, `!` shell blocks, the startup listing | Skill blocks, background-run notices, custom messages |

### Row format

`<gutter><mark> <Tool> <target>  <facts>`, on one line, never wrapped.

- Column 0 is the gutter: a space, or `›` in accent when focused. Column 1 is the mark. The tool name is at column 3.
- The tool name is bold ink. The target is ink. Facts are dim, joined with ` · `. Red only on the failed mark and `exit N`. Green on the done mark and `+n`.
- Focus: the `paneSel` fill across the row, plus `›`. Monochrome: `›` and reverse video.
- Open or closed: a dim `▸` (closed) or `▾` (open) after the header's last fact, one space after it. No extra row, and the mark and the name keep columns 1 and 3. A row that cannot open (no output, TodoWrite, stopped) shows none. See the 2026-11-12 decision.

| State | Mark | Colour (dark / light) | Facts |
|---|---|---|---|
| Running | `◉` | accent `#5cc8bb` / `#00625c` | Elapsed only |
| Done | `●` | ok `#b5bd68` / `#0a6b27` | Primary, then secondary |
| Failed | `⊗` | err `#d58181` / `#b8111e` | `exit N` (red), reason (dim), secondary |
| Stopped by the user | `⊗` | dim | `stopped` |

### Per-tool rows

| Tool | Collapsed row | Primary facts | Expanded body |
|---|---|---|---|
| Read | `● Read <path>  1451 lines` | Lines (or `lines 40–80`) | Numbered lines, head first |
| Edit | `● Edit <path>  +2 −2` | `+n −n` | The diff, head first |
| Write | `● Write <path>  +48` | `+n` | The content, head first |
| Shell | `● Shell <command>  exit 0 · 40 lines · 8.2s` | `exit N` | Output, tail first |
| CodeSearch | `● CodeSearch "<query>"  14 hits · 3 files` | Hits | The listing |
| WebFetch, WebSearch | `● WebFetch <url>  ~8.1k tokens · 12 sections` | `~N tokens` | The result text |
| Agent | `● Agent explore · <desc>  3 calls · 1 failed · 12s` | Calls, failed (if any) | One line per child call |
| MCP, plugin, unrendered | `● mcp__docs__search "<args>"  12 lines` | Lines | The output, head first |
| `!` user shell | `● ! git status  exit 0` | `exit N` | Output, tail first |
| TodoWrite | `● TodoWrite  5 items · 2 done` | Counts | Not a toggle (the plan is in the task panel) |
| Background job launch | `● Shell sleep 60  background job-1` | `background <id>` | Output so far |

- A collapsed row that hides output carries the hidden size as a fact (`1451 lines`, `~8.1k tokens`). That is how "there is more" reads on the row.
- Children of an Agent row are plain lines at indent 3, not buttons. A failed child shows `⊗` and its reason dim. Nested expansion is Later.

### Cut order (tool rows)

Each step runs only while the row is still too wide.

1. Drop secondary facts whole, right to left. Shell facts are ordered `exit N · size · duration`, so the duration goes first.
2. Shorten the target to its minimum. A path drops leading directories and keeps the whole file name. A URL keeps the host. A command or query cuts at a word, at 12 columns or more.
3. Failed rows: cut the reason at a word, keeping at least 12 columns.
4. Failed rows: if 12 columns do not fit, drop the reason. The target takes the space back. `⊗` and `exit N` still say what happened.
5. Primary facts never drop. At 40 columns a very long file name is cut in the middle and keeps its extension (`tool_ch…ain.rs`).

Width ladder, the same rows at each width:

```
Read            40 ┆ ● Read …/src/tool_chain.rs  1451 lines
                80 ┆ ● Read crates/hoocode-code-tui-widgets/src/tool_chain.rs  1451 lines
Shell failed    40 ┆ ⊗ Shell cargo test -p…  exit 1
                60 ┆ ⊗ Shell cargo test -p…  exit 1 · error: could not compile…
                80 ┆ ⊗ Shell cargo test -p…  exit 1 · error: could not compile `hoocode-parser`
Shell done      80 ┆ ● Shell cargo test -p hoocode-config 2>&1 | tail -40  exit 0 · 40 lines · 8.2s
Compaction      40 ┆ Compacted turns 1–3 · saved 9,300
```

Compaction's label is `Compacted turns 1–3`, its primary fact is `saved N`, and the token range is secondary.

### Running, failed and stopped

- A failed call settles collapsed and never opens itself.
- A running Shell, collapsed, shows one dim tail line at column 4. It is gone on settle.
- A stopped call is dim `⊗` with `stopped`, and has no body.

### Blocks that toggle besides tool calls

- Thinking (collapsed: one dim italic line, the first sentence then `…`; expanded: the whole trace), compaction and other summaries, skill block, background-run notice, `!` shell blocks and startup details. `ctrl+x b` still hides all thinking.
- Not toggles: user messages, assistant prose, TodoWrite, the turn-end line and notices.
- Built 2026-11-12: every block above goes through `hoocode_tui_render::FoldHost` (`Component::fold_host`): its header rows, its open flag, its fold line and its raw text. A click, `enter`, `y`, `ctrl+x o` and the `ctrl+x u` / `{` / `}` walk read one list (`chat_folds` in `scroll_view.rs`). Hidden thinking (`ctrl+x b`, or the outline view) is no block and the walk skips it. The startup listing gets a dim `Resources` header row, its click target, since it drew nothing while closed.

### Frozen rows

- Blocks older than the newest 50 are frozen (`LIVE_TOOL_WINDOW`). A frozen block keeps an unstyled row snapshot: mark, target, primary and secondary facts, failure (`capture_frozen`, `crates/hoocode-code-tui-widgets/src/tool_chain.rs`). A done row keeps its green `●`. A failed row keeps its red `⊗`, `exit N` and failure state. Its body is released.
- A click or `enter` on a block that cannot open (a frozen row with no output, TodoWrite) says so in the status slot (`Shell: no output to show.`) and changes nothing.
- Known limitation: a frozen block skips `invalidate`. After a theme change its rows keep the old palette until they scroll out.

### As built

Accepted verification deviations from the mockup (2026-11-07):

- M1: label `+2`, as built.
- M2: body indent 8 (the mockup has 6). 8 is pinned as built.
- M3: tok/turn kept in dense rows.
- M4: mdHeading yellow is out of scope.
- M5: source is one cell wider.
- M6: stale copy fixed.

## Task panel

- **Tabs.** Plan (TodoWrite items, each linked run nested under its item), Agents (subagent runs, background runs too), Schedule (scheduled jobs), Shell (background shell jobs), MCP (servers and calls). A tab shows only while it has rows.
- **Shown or hidden.** Shown by default. `ctrl+x g` shows or hides it. With no rows it draws nothing, and `ctrl+x g` says `No tasks to show.`
- **Cycling.** From the prompt, `ctrl+x l` next tab and `ctrl+x L` previous. With the panel focused, Left and Right change tab.
- **Focus.** Up on the prompt's first line takes the keyboard, when the panel is shown with rows and the transcript is not pinned. Esc gives it back. Its keys are in Keybindings, Where a key acts.
- **Strip and hints.** When more than one tab has rows, an unfocused strip sits at the top of the panel. It shows the tabs (`plan 1/3 · agents 1/1 · mcp 1`, the shown tab in normal text) and ends with `ctrl+x l next tab · ctrl+x g hide`. The hints come from the keymap. When the width is short, the hints drop before the tab labels.
- **Notices join the pane.** When the panel is drawn, tips and longer notices under it paint on the `pane` background at indent 3, with no blank row. They read as the panel's last rows. With no panel they render as before: a blank row, indent 1, no fill. Monochrome: no fill.
- **Colours.** Pane background. Running `◉` accent, never yellow. Done `●` dim. Failed `⊗` red.
- **Tips.** One tip teaches the tabs ("Tasks have tabs").
- **Code.** Locations are in [docs/maps/ui.md](../maps/ui.md#task-panel). This page does not repeat them.

## Panes and dialogs

- A dialog takes the prompt's row: permission, question, quit, models, settings, sessions, theme, tree, fork, log in, scoped models and keys. The task panel and the completion menus keep the prompt row.
- A pane has no border and no fill step beyond its colour. Anatomy: a title chip (accent teal, dark text, bold; yellow for a blocking pane), then dim `/command` and facts on the same row. Rows indent 3. One column of inner padding. No blank padding rows.
- Selected row: `paneSel` fill, accent `›`, normal text (not bold). Only the glyph is coloured. Secondary cells dim. In monochrome the selected row keeps ` › ` and bold, and reverse video is allowed.
- Filterable panes get an input row at the bottom (`filter` placeholder, dim).
- Hint row: last. At most 4 items. `enter` and `esc` never drop. Yellow only for blocking.
- Detail: a split pane shows the detail in a fixed column at 100 columns or more (`SPLIT_MIN_WIDTH`), with a one-column gutter in pane colour. Below 100 columns the detail stacks under the list, at most 3 rows, then `…` (`STACKED_DETAIL_ROWS`). Some panes hide the stacked detail instead.
- Clicks: a click selects a row, and `enter` opens it (Decision 2026-11-07). A second click does not pick.
- Models, sessions, settings, theme (live preview), tree and keys split this way. Below 100 columns they stack.
- Plain titles: Models, Settings, Sessions, Theme, Tree, Fork, Log in, Scoped models, Keys.
- Each row shows one fact that compares rows (models: price; scoped models: `on` or `off`; sessions: age). Full details only on the selected row.
- Completion menus (slash, `@file`, argument) are pmenu style: a pane-colour block directly above the prompt, as wide as its longest row, left edge under the typed token, selected row in selection colour with ` › ` and bold.

### Blocking dialogs

| Dialog | Title and rows | Keys and hint |
|---|---|---|
| Permission | yellow chip `Allow Shell?`. Row ` $ command` (`$` dim). Row `access: pipes 2 programs` only when it writes, uses the network or runs more than one program. Rows `1 yes`, `2 yes, always for <program> this session`, `3 no, tell it why`. | `y` yes and `n` no answer at once while the reply is empty. `a` highlights "always" and Enter confirms it. `e` edits the command first. `1`–`3` pick. Letters type a reply. Hint: `1–3 pick · e edit first · or type a reply`. |
| Question | yellow chip with the question text. Numbered options, each with a dim description. | Digits answer. Letters type into the custom answer when the row allows it. `esc` skips, and the agent states the default it uses. Hint: `1–3 pick · tab next question · esc skip`. |
| Quit with jobs | `Quit? 2 background jobs running`. Job rows; running jobs `◉` accent. | Hint: `k stop jobs and quit · l leave them running · esc stay`. |
| Log in (first run) | Opens by itself when no provider works. | Esc closes it and the status line shows `⊗ not signed in  /login`. Submitting while signed out keeps the text and reopens Log in. |

Bare letters act only when no text field has text (see Keybindings).

## Notices

| Kind | Where | Timeout |
|---|---|---|
| One-row info | Status line left slot | 3 s |
| One-row warning | Status line left slot | 8 s |
| Longer, or more than one row | Dim rows above the prompt, indent 1, no fill, max 3. The third row ends in `…`. A warning's first row is ink bold. When the task panel is drawn, the rows join its pane (see Task panel). | 3 s or 8 s |
| Needs an action (for example not signed in) | Status line left slot, or rows above the prompt | Never. It stays until dismissed. |

- No box, no toast, no title bar, no glyph for info.
- Newest notice shows. The queue waits.
- While a turn runs, `◉ working` keeps the status slot, and notices use the rows above the prompt.
- A key hint after the text sits after two spaces: key in normal ink, action dim.

## Themes

### Built-in themes

Five bundled files in `crates/hoocode-code-tui-theme/themes/`: `dark`, `light`, `dusk`, `high-contrast`, `paper`. `auto` picks dark or light from the terminal background. The picker lists `auto`, the built-ins, then user themes with their file path. The contrast test (`theme_contrast.rs`) covers all five.

Setting `theme` takes a name, `auto`, or `auto:<light>:<dark>`. Choosing one in the picker saves it to the global settings. An unknown saved value falls back to `auto`.

### Palette (dark and light)

| Role | dark | light |
|---|---|---|
| Base (transcript, status line) | `#14141c` | `#f8f8fb` |
| Pane (dialogs, pickers, completion, task panel) | `#1e1e2a` | `#e8e8f0` |
| Detail column (split panes) | `#292b40` | `#d3d7e9` |
| Pane selection | `#233239` | `#c3dcdc` |
| Surface (prompt, user message, pane input row) | `#2a2a36` | `#dadbe6` |
| Text | `#e5e5e7` | `#2b2f36` |
| Dim | `#989898` | `#555864` |
| Accent (`›`, running `◉`, inline code, accent chip) | `#5cc8bb` | `#00625c` |
| Green (done) | `#b5bd68` | `#0a6b27` |
| Red (failed) | `#d58181` | `#b8111e` |
| Yellow (blocking only) | `#f0c674` | `#7a5200` (dark amber, for text) |
| Accent chip (bg / text) | `#5cc8bb` / `#14141c` bold | `#00625c` / `#f8f8fb` bold |
| Blocking chip (bg / text) | `#f0c674` / `#14141c` bold | `#f0c674` / `#2b2f36` bold |

In light, panes are darker than the page. Dark panes are lighter than the base. Yellow is for blocking only. Some non-blocking places still use this token. They are listed under UI-29.

Contrast: every text colour on every surface is at least 4.5:1 (WCAG). The tightest pairs, checked on 2026-11-07, are dim and red on the dark selection (4.59:1), and green on the light selection (4.64:1). Do not lighten dim or darken `paneSel`. Light has one grey, so facts are told apart by position: two spaces after the target, and the body at column 4.

### Colour semantics

The machine-readable source is `crates/hoocode-code-tui-theme/color-semantics.json`. It holds the rules, the roles, the states, the surfaces and the meaning of every token. `crates/hoocode-code-tui-theme/tests/it/color_semantics.rs` keeps it in sync with the schema. The table is the human summary.

| Role | Tokens | Meaning |
|---|---|---|
| Accent | `accent`, `chip`, `chipFg`, `acc` | `›` marker, running `◉`, inline code, title chip, selected-row marker |
| Done (green) | `success`, `ok` | Done `●`, `+n` on Edit and Write rows |
| Failed (red) | `error`, `err` | Failed `⊗`, `exit N`, `−n`. Nothing else. The reason is dim |
| Blocking (yellow) | `warning`, `chipWarn`, `warn` | Title and chip of a permission or question. The only yellow |
| Ink | `text`, `ink`, `toolTitle` | Names, targets, keys in hints, row names |
| Dim | `dim`, `muted`, `faint`, `toolOutput`, `thinkingText`, `halftone` | Facts, hint action words, block bodies, fold and thinking lines |
| Backgrounds | `base`, `pane`, `paneSel`, `paneInput`, `paneDetail`, `surface` | Transcript and status line (`base`); dialogs, menus and task panel (`pane`); selected row (`paneSel`); filter row (`paneInput`); split detail (`paneDetail`); prompt and user message (`surface`) |

Rule ids in the JSON: `yellow-blocking-only`, `red-failure-only`, `green-done-only`, `accent-marker-running`, `never-colour-alone`, `separation-by-background`, `background-sets-foreground`, `dim-for-facts`, `no-new-colours`. Widget code uses a theme token, never a literal colour.

### Token model

- JSON only. Top-level keys: `name`, `description`, `$schema`, `vars`, `colors`, `export`, `inherits`.
- 51 required colour tokens (`REQUIRED_COLOR_TOKENS`, `schema.rs`). 86 declared properties. Pane tokens: `surface`, `pane`, `paneSel`, `paneInput`, `paneDetail`, `chip`, `chipWarn`, `chipFg`.
- Colour values: hex, a `vars` reference, `""` (terminal default), or a 256-colour index. Every painted background also sets its foreground.
- `"inherits": "<name>"`: missing tokens come from the base. A cycle or an unknown base is an error.
- Built-in names always win. A user file adds new names only.
- User themes: `~/.hoocode/themes/<name>.json`, one file each. `~/.hoocode/themes/defaults/` holds reference copies of the built-ins and `theme-schema.json`, rewritten on each version change. The loader ignores that folder.

```json
{"name": "my-dark", "inherits": "dark", "colors": {"accent": "#7aa2f7"}}
```

- Errors: a bad file or an unknown name gives a warning in theme diagnostics. The last good theme stays. At startup, a bad theme falls back to the built-in `dark`. Never crash, never switch silently.
- Hot reload: file watch polled every 50 ms, 100 ms debounce (`registry.rs`). `auto` re-checks the background every 60 s (`AUTO_THEME_POLL_INTERVAL`). No `notify` crate.
- Test `T-5`: every token literal in code must exist in the schema.
- Removed: `daylight`, `deep-space`, `colorsafe-*`, the aliases `warm-*` and `solarized-*`, and the `theme_paths` setting.

## Keybindings

### Principles

| ID | Rule |
|---|---|
| K-P1 | No Alt or Option key in any default. Alt keys a user writes still work (the ESC-prefix parser stays). |
| K-P2 | One leader: Ctrl+X by default, set as `"leader"`. Most actions sit behind it. |
| K-P3 | Ctrl only where universal and needed: interrupt or clear (Ctrl+C), exit (Ctrl+D), suspend (Ctrl+Z), paste image (Ctrl+V), prompt history search (Ctrl+R), readline line editing, and the leader. |
| K-P4 | Lists, pickers and panes use bare letters when no text field has text. At the prompt a bare printable key always types. Transcript letters act only while a transcript block is focused (see Where a key acts). `/` searches, `?` or `f1` shows keys, `q` or Esc closes a list (not a dialog), `j` and `k` or arrows move, Enter accepts. |
| K-P5 | Every key hint on screen comes from the keymap. A rebound key shows its new name. |

### Direct keys (prompt)

| Key | Action |
|---|---|
| Enter | Submit. While a turn runs, Enter steers the agent. |
| Shift+Enter | Newline. Leader Enter is the fallback. |
| Esc | Interrupt the running turn, or close a popup. Esc Esc clears a non-empty prompt (kept in history). |
| Up, Down | History. Up on an empty prompt with queued messages takes the last one back. Up on the first line goes to the task panel when it can take it. |
| Shift+Tab | Cycle agent mode forward. |
| Tab | Completion. |
| PgUp, PgDn | Page the transcript (pins the view). |
| Ctrl+C | Clear the prompt. When empty, interrupt. Double press exits. Reserved. |
| Ctrl+D | Exit when the prompt is empty. Reserved. |
| Ctrl+Z | Suspend. Reserved. |
| Ctrl+V | Paste image (Windows: leader `v`). |
| Ctrl+R | Prompt history search (reverse, incremental). Enter puts the match in the prompt. |
| Ctrl+A/E/K/U/W/Y, Ctrl+Left/Right, Ctrl+Delete | Readline line editing. |
| Ctrl+X | Leader. |

### Leader (Ctrl+X, then)

| Key | Action | Key | Action |
|---|---|---|---|
| `e` | External editor | `g` | Show or hide the task panel |
| `r` | Voice input | `l` / `L` | Task panel tab next / previous |
| `c` | Copy the last reply | `n` | Attach to the subagent run |
| `f` | Queue a follow-up (sent after the turn) | `h` | Resume a session |
| `d` | Take back the last queued message | `w` | Change directory |
| `a` | Mode backward (Shift+Tab goes forward) | `s` | Settings |
| `m` / `M` | Model next / previous | `?` or `k` | Show all keys (`/hotkeys`) |
| `t` / `T` | Thinking level next / previous | `/` | Search the transcript |
| `b` | Show or hide thinking blocks | `u` | Focus the newest tool call and pin the view |
| `y` | Yank pop (the editor's kill ring) | `z` | Redo (the editor) |
| `v` | Paste image (Windows only) | Enter | Newline (works in every terminal) |
| `o` | Open or close all tool rows | | |
| Esc or Ctrl+X | Cancel the leader | | |

Leader rules: `ctrl+x` enters pending. The next key runs its action. Esc, a second Ctrl+X, an unbound key (`no binding`, shown 1.5 s) or the timeout cancels. The timeout is 1500 ms (`leaderTimeoutMs`, `DEFAULT_LEADER_TIMEOUT`). After 400 ms (`WHICH_KEY_DELAY`) a which-key popup lists the leader keys, built from the keymap. The pending key is swallowed and never reaches the editor. The leader works only at the prompt: a picker, pane or task panel above it takes the key.

### Pinned view, pickers and dialogs

| Context | Keys |
|---|---|
| Pinned view, no block focused | Every typed character goes live and types, `j k q [ ] { } / y` included. Arrows (one line over an empty prompt), PgUp PgDn (page), Home (top), End (live), Esc (live), `ctrl+x /` (search). |
| Block focused (pinned, prompt empty) | The keys in Transcript: single view, Keys. Any other typed character clears the focus, goes live and types. |
| Transcript search open | `enter` next, `n` / `p` next or previous, `esc` close. |
| Session picker | `p` path, `o` sort, `n` named filter, `r` rename, `x` delete (asks `y` or `n` first) |
| Model picker | `s` save, `a` enable all, `x` clear all, `g` provider, `c` category |
| Tree | `h` `l` fold, `e` edit label, `t` timestamps, `1`–`5` filters, `c` filter cycle |
| Clearance card | `y` yes, `n` no (both at once while the reply is empty), `a` highlight always (Enter confirms), `e` edit, `1`–`3` aliases |
| Yes/No lists (import, missing cwd, extension and permission gates) | `y` Yes, `n` No, `a` Always when offered; Enter default; Esc deny |
| MCP trust | `y` trust and start, `n` not now |
| MCP elicitation | `y` accept or answer, `n` decline, Esc cancel |
| Ask-user card | No `y` or `n` (letters are text). Enter confirm, Esc skip, digits answer. |
| Quit with jobs | `k` stop jobs and quit, `l` leave them running, Esc stay |
| Task panel (focused) | Up and Down move the pick. Left and Right, or Tab and Shift+Tab, change tab. Enter opens. `x` twice cancels. Esc gives the keyboard back. `ctrl+x g` hides the panel and gives the keyboard back. A typed character that is not one of these keys (`x` is one) returns the keyboard to the prompt and types there. |

Reserved, cannot be rebound: ctrl+c, ctrl+d, ctrl+z, ctrl+m, ctrl+i, ctrl+[, ctrl+h. Warned, can be rebound: ctrl+s, ctrl+q, ctrl+b, ctrl+space.

### Where a key acts

| Context | Who owns bare keys |
|---|---|
| Prompt | The prompt. Bare keys type. Only ctrl keys, the leader, Esc, Enter, arrows and paging act. |
| Block focused (pinned, prompt empty) | The transcript: `{ } enter y j k [ ] / q`. Any other typed character clears the focus and types. |
| Picker or pane, empty filter | The list: `j` `k` move, `/` opens typing, `q` or Esc closes. |
| Blocking dialog, empty reply | The dialog: `y` `n` `a` and digits. Once the reply has text, letters type into it. |
| Task panel focused | The panel: arrows, Left and Right, Tab, Enter, `x`, `ctrl+x g`, Esc. Other typed characters go to the prompt. |

### Config: `~/.hoocode/keybindings.json`

Both forms load.

Flat (kept): `{"app.model.cycleForward": "ctrl+x m"}`. The value is a string or an array.

Grouped (added):

```json
{
  "leader": "ctrl+x",
  "leaderTimeoutMs": 1500,
  "bindings": [
    {"context": "Prompt", "bindings": {"ctrl+x m": "app.model.cycleForward", "ctrl+r": null}}
  ]
}
```

- Contexts: Global, Prompt, Leader, Scroll, Picker, Panel, Dialog, Tree, Models.
- Key syntax: `"ctrl+x m"` (a space is a sequence), `"shift+enter"`. `null` or `"none"` unbinds.
- A grouped binding applies only where its action runs. Elsewhere it is skipped with a warning.
- A leader is one key. A sequence, Esc, Enter, Tab or Backspace is refused, and a printable key too, so `ctrl+x` is used.
- Validation at startup and on `/reload`: parse errors, unknown ids, duplicates in one context, reserved keys. Results show in `/hotkeys`. Never crash.
- Alt keys in a user file keep working. One notice per version lists them. Never rewrite the user's file.
- Written by hoocode: `~/.hoocode/keybindings.defaults.json` (read-only reference, refreshed on a version change), and `keybindings-state.json` (the one-per-version notice).

Commands: `hoocode keybindings --defaults` prints the defaults. `hoocode keybindings check` validates the user file. `hoocode keybindings --read` prints the bytes the terminal sends (like `fish_key_reader`).

### Terminal notes

- Shift+Enter needs the kitty keyboard protocol or modifyOtherKeys. tmux needs `extended-keys` on.
- macOS Terminal.app has neither. There use leader Enter.
- Check that VS Code's integrated terminal does not take Ctrl+X (UI-12).
- Ctrl+J stays Enter in legacy terminals. It is not a newline key (see Not doing).

### Built deviations from the plan

- The leader works only at the prompt.
- `q` closes lists, not dialogs. Dialog answers are `y`, `n`, `a` and digits.
- Picker help is `f1` first. In typing mode, `?` types into the filter.
- Some hint keys are fixed text, not keymap ids: `k` and `l` (quit card).
- Scroll indicators use the keymap names: `pageUp/pageDown`.

## Accessibility

| Setting | Effect |
|---|---|
| `NO_COLOR` (any non-empty value) | Colour off. Shapes, bold and dim stay. Glyphs stay. |
| `FORCE_COLOR` (any value except `0` or `false`) | Colour on, even off a tty. Wins over `NO_COLOR` when both are set (`hoocode-code-cli/src/lib.rs`). |
| `TERM=dumb`, no tty, or fewer than 256 colours | Monochrome, same as `NO_COLOR`. |

Monochrome rules:
- Panes get full light borders (`┌─┐│└┘`, dim where dim exists) with the title in the top border.
- The selected row keeps ` › ` and bold. Reverse video is allowed.
- Title chips render as bold text.
- Split panes get a `│` rule. No `├─┤` rule above the filter row.
- Completion menus get light borders. No colour.

Other rules: every state has a shape. Every key is reachable from the keyboard. Hint keys are lowercase. Plain-text glyph stand-ins exist (`status::ascii`, `hoocode-tui-glyphs`) for fonts or terminals without the glyphs.

## Rendering and performance invariants

| Invariant | Where |
|---|---|
| Full-screen on the alternate screen, with app-owned history. Exit leaves the alternate screen and prints nothing. | `Tui::stop`, `tui-render/src/tui.rs` |
| One frame string per paint, sent through `Terminal::write_frame` on `hoocode-term-out`. The full-screen mode is the custom `Tui`, not a ratatui `Terminal`. | `hoocode-tui-render`, `hoocode-tui-terminal` |
| ratatui is used only through `hoocode-tui-ratatui`. Never the `ratatui` facade (crossterm version clash). | `docs/maps/packages.md` |
| Body = screen minus fixed top and bottom rows. Top chrome comes before the body, bottom chrome after (`Tui::set_fullscreen_body`). No reserved-but-unpainted rows. The task panel's rows come out of the body. | `tui-render/src/fullscreen.rs` |
| A live short transcript is bottom-anchored. A pinned view is held at the top. The scrollbar column is always reserved, and blank while live. | `tui.rs` |
| Mouse: wheel scrolls. A click on a tool row header toggles it. Shift+drag selects (terminal behaviour). | `tui-terminal/src/mouse.rs`, `mode/transcript/focus.rs` |
| SIGTERM and SIGHUP restore the terminal, then exit 128 plus the signal. A UI-thread panic restores once (panic hook), flushes the session with a bound, logs the backtrace and chains the previous hook. Other-thread panics do not touch the terminal. | `tui-terminal/src/interrupt.rs`, `crash_restore.rs` |
| Render-cache contract: state that a `render_stamp` reads changes only through a `&mut` method that calls `fresh_stamp()`, or its writer calls `note_change()`. Debug builds re-render every cache hit and assert equality. `HOOCODE_RENDER_CACHE_CHECK=0` turns the check off. | `tui-render/src/component.rs` |
| Streaming: deltas throttled to 100 ms. Wakes coalesced. A provider partial is an `Arc` with copy-on-write. The loop never sends a partial to a listener that may keep it. | `code-subagents/src/live.rs`, `ai-types` |
| Markdown: a look-around prefilter for long text (fancy-regex is quadratic on 57 KB). Test `lexer_prefilter`. | `tui-util/src/js_regex.rs` |
| Stdin: test every split point of a UTF-8 or escape sequence. A lone ESC flushes after resubscribe (wake pipe). | `tui-terminal/src/stdin_hub.rs` |
| Measure before optimising. Perf changes need a number. | `runtime.md` |

### Attach (subagent view)

- Attach is a read-only swap of the transcript, not an overlay. The prompt is disabled while attached. Esc returns with the scroll offset kept.
- The header row sits above the chat: state and `esc back`.
- Storage: the child transcript is `<session>/subagents/<task_id>.jsonl` with a child header `parent_session`. The parent gets a `custom` entry `subagent_run` (append-only, last wins). Success keeps the transcript.
- Recovery: view and resume, no auto-resume. An interrupted run shows as interrupted unless its pid is alive.

## Goldens and tests

| Kind | Where | When | Command |
|---|---|---|---|
| Component goldens | Per crate, `tests/golden/<crate>/` (for example `hoocode-code-tui-app/layers_*`, `hoocode-tui-components` markdown), written through `crates/hoocode-tui-render/src/golden.rs` | Every change, in nextest | `cargo nextest run -p <crate>`; accept with `UPDATE_GOLDENS=1` |
| Screen goldens | `scripts/tui/goldens.py`, 10 scenarios in `scripts/tui/scenarios/`, stored in `tests/golden/tui/<scenario>/`. Runs the real binary in tmux against the mock LLM. Reports in `target/tui-goldens/`. | Pre-release only, not per change | `python3 scripts/tui/goldens.py check all`; accept with `update <scenario>` |
| Key layout | `crates/hoocode-code-tui-keybindings/tests/it/` (`keybinding_layout.rs`, `tests/fixtures/keybindings-gold.json`) | Every key change | nextest |
| Colour semantics | `crates/hoocode-code-tui-theme/tests/it/color_semantics.rs` (checks `color-semantics.json` against the schema) | Every colour change | nextest |
| Key lint | `scripts/ci/no_literal_keys.py` (CI `guards` job) | Every change | runs in CI |
| Plain words | `scripts/ci/plain_words.sh` (CI `guards` job) | Every change | runs in CI |

Dialogs and notices are pinned by component goldens in dark, light and monochrome at 100 columns (UI-6, 2026-11-11): `tests/golden/hoocode-code-tui-selectors/`, `hoocode-code-tui-app/` and `hoocode-code-tui-widgets/`. The goldens pin the screen as built. Where it differs from this page, the list is in [docs/backlog.md](../backlog.md) (UI-34).

## How TUI work is done

- TUI work is done by Haiku subagents (user, 2026-10-09). The main session orchestrates, briefs and merges. Each subagent reviews, implements and tests one step.
- One step per commit. The commit updates `docs/maps/ui.md` and `docs/maps/packages.md` when a crate, screen or command changes. It updates `docs/user/` when a flag, command, key or path changes.
- Done bar: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo nextest run -p <crates>`. Errors use the hand-written enums.

## Decisions

| Date | Decision |
|---|---|
| 2026-10-09 | TUI work is done by Haiku subagents. The main session orchestrates, briefs and merges. |
| 2026-10-17 | Full-screen on the alternate screen is the default, with app-owned history. Exit prints nothing. |
| 2026-11-03 | Plain words. No space metaphor in TUI text. |
| 2026-11-03 | The status line under the prompt is the only status surface. No HUD, logo, toast, comms frame or session chip. |
| 2026-11-03 | Separation by background and text colour, not borders. Monochrome gets full light borders. |
| 2026-11-03 | Glyphs: `●` `⊗` `◉` `○` `›`, nothing else. Working is `◉`, not `◐`. |
| 2026-11-03 | Dialogs take the prompt's row. The task panel and completion menus keep it. |
| 2026-11-03 | Panes use their own colours. Split list and detail at 100 columns or more. |
| 2026-11-03 | Notices: one row in the status line's left slot, or up to 3 dim rows above the prompt. Action-required notices never time out. |
| 2026-11-03 | Question: `esc` skips, and the agent states the default it uses. |
| 2026-11-03 | Light theme: panes darker than the page, teal-tinted selection, dark amber for yellow text. |
| 2026-11-04 | Views are `outline`, `peek` (default) and `full`. `radar` parses as `outline`. (superseded 2026-11-07 by the single view) |
| 2026-11-04 | Removed: HUD, comms frame, wordmark, starfield, session chip, `/color`, `/chrome`, `alt+z`, toast float, footer rows. |
| 2026-11-04 | No step numbers, rails, `Took` or success end line. Failed and stopped turns end with a line. |
| 2026-11-04 | No thinking duration (`thought 4s` is not built). Thinking is one dim line or hidden. (superseded 2026-11-07: a thinking block also expands) |
| 2026-11-04 | Scroll and search bars sit in the status line's left slot. |
| 2026-11-05 | Five built-in themes: `dark`, `light`, `dusk`, `high-contrast`, `paper`. Supersedes the two-theme rule of 2026-11-03. |
| 2026-11-05 | `auto` follows the background detected from `COLORFGBG` or the macOS setting, and re-checks every 60 s. |
| 2026-11-05 | Clearance card: `y` and `n` answer at once while the reply is empty. `a` highlights always, Enter confirms. `1`–`3` stay as aliases. Supersedes "1/2/3 only" of 2026-11-03. |
| 2026-11-05 | Leader is Ctrl+X. Timeout 1500 ms. Which-key after 400 ms. The leader works only at the prompt. |
| 2026-11-05 | No Alt key in any default. Alt keys in a user file still work, with one notice per version. |
| 2026-11-05 | Bare letters for lists, pickers and panes, active only with no text focus or an empty query. (Refined 2026-11-11 for the transcript: see the bare-key rule.) |
| 2026-11-05 | Enter steers a running turn. Leader `f` queues a follow-up. Leader `d` takes back a queued message. |
| 2026-11-05 | Prompt history is Ctrl+R. Transcript search is leader `/`. Thinking blocks toggle with leader `b`. |
| 2026-11-05 | Ctrl+J is not changed. Newline is Shift+Enter, with leader Enter as the fallback. |
| 2026-11-05 | The theme picker saves `theme` to global settings. Keys change only by editing `keybindings.json`. No in-app key editor. |
| 2026-11-05 | `theme_paths` is removed. An unknown saved theme falls back to `auto`. |
| 2026-11-07 | Single view (UI-21): one transcript mode, one row per tool call, click or `Enter` opens in place. The dial (`outline` / `peek` / `full`, `toolOutputView`, `radar` alias) goes in P5. |
| 2026-11-07 | Tool-row width rule: cut order — secondary facts drop first, the target never drops. Overturns "stats are never dropped". |
| 2026-11-07 | Failed row reason colour: dim; red only on `⊗` and `exit N`. |
| 2026-11-07 | Running Shell while collapsed: one dim tail line, gone on settle. |
| 2026-11-07 | No expand-all in v1: `ctrl+x o` is dropped (`ctrl+x u`, `{` `}`, `Enter`, `G live` cover it). Revisit on user ask. (`G live` was later removed; see 2026-11-08.) |
| 2026-11-08 | Transcript top/bottom move off bare `g`/`G` onto Home/End, so no typed letter ever teleports the view (`g`/`G` type as usual now; rebind them in `keybindings.json` for the vim keys). Supersedes `G live` in the 2026-11-07 expand-all row. |
| 2026-11-07 | A click while live pins the view; the header keeps its row. |
| 2026-11-07 | Settings Tool output row is removed (caps live in `settings.json` only). `DocSearch` is removed with it: discovery via the prompt listing, `Read`, CLI help and user pages (AG-19 closed as No). |
| 2026-11-07 | Pane clicks v1: click selects, `Enter` opens (e.g. attach a subagent run). No second-click pick. |
| 2026-11-07 | Yellow is blocking only (fixes the palette contradiction with principle 17). |
| 2026-11-07 | Single view (UI-21) and pane grammar (UI-22) are done: P0–P6 landed on `feat/u21-u22`. |
| 2026-11-07 | Accepted verification deviations M1–M6 as built: M1 label +2; M2 body indent pinned at 8; M3 tok/turn kept in dense rows; M4 mdHeading yellow out of scope; M5 source +1 cell; M6 stale copy fixed. |
| 2026-11-07 | Frozen-row theme lag is a known limitation: frozen blocks skip `invalidate`, so they keep the old palette until they scroll out. |
| 2026-11-07 | Focus fill and click-pin ride the block `expanded` flag (`mode/transcript/focus.rs`). |
| 2026-11-08 | Subagent-response nudge look (UI-1 closed): the loop header names the run's resume label (`Background agent explore#4 (id …) finished:`, old tool-and-id headers still parse); notice heads use the glyph set with dim words — green `●` on finish, red `⊗` on failure, all dim on cancel, `(partial)` dim; the attach header takes `›` with lowercase `esc back`; a skill block is its bold name only. |
| 2026-11-08 | OSC 7501 program-status emission: the mode loop reports working/idle/blocked (+kind permission/question/auth)/done/error and clears on quit to any terminal, alongside the unchanged Herdr CLI path. Root record only (no subagent child ids in v1); off on `TERM=dumb`. |
| 2026-11-08 | Pulse scope (UI-4 closed): the tree summarizer and the working indicator draw no frames; the compaction loader keeps the pulse as the only spinner, noted in Principles. |
| 2026-11-08 | Docs drift sweep (UI-9 closed): `docs/maps/ui.md` is Snapshot 2026-11-07 with the new attach header; no stale binding claims remain in `docs/user/` or `docs/maps/` (the one `ctrl+g` is a leader-config example). |
| 2026-11-11 | Bare-key rule (refines K-P4): at the prompt a bare printable key always types. Transcript letters act only while a transcript block is focused (the `›` gutter shows on a tool row). With no block focused, a typed character in the pinned view goes live and types. |
| 2026-11-11 | A click never takes the keyboard from the prompt. Click and Enter share one `expanded` flag (`set_block_expanded`). A click with a block already focused also moves the focus to that block. |
| 2026-11-11 | Focusing a block does not expand it. `{` and `}` change no expansion state. |
| 2026-11-11 | Task panel: the unfocused strip ends with `ctrl+x l next tab · ctrl+x g hide`, from the keymap. Hints drop before tab labels when narrow. Supersedes the 2026-10-14 decision to show no task panel key hint. |
| 2026-11-11 | Tips and longer notices join the task pane: painted on the `pane` background at indent 3, with no blank row, when the panel is drawn. With no panel they render as before. |
| 2026-11-11 | While the task panel has the keyboard, a typed character that is not one of its keys returns the keyboard to the prompt and types there. |
| 2026-11-11 | A new tip teaches the tabs ("Tasks have tabs"). Running marks are accent, never yellow. |
| 2026-11-11 | Colour semantics: `crates/hoocode-code-tui-theme/color-semantics.json` is the machine-readable source for agents (rules, roles, states, surfaces, token meanings). It is kept in sync with the schema by `tests/it/color_semantics.rs`. This page keeps a summary. |
| 2026-11-11 | UI-2, UI-3, UI-5, UI-8, UI-10 and UI-11 closed as verified. UI-2: the orphan goldens are deleted. UI-3: the plain-words guard runs in CI `guards`. UI-5: a printable leader is refused (`LeaderKey::new`). UI-8: the contrast test covers all five themes. UI-10: the session picker's `x` asks for confirmation (`confirming_delete`). UI-11: `ctrl+x y` (yank pop) and `ctrl+x z` (redo) are bound. |
| 2026-11-11 | `ctrl+x o` opens every tool row, or closes them all when any is open (UI-24 closed). Frozen rows are skipped and a one-row notice says how many. Supersedes the 2026-11-07 "no expand-all" row. |
| 2026-11-11 | UI-6 closed: every dialog and notice has a component golden in dark, light and monochrome. UI-30 closed: the write-only override map in `TranscriptFocus` is removed. |
| 2026-11-11 | User requests, not built yet (Open UI-31, UI-32): a tool row shows whether it is open, and a click anywhere on its line toggles it; the selected row in every pane is bold; deeper panes match the settings pane. When built, these supersede "an expanded header looks the same as a collapsed one", "a press and release on the same header cell" and "selected row: normal text (not bold)". |
| 2026-11-11 | UI-4 and UI-9 were closed on 2026-11-08 and UI-21 and UI-22 on 2026-11-07. The Open table no longer lists them. |
| 2026-11-12 | UI-25 and UI-31 closed: one toggle mechanism for every block. A block that can open ends its header in a dim `▸` (closed) or `▾` (open) after its last fact; the mark stays at column 1 and the name at column 3, and no row is added. A block that cannot open shows no indicator: no output, TodoWrite, a stopped call, a trace of one line, a thinking trace under hidden thinking. The shape carries the state, so colour and monochrome read alike (`status::CLOSED`, `status::OPEN`; ASCII `+` and `-`). A press and release on the same row toggles anywhere on the line up to the scrollbar column; another row is a drag. A click on a row that cannot open says so in the status slot (`Shell: no output to show.`). Supersedes "an expanded header looks the same as a collapsed one" and "a press and release on the same header cell". `ctrl+x o` no longer skips frozen rows: they keep their open body, and a row with none is left alone. |
| 2026-11-12 | UI-28 closed: the permission card's keys (`app.permission.edit`, `pick1`–`pick3`), the interrupted-run row's keys (`app.subagent.view`, `app.subagent.resume`), Space in settings (`app.settings.toggle`) and the scoped models category key (`app.models.category`) come from the keymap, with a test per key that a rebound key acts and its hint shows the new name. The interrupted-run keys have their own `Subagent` context, since `r` is rename in `Picker`. The scoped models hint row shows four items (on/off, category, save, close) then `f1 more`; search leaves the row and stays under `f1 more`. |
| 2026-11-12 | UI-27 closed: in settings, Space (`app.settings.toggle`) types into a non-empty search and toggles only when the search is empty. The rule holds for whatever key is bound to the toggle when that key is printable. Tests in `picker_keymap.rs`. |

## Open

| ID | Item | Size | Trigger or gate | Design notes |
|---|---|---|---|---|
| UI-7 | Auto theme by OSC 11 (decision 2026-11-03). Not built: `auto` reads `COLORFGBG` and the macOS setting only. | M | User: in scope or drop | Send `ESC ] 11 ; ? BEL`, parse the `rgb:` reply with a short timeout, fall back to `COLORFGBG`. Re-check on the existing 60 s poll, or on focus. If dropped, say so in Decisions. |
| UI-12 | Verify the VS Code terminal does not take Ctrl+X before the default leader is final. | S | Manual check | If it does, keep the default and say so on the user page. |
| UI-13 | Verify `⊗` and `◉` on common fonts (macOS Terminal, Windows Terminal, JetBrains Mono). | S | Manual check | `◉` ships in JetBrainsMono Nerd Font Mono. Record the result in the glyph module note. |
| UI-14 | The inline screen (`HOOCODE_FULLSCREEN=0`) is still built. Mission Control planned to remove it after one release. | M | User decision, next release | Removing it deletes the inline tree, `screen_fill`, `scrollback_fold` tests, `terminal.clearOnShrink`, and the `into_lines` piece diff (dropped with it). |
| UI-15 | Exit prints nothing. A short exit summary was an undecided follow-up. | S | User decision | One dim row after the alternate screen closes (session, time, tokens), only when leaving full-screen. |
| UI-16 | Pinned page scroll step: built as view height minus 2. The spec said minus 12. | S | User or `/perf` | Pick one. Record it in the code comment and on this page. |
| UI-17 | The chat component asks every body component for all its lines each frame (O(transcript)). | M | Only if `/perf` shows it | Add a windowed render API to the chat component. |
| UI-18 | Replay fixture `crates/hoocode-code-main/tests/fixtures/replay.json`: keep or delete. | S | User decision (the one-week wait is past) | Decide. Delete if unused. |
| UI-19 | Held behaviour changes: `@file` ranking through `tui-fuzzy`, editor cursor in bytes, `PickerView` wrapper, tips trimming, `get_conflicts` in `/hotkeys`. | M | User asks for each one | One commit per item. |
| UI-20 | 30 files are named `*_ts*.rs` (a leftover of the retired TypeScript parity). | S | Optional | Rename with `git mv` in one commit. No code change. |
| UI-26 | `/hotkeys` shows one meaning per bare key and omits non-letter keys (`controls.rs` `layer_cap`, `layer_actions`). | S | None | |
| UI-29 | Remaining non-blocking uses of the warning colour. | S | None | The list is in [docs/backlog.md](../backlog.md) (UI-29). The warning colour is for blocking only. |
| UI-32 | Deeper panes (the screens reached from settings) have stray blank rows and whitespace. Match the settings pane. Make the selected row's name bold in every pane, as Models, Tree, Theme and Scoped models already do (user, 2026-11-11). | S | Next | Sessions, Settings and Fork are not bold today. |
| UI-33 | Screen goldens are stale: 9 of 10 scenarios differ. They predate the single view (2026-11-07). | M | Before the next release check | Review each diff, fix the timing waits in `subagent-attach` and `scroll-view`, then `update`. List in [docs/backlog.md](../backlog.md). |
| UI-34 | 21 places where a dialog or notice golden differs from this page. | M | After UI-32 | List in [docs/backlog.md](../backlog.md). Fix the screen or fix this page, one by one. |

## Not doing

- Space-metaphor words in TUI text, and the banned list above.
- Turn-end words `LANDED`, `DID NOT LAND`, `ABORTED`, `IN FLIGHT`, `WAITING`. Only the failed and stopped lines exist.
- Thinking durations (`✻ thought 4s`).
- The `◐` glyph. A spinner. Pulses.
- Step numbers, rails, `Took`, and a success end line in the transcript.
- HUD, logo, wordmark, starfield, toast, comms frame, session chip, `/color`, `/chrome`, `alt+z`.
- Sixel images. Kitty and iTerm2 image rendering stay.
- Alt keys in any default. Any Alt-based newline.
- An in-app key editor. Chords beyond the leader. A second leader.
- Built-in theme override by name. `ansi:<name>` terminal-palette themes (later).
- Ctrl+J as a newline key.
- The `notify` crate. Theme hot reload stays on the poll.
- Pinned mockups and their generators as design sources. The goldens are the source.

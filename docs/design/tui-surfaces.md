# TUI surfaces: one system for every screen

Status: **design agreed 2026-10-17** ([decisions-2026-10-17.md](decisions-2026-10-17.md)). Review page: 46 screens, today vs proposed, captured from the real binary (private artifact, link in the decisions page).

**Next (2026-10-17, later):** the Mission Control card ([mission-control.md](mission-control.md)) builds on this one; where they differ, see its table.

Mission Control wins where they differ (user, 2026-10-17); the TUI goes full screen, so "Not doing: no fullscreen" no longer holds.

## Goal

1. One system for every TUI surface in peek and radar: colour, marks, hints, layers.
2. Fix the bugs the review found. Bugs come first.
3. Land it on ratatui without losing today's look.

## Decisions

| Topic | Decision |
|---|---|
| Rules | Five rules. Attention is the budget. Three layers, never mixed: You, Agent, Frame. Everything is a block. The terminal is home. Modern underneath, plain on top. Today's look is the base. Change only what a call below says. |
| Layers and regions | Scrollback holds the start header once, then only You and Agent blocks. The live region at the bottom is redrawn and never printed into history: running blocks, the notice row, the plan line, the prompt frame, the footer. "System" lines (colour set, chrome changed, the working line, the per-turn token line) move to the live region or go. |
| Colour | One meaning per colour. Green `#b5bd68`: done. Red `#d58181`: failed. Accent `#5cc8bb`: now (running `◐`, the `›` marker, the mode chip, paths, the active tab, frame titles). Yellow `#ffff00`: needs you (the `◉` block, the context `!`). Blue `#6e93ff`: you have the keys (question, picker and focused task panel frames). Grey `#757575`: the prompt frame. Dim `#989898` and muted `#a8a8a8`: the rest. Running is no longer yellow. Rails have no state colour. |
| Marks and glyphs (C2, C22) | Status marks are circles only: `○` not started, `◐` running (still), `●` done, `⊗` failed, `◉` needs you, `⊘` cancelled. They read the same on blocks, plan rows, agents and shell jobs. No spinner. No `○●` pulse. Shapes differ, so they read without colour (`NO_COLOR`). Kinds: `✦` skill, `◈` agent, `⌘` command, `✻` thinking. Pointers: `❯` prompt, `›` selected, `◂` back, `…` more. |
| Hints (C3) | `key action · key action`. Keys in ink, actions dim, keys lowercase (`esc`, `ctrl+o`, `alt+↑`). Hints appear only where the keys are: the last row of a blue frame, the scroll row, the attach header, the prompt border. Never in the footer. Never as a sentence: `esc back`, not "Escape goes back". |
| Rhythm (C4) | One blank row between blocks. None inside a block or a frame. Start: wordmark, one blank, the resources line. |
| Block grammar (C5, C19) | `  ● Name  target  stats`: two spaces, mark, space, bold name, two spaces, target (paths in accent), two spaces, dim stats. The body starts at column 4. Fold line: `… N more lines`, no brackets. File tools (Read, Write, Edit) sit on the tool band. A subagent's own words keep the grey `│` rail at column 6. A failed Shell ends with `exit N` in red. A running block shows `◐` in place of the dot, with elapsed time dim after the stats. Your `!` command: your band, then its output at column 4. |
| Prompt border state (C7) | The top border of the prompt frame carries the session state: `◐ working · 12s · esc stop`, `done · 3.1s`, `⊗ failed · 2.0s`, `◂ explore · running`. Idle at start: plain border. It replaces the "Working..." row and the per-turn token row. Tokens stay in the footer. |
| Queued notes (C8) | Queued messages sit inside the prompt frame, above the `❯` line, on the user band. Each one says when it sends: "sends after this step" or "sends when this turn ends", right-aligned and dim. `alt+↑ edit` shows once, on the last one. |
| Notice row (C9) | One row directly above the prompt frame. It holds tips, warnings, extension status, startup progress and "session colour set". The newest wins. It shows while relevant and never enters scrollback. Yellow `◉` only when it asks for action: `◉ context 88% · /compact to summarise`. Otherwise dim text with keys in ink. |
| Task panel (C10) | Unfocused: one plan line above the notice row and the prompt. `Plan 3/5  ◐ add CJK width fixtures` on the left, `Agents 1 · Shell 1 · ctrl+g` on the right. Focused (`ctrl+g`): the panel takes the prompt's place in a blue frame. Tabs sit in the top border; the active tab is accent bold. Rows carry the circle marks, `›` marks the selected row, and the hint row is `tab next tab · enter open · x cancel · esc back`. |
| Lists (C11) | One picker pattern. Blue frame, title in the top border. Optional query row `❯ text`. Rows: `name`, dim description, dim meta on the right. Selected row: `› name` on the selection band across the inner width. At most 10 rows, then `  … 14 more`. Hint row last. Slash and `@` autocomplete stay a plain drop-down under the prompt, with no frame, in the same row format. The footer hides while it is open. |
| Questions (C12) | Approval, AskUserQuestion, MCP forms, MCP trust and quit-with-jobs share one pattern: blue frame, the question as the title in plain words, choices as list rows, the safe default selected, hint row. The block that asks is the only yellow thing. Example title: `Allow Shell  printf …?` |
| Thinking (C13) | Folded by default: one dim italic row ` ✻ thought for 4s · ctrl+t`. Not "Thinking...", which reads as still running. Expanded (`ctrl+t`): ` ✻ ` then the text, dim italic, wrapped at column 3. |
| Footer (C14) | Today's two-row footer as built, plus the agreed footer card ([footer-two-modes.md](footer-two-modes.md)) where today differs. Nothing else is added. Only the dial words change. |
| Attach view (C15) | The header row is the session's frame: `◂ explore · Find the footer code · running`, with `esc back` at the right. The prompt frame stays in place, empty and dim, with `read-only`. Its border reads `◂ explore · running` (the key is in the header, once). |
| Scroll view (C16) | The inverse status bar becomes a plain row in the footer's place: ` 43–61 of 78 · 55%` dim on the left. Hints on the right: `↑↓ line · pgup pgdn page · ctrl+home top · esc live`. Scrollbar `┃` and `░` unchanged. |
| Start (C17) | Wordmark, one blank, the resources line. "no project resources loaded" goes: nothing to say, so say nothing. Startup errors go to the notice row. |
| Model errors (C18) | One failed block in scrollback, not one row per attempt: `  ⊗ Model  529 upstream overloaded`. Body: `gave up after 6 attempts in 6.2s · send again to retry`. While it retries, the notice row says so. Border state `⊗ failed · 6.2s`. |
| `!` command (C19) | Your command is yours: your band shows ` ! printf 'a\nb\n'`. Its output sits below as a block body at column 4, muted. `exit N` in red only when it fails. |
| Compaction (C20) | A dim divider across the width: `── compacted · 38k → 6k tokens · ctrl+o summary ──…`. It is real history, because the context changed there, so it stays in scrollback. |
| Radar (C21) | Radar shows peek's headers and hides the bodies. One call is exactly its peek header: `  ● Read  src/wrap.rs  42 lines`. A run of 2+ calls folds into one row: tool names with counts in order of first use, `  ● Read 3 · Shell 2`. Stats are right-aligned, dim, and stay whole; the label is dropped when it does not fit beside them, and stats are cut only when they fit nowhere (built, U0.6). Chain mark: `◐` running, `●` done, `⊗` any call failed, `◉` a call waits on you, `⊘` interrupted (never green). Under a chain row, only calls that need a look get a header, indented 4: the running call, the call waiting on you (`◉`), each failed call (`⊗`) with its last 2 output lines at column 6 and `… N more lines · ctrl+o`. Done calls stay folded. Agent rows use the description, not the prompt. Thinking is hidden in radar and never splits a chain in either view. |

## What we build

### Phase U: surfaces

Needs: nothing new. The table is the build order. Open question 1 sets where UR goes.

| Step | Work | Size |
|---|---|---|
| U0.1 | **Retries and model errors.** Retry events are never shown: `AutoRetryStart` and `AutoRetryEnd` are emitted in `crates/hoocode-code-agent-session/src/retry.rs`, and no TUI code matches them. Six identical `Error: 529` rows print. Show one notice while it retries and one red Model block at the end (C18). Event handling: `crates/hoocode-code-tui-app/src/mode/events.rs`. **Done 2026-10-17** (8a4c231, c9fdeca). *As built:* while a streak retries, the notice band reads `retrying · attempt 2 of 6 · <error>`. When it gives up or is cancelled, the attempts' rows are removed and one `⊗ Model  <error>` block replaces them: `gave up after N attempts in X.Xs · send again to retry`. Esc and Ctrl+C cancel a backoff. Logic in `mode/retry.rs`. | M |
| U0.2 | **Content printed twice.** A Write block shows the file as a preview, then again numbered (`crates/hoocode-code-tui-widgets/src/tools/write.rs`; cause not traced). The plan prints as a TodoWrite list in the transcript and again in the panel (tool: `crates/hoocode-code-tools-optin/src/todo.rs`; panel: `crates/hoocode-code-tui-widgets/src/task_panel.rs`). Keep one copy of each. **Done 2026-10-17** (d4a4c0c, reviewed in e7b8c9a). *As built:* a Write block previews its content only while there is no result; the numbered result is then the one copy (or the error, for a failed call). TodoWrite is one header row, `TodoWrite  3 items · 1 done`. The plan shows only in the task panel's summary row, and resume rebuilds it from the last TodoWrite (`restore_plan`). | S |
| U0.3 | **Ctrl+T rebuild.** After `ctrl+t` the transcript is rebuilt and loses the `Took` rows (written in `crates/hoocode-code-tui-widgets/src/tools/bash.rs:151`) and the token row. The toggle is `crates/hoocode-code-tui-app/src/mode/transcript.rs:600`. In radar, a second `ctrl+t` changes nothing on screen. **Done 2026-10-17** (d9954f5). *As built:* each tool result records its wall-clock duration (`ToolResultMessage.duration_ms`, not written to session files), so a rebuild after `ctrl+t` shows `Took` again. The rebuild also draws the per-turn token row. In radar, `ctrl+t` now says in the notice that thinking is hidden there, and points at `ctrl+o`. | S |
| U0.4 | **Counts and durations.** Line counts count the trailing newline (`crates/hoocode-code-tui-widgets/src/tool_execution.rs:406`). `3h60m left` rounds minutes apart from hours (`crates/hoocode-code-agent-session/src/format.rs:69`, called from `task_panel.rs:1066`). **Done 2026-10-17** (8a4c231, c9fdeca). *As built:* a line count does not count a trailing newline, and the fallback counts lines the way the rest of the tool block does. `format_duration_secs` rounds the total first, so 3h59.5m reads `4h00m` and 119.6s reads `2m00s`. | S |
| U0.5 | **Radar chain state and boundaries.** An interrupted chain with no failure shows green. A background call shows green while it runs. A finished Shell stays `◐ … running` until the model answers (`crates/hoocode-code-tui-widgets/src/tool_chain.rs`: `rows` at :97, `close` at :63). Chain boundaries depend on the view: thinking splits a chain in peek only (`tool_chain.rs` `set_view` at :67; `crates/hoocode-code-tui-app/src/mode/transcript.rs:147`). **Done 2026-10-17** (93e5c24, 9638ed8). *As built:* an interrupted chain with no failure reads `⊘` in a muted tone, never green. A background Shell reads `◐` only while its task-store row (`shell:<id>`) is in progress; a replayed history reads done. Aborting a turn does not kill background jobs. A chain reads done once every call has its result. Thinking never splits or ends a chain, in peek or radar. | M |
| U0.6 | **Radar bodies and width.** Failure bodies print every line, uncapped (`tool_chain.rs` `failure_lines`, :189). At 80 columns a long row drops the stats. **Done 2026-10-17** (93e5c24, 9638ed8). *As built:* a failed call shows its header at column 4 and its last 2 reason lines at column 6, then `… N more lines · ctrl+o` (the key follows `app.tools.expand`). At narrow widths the label is dropped first, and the stats stay whole; they are cut only when they fit nowhere. | S |
| U0.7 | **Radar labels.** Write reads "Edited" with a wrong count. WebSearch reads "Fetched … pages". Plurals read "2 fetchs" and "3 searchs". Non-family tools such as MCP drop out of the headline. An Agent row shows its whole prompt, not its description (`crates/hoocode-code-tui-widgets/src/tool_chain_summary.rs`, `FAMILIES` at :277). A finished subagent leaves a task-panel row with an unlabelled number (`120`). **Done 2026-10-17** (93e5c24). *As built:* file families count distinct files, so a file touched twice counts once. WebSearch has its own "Searched" family. Plurals come from explicit forms. Calls outside every family add `· N other calls` to the headline. An Agent row is named by its description. A finished task's total reads `120 tokens` in the task panel. | S |
| U0.8 | **Transcript prints wrong.** Markdown fences print as literal fence rows (`crates/hoocode-tui-components/src/markdown/render.rs:176` and `:186`). `/hotkeys` prints a literal `###` heading (`crates/hoocode-code-tui-app/src/hotkeys.rs:185` and `:205`). The skill block prints before your message (`crates/hoocode-code-tui-app/src/mode/transcript.rs`, the skill block is pushed first). **Done 2026-10-17** (d9954f5). *As built:* fence lines do not print (the code block renders as before). Headings of level 3 and up render styled, without the literal `###`, so `/hotkeys` is fixed. A skill invocation's own message draws before its skill block. | S |
| U0.9 | **Cut text.** Theme descriptions are cut mid-word (`crates/hoocode-code-tui-selectors/src/settings_selector.rs`). The `/tree` hint row is cut to `a...` (`crates/hoocode-code-tui-selectors/src/tree_selector.rs`, hint at :1322). **Done 2026-10-17** (d4a4c0c). *As built:* select-list descriptions are cut at a word boundary with `…`, padded to the cell (`truncate_at_word`, ANSI-aware since e7b8c9a). The `/tree` hint row drops whole items from its end instead of ending in `...`. | S |
| U0.10 | **Dead glyphs and branch.** `⬢` (`crates/hoocode-code-tui-widgets/src/brand.rs:6`) and `◒` (`brand.rs:27`) are defined and never drawn. The `◆` branch for the Main owner in `crates/hoocode-code-tui-widgets/src/task_panel.rs:135` cannot be reached. Remove them. **Done 2026-10-17** (d4a4c0c). *As built:* `BRAND_MARK` (`⬢`) and the Themes glyph (`◒`) are removed, and so is the unreachable Main-owner `◆` branch. The subagent and MCP glyphs are unchanged. | S |
| U1 | **Glyphs and colour (C1, C22).** Circles everywhere. Drop the `○●` pulse. Dial words `peek` and `radar`. One separator `·`. `…` not `...`. One "N more" wording. | M |
| U2 | **Block grammar in peek (C4, C5, C19).** Stats in headers. `exit N` on a failed Shell. CodeSearch hits. Fold wording. `!` on the user band. | M |
| U3 | **Radar = peek headers (C21).** One call is its peek header. A run folds into one row of counts. Stats are never dropped. | M |
| U4 | **Prompt border state, queued notes, notice row (C7 to C9).** | M |
| U5 | **Task panel (C10).** One plan line. Blue frame on `ctrl+g`. | S |
| U6 | **Lists and questions (C3, C11, C12).** One pattern. `/hotkeys` becomes a list. One hint grammar. | M |
| U7 | **The rest.** Thinking folded (C13). Attach view (C15). Scroll and search rows (C16). Start (C17). Model error block (C18). Compaction divider (C20). | M |
| UR | **The ratatui port (place: open question 1).** `Viewport::Inline` with `Terminal::insert_before`. Finished blocks go to native scrollback. The live region (running blocks, notice row, plan line, prompt, footer) is the viewport. Port screen by screen. Goldens stay identical. | L |

**Phase U0 is done (2026-10-17).** Follow-ups it leaves open:

- A background Shell needs a job-end signal. The TUI cannot see a job exit. It reads the task store row (`shell:<id>`), so a finished job reads `◐` until that row changes.
- Folded thinking still reads `Thinking...`. C13 says ` ✻ thought for 4s · ctrl+t`. That moves to M4 in [mission-control.md](mission-control.md), where it becomes `✻ thought Ns · ctrl+t`.
- Sessions loaded from disk have no `Took` duration. The duration lives in memory only, so U0.3 covers the live path and the rebuild, not a resumed session.

Haiku subagents do every step ([CLAUDE.md](../../CLAUDE.md)). The main session briefs, reviews and merges. Each step updates its goldens and `docs/maps/ui.md` in the same commit.

## Not doing

- No spinner animation.
- No new colours.
- No fullscreen or alternate-screen transcript. Scrollback stays native.
- No new always-on chrome rows.
- No change to footer contents beyond the dial words.

## Open questions

1. **Order of the ratatui port.**

   | Option | Order | Trade-off |
   |---|---|---|
   | (a) recommended | U0 bugs on today's renderer, then UR (the port, goldens identical), then U1 to U7 in ratatui code | Each widget is written once. The port is checked against screens we already know. Visible changes come later. |
   | (b) | U1 to U7 on today's renderer, then UR | Users see changes sooner. Every widget is touched twice. |

2. **Glyph fonts.** `⊗` and `◉` are new to hoocode. `◐`, `○` and `⊘` are already drawn.

   | Option | Work | Trade-off |
   |---|---|---|
   | (a) recommended | Check `⊗` and `◉` in the screen goldens and on common terminal fonts at U1. Use `●` and `○` as fallbacks only if a font lacks them. | One check now. Fallbacks may still be needed on some fonts. |
   | (b) | Ship without a check. | No work now. A font gap shows up in users' terminals, not in our checks. |

## Details

### Supersedes

| Card | Part superseded |
|---|---|
| [tool-calls-peek-radar.md](tool-calls-peek-radar.md) | The radar row, and the `◍ ◌` dial glyphs. The dial says `peek` or `radar`. |
| [footer-two-modes.md](footer-two-modes.md) | The dial glyphs `◍ ◌`, and the running count `◇N`, which becomes `◈ N`. |

### Facts from the review

- The live radar row is built by `ToolChainComponent::rows` in `crates/hoocode-code-tui-widgets/src/tool_chain.rs` (line 97). `crates/hoocode-code-tui-app/src/mode/transcript.rs:470` creates the chain.
- `render_tool_signal_line` in `crates/hoocode-code-tui-widgets/src/tool_signal.rs` (line 229) is not the live radar row. [ui.md](../maps/ui.md) lists `tool_signal.rs` with the radar row. U3 corrects the map.
- Chain labels come from the family table in `tool_chain_summary.rs` (`FAMILIES`, line 277). Write falls in the "Edited" family, which is why it reads "Edited".
- Line counts split on `\n` (`tool_execution.rs:406`), so a trailing newline adds a phantom line.
- The working indicator is a two-frame `○` and `●` pulse: `DEFAULT_FRAMES` in `crates/hoocode-tui-components/src/loader.rs` (line 18), with `PULSE_INTERVAL_MS` at 640 (line 22).
- Failure output has no cap: `failure_lines` prints every non-blank line of the error text (`tool_chain.rs:205`).
- `format_duration_secs` rounds minutes separately from hours, so 3h59.5m prints as `3h60m`. It is the shared formatter, used by the task panel's `describe_run`.
- Retry events have no TUI handler. The only emitter is `retry.rs`.

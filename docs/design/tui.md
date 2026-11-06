# TUI

Status (2026-11-06): built. The look is the declutter (built 2026-11-04, PR #174). Full-screen is the default. Themes and keybindings are built (2026-11-05). Screen anatomy, part by part, is in [docs/maps/ui.md](../maps/ui.md). This page holds the rules.

## Principles

| Rule | What it means |
|---|---|
| Plain words | Say what happened, in plain English. No space metaphor in help, tips, slash descriptions, settings, notices or status text. `scripts/ci/plain_words.sh` bans these in TUI string literals: SHIP, FUEL, CR, LANDED, HIT, M1, CLEARANCE, TRANSMISSION, HANGAR, DOCKING, MISSION, SYSTEMS, RESOURCES, "flight path", "end shift", "satellite", "jettison", "comms", "hull", "mission control". |
| One fact, one place | Status line: view, session, model, context %, cost, live state or last turn result. Transcript: per-call facts (`+2 −2`, `exit 0 · 8.2s`). Prompt: input only. |
| Status line is the only status surface | No HUD, logo, toast, comms frame, session chip, footer rows or footer dial. |
| Everything is a block | Every item in the transcript and every dialog is a block with one grammar (below). |
| Blocks are separated by one blank row | One blank row between blocks. None inside a block. A tool row with a body is followed by one blank row. Consecutive body-less tool rows stay tight. |
| Separation by colour, not borders | Panes and the prompt use background steps and text weight. The only borders are in monochrome (see Accessibility). |
| Glyph set from code | `crates/hoocode-tui-glyphs/src/lib.rs` (`status`): `●` done or call, `◉` working, `○` todo or queued, `⊗` failed, stopped or denied, `›` marker. Nothing else. No spinner: running is still and only the elapsed time moves. `◐` is not used (missing from JetBrainsMono Nerd Font Mono). |
| Shape carries state | Each state has its own shape, so state reads under `NO_COLOR`. Colour is added on top. Never colour alone. |
| The blocking thing is the only yellow | A permission or question title is yellow bold, the safe default is selected, and nothing else on screen is yellow. |
| Block grammar | Mark, one space, bold name, target, two spaces, dim facts. Body at column 4, dim. Fold line `    … N more lines`. |
| Hint grammar | `key action · key action`. Keys in normal ink, actions dim. Keys lowercase (`esc`, `ctrl+o`, `f1 more`). At most 4 items, then `f1 more`. Hints appear only where the key acts. Never a sentence (`esc back`, not "Escape goes back"). Hints come from the keymap, never typed literally. |
| Width | Stats are never dropped. Labels are cut first, at a word boundary, with `…`. Every mark is one cell wide. |

## Layout and views

### Rows, top to bottom (full-screen)

1. Start header (two lines, start screen) or one blank row.
2. Attach header, only while attached to a subagent run.
3. Body: the transcript. Live follows the tail. Pinned holds still. A short live transcript is bottom-anchored.
4. Task panel: a pane above the prompt, rows only.
5. Notice rows (max 3).
6. Completion menu (slash, `@file`, argument choices), above the prompt.
7. Transient rows: leader hint, warnings, retry notice, extension status, startup progress.
8. Queued messages: dim `○` rows directly above the prompt.
9. Prompt row: ` › ` on the surface colour, no frame, grows with input. A dialog takes this row.
10. Status line: the last row.

Start screen: ` hoocode v<ver>` (bold) and ` <cwd> · N skills · M agents` (dim). Nothing else. Idle prompt placeholder is empty.

### Views

The tool output view has three levels: `outline`, `peek` (the default) and `full`. Leader `o` cycles forward, `O` backward, `p` steps outline → peek → full → outline. The setting is `toolOutputView`. `radar` still parses and means `outline`.

| Element | outline | peek | full |
|---|---|---|---|
| User message | whole | whole | whole |
| Thinking | hidden | one dim italic line, first sentence then `…` | whole, dim italic |
| Tool calls | one recap row per batch: ` ● Read src/config.rs · Edit src/config.rs · Shell cargo test`, no count | one row per call: ` ● Edit src/config.rs  +2 −2` | one row per call |
| Tool body | none | Edit: 4 diff lines then `… N more lines`. Shell: last 2 lines. Any failure: last 4. Read, Grep, search, web and settled Agent: none. Bodies of 4 lines or fewer show whole. | every line |
| Failed call | own red row under its batch: ` ⊗ Grep "…  regex parse error` | row with red `⊗`, body as above | row with full body |
| Calls inside a subagent | indent 3 under the Agent row | indent 3, failures only | indent 3, all |
| Task panel | hidden | shown | shown |
| Prose | column 1 | column 1 | column 1 |
| End line | failed or stopped only | failed or stopped only | failed or stopped only |

Full-screen is the default (`fullscreen` setting, default on). `HOOCODE_FULLSCREEN=0` gives the inline screen for one run (see Open UI-14).

### Turn result and status line

| State | Left slot of status line | Right side (always, dim) |
|---|---|---|
| Working | `◉ working · 4s · esc stop` | `peek · amber-harbor · claude-haiku-5-5 · 42% context · $1.23` |
| Idle after success | `● done · 11s · 23k tokens` | same; `plan` first in plan mode |
| Idle after failure | `⊗ failed · 5.0s · 14k tokens` (`⊗` red) | same |
| Done with a failed call | `● done · 8.5s · 7.1k tokens · 1 failed call` | same |
| Not signed in | `⊗ not signed in  /login` (outranks the last turn, yields to working) | view and session only |
| Permission or question open | empty | same |
| Pinned (scrolled back) | `scrolled 43–61 of 78 · ↓ 3 new` or `search  query  1 match` | same |

Narrow terminal: the right side drops, in order, cost, session name, the word `context` (keep `42%`), model effort. The left side keeps its text and is cut last. The view name is always first on the right.

Failed and stopped turns also end the transcript with a line: ` ⊗ failed · 5.0s` (`⊗` and `failed` red) or dim ` ⊗ stopped · 2.0s`. A successful turn writes nothing to the transcript.

## Panes and dialogs

- A dialog takes the prompt's row: permission, question, quit, models, settings, sessions, theme, tree, fork, log in, scoped models and keys. The task panel and the completion menus keep the prompt row.
- A pane has no border and no fill step beyond its colour. Anatomy: a title chip (accent teal, dark text, bold; yellow for a blocking pane), then dim `/command` and facts on the same row. Rows indent 3. Selected row: selection colour, ` › ` teal, normal text, secondary cells dim. Filterable panes get an input row at the bottom (`filter` placeholder, dim). A hint row is last. One column of inner padding. No blank padding rows.
- Split: at 100 columns or more (`SPLIT_MIN_WIDTH`), list left and detail right, separated by a one-column gutter in pane colour. Models, sessions, settings, theme (live preview), tree, and keys (a grid of `key action`, grouped). Below 100 columns: stacked, detail under the list.
- Plain titles: Models, Settings, Sessions, Theme, Tree, Fork, Log in, Scoped models, Keys.
- Each row shows one fact that compares rows (models: price; scoped models: `on` or `off`; sessions: age). Full details only on the selected row.
- Completion menus (slash, `@file`, argument) are pmenu style: a pane-colour block directly above the prompt, as wide as its longest row, left edge under the typed token, selected row in selection colour with ` › ` and bold.

### Blocking dialogs

| Dialog | Title and rows | Keys and hint |
|---|---|---|
| Permission | yellow chip `Allow Shell?`. Row ` $ command` (`$` dim). Row `access: pipes 2 programs` only when it writes, uses the network or runs more than one program. Rows `1 yes`, `2 yes, always for <program> this session`, `3 no, tell it why`. | `y` yes and `n` no answer at once while the reply is empty. `a` highlights "always" and Enter confirms it. `e` edits the command first. `1`–`3` pick. Letters type a reply. Hint: `1–3 pick · e edit first · or type a reply`. |
| Question | yellow chip with the question text. Numbered options, each with a dim description. | Digits answer. Letters type into the custom answer when the row allows it. `esc` skips, and the agent states the default it uses. Hint: `1–3 pick · tab next question · esc skip`. |
| Quit with jobs | `Quit? 2 background jobs running`. Job rows; running jobs `◉` yellow. | Hint: `k stop jobs and quit · l leave them running · esc stay`. |
| Log in (first run) | Opens by itself when no provider works. | Esc closes it and the status line shows `⊗ not signed in  /login`. Submitting while signed out keeps the text and reopens Log in. |

Bare letters act only when no text field has text (see Keybindings).

## Notices

| Kind | Where | Timeout |
|---|---|---|
| One-row info | Status line left slot | 3 s |
| One-row warning | Status line left slot | 8 s |
| Longer, or more than one row | Dim rows above the prompt, indent 1, no fill, max 3. The third row ends in `…`. A warning's first row is yellow bold. | 3 s or 8 s |
| Needs an action (for example not signed in) | Status line left slot, or rows above the prompt | Never. It stays until dismissed. |

- No box, no toast, no title bar, no glyph for info.
- Newest notice shows. The queue waits.
- While a turn runs, `◉ working` keeps the status slot, and notices use the rows above the prompt.
- A key hint after the text sits after two spaces: key in normal ink, action dim.

## Themes

### Built-in themes

Five bundled files in `crates/hoocode-code-tui-theme/themes/`: `dark`, `light`, `dusk`, `high-contrast`, `paper`. Each passes `theme_contrast.rs`. `auto` picks dark or light from the terminal background. The picker lists `auto`, the built-ins, then user themes with their file path.

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
| Accent (`›`, inline code, accent chip) | `#5cc8bb` | `#00625c` |
| Green (done) | `#b5bd68` | `#0a6b27` |
| Red (failed) | `#d58181` | `#b8111e` |
| Yellow (working, needs you) | `#f0c674` | `#7a5200` (dark amber, for text) |
| Accent chip (bg / text) | `#5cc8bb` / `#14141c` bold | `#00625c` / `#f8f8fb` bold |
| Blocking chip (bg / text) | `#f0c674` / `#14141c` bold | `#f0c674` / `#2b2f36` bold |

Every text colour on every surface is at least 4.5:1 (WCAG). In light, panes are darker than the page. Dark panes are lighter than the base.

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
| K-P4 | Lists, pickers, panes and scroll mode use bare letters. They are active only when no text field has focus or the query is empty. `/` searches, `?` or `f1` shows keys, `q` or Esc closes a list (not a dialog), `j` and `k` or arrows move, Enter accepts. |
| K-P5 | Every key hint on screen comes from the keymap. A rebound key shows its new name. |

### Direct keys (prompt)

| Key | Action |
|---|---|
| Enter | Submit. While a turn runs, Enter steers the agent. |
| Shift+Enter | Newline. Leader Enter is the fallback. |
| Esc | Interrupt the running turn, or close a popup. Esc Esc clears a non-empty prompt (kept in history). |
| Up, Down | History. Up on an empty prompt with queued messages takes the last one back. |
| Shift+Tab | Cycle agent mode forward. |
| Tab | Completion. |
| PgUp, PgDn | Scroll the transcript (enters scroll mode). |
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
| `b` | Show or hide thinking blocks | `o` / `O` | Tool output view next / previous |
| `p` | Step tool output view | `v` | Paste image (Windows only) |
| Enter | Newline (works in every terminal) | Esc or Ctrl+X | Cancel the leader |

Leader rules: `ctrl+x` enters pending. The next key runs its action. Esc, a second Ctrl+X, an unbound key (`no binding`, shown 1.5 s) or the timeout cancels. The timeout is 1500 ms (`leaderTimeoutMs`, `DEFAULT_LEADER_TIMEOUT`). After 400 ms (`WHICH_KEY_DELAY`) a which-key popup lists the leader keys, built from the keymap. The pending key is swallowed and never reaches the editor. The leader works only at the prompt: a picker, pane or task panel above it takes the key.

### Scroll mode, pickers and dialogs

| Context | Keys |
|---|---|
| Scroll mode (after PgUp, no text focus) | `j` `k` line; PgUp PgDn page; `[` `]` previous or next message; `g` `G` top or bottom; `/` search; `q` or Esc back to the prompt |
| Session picker | `p` path, `o` sort, `n` named filter, `r` rename, `x` delete (no confirm yet, see UI-10) |
| Model picker | `s` save, `a` enable all, `x` clear all, `g` provider, `c` category |
| Tree | `h` `l` fold, `e` edit label, `t` timestamps, `1`–`5` filters, `c` filter cycle |
| Clearance card | `y` yes, `n` no (both at once while the reply is empty), `a` highlight always (Enter confirms), `e` edit, `1`–`3` aliases |
| Yes/No lists (import, missing cwd, extension and permission gates) | `y` Yes, `n` No, `a` Always when offered; Enter default; Esc deny |
| MCP trust | `y` trust and start, `n` not now |
| MCP elicitation | `y` accept or answer, `n` decline, Esc cancel |
| Ask-user card | No `y` or `n` (letters are text). Enter confirm, Esc skip, digits answer. |
| Quit with jobs | `k` stop jobs and quit, `l` leave them running, Esc stay |

Reserved, cannot be rebound: ctrl+c, ctrl+d, ctrl+z, ctrl+m, ctrl+i, ctrl+[, ctrl+h. Warned, can be rebound: ctrl+s, ctrl+q, ctrl+b, ctrl+space.

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
- A leader is one key. A sequence, Esc, Enter, Tab or Backspace is refused and `ctrl+x` is used (see UI-5 for the printable case).
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
- Picker help is `f1` first. `?` types into a filter.
- Some hint keys are fixed text, not keymap ids: Space (settings toggle), `k` and `l` (quit card), `v` and `r` (interrupted-run row), `e` (permission card).
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
| Mouse: wheel scrolls. Shift+drag selects (terminal behaviour). | `tui-terminal/src/mouse.rs` |
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
| Key lint | `scripts/ci/no_literal_keys.py` (CI `guards` job) | Every change | runs in CI |
| Plain words | `scripts/ci/plain_words.sh` | Every change | run by hand (not in CI, UI-3) |

Dialogs and notices have no golden pin yet. Their mockup text is the only copy (UI-6).

## How TUI work is done

- TUI work is done by Haiku subagents (user, 2026-10-09). The main session orchestrates, briefs and merges. Each subagent reviews, implements and tests one step.
- One step per commit. The commit updates `docs/maps/ui.md` and `docs/maps/packages.md` when a crate, screen or command changes. It updates `docs/user/` when a flag, command, key or path changes.
- Done bar: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo nextest run -p <crates>`. Errors use the hand-written enums.

## Proposed: single view (UI-21, UI-22)

Proposed 2026-11-06, pending the decisions below. Mockup: `docs/assets/mockups/tui-single-view.html` (open in a browser; Dark and Light, and 40, 60, 80 and 120 column controls). Nothing here is built or decided. The rules above hold until the user decides, and no row is added to Decisions yet.

### Interaction model

- One transcript mode. Each tool call is one row, collapsed. A click on the header, or Enter on a focused row, shows the full output. A second click hides it. No outline, peek or full, and no view name.
- Each block has an `expanded` flag. It starts false, lives in memory and is not saved, so a resumed session opens collapsed. It survives resize, theme change and rewrap.
- Live: a click pins the view first, with the header on the same screen row, then toggles. `↓ N new` counts rows that arrive below. While a turn runs the hint is `G live`, because `esc` there also stops the turn.

| Key | Where | Action |
|---|---|---|
| `enter` | pinned, a block focused | Toggle it. With no focus, unchanged |
| `{` `}` | pinned, empty prompt | Focus the previous or next block |
| `ctrl+x u` | prompt | Focus the newest block and pin the view |
| `ctrl+x o` | prompt | Expand every block. If all are open, collapse all |
| `y` | block focused | Copy the raw output (never the rendered rows) |
| `esc` | block focused | Clear the focus only. Never stops a turn |
| `G` | pinned | Back to live, focus cleared |

- Mouse: a press and release on the same header cell toggles it. Body cells do nothing. A link opens. The scrollbar column pages. Capture stays `?1000h ?1006h`. Full-screen only.

### Row format

`<gutter><mark> <Tool> <target>  <facts>`, on one line, never wrapped.

- Column 0 is the gutter: a space, or `›` in accent when focused. Column 1 is the mark. The tool name is at column 3.
- The tool name is bold ink. The target is ink. Facts are dim, joined with ` · `. Red only on the failed mark and `exit N`. Green on the done mark and `+n`.
- Focus: the `paneSel` fill across the row, plus `›`. Mono: `›` and reverse video.
- An expanded header looks the same as a collapsed one. Only the body under it differs.

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

Other rows keep "labels cut first, stats never dropped". Compaction's label is `Compacted turns 1–3`, its primary fact is `saved N`, and the token range is secondary.

### Running, failed and stopped

- A failed call settles collapsed and never opens itself. A running Shell, collapsed, shows one dim tail line at column 4 (Decision 4). A stopped call is dim `⊗` with `stopped`.

### Blocks that toggle besides tool calls

- Thinking (collapsed: today's dim italic line), compaction and other summaries, skill block, background-run notice and startup details. `ctrl+x b` still hides all thinking.
- Not toggles: user messages, assistant prose, TodoWrite, the turn-end line and notices.

### Pane grammar (summary)

The pane rules are in "Panes and dialogs" above. The proposal changes them as follows.

- Title chip: the noun in bold accent, yellow only when blocking. Rows indent 3. Selected row: `paneSel` fill, accent `›`, no bold. Only the glyph is coloured.
- Detail: a fixed column at 100 or more (`SPLIT_MIN_WIDTH`), stacked from 60 to 99, at most 3 rows then `…` under 60.
- Hint row: `key verb`, at most 4 items, `enter` and `esc` never drop. Yellow only for blocking. No clicks in panes in v1 (Decision 8).

### Tokens and contrast

No new tokens. Every pair in the mockup is at least 4.5:1 (WCAG, from `crates/hoocode-code-tui-theme/themes/dark.json` and `light.json`). The tightest are dim and red on the dark selection (4.59:1), and green on the light selection (4.64:1). Do not lighten dim or darken `paneSel`. Light has one grey, so facts are told apart by position: two spaces after the target, and the body at column 4.

### What it would remove

- The setting `toolOutputView` (`crates/hoocode-code-settings/src/types.rs:98-101`), the Settings View leaf (the Tool output row stays, Decision 7), and leader `o`, `O` and `p` as view keys. `o` becomes expand-all.
- `tool_output_view.rs`, `peek_fold`, per-tool `Fold`, the outline branches, `tool_chain_summary.rs`, most of `outline.rs`, the view name in the status line, and failed-body auto-show.
- About 95 golden files, regenerated only after a text-grid review. Docs in the same commit: `docs/user/configuration.md`, `docs/user/keybindings.md`, this page, and `docs/maps/ui.md`.

### Rules it would change

Line numbers are for this file as it stands on this branch.

| Line | Today | Would become |
|---|---|---|
| 10 | "Status line: view, session, model, context %…" | The word "view" goes |
| 20 | "Stats are never dropped. Labels are cut first…" | Kept, except tool rows follow the cut order |
| 41–53 | Three views (`outline`, `peek`, `full`) and their table | One mode |
| 61, 69 | The view name on the right of the working line and "always first on the right" | Deleted |
| 80 | Selected row "… with ` › ` and bold" | ` › `, not bold |
| 88 | Running jobs `◉` yellow | `◉` in accent |
| 99 | "A warning's first row is yellow bold." | Ink bold |
| 129 | Yellow "(working, needs you)" | Yellow for blocking only. Principle 17 already says this, so the palette row contradicts it today |
| 196–197 | `o` / `O` view next and previous; `p` step view | `o` expands or collapses all; `u` focuses the newest block |
| 206 | Scroll mode keys | Add `{` `}` focus, `enter` open, `y` copy |
| 288 | "Mouse: wheel scrolls…" | Add: a click on a block's header toggles it |
| 336 | 2026-11-04 Views row | Superseded, if the proposal is taken |
| 339 | 2026-11-04 "one dim line or hidden" for thinking | Superseded, if the proposal is taken |

### Frozen rows (found by reading, confirm with a failing test first)

Blocks older than the newest 50 are frozen (`LIVE_TOOL_WINDOW`, `crates/hoocode-code-tui-app/src/mode/mod.rs:159`, applied in `trim_transcript_memory`, `mode/transcript.rs:1362`). On the next render, `step_parts` (`crates/hoocode-code-tui-widgets/src/tool_chain.rs:374`) calls `capture_frozen`, which calls `release_heavy_state` (`tool_execution.rs:1520`) and sets `result` to `None`. The `mark()`, `facts()` and `is_error()` reads at `tool_chain.rs:383-389` then see no result. A done row draws a dim running `●`. A failed row loses its red `⊗`, `exit N` and failure state. This happens on every render, and today's peek view is affected too.

A second defect: `ToolExecutionComponent::invalidate` returns early for frozen blocks (`tool_execution.rs:1458-1462`), so a theme change leaves them in the old colours.

Proposed fix: `freeze()` saves an unstyled row snapshot (mark, target, primary facts, secondary facts, failure). The header is built from it on each render with the current palette. The first test (P0) freezes a done Edit, a failed Shell and a Read in one chain, renders twice, and asserts a green `●`, red `⊗`, `+2 −2`, `exit 1` and `1451 lines`.

### Decisions needed

None of these is decided. The default is what the mockup draws. When the user decides, each answer becomes a dated row in the Decisions table below.

| # | Decision | Default (mockup) | Alternative |
|---|---|---|---|
| 1 | Remove the outline, peek and full dial | One mode. The dial goes in P5 | Keep the dial beside the toggles |
| 2 | Width rule for tool rows (overturns "stats are never dropped") | Cut order: secondary facts drop first, the target never drops | Keep every fact and cut the target mid-word if needed |
| 3 | Failed row reason colour | Dim. Red only on `⊗` and `exit N` | Ink |
| 4 | Running Shell while collapsed | One dim tail line, gone on settle | Header only |
| 5 | Expand-all scope (`ctrl+x o`) | Every block in the transcript | The current turn only |
| 6 | A click while live | Pins the view; the header keeps its row | Stays live; the row may move |
| 7 | Settings Tool output row | Kept: `800 lines · 32 KB`; View leaf removed | Removed; the caps live only in `settings.json` |
| 8 | Clicks in panes | None in v1 | Click selects; a second click picks |
| 9 | Yellow | Blocking only | Keep yellow for working `◉` |

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
| 2026-11-04 | Views are `outline`, `peek` (default) and `full`. `radar` parses as `outline`. |
| 2026-11-04 | Removed: HUD, comms frame, wordmark, starfield, session chip, `/color`, `/chrome`, `alt+z`, toast float, footer rows. |
| 2026-11-04 | No step numbers, rails, `Took` or success end line. Failed and stopped turns end with a line. |
| 2026-11-04 | No thinking duration (`thought 4s` is not built). Thinking is one dim line or hidden. |
| 2026-11-04 | Scroll and search bars sit in the status line's left slot. |
| 2026-11-05 | Five built-in themes: `dark`, `light`, `dusk`, `high-contrast`, `paper`. Supersedes the two-theme rule of 2026-11-03. |
| 2026-11-05 | `auto` follows the background detected from `COLORFGBG` or the macOS setting, and re-checks every 60 s. |
| 2026-11-05 | Clearance card: `y` and `n` answer at once while the reply is empty. `a` highlights always, Enter confirms. `1`–`3` stay as aliases. Supersedes "1/2/3 only" of 2026-11-03. |
| 2026-11-05 | Leader is Ctrl+X. Timeout 1500 ms. Which-key after 400 ms. The leader works only at the prompt. |
| 2026-11-05 | No Alt key in any default. Alt keys in a user file still work, with one notice per version. |
| 2026-11-05 | Bare letters for lists, pickers, panes and scroll mode, active only with no text focus or an empty query. |
| 2026-11-05 | Enter steers a running turn. Leader `f` queues a follow-up. Leader `d` takes back a queued message. |
| 2026-11-05 | Prompt history is Ctrl+R. Transcript search is leader `/`. Thinking blocks toggle with leader `b`. |
| 2026-11-05 | Ctrl+J is not changed. Newline is Shift+Enter, with leader Enter as the fallback. |
| 2026-11-05 | The theme picker saves `theme` to global settings. Keys change only by editing `keybindings.json`. No in-app key editor. |
| 2026-11-05 | `theme_paths` is removed. An unknown saved theme falls back to `auto`. |

## Open

| ID | Item | Size | Trigger or gate | Design notes |
|---|---|---|---|---|
| UI-1 | Drift: `◆` is still drawn. Skill block `◆ skill: <name>` (`skill_block.rs:51`), attach header `◆ <type>` (`attach_view.rs:42,46`). Spec says `◆` is gone. | S | None | Pick one glyph from the set. A skill block has no state, so bold name only. The attach header takes `›`. Update the test `tests/it/attach_view.rs:18`, goldens `tests/golden/tui/subagent-attach/`, and `docs/maps/ui.md`. |
| UI-2 | Drift: orphan goldens `tests/golden/tui/color-chrome/` (7 files). The scenario JSON was deleted. The files show HUD, SHIP, FUEL, CR. | S | None | Delete the directory. |
| UI-3 | Plain-words guard is not in CI | S | None | Same item as EN-8 in README.md. |
| UI-4 | Drift: the `○●` pulse is still drawn (`tui-components/src/loader.rs`, `DEFAULT_FRAMES`, `PULSE_INTERVAL_MS`). The spec says no spinner. | S | None | Drop it, or keep it for compaction only and say so in Principles. |
| UI-5 | Gap: a printable leader is accepted. `leader_problem` (`code-tui-keybindings/src/config.rs`) refuses only sequences and Esc, Enter, Tab, Backspace. `"leader": "a"` would eat typing at the prompt. | S | None | Refuse a bare printable key (one character, no modifier), warn, and use `ctrl+x`. Add a test under `tests/it/`. Update the user page's leader rule. |
| UI-6 | Pin dialogs and notices with component goldens. The text mockups were deleted on 2026-11-06, so dialogs and notices have no text copy until this lands | M | Next | Add a component golden for each dialog (permission, question, quit, models, settings, sessions, theme, tree, fork, log in, scoped models, keys, slash, files, task panel variants) and each notice (info, info with value, usage, warning). Light and mono: one each. Use `UPDATE_GOLDENS=1`. Also fix the comment at `code-tui-widgets/tests/it/tool_rows.rs:3`, which names the deleted mockups. |
| UI-7 | Auto theme by OSC 11 (decision 2026-11-03). Not built: `auto` reads `COLORFGBG` and the macOS setting only. | M | User: in scope or drop | Send `ESC ] 11 ; ? BEL`, parse the `rgb:` reply with a short timeout, fall back to `COLORFGBG`. Re-check on the existing 60 s poll, or on focus. If dropped, say so in Decisions. |
| UI-8 | Check `dusk`, `high-contrast` and `paper` against `theme_contrast.rs` (five themes are built; the earlier "dark and light only" decision is replaced) | S | None | Add the three to the contrast test if missing. |
| UI-9 | Docs drift: `docs/maps/ui.md` header says "Snapshot 2026-10-17" and shows `◆ <type>` for attach. Old key names remain in old pages: Ctrl+G (task panel), Ctrl+N (attach), Ctrl+T (thinking), `alt+x` (session delete). | S | None | Update the header. Grep `docs/user/` and `docs/maps/` for `ctrl+[gnt]\b` and `alt+`. Match `docs/user/keybindings.md` to the app table (`code-tui-keybindings/src/keybindings.rs`). |
| UI-10 | Session delete (`x` in the session picker, `app.session.delete`) deletes at once, with no confirm. | S | User decision | Add a yes or no step, as the other destructive lists do: `y` yes, `n` no, Esc cancel. |
| UI-11 | Yank-pop (`ctrl+x y`) and redo (`ctrl+x z`) were in the keymap plan but are not bound. | S | User: bind or drop | Redo needs an undo stack in `tui-components` Editor. If dropped, delete from the plan. |
| UI-12 | Verify the VS Code terminal does not take Ctrl+X before the default leader is final. | S | Manual check | If it does, keep the default and say so on the user page. |
| UI-13 | Verify `⊗` and `◉` on common fonts (macOS Terminal, Windows Terminal, JetBrains Mono). | S | Manual check | `◉` ships in JetBrainsMono Nerd Font Mono. Record the result in the glyph module note. |
| UI-14 | The inline screen (`HOOCODE_FULLSCREEN=0`) is still built. Mission Control planned to remove it after one release. | M | User decision, next release | Removing it deletes the inline tree, `screen_fill`, `scrollback_fold` tests, `terminal.clearOnShrink`, and the `into_lines` piece diff (dropped with it). |
| UI-15 | Exit prints nothing. A short exit summary was an undecided follow-up. | S | User decision | One dim row after the alternate screen closes (session, time, tokens), only when leaving full-screen. |
| UI-16 | Pinned page scroll step: built as view height minus 2. The spec said minus 12. | S | User or `/perf` | Pick one. Record it in the code comment and on this page. |
| UI-17 | The chat component asks every body component for all its lines each frame (O(transcript)). | M | Only if `/perf` shows it | Add a windowed render API to the chat component. |
| UI-18 | Replay fixture `crates/hoocode-code-main/tests/fixtures/replay.json`: keep or delete. | S | User decision (the one-week wait is past) | Decide. Delete if unused. |
| UI-19 | Held behaviour changes: `@file` ranking through `tui-fuzzy`, editor cursor in bytes, `PickerView` wrapper, tips trimming, `get_conflicts` in `/hotkeys`. | M | User asks for each one | One commit per item. |
| UI-20 | 30 files are named `*_ts*.rs` (a leftover of the retired TypeScript parity). | S | Optional | Rename with `git mv` in one commit. No code change. |
| UI-21, UI-22 | Proposed single view: one row per tool call, click or Enter to expand, one pane grammar, yellow only for blocking. See "Proposed: single view" above. | L | The 9 decisions in "Decisions needed" (above) | Not decided. Phases P0 to P6 are in the roadmap. P0 starts with the frozen-row test. The Decisions table has no rows for it yet. |

## Not doing

- Space-metaphor words in TUI text, and the banned list above.
- Turn-end words `LANDED`, `DID NOT LAND`, `ABORTED`, `IN FLIGHT`, `WAITING`. Only the failed and stopped lines exist.
- Thinking durations (`✻ thought 4s`).
- The `◐` glyph. A spinner. Pulses.
- Step numbers, rails, `Took`, and a success end line in the transcript.
- HUD, logo, wordmark, starfield, toast, comms frame, session chip, `/color`, `/chrome`, `alt+z`.
- Sixel images. Kitty and iTerm2 image rendering stay.
- Task panel key hint (decision of 2026-10-14 #20 won; not built).
- Alt keys in any default. Any Alt-based newline.
- An in-app key editor. Chords beyond the leader. A second leader.
- Built-in theme override by name. `ansi:<name>` terminal-palette themes (later).
- Ctrl+J as a newline key.
- The `notify` crate. Theme hot reload stays on the poll.
- Pinned mockups and their generators as design sources. The goldens are the source.

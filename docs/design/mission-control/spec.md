# Mission Control: TUI design spec

Extracted from the Mission Control design page and its simulator. This file is the reference for the TUI design. The HTML files stay the source.

Sources (read only, not edited):

- `docs/design/mission-control/index.html`: the page, "hoocode TUI · Mission Control". 46 surfaces in groups A to E and R, each with a "this" view and a "previous proposal" view.
- `docs/design/mission-control/simulator.html`: the simulator. Keys, slash commands, scenarios, routing and ship behaviour live in its inline script.
- Context: `docs/design/tui-surfaces.md` (the previous proposal, decisions C1 to C22) and `docs/design/decisions-2026-10-17.md`.

How this file was made:

- Terminal blocks are the "this" view (`.vn`) of each surface, read from the page HTML. Spacing and glyphs are as in the source. Trailing spaces are removed.
- The previous view (`.vo`) is not copied. Each surface has one "Differs from previous" line that sums up the change.
- Simulator facts come from reading its script. Nothing is run.
- Where the source says nothing, this file says "unclear in source".
- Section 8 lists places where the page, the simulator and the decisions doc disagree.

Contents: 1 Philosophy and vocabulary. 2 Colour themes. 3 Glyphs and marks. 4 The 46 surfaces. 5 HUD, comms frame and layout. 6 Keys, slash commands and scenarios. 7 Open questions, decisions and build order. 8 Discrepancies and unclear items.

## 1. Philosophy and vocabulary

### 1.1 The page's own words

The page section is titled "Same philosophy, new cockpit". Its note reads:

> The five rules still hold: attention is the budget, three layers, everything is a block, two views (peek and radar), and status circles that read without colour. What changes is the cockpit: a one-row HUD, numbered steps on a trajectory, and a comms frame whose colour is the state. **The metaphor is in the chrome; the log stays literal.** Tool names, paths and numbers are never renamed.

The page does not give the five rules as a list. Read from that sentence, they are: attention is the budget; three layers; everything is a block; two views (peek and radar); status circles that read without colour. The five rules in `decisions-2026-10-17.md` are worded differently. See section 8.

The page's lede adds: every prompt is a mission, every model a ship, and every tool call a numbered step. Each screen can be flipped to the previous proposal with `p` (this) and `o` (previous).

### 1.2 Cockpit (page section "The cockpit")

- **Four rows of chrome.** The HUD (1 row) shows mode, folder, ship, fuel and credits. Comms (3 rows, heavy frame) takes its colour from the state. Faint means READY, cyan means IN FLIGHT, amber means CLEARANCE, red means COMMS LOST, green means LANDED. Its bottom border shows peek or radar and the tokens.
- **Two views.** Peek is the trajectory: every step numbered, with up to three lines of what it saw, and one landing line at the end. Radar is the flight log: one row per mission with steps, hits, time and credits. `ctrl+o` switches between them.
- **Every glyph is one terminal cell.** Nothing needs Canvas, images or extra crates.

The page gives this mapping to ratatui:

| Part | ratatui (page) |
|---|---|
| HUD, comms | `Paragraph` · `Block::bordered().border_type(Thick)` with top and bottom titles |
| Trajectory, log | `Paragraph` of styled `Line`s |
| Consoles | `List` or `Table` inside a thick `Block` |
| Fuel, progress | `LineGauge`, or block `Span`s |
| Toasts | `Clear` + `Block` over a `Rect` |
| Wordmark, QR | `▀ ▄ ▟ ▙` spans with `Color::Rgb` |

Page footer note: each screen is a grid of terminal cells at 100 columns (r10 at 80). Ship names (mock-large, mock-fast, mock-local) and every figure are mock data. The QR code is real and opens example.com.

### 1.3 Vocabulary map

Rows 1 to 8 are the page's own table ("Same philosophy, new cockpit"). The other rows come from the simulator code and the mock text. The "TUI thing" column says what the word means on screen.

| Word | Meaning | TUI thing (where it shows) |
|---|---|---|
| mission `M3` | one prompt and everything the agent did for it | a header row `▲ M3 <prompt>` in peek, one row in radar. A queued prompt becomes the next mission. |
| ship | the model | `SHIP mock-large` in the HUD. `/model` opens the HANGAR. |
| step `04` | one tool call. An agent's calls are `1.1`, `1.2` | gold step number at the start of each tool row. `┊` is the trajectory rail. |
| hit | a call that errored or exited non-zero | red `HIT` chip, a `⊗` mark, and `N✕` in radar. |
| hail | a provider retry (529, timeout) | a `⊗` tick on the time axis of a Model block. After its retries run out the mission shows COMMS LOST. |
| fuel · credits | context used · money spent | HUD `FUEL ▰▱…` gauge (8 cells, with %) and `CR $0.000`. |
| agent · skill | keep their own names | agent: `◈ name` with a violet `┃` rail. skill: `✦ name` chip on the mission header. |
| satellite | a background job | `⌁ sat job-1 · in orbit`. HUD `⌁ N sat`. It lands on its own. |
| trajectory | peek view | numbered steps, `┊` rail, up to three lines of output per step, one landing line. |
| flight log | radar view | one row per mission: steps, hits, time, credits. |
| comms | the 3-row heavy frame at the bottom | colour and title give the state. Holds the input row. Foot shows the view and tokens. |
| HUD | the one-row bar at the top | mode, folder, ship, fuel, credits. |
| clearance | an amber card asking to run a Shell command | `◉ CLEARANCE NEEDED` with the command, a REACH line and keys `y a n e`. |
| console | a heavy-framed list that replaces the comms frame | commands, `@` files, hangar, controls, hull, mission log, docking, flight path, flight plan, end of shift. |
| flight plan | the mission's waypoints | `FLIGHT PLAN ●●◐○○ 2/5` in the log. `ctrl+g` or `/plan` opens the console. |
| transmission | a notice | toast box at top right with a draining bar. |
| hull | the theme | `/theme` opens the HULL console. |
| hull paint | the session colour | `/color` cycles parity (gold), cyan, violet, lime, red, none. It tints the HUD and the comms frame. |
| hangar | the model picker | `/model`. Ships, provider, context, price, thinking dial. |
| docking | sign in | `/login`. QR code and a typed code. |
| mission log | session list | `/resume` (alias `/log`). |
| jettison | compaction | `/compact` folds earlier missions into one summary stage. Shows `≋≋≋ JETTISONED`. |
| stage separation | fork | `/fork`. A line marks what the new session carries. |
| flight path | session tree | `/tree`. `YOU ARE HERE` marker. |
| scan | search | `ctrl+r` opens `⌕ SCAN`. Matches are contacts. |
| rewind | scrolled back | PgUp or shift+↑. `◂◂ REWIND` bar above the prompt. |
| end of shift | quit | `/quit`, or `ctrl+c` twice. |
| BUILD, PLAN | modes | BUILD may change files and asks clearance for Shell. PLAN changes nothing. `shift+tab` toggles. |
| queued note | a prompt waiting its turn | `○ text` inside the comms frame, with when it sends. |
| REACH | what a command can touch | read, write, network, pipe. Shown on the clearance card. |
| LANDED, DID NOT LAND, ABORTED | mission end states | the debrief row and the comms title. |

### 1.4 Rules the page states about the look

- Attention is the budget. Three layers. Everything is a block. These are the page's words. The decisions doc adds "never mixed" to the layers rule, and names the layers You, Agent and Frame. See section 8, item 1.
- Status circles read without colour: `○ ◐ ● ⊗ ◉ ⊘` differ by shape.
- Every glyph is one terminal cell.
- Tool names, paths and numbers are never renamed. The metaphor is in the chrome only.

## 2. Colour themes

### 2.1 Themes

The page and the simulator both offer three themes. The page labels the group "terminal theme". No button or value named "terminal" exists. Unclear in source whether "terminal" was meant as a fourth theme. It is not in the code.

| Name in UI | `data-term` value | Default | Description in simulator | Token block |
|---|---|---|---|---|
| deep-space | `deep` (also used when the attribute is absent) | yes | the default · navy void | `html:not([data-term]), html[data-term=deep]` |
| daylight | `daylight` | no | paper white cockpit | `html[data-term=daylight]` |
| NO_COLOR | `nocolor` | no | shapes only: every state still reads | `html[data-term=nocolor]` |

The page bar reads: "terminal theme", then the three buttons, then "compare: p this · o previous". The simulator has the same three buttons and a `/theme` console with the same three rows. Its speed buttons are 1×, 2× and 4×.

The page and the simulator carry identical token values. This was checked by diffing both files.

The page saves the chosen theme in `localStorage` under the key `hoo-term`, and restores it on the next load. The page has one inline script. It handles the theme buttons, that storage, and the `p` and `o` keys. The simulator does not save its theme.

Fonts: the page loads Orbitron (600, 700), Space Grotesk (400, 500, 600) and JetBrains Mono (400, 700) from Google Fonts.

### 2.2 Page chrome tokens (`:root`)

These colour the page, not the terminal. They do not change with the theme.

| Token | Value |
|---|---|
| `--pg-bg` | `#05070f` |
| `--pg-ink` | `#e6ecff` |
| `--pg-dim` | `#8690ad` |
| `--pg-acc` | `#5ce1e6` |
| `--pg-line` | `#1a2036` |
| `--pg-line2` | `#111628` |
| `--pg-card` | `#0b1020` |
| `--sans` | `"Space Grotesk",system-ui,sans-serif` |
| `--display` | `"Orbitron","Space Grotesk",system-ui,sans-serif` |
| `--mono` | `"JetBrains Mono",ui-monospace,"SF Mono",Menlo,Consolas,"DejaVu Sans Mono",monospace` |
| `--t-shadow` | `0 0 0 1px #ffffff05 inset,0 40px 80px -40px #5ce1e630` |
| `--t-glow` | `none` |

`--t-glow` is set to `none`. The `--sans`, `--display` and `--mono` values are font stacks.

### 2.3 Terminal tokens per theme

Every theme sets these custom properties. `(not set)` means the theme block does not set the token. Only `--t-shadow` shows `(not set)`, in deep-space and NO_COLOR. Daylight alone sets it.

| Token | deep-space (default) | daylight | NO_COLOR |
|---|---|---|---|
| `--t-bg` | `#070b18` | `#f7f8fc` | `#0b0c10` |
| `--t-edge` | `#1a2340` | `#dde1ee` | `#22252e` |
| `--ink` | `#e6ecff` | `#141a2e` | `#e6e6e6` |
| `--dim` | `#7d87a6` | `#5d6684` | `#8d8f96` |
| `--faint` | `#3a4462` | `#b0b6c9` | `#55575e` |
| `--acc` | `#5ce1e6` | `#0a8a91` | `#e6e6e6` |
| `--you` | `#9b8cff` | `#5b4bd1` | `#e6e6e6` |
| `--vio` | `#9b8cff` | `#5b4bd1` | `#e6e6e6` |
| `--ok` | `#7ee081` | `#2f8a3a` | `#e6e6e6` |
| `--err` | `#ff5c7a` | `#d23355` | `#e6e6e6` |
| `--warn` | `#ffb547` | `#b26a00` | `#e6e6e6` |
| `--gold` | `#ffd76a` | `#b88a00` | `#e6e6e6` |
| `--mag` | `#ff7ad9` | `#b0307e` | `#e6e6e6` |
| `--ghost` | `#1e2640` | `#dfe2ec` | `#33353b` |
| `--kw` | `#b7a8ff` | `#5b4bd1` | `#e6e6e6` |
| `--str` | `#7ee081` | `#2f8a3a` | `#e6e6e6` |
| `--ty` | `#ffd76a` | `#9a6a00` | `#e6e6e6` |
| `--num` | `#ffd76a` | `#9a6a00` | `#e6e6e6` |
| `--com` | `#56607e` | `#a0a7bd` | `#8d8f96` |
| `--fn` | `#8af0f3` | `#0a6f75` | `#e6e6e6` |
| `--bU` | `#141432` | `#ecebfb` | `transparent` |
| `--bS` | `#1a2342` | `#e2e6f4` | `#33353b` |
| `--bA` | `#0e2418` | `#e3f3e2` | `transparent` |
| `--bD` | `#2b1020` | `#fbe3e8` | `transparent` |
| `--bA2` | `#225a2c` | `#bfe6be` | `#33353b` |
| `--bD2` | `#5e1a2c` | `#f5bccb` | `#33353b` |
| `--bW` | `#0b1124` | `#eff1f8` | `transparent` |
| `--bE` | `#2b1020` | `#fbe3e8` | `transparent` |
| `--bY` | `#4d3a0d` | `#fae3a3` | `#e6e6e6` |
| `--bH` | `#0d1328` | `#eceff7` | `transparent` |
| `--bP` | `#0d1328` | `#eceff7` | `transparent` |
| `--bM` | `#121a33` | `#e6e9f3` | `transparent` |
| `--t-shadow` | `(not set)` | `0 30px 60px -36px #0003` | `(not set)` |

### 2.4 What each token is used for

Class names come from the page CSS (lines that set `.ink`, `.kAcc` and so on). Usage comes from the simulator script. Where a class is CSS only, the table says so.

| Class | Token | Used for |
|---|---|---|
| `.ink` | `--ink` | main text: names, commands, prompt text, selected row |
| `.dim` | `--dim` | stats, targets, descriptions, action words in hints |
| `.faint` | `--faint` | `·` separators, `┊` rails, placeholder text, empty gauge cells, READY frame |
| `.acc` | `--acc` | running `◐`, IN FLIGHT frame and title, elapsed time while running, `›` selected marker, `◭` mark, input glyph while flying |
| `.you` | `--you` | mission header `▲ M1`, console frames (the ones that take keys) |
| `.vio` | `--vio` | agent `◈` and `┃` rail, thinking `✻`, PLAN chip, `≋` jettison |
| `.ok` | `--ok` | done `●`, LANDED, `+` counts in diffs, `● flying`, `● in use` |
| `.err` | `--err` | failed `⊗`, `HIT`, removed lines `−`, COMMS LOST, DID NOT LAND |
| `.warn` | `--warn` | needs you `◉`, clearance and question cards, holding text, skill findings `▲`, REACH `■` |
| `.gold` | `--gold` | step numbers (`01`), the default hull paint (parity) |
| `.hull` | `--hull` | text in the hull paint colour. Set by `/color` on `#term` |
| `.kw`, `.ty`, `.fn` | `--kw`, `--ty`, `--fn` | syntax in the code sample of an answer |
| `.str`, `.num`, `.com` | `--str`, `--num`, `--com` | defined; the simulator does not use them |
| `.mag` | `--mag` | defined; the simulator does not use it |
| `.bU` | `--bU` | user band: mission header, `!` command line |
| `.bS` | `--bS` | selected row band; background of `.kDim` chips |
| `.bW` | `--bW` | code block; command line on the clearance card |
| `.bH` | `--bH` | HUD row; SCAN and REWIND bars |
| `.bM` | `--bM` | toast box |
| `.bA2`, `.bD2` | `--bA2`, `--bD2` | highlight of changed characters in an Edit diff line |
| `.bTint` | (CSS, uses `--hull`) | HUD background after a hull paint is set |
| `.bA`, `.bD`, `.bE`, `.bP`, `.bY` | tokens | defined; diff bands and other fills. The simulator uses none of them directly |
| `.kAcc` | `--acc` bg, `--t-bg` text, bold | MODE chip in BUILD; bound key caps |
| `.kVio` | `--vio` bg | PLAN chip; REWIND bar label; the active layer on the keys console |
| `.kWarn` | `--warn` bg | SCAN bar label |
| `.kErr` | `--err` bg | `HIT` chip; COMMS LOST chip |
| `.kOk` | `--ok` bg | `NEW` chip on a new file |
| `.kDim` | `--bS` bg, `--ink` text | unbound key caps such as `y`, `a`, `enter` |
| `.kYou`, `.kInk` | `--you`, `--ink` bg | CSS only; the simulator does not use them |
| `.cur` | `--ink` bg, `--t-bg` text | cursor cell (blinks) |
| `.ghost` | `--ghost` | CSS only. The simulator draws the suggestion as a faint placeholder |
| `.uc`, `.u` | `--err`, underline | `.uc` wavy underline (CSS only). `.u` underlines the matched part in palette rows |

### 2.5 NO_COLOR rules

The NO_COLOR theme uses one foreground colour, `#e6e6e6`, and very few background colours. It keeps shapes, not hues.

- All status and accent tokens (`--acc`, `--you`, `--vio`, `--ok`, `--err`, `--warn`, `--gold`, `--mag`, syntax tokens) are `#e6e6e6`.
- `--dim` is `#8d8f96` and `--faint` is `#55575e`, so secondary text stays dim.
- Bands are transparent, except `--bS`, `--bA2` and `--bD2` (`#33353b`) and `--bY` (`#e6e6e6`). Chips keep their filled background, with the dark text colour.
- Two CSS rules force inline colours back to ink and inline backgrounds to transparent inside `.r` rows:

```css
html[data-term=nocolor] .r [style*="color:#"]:not([style*="#f4f4f2"]){color:var(--ink)!important}
html[data-term=nocolor] .r [style*="background:#"]:not([style*="#f4f4f2"]){background:transparent!important}
```

- The exception `#f4f4f2` is the QR code's light background, so the QR keeps its light background and dark modules in every theme.
- The simulator says the theme is "shapes only: every state still reads". State is carried by the marks in section 3 (`○ ◐ ● ⊗ ◉ ⊘`), the chip text (`HIT`, `NEW`, `COMMS LOST`), and the frame titles.

### 2.6 Colours in the previous proposal and the decisions doc

The "this" view uses tokens, so its colours change with the theme. The previous view (`.vo`) and `tui-surfaces.md` use fixed hex values. Those are listed here for comparison.

| Role in tui-surfaces.md | Hex there | Deep-space token (default theme) |
|---|---|---|
| done | `#b5bd68` | `--ok` `#7ee081` |
| failed | `#d58181` | `--err` `#ff5c7a` |
| now, accent | `#5cc8bb` | `--acc` `#5ce1e6` |
| needs you | `#ffff00` | `--warn` `#ffb547` (amber in the page text) |
| you have the keys (frames) | `#6e93ff` | `--you` `#9b8cff` |
| prompt frame | `#757575` | `--faint` `#3a4462` |
| dim | `#989898` | `--dim` `#7d87a6` |
| muted | `#a8a8a8` | no token of that value |

Unclear in source whether the hex values in `tui-surfaces.md` were meant as the deep-space values. They are not the same. See section 8.

### 2.7 Theme objects in the simulator

The simulator script defines:

```js
const THEMES = [['deep', 'deep-space', 'the default · navy void'], ['daylight', 'daylight', 'paper white cockpit'], ['nocolor', 'NO_COLOR', 'shapes only: every state still reads']];
```

`setTheme(name)` sets `data-term` on `<html>` and updates the `aria-pressed` flags on the buttons. In the `/theme` console, moving the selection previews the theme live. `enter` keeps it. `esc` reverts to the theme that was active when the console opened.

Hull paint (`/color`) is separate from the theme. It sets the CSS variable `--hull` on `#term`. It uses `--gold` by default, and the cycle is: none, parity (`--gold`), cyan (`--acc`), violet (`--vio`), lime (`--ok`), red (`--err`), none.

## 3. Glyphs and marks

Every mark is one terminal cell. The circles are the status marks. Shapes differ, so the marks still read under NO_COLOR.

### 3.1 Status marks

Names and tokens come from the simulator's `MK` table. The page uses the same glyphs.

| Mark | State | Simulator key | Token | Where it is used |
|---|---|---|---|---|
| `○` | not started, next, queued | `next` | `faint` | next step in a mission (`02 ○ Shell … next`); queued notes (`○ keep it short`); flight-plan waypoints not yet reached; thinking level off (`○ off`) |
| `◐` | running | `run` | `acc` | running step, running agent or satellite; the IN FLIGHT title; the current flight-plan waypoint; waiting for docking (`◐ waiting for docking clamps…`). The simulator does not animate it. Only the elapsed time moves. |
| `●` | done | `done` | `ok` | done step; done agent; done waypoints; LANDED; `● flying` in the hangar; `● in use` in the hull list |
| `⊗` | failed | `fail` | `err` | failed step with a `HIT` chip; a hail tick on a Model block; COMMS LOST; DID NOT LAND; failed rows in radar; `⊗ failed` on the border in the previous view |
| `◉` | needs you | `hold` | `warn` | step waiting for clearance (`holding for clearance`); question waiting for an answer (`holding for you`); the clearance card title; the question card title; the end-of-shift card title |
| `⊘` | cancelled, denied, aborted | `deny` | `dim` | denied step (`denied · “reason”`); ABORTED in the debrief and the comms title; an aborted mission in radar |

The decisions doc says `⊘` means cancelled. The simulator uses it for denied and aborted. The source does not say whether cancelled and aborted are meant to be the same state.

### 3.2 Kind, identity and layout marks

| Mark | Meaning | Where it is used |
|---|---|---|
| `▲` | mission marker | the mission header `▲ M3 <prompt>`; also a skill finding line (`▲ wrap_text has no doc comment.`), in `warn` |
| `✦` | skill | skill chip `✦ review` on the mission header; `✦ 3 skills` on the start line; `✦ skill · review a file` in the command list |
| `◈` | agent | `◈ explore · Find the footer code`; HUD `◈ 1 agent`; start line `◈ 5 agents`; agents in flight on `/plan` |
| `⌘` | command count | start line `⌘ N commands` |
| `✻` | thinking | `✻ 4s` on a Think row; `✻ thought 3s · gist` in radar |
| `⌁` | satellite (background job) | HUD `⌁ 1 sat`; `⌁ sat job-1 · in orbit` on the step row |
| `◭` | the hoocode mark | HUD, left end. Also the page's philosophy heading |
| `≋` | jettison (compaction) | `≋≋≋ JETTISONED …` row |
| `⌕` | scan (search) | `⌕ SCAN <query>` bar |
| `✂` | stage separation (fork) | `✂ separation` line in `/fork` |
| `◂` | back, contact, or read-only | `◂ contact` on a scan match; `◂ READ-ONLY · this agent's own log`; `◂ hull paint` in the session-colour view |
| `◂◂` | rewind | `◂◂ REWIND 43–61 of 78 · 55%` bar above the prompt |
| `✎` | editing a command | clearance card, after `e`, while the command is edited |
| `▸` | input prompt | the input row (`▸ name your mission`). The previous proposal used `❯` |
| `›` | selected row | the highlighted row of a picker or console; the selected mission in the log |
| `…` | truncation or fold | cut text (`…`); `… N more` folds |
| `·` | separator | between every pair of fields on a line |
| `↑` `↓` | tokens in, tokens out | comms foot (`↑100 ↓20`); keys in hints (`↑↓`) |
| `↔` | toggle | `peek ↔ flight log` in the keys console |
| `→` | answer or result | `→ answer` on an answered question; `→ boom` on a radar hit line |
| `+` `−` | diff added, removed | `+1 −1` on an Edit header; `+` and `−` prefixes on diff lines |
| `✕` | hits count | radar column `0✕`, `1✕` |
| `■` `□` | reach on, reach off | REACH line on the clearance card. The simulator draws `■` in `warn` when on and `□` in `faint` when off |
| `◔` `◑` `◕` | thinking levels | the HANGAR thinking dial. Low, medium, high. `○` is off. |
| `▰` `▱` | fuel gauge cells | HUD `FUEL` gauge, 8 cells, filled `▰` and empty `▱`. The page mock uses `▱` only. |
| `▔` | toast timer | the second row of a transmission toast. It drains as the toast times out. |
| `┊` | trajectory rail | every step row and prose line in peek |
| `┃` | agent rail (violet) | the sub-steps of an agent (`┃ 1.1 ●`) |
| `┃` `│` | scroll thumb, track | in a scrolled view, the right column shows `┃` for the thumb and `│` for the track (sim) |
| `━` `┄` | flight-plan track | done waypoints joined by `━━━`; waypoints to come joined by `┄┄┄` |
| `░` | progress track | the running call's bar in radar (`░░░░░░░░ 3s of ~6s`) |
| `▟ ▀ ▄ ▌ ▐ ▙ ▜ ▛` | wordmark and logo | the hoocode mark (`▟▀▀▀▀▀▙`, `▌▟▙ ▟▙▐`, `▜▄▄▄▄▄▛`) and the wordmark |
| `⋆` | decoration | one in the launch mock only. Unclear in source what it marks. |

### 3.3 Frames and hints

- **Heavy frame** `┏ ┓ ┗ ┛ ━ ┃` is the comms frame and every console (the simulator's `BS.heavy`). The top border holds the title (state or console name). A right label sits on the top border too (`app-edit`, or the hull name). The bottom border holds the foot text and the tokens.
- **Round frame** `╭ ╮ ╰ ╯ ─ │` is the transmission toast (`BS.round`).
- **Hints** use one format: keys in ink, action words dim, separated by `·` (the simulator's `H_` helper). Example: `enter next · esc close`.
- **Clipping.** A title or foot is clipped with `…` when it does not fit. The left part is cut first, then the right part is halved.

### 3.4 Glyph counts in the "this" views

Counts are for the `.vn` blocks of all 46 surfaces. Box-drawing characters, the logo blocks and the shape-only symbols are left out.

| Mark | Count | Surfaces |
|---|---|---|
| `·` | 236 | a1, a2, a3, a4, b1, b2, b3, b4, b5, b6, b7, r1, r2, r3, r4, r6, r7, r8, r9, r10, r11, c2, c3, c4, c5, c6, c7, c8, d1, d2, d3, d4, d5, d6, d7, d8, d9, d10, e1, e2, e3, e5, e6 |
| `●` | 53 | a3, b1, b2, b3, b4, b5, b6, b7, r1, r2, r3, r4, r6, r8, r9, r10, r11, c5, c6, c7, d1, d2, d3, d5, d7, d8, e4 |
| `◭` | 46 | a1, a2, a3, a4, b1, b2, b3, b4, b5, b6, b7, r1, r2, r3, r4, r5, r6, r7, r8, r9, r10, r11, c1, c2, c3, c4, c5, c6, c7, c8, d1, d2, d3, d4, d5, d6, d7, d8, d9, d10, e1, e2, e3, e4, e5, e6 |
| `↑` | 32 | a1, a2, a3, a4, b5, b6, r1, r2, r3, r4, r6, r7, r8, r9, r11, c2, c3, c4, c5, c6, c7, d1, d2, d3, d4, d5, d7, d8, e2, e4, e5, e6 |
| `↓` | 32 | a1, a2, a3, a4, b5, b6, r1, r2, r3, r4, r6, r7, r8, r9, r11, c2, c3, c5, c6, c7, d1, d2, d3, d4, d5, d7, d8, e2, e4, e5, e6 |
| `–` | 30 | a3, b1, c2, c6, e2, e3, e6 |
| `▸` | 28 | a1, a2, a3, a4, b5, b6, r1, r2, r3, r4, r6, r7, r8, r9, r10, r11, c4, c5, c7, d1, d2, d3, d4, d6, e1, e4, e5, e6 |
| `▲` | 26 | a2, a3, a4, b1, b2, b3, b4, b5, b7, r5, r6, r7, r9, c1, c2, c4, c5, c6, c7, d7, d8, e1, e6 |
| `◐` | 22 | a2, b5, r4, r6, r7, r9, c3, c4, c5, c6, c7, c8, d10 |
| `✕` | 15 | b6, r1, r2, r3, r4, r8, r10, r11, d1, d2 |
| `⊗` | 12 | b2, b6, r1, r3, e1 |
| `○` | 11 | r6, c1, c4, c5, c6, d3, d4 |
| `◉` | 9 | r5, c1, c2, c8, d4 |
| `◈` | 8 | a1, b5, r7, c3, e4 |
| `→` | 7 | a2, b6, r1, r3, d3, e6 |
| `›` | 7 | c6, d1, d2, d3, d4, d5, d6 |
| `◂` | 6 | c3, e2, e3, e5 |
| `■` | 5 | r5, c1, d4 |
| `□` | 5 | r5, c1 |
| `⌁` | 5 | r9, c7, c8 |
| `✦` | 4 | a1, b7, e4 |
| `≋` | 3 | e6 |
| `✻` | 2 | a2, r11 |
| `−` | 2 | b3 |
| `…` | 2 | b5, d10 |
| `⋆` | 1 | a1 |
| `⌘` | 1 | a1 |
| `◔` | 1 | d3 |
| `◑` | 1 | d3 |
| `◕` | 1 | d3 |
| `←` | 1 | d3 |
| `✂` | 1 | d8 |
| `↔` | 1 | d9 |
| `—` | 1 | d10 |
| `⌕` | 1 | e3 |

### 3.5 Symbols drawn as shapes

The simulator puts some symbols in SVG, so each fills its cell exactly. These are: `⊗ ⊘ ◉ ○ ● ◐ ◑ ◔ ◕`; the box-drawing set (`│ ┃ ┆ ┊ ┌ ┏ ┐ ┓ └ ┗ ┘ ┛ ├ ┝ ┠ ┣ ┤ ┨ ┫ ┬ ┯ ┳ ┴ ┷ ┻ ┼ ╋ ╎ ╭ ╮ ╯ ╰` and the thin line pieces `╴ ╵ ╶ ╷ ╸ ╹ ╺ ╻`); the block elements and shades (`▀ ▁ … █ ▉ ▊ ▋ ▌ ▍ ▎ ▏ ▐ ░ ▒ ▓ ▔ ▕ ▖ ▗ ▘ ▙ ▚ ▛ ▜ ▝ ▞ ▟`); and all 256 braille patterns, U+2800 to U+28FF. Horizontal rules (`─ ━ ┄ ┈ ╌`) are drawn as CSS lines. Other characters are plain text. The page's own rule is "every glyph is one terminal cell". Unclear in source whether this set is complete for the real TUI.

## 4. The 46 surfaces

Order and grouping follow the page: A, B, R, C, D, E. Each surface has its page caption, the Mission Control view as rendered text, the page notes, and one "Differs from previous" line. Text blocks are the `.vn` view. Trailing spaces are trimmed. The previous proposal view (`.vo`) is not copied here.

### A · Start and conversation

#### a1 · Start

Caption on the page: The first screen.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode                     SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 0%  CR $0.000

 ▟▀▀▀▀▀▙  hoocode                                   ✦                                          ·
 ▌▟▙ ▟▙▐  coding agent · v0.1.24                                      ·
 ▜▄▄▄▄▄▛  ~/src/hoocode                                                                 ⋆
                                                             ·
 SHIP mock-large   ✦ 3 skills  ◈ 5 agents  ⌘ 1 command   · AGENTS.md 27 tok/turn

  1  why does wrap_text ignore width    2  add a width check       /  commands   @  files   !  shell

┏ READY  app-edit ┓
┃ ▸  name your mission                                                                             ┃
┗ peek  ↑100 ↓20 ┛
```

Notes: **Launch screen.** The hoocode mark, then the ship (the model), skills, agents and the first keys. A one-row HUD runs across the top from here on: mode, folder, ship, fuel (context used) and credits spent.

Differs from previous: Adds the one-row HUD on top (mode, folder, ship, fuel, credits). The prompt sits in a 3-row heavy comms frame whose title says READY. The logo has ship, skills, agents and a key line under it. The previous had a plain box and a `BUILD · peek` footer line.

#### a2 · Thinking

Caption on the page: What the model thought before it answered.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode                     SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

 ▲ M1  review the wrap helper
  ·· ◐ Think   read wrap.rs → find callers → two checks                                        ✻ 4s
   ┊  Reading the wrapper and its callers.

┏ ◐ IN FLIGHT · step 1 · 4s · esc abort  app-edit ┓
┃ ▸                                                                                                ┃
┗ peek  ↑100 ↓20 ┛
```

Notes: Thinking gets the `··` slot: it isn't a tool call, so it takes no step number. The gist is one faint line.

Differs from previous: Thinking is a live Think row in the `··` slot, with its gist and time (`✻ 4s`) and no step number. The previous was one folded row, `✻ thought for 4s · ctrl+t`.

#### a3 · An answer with code

Caption on the page: Markdown in an answer.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode                     SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

 ▲ M2  review wrap_text
  01 ● Read    src/wrap.rs                                                                 30 lines
   ┊  Review of wrap_text
   ┊  – It splits on whitespace, so tabs and runs of spaces collapse into one break.
   ┊  – A word longer than width is never split; it gets a line of its own.
   ┊
   ┊   let mut lines = vec![String::new()];                                              rust
   ┊   for word in text.split_whitespace() {
   ┊       lines.last_mut().unwrap().push_str(word);
   ┊   }
   ┊
   ┊  The three cases are covered by the tests in src/wrap.rs.
  ● LANDED · 1 step · 0 hits · 3.1s · 1.5k tokens · $0.004

┏ ● LANDED · 3.1s · 1 step  app-edit ┓
┃ ▸                                                                                                ┃
┗ peek  ↑100 ↓20 ┛
```

Notes: **Every mission ends with one landing line:** steps, hits, time, tokens and cost. The comms frame says `LANDED`.

Differs from previous: Adds step 01 (`● Read`, 30 lines) and a LANDED line with steps, hits, time, tokens and cost. The code block gets a band and a language tag. The previous had no step row and no landing line.

#### a4 · Your shell command

Caption on the page: A command you ran with !.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode                     SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

 ▲ $  printf 'a\nb\n'                                            exit 0 · 4ms · no step, it's yours
   ┊  a
   ┊  b

┏ READY · shell mode  app-edit ┓
┃ ▸ !                                                                                              ┃
┗ peek  ↑100 ↓20 ┛
```

Notes: Your own `!` commands are yours, not the agent's steps, so they get no step number.

Differs from previous: Your `!` command gets a user band with its exit status and the words "no step, it's yours". The frame title reads READY · shell mode. The previous was a plain `! printf` line.

### B · Peek: the trajectory

#### b1 · Read and search

Caption on the page: File tools on the tool band; search plain.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode                     SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

 ▲ M1  how does wrapping work?
  01 ● Read    src/wrap.rs                                                          lines 1–3 of 42
   ┊          //! Word wrapping for the transcript and the editor.
   ┊
   ┊          /// Wrap `text` so that no line is wider than `width` columns.
  02 ● Search  wrap_text                                                            5 hits · 1 file
   ┊           1 //! Word wrapping for the transcript and the editor.
   ┊           3 /// Wrap `text` so that no line is wider than `width` columns.
   ┊          +3 more
```

Notes: **Steps are numbered.** Every tool call gets a gold step number on the trajectory line (`┊`), so how many steps a mission took is always readable straight off the screen.

Differs from previous: Each tool call is a numbered step, with its stats on the header row (`lines 1–3 of 42`, `5 hits · 1 file`). The previous showed `● Read src/wrap.rs 42 lines` and CodeSearch rows with `path:line` hits.

#### b2 · Shell

Caption on the page: A command that worked and one that failed.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode                     SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

 ▲ M2  run the two checks
  01 ● Shell   printf 'line %d\n' 1 2 3 4 5 6 7 8                           exit 0 · 8 lines · 0.4s
   ┊          line 6
   ┊          line 7
   ┊          line 8
  02 ⊗ Shell   echo boom; exit 3                                                 HIT  exit 3 · 0.1s
   ┊          boom
  ● LANDED · 2 steps · 1 hit · 0.5s · 0.8k tokens · $0.002
```

Notes: A failed call is a `HIT`: one red chip where the success meta would be, plus the exit code.

Differs from previous: A failed call gets a red `⊗` and a `HIT  exit 3` chip in place of its success stats. The mission ends with a LANDED line counting steps and hits. The previous had no landing line.

#### b3 · Edit and Write

Caption on the page: The same turn as the edit golden.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode                     SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

 ▲ M3  edit the app
   ┊  Editing.
  01 ● Edit    src/app.ts                                                                     +1 −1
   ┊          −   return `hello ${name}`;
   ┊          +   return `Hello, ${name}!`;
  02 ● Write   tools/util.py                                                               NEW  +12
   ┊          import sys
   ┊          def main(args: list[str]) -> int:
   ┊  Edited src/app.ts and wrote tools/util.py.
  ● LANDED · 2 steps · 0 hits · 1.2s · 2.5k tokens · $0.007
```

Notes: Diffs show only the changed lines and mark the changed characters. A new file gets a `NEW` chip.

Differs from previous: Diffs show only the changed lines, with `+1 −1` counts on the header. A new file gets `NEW +12` and a LANDED line. The previous showed numbered context lines with `+2 −2` and `… 3 more lines`. The counts differ for the same edit, and the source does not say why.

#### b4 · Write

Caption on the page: A new file.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode                     SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

 ▲ M4  add a backoff helper
   ┊  Adding the backoff helper.
  01 ● Write   src/backoff.rs                                                       NEW  +12 · rust
   ┊          use std::time::Duration;
   ┊          pub struct Backoff {
   ┊              pub base: Duration,
   ┊  Wrote src/backoff.rs with exponential backoff.
  ● LANDED · 1 step · 0 hits · 0.9s · 1.8k tokens · $0.005
```

Notes: Same shape as every other step: number, status, tool, path, then what it did.

Differs from previous: A new file gets a `NEW` chip and its line count. The first lines show with no line numbers. The previous showed numbered lines and `… 7 more lines`.

#### b5 · A subagent at work

Caption on the page: Another agent's words keep their rail.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode          ◈ 1 agent  SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

 ▲ M5  find the footer code
  01 ◐ Agent   ◈ explore · Find the footer code                                               1m12s
   ┊  ┃ 1.1 ● Grep    fn render                                          4 hits
   ┊  ┃ 1.2 ● Read    crates/hoocode-code-tui-widgets/src/status.rs        88 lines
   ┊  ┃          "The footer is built in footer.rs. It draws the model…"
   ┊  ┃ 1.3 ◐ Read    crates/hoocode-code-tui-widgets/src/chip.rs

┏ ◐ IN FLIGHT · step 1.3 · 1m12s · esc abort  app-edit ┓
┃ ▸                                                                                                ┃
┗ peek  ↑100 ↓20 ┛
```

Notes: **Agents keep their name.** A subagent runs on a violet rail, and its steps are numbered under the parent's step (`1.1`, `1.2`…).

Differs from previous: The subagent's steps are numbered 1.1, 1.2, 1.3 under the parent step, on a violet `┃` rail. The frame says IN FLIGHT · step 1.3. The previous used `→` arrows with no numbers.

#### b6 · Radar view

Caption on the page: After ctrl+o: one row per chain of calls.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode                     SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

 M1 ●  what does the wrap crate do?                                       1 step  0✕   0.8s  $0.003
        It wraps text greedily, one word at a time.
 M2 ●  review the wrap helper                                            4 steps  1✕   0.6s  $0.006
            ⊗ 04 Shell  echo boom; exit 3  → boom  HIT
            The three cases are covered by the tests in src/wrap.rs.

┏ ● LANDED · 0.6s · 4 steps  app-edit ┓
┃ ▸                                                                                                ┃
┗ radar · flight log  ↑100 ↓20 ┛
```

Notes: **Radar is the flight log.** One row per mission: steps, hits, time, credits. Only hits, holds and live calls get a row of their own.

Differs from previous: Radar: one row per mission (steps, `✕` hits, time, cost), and a hit shows as `⊗ 04 Shell … HIT`. The previous showed a chain summary, `Read 1 file · searched 1 · ran 2 commands`, with full bodies.

#### b7 · A skill

Caption on the page: After /skill:review check wrap.rs.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode                     SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

 ▲ M6  check wrap.rs                                                                      ✦ review
  01 ● Read    src/wrap.rs                                                                 30 lines
   ┊  ▲ wrap_text has no doc comment.
   ┊  ▲ current.len() counts bytes, not columns, so wide characters can overflow.
   ┊  ▲ No unit test covers wrap_text.
  ● LANDED · 1 step · 0 hits · 1.4s · 2.2k tokens · $0.006
```

Notes: A skill is a chip on the mission header.

Differs from previous: The skill is a `✦ review` chip on the mission header. Findings use `▲` and the step is numbered 01. The previous used `-` bullets under `skill review · 7 lines · in the pager`.

### R · Radar: the flight log

#### r1 · One call each

Caption on the page: Six calls, each after a line of text.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode                     SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

 M1 ●  what does the wrap crate do?                                       1 step  0✕   0.8s  $0.003
        It wraps text greedily, one word at a time.
 M2 ●  edit the app                                                      2 steps  0✕   1.2s  $0.007
        Edited src/app.ts and wrote tools/util.py.
 M3 ●  tighten wrap and check it                                         6 steps  1✕   0.9s  $0.011
            ⊗ 06 Shell  echo boom; exit 3  → boom  HIT
            Read, edited, wrote a width test and searched; the last check failed as expected.

┏ ● LANDED · 0.9s · 6 steps  app-edit ┓
┃ ▸                                                                                                ┃
┗ radar · flight log  ↑100 ↓20 ┛
```

Notes: Three missions read as a log. The one with a hit shows the step that missed.

Differs from previous: Radar, not a transcript: each mission is one row with its last line of prose. Only the hit shows its step (`⊗ 06 Shell`). The previous printed every call in full, with its output.

#### r2 · A chain

Caption on the page: Five calls with no text between them.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode                     SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

 M1 ●  what does the wrap crate do?                                       1 step  0✕   0.8s  $0.003
 M2 ●  read the wrap module and run two checks                           5 steps  0✕   0.6s  $0.011
        Both checks passed and the module reads cleanly.

┏ ● LANDED · 3.1s · 5 steps  app-edit ┓
┃ ▸                                                                                                ┃
┗ radar · flight log  ↑200 ↓40 ┛
```

Notes: Five steps, no hits: one row.

Differs from previous: Five calls fold into one mission row, `5 steps 0✕`. The previous used a chain row, `Read 3 · Shell 2`, with `5 calls · 30 lines`.

#### r3 · A chain with a failure

Caption on the page: Four calls; the third fails.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode                     SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

 M3 ●  now run three checks, the third one fails                         4 steps  1✕   0.4s  $0.008
            ⊗ 03 Shell  echo boom; exit 3  → boom  HIT
            The third check failed with exit 3; the other calls ran.

┏ ● LANDED · 2.2s · 4 steps  app-edit ┓
┃ ▸                                                                                                ┃
┗ radar · flight log  ↑100 ↓20 ┛
```

Notes: The hit names its step number (`03`).

Differs from previous: The failed call shows its step number (`03`) and a `HIT` chip on the mission row. The previous showed a chain row, `Shell 3 · Read 1`, and the failed call's body.

#### r4 · Running

Caption on the page: A slow command.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode                     SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

 M1 ●  what does the wrap crate do?                                       1 step  0✕   0.8s  $0.003
 M2 ◐  run a slow check                                                   1 step  0✕     4s  $0.001
        ◐ 01 Shell  sleep 6; echo done  ░░░░░░░░ 3s of ~6s

┏ ◐ IN FLIGHT · step 1 · 4s · esc abort  app-edit ┓
┃ ▸                                                                                                ┃
┗ radar · flight log  ↑100 ↓20 ┛
```

Notes: A mission in flight shows `◐` and the live step, with its progress estimated from its last run.

Differs from previous: The running call is one row with a `░` progress bar and `3s of ~6s`. The frame reads IN FLIGHT. The previous showed a `working · 4s · esc stop` frame and a `▱` fuel line.

#### r5 · Waiting on you

Caption on the page: A command needs approval.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode                     SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

 ▲ M1  how long is the wrap module
  01 ◉ Shell   wc -l src/wrap.rs                                              holding for clearance

┏ ◉ CLEARANCE NEEDED  BUILD asks for Shell ┓
┃ $ wc -l src/wrap.rs                                                                              ┃
┃ REACH   ■ read src/wrap.rs   □ write   □ network                                                 ┃
┃  y  go, once    a  always wc in BUILD    n  abort + say why    e  edit first                     ┃
┗ or type a reply ┛
```

Notes: **Clearance.** The comms frame turns into an amber card: the command, what it can reach, and four keys. You can also ignore the card and type a reply.

Differs from previous: Approval is an amber CLEARANCE card with the command, a REACH line (read, write, network, pipe) and four keys. The previous used a rounded three-choice picker.

#### r6 · Plan, then waiting for the model

Caption on the page: Both calls are done; the model is still writing.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode                     SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

 ▲ M1  plan the wrap helper
  01 ● Plan    5 waypoints                                                                     0.1s
  02 ● Shell   wc -l src/wrap.rs                                                               0.1s
   ┊  FLIGHT PLAN  ●●◐○○  2/5  Adding a word-wrap helper

┏ ◐ IN FLIGHT · step 3 · 9s · writing · esc abort  app-edit ┓
┃ ▸                                                                                                ┃
┗ radar · flight log  ↑100 ↓20 ┛
```

Notes: The flight plan is a track of waypoints. The comms title shows which step the ship is on.

Differs from previous: The flight plan is a waypoint track (`●●◐○○ 2/5`) under the plan step. The frame reads IN FLIGHT · step 3 · writing. The previous had a `Plan 2/5` line above a working frame.

#### r7 · A subagent, running

Caption on the page: The Agent call is out working.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode          ◈ 1 agent  SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

 ▲ M1  Ask a subagent what 6 times 7 is.
  01 ◐ Agent   ◈ general-purpose · Compute a product                                             2s

┏ ◐ IN FLIGHT · step 1 · 2s · esc abort  app-edit ┓
┃ ▸                                                                                                ┃
┗ radar · flight log  ↑100 ↓20 ┛
```

Notes: The HUD counts agents in flight.

Differs from previous: A running agent is one radar row, `◈ general-purpose · …`, and the HUD shows `◈ 1 agent`. The previous used a separate `Agents 1` task line and `◈ 1 running` in the footer.

#### r8 · A subagent, done

Caption on the page: The Agent call came back.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode                     SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

 M1 ●  Ask a subagent what 6 times 7 is.                                  1 step  0✕   2.1s  $0.001
        The subagent says 6 times 7 is 42.

┏ ● LANDED · 3.1s · 1 step  app-edit ┓
┃ ▸                                                                                                ┃
┗ radar · flight log  ↑200 ↓40 ┛
```

Notes: An agent's steps count toward the mission's steps.

Differs from previous: The finished agent is one step in its mission row, and the frame reads LANDED. The previous showed `● Agent … 2.1s` and a `done · 3.1s` frame.

#### r9 · A background job

Caption on the page: A Shell call left running.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode            ⌁ 1 sat  SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

 ▲ M1  start a long sleep in the background
  01 ◐ Shell   sleep 60                                                   ⌁ sat job-1 · in orbit 4s
   ┊  Running in the background as job-1; a transmission arrives when it lands.

┏ ● LANDED · 2.4s · 1 step  app-edit ┓
┃ ▸                                                                                                ┃
┗ radar · flight log  ↑200 ↓40 ┛
```

Notes: **Satellites.** A background job stays in orbit, and the HUD shows `⌁ 1 sat` until it lands.

Differs from previous: A background job shows as a satellite, `⌁ sat job-1 · in orbit`, and the HUD shows `⌁ 1 sat`. The previous showed a `Shell 1` task line and a `done` frame. The mock frame reads LANDED while job-1 is still in orbit; unclear in source whether that is intended.

#### r10 · 80 columns

Caption on the page: A long command and a deep path at 80 columns.  
Width: 80 columns.

```text
 ◭ HOOCODE   BUILD                                     FUEL ▱▱▱▱▱▱▱▱  CR $0.051

 M1 ●  check the long names                          2 steps  0✕   0.3s  $0.003
            Both finished: the helper measures the widest line, and the
            counts are above.

┏ ● LANDED · 1.4s · 2 steps  app-edit ┓
┃ ▸                                                                            ┃
┗ radar ┛
```

Notes: At 80 columns the HUD drops the folder and ship and keeps fuel and credits.

Differs from previous: At 80 columns the HUD keeps only mode, fuel and credits, and drops the folder and the ship. The previous kept the folder in its footer.

#### r11 · Thinking hidden

Caption on the page: A turn with thinking, in radar.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode                     SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

 M1 ●  review the helper with thinking on                                2 steps  0✕   3.4s  $0.009
            ✻ thought 3s · The helper is 13 lines, splits on whitespace, and needs no change.

┏ ● LANDED · 3.4s · 2 steps  app-edit ┓
┃ ▸                                                                                                ┃
┗ radar · flight log  ↑100 ↓20 ┛
```

Notes: Thinking time is shown, but it isn't a step.

Differs from previous: Thinking shows as a `✻ thought 3s` row under the mission, with its gist. The previous showed the reply text only.

### C · Needs you, and while it works

#### c1 · Approval

Caption on the page: The agent wants to run a command.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode                     SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

 ▲ M1  run the commands
   ┊  Running them.
  01 ◉ Shell   printf 'alpha\nbeta\ngamma\n' | sort -r                        holding for clearance
  02 ○ Shell   echo boom; exit 3                                                               next

┏ ◉ CLEARANCE NEEDED  BUILD asks for Shell ┓
┃ $ printf 'alpha\nbeta\ngamma\n' | sort -r                                                        ┃
┃ REACH   □ read   □ write   □ network   ■ pipe 2 programs                                         ┃
┃  y  go, once    a  always printf in BUILD    n  abort + say why    e  edit first                 ┃
┗ or type a reply ┛
```

Notes: The held step is `◉`. The next one is faint, so you know clearing one doesn't clear both.

Differs from previous: Approval is an amber CLEARANCE card: the command, a REACH line and four keys (y, a, n, e). The held step is ◉ and the next step is ○. The previous used a rounded three-choice picker.

#### c2 · A question

Caption on the page: The agent asks you to choose.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode                     SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

 ▲ M1  add retries to the provider wrapper
   ┊  Before I write the retry loop I need one decision.
  01 ◉ Ask     1 question                                                           holding for you

┏ ◉ MISSION CONTROL ASKS  sets the default for every call ┓
┃ Which retry strategy should the provider wrapper use?                                            ┃
┃                                                                                                  ┃
┃  1  exponential backoff   doubling delay, capped at 60 s                           recommended   ┃
┃  2  fixed delay           same wait between every attempt                                        ┃
┃  3  no retry              fail on the first error                                                ┃
┃  4  something else        type it                                                                ┃
┗ 1–4 answer · ↑↓ move · esc skip ┛
```

Notes: Mission control's questions use the same amber card. Number keys answer.

Differs from previous: Questions use the same amber card, with four numbered answers and the recommended one tagged. The previous was a rounded picker with a description line.

#### c4 · Working, notes queued

Caption on the page: You typed while it works.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode                     SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

 ▲ M1  explain the wrap module
  01 ◐ Read    src/wrap.rs                                                                       6s

┏ ◐ IN FLIGHT · step 1 · 6s · esc abort  app-edit ┓
┃ ○ keep it short                                                 queued · sends after this step   ┃
┃ ○ then list the tests                                        queued · becomes M2 after landing   ┃
┃ ▸                                                                                                ┃
┗ peek · alt+↑ edit queue ┛
```

Notes: Queued notes sit inside the comms frame. The second one says it will become mission M2.

Differs from previous: Queued notes sit inside the comms frame, one row each, with their timing (`sends after this step`, `becomes M2 after landing`). The previous put them in the working box with `sends when this turn ends`.

#### c5 · Plan while it works

Caption on the page: The agent made a plan and runs a step.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode                     SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

 ▲ M2  add a word-wrap helper
   ┊  FLIGHT PLAN  ●●◐○○  2/5  Adding a word-wrap helper
  03 ● Shell   wc -l src/wrap.rs Cargo.toml                                           exit 0 · 0.1s
   ┊           30 src/wrap.rs    7 Cargo.toml    37 total

┏ ◐ IN FLIGHT · step 4 · 9s · esc abort  app-edit ┓
┃ ▸                                                                                                ┃
┗ peek  ↑100 ↓20 ┛
```

Notes: The plan is one line in the trajectory.

Differs from previous: The flight plan is a row in the trajectory, not a footer line, and the frame title shows step 4. The previous had a `Plan 2/5` line above a working frame.

#### c6 · The task panel

Caption on the page: After ctrl+g.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode                     SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

 ▲ M2  add a word-wrap helper

┏ FLIGHT PLAN 2/5  agents 0 · sats 0  /tasks · ctrl+g ┓
┃ › ● Read the wrap module                                                    steps 01–01 · 0.4s   ┃
┃   ● Check Cargo.toml dependencies                                           steps 02–03 · 0.2s   ┃
┃   ◐ Adding a word-wrap helper                                                     step 04 · 9s   ┃
┃   ○ Write unit tests for the helper                                                              ┃
┃   ○ Update the changelog                                                                         ┃
┗ ↑↓ move · enter jump to its steps · x drop · esc back ┛
```

Notes: Each waypoint lists the steps it used (`steps 02–03`).

Differs from previous: Ctrl+g opens the flight plan as a heavy console. Each waypoint shows its steps (`steps 02–03`) and its time. The previous was a rounded `Plan 2/5` box.

#### c7 · A background job

Caption on the page: A Shell call left running.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode            ⌁ 1 sat  SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

 ▲ M1  start a long sleep in the background
  01 ◐ Shell   sleep 60                                                   ⌁ sat job-1 · in orbit 4s
   ┊          log jobs/job-1.log · silent so far · ctrl+g to watch
   ┊  Running in the background as job-1; a transmission arrives when it lands.

┏ ● LANDED · 2.4s · 1 step  app-edit ┓
┃ ▸                                                                                                ┃
┗ peek  ↑200 ↓40 ┛
```

Notes: The satellite's log path is relative, and the row says it's silent so far.

Differs from previous: The log line says `silent so far`, and the HUD shows `⌁ 1 sat`. The previous had a `Shell 1` task line and a `done` frame.

#### c8 · Quitting with jobs running

Caption on the page: After ctrl+c twice.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode            ⌁ 1 sat  SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

   ┊  Running in the background as job-1; a transmission arrives when it lands.

┏ ◉ END SHIFT WITH 1 SATELLITE UP?  esc stay ┓
┃  ◐ job-1  sleep 60                                                        in orbit 4s of ~60s    ┃
┃                                                                                                  ┃
┃   k  bring it down, then end shift     l  leave it in orbit · log stays in jobs/job-1.log        ┃
┗┛
```

Notes: Ending the shift with something still up is a clearance card like any other.

Differs from previous: Quitting with a job running is an amber END SHIFT card with two keys: k (bring it down, then end shift) and l (leave it in orbit). The previous was a two-choice picker.

#### c3 · Watching a subagent

Caption on the page: Attached to a running subagent.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode◈ INSIDE AGENT  SHIP general-purpose  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

 ◈ general-purpose  Compute a product                                    launched from M1 · step 01
   ┊  task: what is 6 times 7? reply with the number only
  ·· ◐ Think                                                                                     2s

┏ ◂ READ-ONLY · this agent's own log  app-edit ┓
┃ ◂  esc back to the main ship                                                                     ┃
┗ peek  ↑100 ↓20 ┛
```

Notes: **Inside the agent.** The HUD says so, the header names where it launched from, and the comms frame is read-only.

Differs from previous: Inside an agent the HUD says INSIDE AGENT, the header names the launch step, and the frame is read-only. The previous showed `1h60m left`, the rounding bug in tui-surfaces.md U0.4.

### D · Lists: the consoles

#### d1 · Slash commands

Caption on the page: Typing /sess.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode                     SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

 M1 ●  what does the wrap crate do?                                       1 step  0✕   0.8s  $0.003

┏ CONSOLE · commands  2 of 41 ┓
┃ › /session                                                              session info and stats   ┃
┃   /subagent-stats                                           agent reliability · 24h · 7d · all   ┃
┃ ▸ /sess                                                                                          ┃
┗ ↑↓ pick · tab complete · enter run ┛
```

Notes: Every list is a heavy-framed console that replaces the comms frame, with its real command in the title.

Differs from previous: The command list is a heavy console with the command name in its title, a match count, and a right-aligned description for each command. The previous was a rounded box with a plain list.

#### d2 · @ files

Caption on the page: Typing @wr.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode                     SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

 M1 ●  what does the wrap crate do?                                       1 step  0✕   0.8s  $0.003

┏ CARGO · @ files  2 matches ┓
┃ › src/wrap.rs                                                         30 lines · touched in M1   ┃
┃   tests/wrap_tests.rs                                                                 41 lines   ┃
┃ ▸ look at @wr                                                                                    ┃
┗ ↑↓ pick · tab load · / into folder ┛
```

Notes: Files show which mission last touched them.

Differs from previous: @ files is a console. Files a mission touched are tagged `touched in M1`. The previous was a plain list.

#### d3 · Model

Caption on the page: /model.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode                     SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

┏ HANGAR · /model  3 ships signed in ┓
┃ ▸   filter ships                                                                                 ┃
┃                                                                                                  ┃
┃   ship          provider   context   price in / out per 1M                                       ┃
┃ › mock-large    mock       200k      $3.00 / $15.00                                   ● flying   ┃
┃   mock-fast     mock       128k      $0.45 / $1.80                                               ┃
┃   mock-local    ollama     32k       free · runs on this machine                                 ┃
┃                                                                                                  ┃
┃ thinking   ○ off   ◔ low  ◑ medium  ◕ high                                                       ┃
┗ ↑↓ ship · enter fly it · ←→ thinking · esc close ┛
```

Notes: **The hangar.** Ships with their provider, context window and price, plus the thinking dial.

Differs from previous: The model picker is the HANGAR: three ships with provider, context and price, the flying ship marked `● flying`, and a thinking dial. The previous was one row, `mock-model · current`.

#### d4 · Settings

Caption on the page: /settings.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode                     SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

┏ SYSTEMS · /settings  5,805 tok/turn ┓
┃ ▸   search 41 settings                                                                           ┃
┃                                                                                                  ┃
┃ › Tools                   5 on │ LAUNCH WEIGHT every turn carries 5,805 tokens                   ┃
┃   Tool output             peek │           ┃
┃   Models                   all │ ■ prompt 2,522  ■ core 1,904  ■ search 861                      ┃
┃   Context                    3 │                                                                 ┃
┃   Behavior                   6 │ ◉ core     read · edit · shell                                  ┃
┃   Interface                  8 │ ◉ search   grep · code search                                   ┃
┃   Plugins                    3 │ ○ web      off · would add 740                                  ┃
┃   + 3 more                     │                                                                 ┃
┗ ↑↓ move · space toggle · enter open · esc close ┛
```

Notes: Launch weight: the tokens every turn carries before you type, split by what adds them.

Differs from previous: Settings is the SYSTEMS console. A launch-weight panel splits the tokens each turn adds by source, and each tool group shows on or off. The previous was a rounded list of sections.

#### d5 · Theme

Caption on the page: /settings › Interface › Theme.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode                     SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

┏ HULL · /theme  8 themes ┓
┃ ›   deep-space        the default · navy void                                 ● in use   ┃
┃     colorsafe-dark    Okabe-Ito, no red/green                                            ┃
┃     colorsafe-light   blue, violet, magenta                                              ┃
┃     dark              the current TUI                                                    ┃
┃     light             the current TUI, on paper                                          ┃
┃     solarized-dark    sixteen canonical colours                                          ┃
┃     vox-cutout-dark   paper on newsprint black                                           ┃
┃     vox-cutout-light  flat stocks on cream                                               ┃
┗ ↑↓ preview live · enter keep · esc back ┛
```

Notes: Hull paints, each with four swatches from its palette.

Differs from previous: Theme is a HULL console with full one-line descriptions (the previous cut them mid-word) and a `● in use` marker. It lists deep-space and drops solarized-light, so it has eight rows.

#### d6 · Resume

Caption on the page: --resume.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode                     SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 0%  CR $0.000

┏ MISSION LOG · --resume   This folder   All   2 sessions ┓
┃ ▸   search the mission log                                                                       ┃
┃                                                                                                  ┃
┃   mission                                        ship          steps   hits   when               ┃
┃ › why does wrap_text ignore width                mock-large     5       1      now               ┃
┃   add a width check to wrap_text in src/wrap.rs  mock-fast      3       0      now               ┃
┗ tab scope · alt+o sort · alt+x delete · esc close ┛
```

Notes: The mission log shows which ship flew each mission, its steps and its hits.

Differs from previous: `--resume` is a MISSION LOG table with ship, steps, hits and time columns, and scope tabs. The previous was a rounded list of sessions with message counts.

#### d7 · Session tree

Caption on the page: /tree.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode                     SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

┏ FLIGHT PATH · /tree  2 nodes ┓
┃ ▲ M1  what does the wrap crate do?                                                             ┃
┃   ┃                                                                                              ┃
┃   ┗● It wraps text greedily, one word at a time.   YOU ARE HERE                       f fork   ┃
┗ ↑↓ move · enter go here · f fork · alt+l label ┛
```

Notes: The flight path as a graph, with a `YOU ARE HERE` marker.

Differs from previous: `/tree` is a FLIGHT PATH graph with a YOU ARE HERE marker and an `f fork` key. The previous was a list of you and agent rows.

#### d8 · Fork

Caption on the page: /fork.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode                     SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

┏ STAGE SEPARATION · /fork ┓
┃ Everything above the line launches as a new session; this one keeps flying.                      ┃
┃                                                                                                  ┃
┃ ▲ M1 what does the wrap crate do?                                carries 1 message · 0.2k fuel   ┃
┃ ✂  separation    ┃
┃   ● It wraps text greedily, one word at a time.                                                  ┃
┗ ↑↓ move the line · enter fork · esc close ┛
```

Notes: Stage separation: move the line, and everything above it launches as a new session.

Differs from previous: `/fork` is STAGE SEPARATION: a `✂` line sits between messages and says what the new session carries. The previous was a picker that copied the conversation up to one message.

#### d9 · Keys

Caption on the page: /hotkeys.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode                     SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

┏ CONTROLS · ?   plain   ctrl   alt   62 bindings ┓
┃ esc   1   2   3   4   5   6   7   8   9   0        ctrl layer                                    ┃
┃                                                    c clear · twice ends shift                    ┃
┃ tab   q   w   e   r   t   y   u   i   o   p        o peek ↔ flight log                           ┃
┃                                                    t thinking  g flight plan                     ┃
┃  ctrl  a   s   d   f   g   h   j   k   l           r scan the session                            ┃
┃                                                    esc abort, or go back                         ┃
┃ shift   z   x   c   v   b   n   m                                                                ┃
┗ tab layer · type search · esc close ┛
```

Notes: Controls on a drawn keyboard. The ctrl layer is lit.

Differs from previous: Keys are drawn on a keyboard with the ctrl layer lit, and the title counts the bindings (62). The previous was a list of rows ending in `… 52 more`.

#### d10 · Sign in

Caption on the page: /login.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode                              SHIP —  FUEL ▱▱▱▱▱▱▱▱ 0%  CR $0.000

┏ DOCKING · /login ┓
┃ Dock with a provider to fly.                         ▄▄▄▄▄▄▄ ▄     ▄▄      ▄▄▄▄▄▄▄               ┃
┃                                                       ▄▄▄  ▄ ▀▀▄ ▄ ▀▄  ▄▄▄                ┃
┃  1  subscription                                        ▀   ▄▄                    ┃
┃     scan, or enter to open the browser               ▄▄▄▄▄ ▀ ▄▀  ▀ ▄▄▄▄▄               ┃
┃                                                      ▄▄  ▄▄▄   ▀▄ ▀▄  ▄ ▄▄▄▄               ┃
┃  2  API key · kept in the OS keychain                ▄▄▄▄▀▄  ▄▄   ▄ ▀ ▄▀▀▀▀▀               ┃
┃                                                        ▀ ▄▄ ▀ ▄▀ ▄▀▀▀ ▀▀ ▀               ┃
┃ code WXYZ-1234 · 9:41 left                           ▀▀▄ ▄▄▄▀▄ ▄▀▄  ▀▀▀▀               ┃
┃ ◐ waiting for docking clamps…                        ▀▀▀▀ ▄▀▄ ▀▄  ▄ ▀▀   ▀               ┃
┃                                                      ▀▀ ▀  ▄▄▄▀   ▀▄▄▀▀▄ ▄ ▀▀▄▀               ┃
┃                                                      ▄▄▄▀▄▀▀  ▄▀▄▄▄▀▀▄▄ ▀               ┃
┃                                                      ▄▄▄▄▄▄▄ ▀▀▀▄▄  ▀ ▄ ▄▄                ┃
┃                                                       ▄▄▄  ▄▀▀▄ ▀ ▀▄▄▄▄                 ┃
┃                                                           ▄  ▀▀▀  ▄ ▀ ▄▄▄               ┃
┃                                                      ▄▄▄▄▄ ▀▀▄▀▄▀ ▀  ▄▄▄ ▀               ┃
┃                                                                                                  ┃
┗ 1 2 choose · enter open browser · c copy code · esc cancel ┛
```

Notes: Docking with a provider: a real QR code (it opens example.com), a code you can type, and a live wait.

Differs from previous: Sign-in is the DOCKING console: a real QR code (it opens example.com), a typed code with a timer, and `c copy code`. The previous was a two-choice picker.

### E · History and chrome

#### e1 · Model error

Caption on the page: The provider keeps failing.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode                     SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

 ▲ M1  summarize src/wrap.rs
  ·· ⊗ Model   529 upstream overloaded                                   COMMS LOST  6 hails · 6.2s
   ┊          ·⊗·⊗···⊗·······⊗··············⊗·····························⊗  backoff doubles

┏ ⊗ COMMS LOST · enter hail again · alt+m switch ship  app-edit ┓
┃ ▸                                                                                                ┃
┗ peek ┛
```

Notes: **Comms lost.** Six provider retries are hails on a time axis, with the gaps doubling. They are the provider's failures, not the agent's, so they aren't hits.

Differs from previous: Six provider retries show as hails on a time axis under one red Model block. The frame reads COMMS LOST with `enter hail again`. The previous was one block that gave up after six attempts, with a `failed · 6.2s` frame.

#### e2 · Scrolled back

Caption on the page: PgUp pins the view.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode                     SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

   ┊                                                                                               │
   ┊  Second answer.                                                                               │
   ┊                                                                                               │
   ┊  – beta item 1                                                                                │
   ┊  – beta item 2                                                                                │
   ┊  – beta item 3                                                                                │
   ┊  – beta item 4                                                                                ┃
   ┊  – beta item 5                                                                                ┃
   ┊  – beta item 6                                                                                ┃
   ┊  – beta item 7                                                                                ┃
   ┊  – beta item 8                                                                                ┃
   ┊  – beta item 9                                                                                │
   ┊  – beta item 10                                                                               │
   ┊  – beta item 11                                                                               │
   ┊  – beta item 12                                                                               │
  ◂◂ REWIND  43–61 of 78 · 55%  ↓ 3 new                         ↑↓ line · pgup pgdn page · esc live
```

Notes: Rewind, with a count of what's new below.

Differs from previous: The rewind row sits above the prompt: `◂◂ REWIND 43–61 of 78 · 55%`, with `↓ 3 new` and `esc live`. The previous put the range and the hints in the footer.

#### e3 · Search

Caption on the page: ctrl+r while scrolled back.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode                     SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

   ┊  – beta item 17
   ┊  – beta item 18
   ┊  – beta item 19
   ┊  – beta item 20
   ┊  – beta item 21
   ┊  – beta item 22
   ┊  – beta item 23
   ┊  – beta item 24
   ┊  – beta item 25  ◂ contact
   ┊  – beta item 26
  ⌕ SCAN  beta item 25                                   1 contact  enter lock · n next · esc clear
```

Notes: Scan: matches are contacts.

Differs from previous: Search is a SCAN bar above the prompt, and each match is a contact (`◂ contact`) in the scrollback. The previous used a `/` query line in the footer.

#### e6 · Compaction

Caption on the page: After /compact.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode                     SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 0%  CR $0.051

 ≋≋≋ JETTISONED M1–M3 into one summary stage · 120 → 144 tokens (grew 24) · enter read

 ▲ $  /compact                                                                                 1.2s

┏ READY · fuel recounts on the next mission  app-edit ┓
┃ ▸                                                                                                ┃
┗ peek  ↑100 ↓20 ┛
```

Notes: Jettison: earlier missions become one summary stage, and the numbers show it grew.

Differs from previous: Compaction is a JETTISONED row (`≋≋≋`) with the missions it folded and the token change (120 → 144, grew 24). The previous was a dim divider that said `compacted · ctrl+o summary`.

#### e4 · Notices

Caption on the page: After alt+z.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode                     SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 0%  CR $0.000
                                                       ╭ ● TRANSMISSION  shift+alt+z undo ╮
 ✦ 2 skills  ◈ 5 agents                                │ chrome is now compact                    │
                                                       │ ▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔ │
                                                       ╰╯
┏ READY  app-edit ┓
┃ ▸                                                                                                ┃
┗ peek  ↑100 ↓20 ┛
```

Notes: Transmissions float top-right with a draining bar and an undo key.

Differs from previous: A notice is a transmission toast at top right, with a draining bar and an undo key (`shift+alt+z undo`). The previous was a line in the transcript.

#### e5 · Session colour

Caption on the page: After /color.  
Width: 100 columns.

```text
 ◭ HOOCODE   BUILD   ~/src/hoocode             parity  SHIP mock-large  FUEL ▱▱▱▱▱▱▱▱ 4%  CR $0.051

 hull paint  ◂          parity · yellow
 the HUD, the comms frame and the terminal tab title all take it

┏ READY  parity ┓
┃ ▸                                                                                                ┃
┗ peek  ↑100 ↓20 ┛
```

Notes: Hull paint tints the HUD, the comms frame and the terminal tab title.

Differs from previous: The hull paint shows as `parity` in the HUD and as a `hull paint` line. The frame title and border take the paint colour. The previous had only a `Session color set` line in the transcript.

## 5. HUD, comms frame and layout

These rules come from the simulator's code, which draws the live version of the page's views. The page's own mock rules are noted where they differ.

### 5.1 The HUD (one row at the top)

Fields, in order, left to right (simulator `hudRow`):

Left group:

1. `◭` (bold, accent), a space, then `HOOCODE` (bold, ink).
2. Two spaces, then the mode chip: `BUILD` (accent chip) or `PLAN` (violet chip). One space is inside each side of the chip.
3. Two spaces, then the folder in dim text, for example `~/src/hoocode`.

Right group, in this order:

4. Live status, only when it applies: `◈ N agent` in violet while agents run; `⌁ N sat` in accent while satellites are up. (`◐ RACING` is in the code but never set.)
5. `SHIP` (faint) and the ship name in the ship's colour. mock-large is accent, mock-fast is violet, mock-local is dim.
6. `FUEL` (faint), then the gauge: `▰` for each filled cell and `▱` for each empty one, 8 cells in all. Then the percent in dim. The filled cells take one colour: accent below 70%, warn below 90%, err from 90% up.
7. `CR` (faint), then the money spent, for example `$0.051`. The running mission's cost is added in.

The row background is `--bH`. When a hull paint is set, it is the hull tint instead.

Fuel percent = input tokens used so far (finished missions plus the running one) ÷ 128,000, capped at 100. Every ship uses 128,000 in the code. See section 8.

Truncation (simulator `lr`, used for every two-sided row):

- The left part and right part are measured in cells. If they do not fit with one space to spare, the left part is cut first, with `…` at its end.
- If the right part still does not fit, it is cut from the right.
- The row is then padded to the full width.

The page's 80-column mock (r10) shows a different rule: at 80 columns the HUD drops the folder and the ship and keeps mode, fuel and credits. The simulator has a fixed width of 100 cells and no narrow layout. See section 8.

### 5.2 The comms frame (the bottom frame)

The comms frame is a heavy frame, 3 rows: top border, one input row, bottom border. Each queued note adds one row above the input row, inside the frame. Its colour and title come from the state (simulator `commsRows`).

| State | Title (top border, left) | Frame colour | Input row placeholder (when empty) |
|---|---|---|---|
| READY | `READY` (dim) | faint | `name your mission · / commands · @ files · ! shell` |
| IN FLIGHT | `◐ IN FLIGHT · step N · time · esc abort` | acc | `type to queue the next mission` |
| COMMS LOST | `⊗ COMMS LOST · enter hail again · /model switch ship` | err | `press enter to hail again` |
| LANDED | `● LANDED · time · N steps`. Only the title is green. | faint | the suggestion and `tab`, if one is set. Otherwise the READY placeholder. |
| DID NOT LAND | `⊗ DID NOT LAND · time · N steps` | faint | the suggestion and `tab`, if one is set. Otherwise the READY placeholder. |
| ABORTED | `⊘ ABORTED · READY` (dim) | faint | the suggestion and `tab`, if one is set. Otherwise the READY placeholder. |

The clearance card and the question card replace the comms frame while they wait. Their colour is warn.

- Clearance card title: `◉ CLEARANCE NEEDED · step NN`. Right label: `BUILD asks for Shell`.
- Question card title: `◉ MISSION CONTROL ASKS`. Right label: `sets the default for every call`.

Frame labels and foot:

- Right label on the top border: `app-edit` by default. After `/color`, the hull paint name (for example `parity`).
- Foot, left, on the bottom border: `peek` or `radar · flight log`. In PLAN mode, `PLAN · nothing gets changed` in violet. When queued notes exist, `alt+↑ edit queue`.
- Foot, right: `↑in ↓out` token totals across all missions, dim.

Input row:

- The glyph is `▸`. It is accent during a mission, err after a lost mission, and ink otherwise.
- Typed text follows, with the cursor on one cell. The row keeps the cursor in view when the text is longer than the row.

Queued notes:

- One row each, inside the frame, above the input row.
- `○` (faint), then the text (ink). On the right, dim: `queued · becomes M2`. The row has the user band.
- Notes run one after another, each after the mission before it. A 300 ms gap separates them (scaled by speed).

### 5.3 Screen layout, top to bottom (simulator)

The simulator fills the terminal from top to bottom in this order:

1. HUD row (1 row).
2. Blank row (1 row).
3. Transcript (the rest of the screen). Missions, each with a header and its items, then a debrief row. In peek a blank row sits between blocks. In radar each mission is one row.
4. A search or rewind bar (1 row). Only when SCAN or REWIND is active: `⌕ SCAN <query>` or `◂◂ REWIND`.
5. The bottom area, one of: a console (its own heavy frame), the clearance card, the question card, the command list (`/`), the file list (`@`), or the comms frame.

The transcript shrinks to make room for the bottom area. Toasts draw over the transcript at the top right, starting on row 3. The newest toast shows. Each toast is 46 cells wide.

Start block: the start block is the first transcript entry, so it scrolls away once a mission starts. It shows:

- The mark on three rows: `▟▀▀▀▀▀▙`, `▌▟▙ ▟▙▐`, `▜▄▄▄▄▄▛`, with `hoocode` (bold), `coding agent · v0.1.24` and `~/src/hoocode` (dim) to the right.
- A blank row, then the ship, skills, agents and commands line, then a blank row.
- The key line: `/ commands · @ files · ! shell · ctrl+o peek ↔ flight log · /model switch ship · /help keys`.

The page's launch mock (a1) also shows two numbered suggestion chips (`1 why does wrap_text ignore width` and `2 add a width check`). The simulator does not. Unclear in source whether they are planned.

Size: the simulator is 100 cells wide. Its height is set by the window: rows = floor((window height − 120) ÷ line height), kept between 24 and 48. The font size is fitted to the width, kept between 7.5 and 14 px (simulator `fit`).

### 5.4 Page layout, top to bottom

1. A sticky top bar: the title `hoocode TUI · Mission Control`, a link to the simulator, the theme buttons (deep-space, daylight, NO_COLOR), and `compare: p this · o previous`.
2. Header: eyebrow `hoocode TUI · space cockpit`, the title, and the lede. The lede links to the simulator.
3. Index: group chips A to E and R, each linking to its surface.
4. Section "Same philosophy, new cockpit": the note, the vocabulary table, and "The cockpit" with its two notes and the ratatui table.
5. One group heading per group (A, B, R, C, D, E). Under each, one section per surface: heading with id, caption, a this and previous compare toggle, the terminal block, then the note.
6. Footer note.

Compare: each surface has a radio pair, "This version" and "Previous proposal". The keys `p` and `o` set every surface's pair at once, unless focus is in an input. The page script does this; no key is set per surface.

## 6. Keys, slash commands and scenarios (simulator)

The simulator is the live model of the design. It flies scripted missions. The scripts are not model output, and every figure is mock data (page footer).

### 6.1 Keys: whole screen

Key names follow the simulator's key handling. "ctrl or alt" means either modifier works for the letter keys.

| Key | What it does | Notes |
|---|---|---|
| `enter` | Sends the input. Starts a mission, runs a `/` command, runs a `!` command, or queues the input if a mission is running. | `alt+enter` queues the input without sending. On empty input after COMMS LOST, `enter` sends the last prompt again as an explain mission. |
| `esc` | Leaves a rewound view first. Otherwise aborts the running mission and clears the queue. Otherwise clears the input. | In a clearance card it denies. In a question card it picks option 1. |
| `ctrl+o` or `alt+o` | Toggles peek and radar. | |
| `ctrl+g` or `alt+g` | Opens or closes the flight plan console. | |
| `ctrl+r` or `alt+r` | Opens or closes SCAN (search of the scrollback). | Typing changes the query. `enter` jumps to the next contact. `esc` closes SCAN. `backspace` deletes. |
| `PgUp` or `shift+↑` | Rewinds by the screen height minus 12 rows, at least 3 rows. | `PgDn` or `shift+↓` moves forward. The mouse wheel moves 3 rows. |
| `shift+tab` | Toggles BUILD and PLAN. Shows a toast. | Does nothing while a console is open. |
| `ctrl+c` | First press clears the input. A second press within 1.5 s ends the shift (END OF SHIFT card). | With nothing to clear, the first press shows the toast "ctrl+c again to end the shift". |
| `tab` | Accepts the suggestion, when the input is empty. In the `/` and `@` lists, completes the choice. | |
| `↑` `↓` | Recall input history. In the `/` and `@` lists they move the selection. | |
| `alt+↑` | Takes the last queued note back into the input. | |
| `←` `→` `Home` `End` `Backspace` `Delete` | Edit the input. | |
| `/` at the start | Opens the command list. | The list closes once a space is typed. |
| `@` | Opens the file list for the word being typed. | |
| `!` at the start | Runs your own shell command. It is not a step. | See 6.4. |
| any printable key | Types into the input. | |

Chips: the simulator page lists 16 chips under "Try a mission". Clicking one fills the input. It does not send it. Press `enter` to launch.

### 6.2 Keys: consoles and cards

| Context | Keys |
|---|---|
| Theme (`/theme`) | `↑` `↓` preview the theme live. `enter` keeps it. `esc` reverts to the theme the console opened with. |
| Hangar (`/model`) | `↑` `↓` choose a ship (3). `←` `→` set thinking level 0 to 3. `enter` flies that ship (toast "now flying"). `esc` closes. |
| Controls (`/help`, `/keys`, `/hotkeys`) | `tab` cycles the layer: plain, ctrl, alt. `esc` closes. |
| Mission log (`/resume`, `/log`) | `↑` `↓` move over the last 10 missions. `enter` jumps to that mission in the scrollback. `esc` closes. |
| Docking (`/login`) | `1` subscription, `2` API key. `↑` `↓` switch. `enter` docks (toast). `esc` cancels. |
| Flight plan (`ctrl+g`, `/plan`) | `esc` closes. |
| Command list (`/…`) | `↑` `↓` pick. `tab` completes with a trailing space. `enter` runs the command. For `/skill:review`, `enter` completes the name instead of running it. `esc` clears the input. |
| File list (`@…`) | `↑` `↓` pick. `tab` or `enter` inserts `@path ` (the path and a space). |
| Clearance card | `y` go once. `a` always: adds the command's first word to the BUILD allowlist for this session. `n` abort: with typed text, `enter` aborts and sends the text as the reason. `e` edit first: `enter` runs the edited command, `esc` cancels the edit. `enter` alone is yes. `esc` alone is no. |
| Question card | `1` to `3` choose that option. `4`, or `↓` down to row 4, opens "something else" for typed text, sent with `enter`. `↑` `↓` move. `enter` chooses the highlighted option. `esc` chooses option 1, the default. |
| End of shift card | `enter` ends the shift. The page then shows "shift ended" and "Reload the page to start a new shift." `esc` stays. |

### 6.3 Slash commands

Twelve commands are listed (the simulator's `CMDS`). Aliases are in the code. The page's own mocks show more commands than this list: `/tree`, `/fork`, `/settings`, `/session` and `/subagent-stats` (d1) appear only as page surfaces. They are not in the simulator.

| Command | Opens or does | Notes |
|---|---|---|
| `/model` | HANGAR. Switch ship. | See 6.8. |
| `/help` (`/keys`, `/hotkeys`) | CONTROLS. Keys by layer. | |
| `/plan` | FLIGHT PLAN. Waypoints, agents and satellites. | Same as `ctrl+g`. |
| `/theme` | HULL. Theme list with live preview. | Themes: see section 2. |
| `/color` | HULL PAINT. Cycles the session colour. | Cycle: none, parity, cyan, violet, lime, red, none. Toast shows the name. |
| `/resume` (`/log`) | MISSION LOG. Jump to a mission. | |
| `/login` | DOCKING. Sign in: subscription or API key. | Simulator only shows the screen. |
| `/compact` | JETTISON. Folds earlier missions into one summary. | Needs at least two missions or shell commands, else toast "nothing to jettison yet". Afterwards the fuel is 6% of its earlier size. |
| `/mode` | Toggles BUILD and PLAN. | Shows a toast. |
| `/clear` | Clears the screen. | Keeps only the start block. |
| `/skill:review <file>` | Runs the review skill. | Shows the chip `✦ review`. The argument is the mission text (default "review"). The skill script always reads `src/wrap.rs`. |
| `/quit` (`/exit`) | END OF SHIFT card. | |

An unknown command shows the toast "unknown command /x · try /help".

### 6.4 Your shell (`!`)

Your own commands are lines with the user band (`▲ $  <cmd>`), not steps. The output sits below, at column 4. Exit status: `exit 0` in green, `exit N` in red. Built-in outputs:

| Command | Output |
|---|---|
| `ls` | `Cargo.toml  README.md  src/  tests/  target/` |
| `pwd` | `~/src/hoocode` |
| `git status` | `On branch main`, `Changes not staged for commit:`, `modified:   src/wrap.rs` |
| `git log --oneline` | `a1c2e3f wrap: measure columns, not bytes`, `9be0d41 add backoff helper` |
| `cargo test` | `test result: ok. 12 passed; 0 failed` |
| `whoami` | `pilot` |
| `echo <text>` | The text. |
| anything else | `<command>: not available in this simulator`, exit 127. |

### 6.5 Starter chips

The page's simulator has 16 chips above the start block. Each one fills the input. The "Route" column shows the scenario kind chosen by the keyword routing in 6.6.

| # | Chip text | Route (kind) | What the scenario does |
|---|---|---|---|
| 1 | fix the off-by-one in wrap_text | bugfix | Reads `src/wrap.rs` and its tests. Edits `wrap.rs`. Runs `cargo test wrap`. |
| 2 | what does the wrap crate do? | explain | Thinks, reads `wrap.rs`, may search, then explains with a code sample. |
| 3 | add retries to the provider wrapper | retries | Asks which strategy (question card). Plans four waypoints. Reads `provider.rs`, writes `backoff.rs`, edits `provider.rs`, runs tests. |
| 4 | add a test for tabs | tests | Reads `wrap.rs`, writes `tests/wrap_tabs.rs`, runs `cargo test`. |
| 5 | run the commands | check | Runs `printf … \| sort -r` (asks clearance in BUILD, since `printf` is not on the allowlist), then `echo boom; exit 3` (fails). |
| 6 | find the footer code with an agent | agent | Sends an `explore` agent: `grep fn render`, reads `status.rs` and `footer.rs`, sometimes `theme.rs`. |
| 7 | start the dev server in the background | job | Starts `cargo run --bin devserver` as satellite `job-1`. It lands after 18 s. |
| 8 | run a slow check | slow | Runs `sleep 6; echo done`. The wait is scaled by ship speed. |
| 9 | summarize the flaky provider | flaky | Six hails, then COMMS LOST. The mission is lost. `enter` hails again. |
| 10 | refactor wrap into a module | refactor | Searches `wrap_text`, edits `wrap.rs`, `editor.rs` and `transcript.rs`, runs tests. |
| 11 | delete the target folder | risky | Runs `rm -rf target/`. Asks clearance. Frees 1.2 GB. |
| 12 | /skill:review src/wrap.rs | skill | Reads `wrap.rs`. Gives three review findings with `▲`. |
| 13 | /model | command | Opens HANGAR. |
| 14 | /plan | command | Opens FLIGHT PLAN. |
| 15 | /help | command | Opens CONTROLS. |
| 16 | !git status | your shell | Prints the git status output. Not a step. |

### 6.6 Keyword routing

The first matching test decides the kind. Tests run in this order. Matching is case-insensitive. The test is from the simulator's `classify`.

| Order | Test | Kind | Tag |
|---|---|---|---|
| 1 | starts with `/skill:` | skill | review |
| 2 | `flaky`, `overload`, `529` | flaky | explain |
| 3 | `background`, `server`, `watch`, `sleep 60` | job | ops |
| 4 | the word `slow` | slow | check |
| 5 | `delete`, `rm -`, `remove the`, `push`, `deploy`, `install` | risky | ops |
| 6 | `agent`, `subagent`, `explore`, the word `find`, `where is` | agent | explore |
| 7 | `retry`, `retries`, `strategy`, `decide` | retries | feature |
| 8 | `fix`, `bug`, `broken`, `off-by`, `crash` | bugfix | bugfix |
| 9 | `refactor`, `rename`, `clean up` or `cleanup`, `module` | refactor | refactor |
| 10 | `test` | tests | tests |
| 11 | `run the`, `commands`, `check` | check | check |
| 12 | `add`, `implement`, `create`, `build`, `helper`, `make`, `write` | feature | feature |
| 13 | anything else | explain | explain |

The tag is stored on each mission but never drawn. Only the kind is used to choose the script.

### 6.7 What each scenario shows

Each scenario is a fixed script. Random draws (the detours, slips and crashes in 6.8) change the steps a little. Every script ends with an outcome: landed, or did not land.

| Kind | Script (in order) | Shows |
|---|---|---|
| bugfix | One line of prose. Read `wrap.rs` (31 lines) and `wrap_tests.rs` (41 lines). Maybe a search for `len()`. Maybe a failing `cargo test wrap`, then an edit. Edit `wrap.rs` (+2 −1). Maybe a second read. Final `cargo test wrap`: 12 passed, or 10 passed and 2 failed. | Steps, an edit diff, a hit, and a landing line |
| tests | Read `wrap.rs`. Write `tests/wrap_tabs.rs` (18 lines, NEW). Maybe a failing run and an edit. Final `cargo test`: 13 passed, or 12 passed and 1 failed. | A new file (NEW chip) and a test hit |
| retries | Prose: "Before I write the retry loop I need one decision." A question card with three options (exponential backoff, recommended; fixed delay; no retry) and "something else". Plan of four waypoints. Read `provider.rs` (88 lines). Maybe a search for `retry`. Write `backoff.rs` (24 lines, NEW). Edit `provider.rs` (+18 −3). Maybe a failing test and fix. Final test: 14 passed, or 2 failed. | Questions, plan, the answer fed back into the closing line (`{strategy}`), and a failure that says the work is not done |
| feature | Plan of three waypoints. Read `wrap.rs`. Maybe a search for `Duration`. Write `backoff.rs` (12 lines, NEW). Maybe a failing build and fix. Final test: 12 passed, or 1 failed. | Plan waypoints advancing, a new file |
| refactor | Search `wrap_text` (9 hits, 4 files). Edit `wrap.rs` (+9 −5), `editor.rs`, `transcript.rs`. Maybe a read of `lib.rs`. Maybe a failing build and fix. Final test. | Several edits in one mission |
| check | Prose. `printf … \| sort -r` (3 lines, needs clearance in BUILD). `echo boom; exit 3` (fails, exit 3). Prose. | Clearance, and a hit with its exit code |
| risky | Prose: "That deletes build output; it needs your clearance." `rm -rf target/` (needs clearance). Prose. | Clearance for a destructive command |
| job | Prose. A satellite `job-1` runs `cargo run --bin devserver` for 18 s. Prose: a transmission arrives when it lands. | Background jobs, the HUD satellite count, the toast on landing |
| slow | Prose. `sleep 6; echo done`. Prose. | A running call with a clock |
| agent | Prose. Agent `explore`, task "Find the footer code": `grep fn render` (4 hits), read `status.rs` (88 lines), maybe `theme.rs` (61 lines), read `footer.rs` (54 lines). Prose with the answer. | Sub-steps under the parent step (`1.1`, `1.2`…) and the agent counter |
| flaky | Six hails at growing gaps, then COMMS LOST. The mission is lost. | Hails, the COMMS LOST state, retry on enter |
| skill | Read `wrap.rs` (31 lines). Three `▲` findings. | The skill chip and the warn-coloured findings |
| explain | Thinking row (1.2 s × the ship's think factor). Read `wrap.rs`. Maybe a search for `wrap_text` (5 hits, 1 file). Prose. A code sample of four lines. Prose about long words. | Thinking row, a code block, the default route |

In PLAN mode every scenario changes shape. It keeps only the reading steps, the thinking and the prose. It adds a plan of three waypoints (`Read`, `Change`, `Verify`). It ends with: "PLAN mode: nothing was changed. shift+tab to BUILD, then fly it." The mission lands.

### 6.8 Ship and model behaviour

Each mission uses the ship that is selected when it starts. The same prompt therefore plays differently on each ship. The values below are from the simulator's `PROF` and `SHIPINFO` tables.

| Ship | Provider (hangar) | Context (hangar) | Price in / out per 1M | Speed (step time ×) | Detour chance | Slip chance (failed call, then fix) | Crash chance (final test fails) | Think time × | Colour |
|---|---|---|---|---|---|---|---|---|---|
| mock-large | mock | 200k | $3.00 / $15.00 | 1.0 | 0.10 | 0.10 | 0.05 | 1.0 | accent |
| mock-fast | mock | 128k | $0.45 / $1.80 | 0.45 | 0.45 | 0.35 | 0.12 | 0.6 | violet |
| mock-local | ollama | 32k | free (runs on this machine) | 1.9 | 0.60 | 0.50 | 0.45 | 1.4 | dim |

What each column changes:

- **Speed.** Every step's time is multiplied by this value and by a random factor from 0.8 to 1.2. A fast ship finishes sooner.
- **Detour.** Adds an extra search or read to a scenario. Some scenarios need two detours in a row to re-read.
- **Slip.** Adds a failing Shell call, then a fix. This creates a hit.
- **Crash.** Makes the final test fail, so the mission does not land.
- **Think time.** Multiplies the Think row's time.
- **Price.** Cost = input tokens × input price ÷ 1,000,000, plus output tokens × output price ÷ 1,000,000.

Token counts per step: input = 1,100 + 14 per source line + up to 500 random. Output = 70 + up to 160 random, plus 220 for an Edit or Write. A prose line adds 60 output. A thinking row adds 120 output. Mock-local costs nothing.

Thinking dial (hangar, `←` `→`): levels 0 to 3, shown as `○ off`, `◔ low`, `◑ medium`, `◕ high`. Each level adds 60% to the Think row's time. It changes no other behaviour.

The mission status is set at the end: landed, did not land, aborted (`esc`), or lost (hails ran out). Only the scenario's own outcome decides landed or did not land.

### 6.9 Other rules in the simulator

- **Clearance.** In BUILD, a Shell step asks for clearance unless its command starts with an allowlisted word. The default allowlist is `cargo`, `wc`, `ls`, `cat`, `grep`, `git status`, `git diff`, `echo`, `sleep`. `a` adds the command's first word to the list for the session. PLAN mode asks for nothing, because PLAN runs no Shell steps.
- **Queue.** Input sent during a mission joins the queue. Queued notes run in order after the current mission.
- **Suggestion.** After a mission that did not lose comms, the input shows a suggestion as a placeholder. `tab` fills it. The suggestion depends on the kind (see the table below).
- **Retry.** `enter` on empty input after COMMS LOST sends the last prompt again. The retry always uses the explain scenario.
- **Satellites.** A `job-N` lands after its own seconds, scaled by speed. Then a toast shows `job-N landed · exit 0 · <command>` for 5 s. The HUD count drops.
- **Agents.** An agent's steps are numbered under the parent step (`1.1`, `1.2`). They count toward the mission's steps. The HUD shows the running count.
- **Hails.** Six retries, each gap about double the one before. Each retry is a tick on the time axis. After the last, the mission is lost.
- **Speed.** The simulator's 1×, 2× and 4× buttons scale every wait. At 2× the playback runs twice as fast.
- **Randomness.** Each mission has its own seeded random stream. The seed starts from the clock when the page loads, so outcomes differ between loads. Unclear in source whether that is wanted.
- **Shift totals.** END OF SHIFT lists missions, landed, steps, hits, hails, time and cost.
- **Dead code.** `S.race` is never set, so the RACING state is not reachable. `/race` is not in the command list, but the key handling still mentions it.

Suggestions after a mission (the simulator's `SUG`):

| Kind | Suggestion |
|---|---|
| bugfix | add a regression test for that |
| feature | write tests for it |
| retries | write tests for the backoff helper |
| tests | run the whole suite |
| explain | fix the off-by-one in wrap_text |
| agent | explain the footer code |
| refactor | run the tests again |
| check | add retries to the provider wrapper |
| job | /plan |
| slow | run the commands |
| risky | what does the wrap crate do? |
| flaky | (in the code as "what does the wrap crate do?", but a lost mission shows no suggestion) |
| skill | fix the off-by-one in wrap_text |

### 6.10 Test list from the page

The simulator's "Test end to end" list, as written on the page:

- Fly `fix the off-by-one in wrap_text`, then `ctrl+o` to see it in the flight log.
- `run the commands`: answer the clearance with `y`, `n`, `a` or `e`.
- `add retries to the provider wrapper`: answer mission control with `1`–`4`, then `ctrl+g` for the flight plan.
- An agent, a background satellite, a slow step, a provider that fails: each has a chip on the left.
- Type while a mission flies, and the note queues as the next mission.
- `/model` switches ship; the same prompt flies differently on each.

The page also gives the simulator's keys block in its own words: `enter` launch a mission or queue one while flying; `esc` abort, close a console, back to live; `ctrl/alt+o` peek ↔ radar; `ctrl/alt+g` flight plan; `ctrl/alt+r` scan (search) the session; `pgup pgdn` rewind, wheel works too; `shift+tab` BUILD ↔ PLAN mode; `tab` accept the ghost suggestion; `/` commands; `@` attach a file; `!` run your own shell command; `ctrl+c ×2` end of shift.

## 7. Open questions, decisions, not doing, build order

The page and the simulator have no section with these labels. Both files were searched for "open question", "decision", "not doing", "build order", "TODO", "phase" and "next step". None is a section label. The page does not link to `decisions-2026-10-17.md` or `tui-surfaces.md`.

- "decision" appears once in the simulator, as part of a scenario line: "Before I write the retry loop I need one decision." That is mock text, not a decision record.
- "step 0" appears only inside mock text, in file-touch rows. It is not a build step.
- Nothing is marked "not doing" or as a build order.

Closest content that is not labelled:

- The ratatui table (1.2): which widget builds each part.
- "Four rows of chrome" and "Two views" (1.2).
- The page footer note (1.2): mock data; the QR code is real.
- The simulator's "Test end to end" list (6.10).

### 7.1 How the page compares with the 2026-10-17 decisions

This is a plain comparison, for reference. It is not a decision.

| # | Decision (decisions doc) | What the page shows |
|---|---|---|
| 1 | Five rules | The page gives different rules. See section 8, item 1. |
| 2 | Keep today's look; change only what the rules cut | The page says "what changes is the cockpit": a one-row HUD and a three-row comms frame. Unclear in source how far today's look is kept. |
| 3 | The prompt frame's top border carries the session state | Yes. The comms title holds the state (5.2). |
| 4 | Task panel as one plan line; full panel only on ctrl+g | Partly. `ctrl+g` opens a console (c6). The plan is a row inside the trajectory (c5), not a line above the prompt. |
| 5 | Thinking folded by default; ctrl+t opens it | Not in the page. The Think row is live (a2). Radar shows `✻ thought 3s` (r11). No `ctrl+t` key appears in the page or the simulator. |
| 6 | Turn the review's bugs into steps | Not covered by the page. |
| 7 | Status marks are circles | Yes (3.1). |
| 8 | No spinner. Running is a still `◐` | Yes in the simulator. The glyph does not animate. Only the elapsed time changes. |
| 9 | Radar = peek's headers; runs of calls fold into one row of counts | Partly. Radar shows one row per mission (b6, r1). The per-tool count row (`Read 3 · Shell 2`) appears only in the previous view (r2). |

## 8. Discrepancies and unclear items

Each item says where the sources differ, or where the source does not say. Nothing here has been changed in the sources.

1. **The five rules differ.** The page's sentence (1.1) names: attention is the budget; three layers; everything is a block; two views; status circles that read without colour. The decisions doc names: attention is the budget; three layers, never mixed (You, Agent, Frame); everything is a block; the terminal is home; modern underneath, plain on top. The page drops "the terminal is home" and "modern underneath, plain on top". It adds "two views" and "status circles". Unclear in source which list is current.
2. **Prompt glyph.** This view uses `▸` for the prompt. The previous view and the decisions doc use `❯`.
3. **Colour values.** The deep-space tokens are not the hex values in `tui-surfaces.md` (2.6). For example, the accent is `#5ce1e6` against `#5cc8bb`. Unclear in source whether that is intended.
4. **LANDED colour.** The page text says the comms colour for LANDED is green. In the simulator, and in the mock frames (a3), the frame stays faint. Only the title is green.
5. **Question card and `esc`.** The mock footer says `esc skip`. In the simulator `esc` picks option 1, the recommended answer. It does not skip the question.
6. **Fuel scale.** The simulator divides by 128,000 for every ship. The hangar lists 200k for mock-large.
7. **Command and skill counts.** The launch line in the mock says `⌘ 1 command`. The simulator says `⌘ 12 commands` (its list). The mock command list in d1 says "2 of 41", which is the real app. The mocks d4, d5 and d9 show real app screens (settings, the theme list, keys with 62 bindings). The simulator has no such screens. The page does not say which views come from the real app. Unclear in source.
8. **Theme list.** d5 lists eight real themes, including colorsafe-dark, colorsafe-light, solarized-dark, vox-cutout-dark and vox-cutout-light. The simulator has three. The previous view also lists solarized-light, which this view drops.
9. **Model error footer.** Mock e1 says `alt+m switch ship`. The simulator says `/model switch ship`.
10. **Scan and rewind footers.** Mock e3 says `enter lock · n next · esc clear`. The simulator says `enter next · esc close`. Mock e2 shows `↓ 3 new`. The simulator shows only `N lines above live`.
11. **Clearance card.** The mock's REACH line reads `read src/wrap.rs`. The simulator reads `read repo`. The simulator's title adds `· step NN`. The mock omits it.
12. **Satellite and LANDED.** Mock r9 shows LANDED while job-1 is still in orbit. The simulator does the same. The mission ends when its steps end, and the job lands later. Unclear in source whether that is intended.
13. **80 columns.** Mock r10 says the HUD drops the folder and the ship at 80 columns. The simulator is fixed at 100 cells and cuts the left part of a row when it does not fit (5.1). Unclear in source whether the real TUI drops fields or cuts them.
14. **Layout quirks, kept as in source.** c3's header has no space before the mark: `~/src/hoocode◈ INSIDE AGENT`. c8's bottom border is only `┗┛`. e4's toast bottom edge is only `╰╯`. b3 counts the same edit as `+1 −1` in this view and `+2 −2` in the previous view.
15. **Radar progress bar.** The bar in r4 (`░░░░░░░░ 3s of ~6s`) is mock only. The simulator shows elapsed time, not a bar. The r4 note says progress is "estimated from its last run". The simulator does not do that. Unclear whether the bar is planned.
16. **Launch suggestions.** The launch mock (a1) shows two numbered suggestion chips, `1` and `2`. The simulator does not show them. Unclear whether they are planned.
17. **Signed-out header.** Mock d10 shows `SHIP —` in the HUD. The simulator always has a ship. Unclear whether signed out is a real state for the TUI.
18. **The `⋆` mark.** It appears once, in the launch mock. Unclear what it marks.
19. **"Terminal" theme.** The page groups the three themes under "terminal theme". No theme is named "terminal".
20. **Random outcomes.** Each simulator load gives different mission outcomes (6.9). Unclear whether review needs fixed seeds.
21. **Mission tags.** Each mission stores a tag (6.6), but nothing draws it.
22. **Dead race code.** The RACING state and the `/race` key handling are never reached (6.9).

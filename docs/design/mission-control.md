# TUI: Mission Control (the cockpit) on ratatui

Status: **design agreed 2026-10-17 (user answers recorded below); M0 done.** The rest is not built. Next: Phase U0 (the bugs), then M1 (the full-screen shell).

Sources (read only, kept as the reference):

- [mission-control/spec.md](mission-control/spec.md): the extracted spec. This card cites it as "spec §". It is the detailed reference for every surface, token and key.
- [mission-control/index.html](mission-control/index.html): the design page, 46 surfaces in groups A to E and R.
- [mission-control/simulator.html](mission-control/simulator.html): the live model. Keys, commands and scenarios are in its script.

## Goal

1. Build the Mission Control design in the TUI: the cockpit chrome, the trajectory and flight log, the cards and the consoles.
2. Build it on ratatui.
3. Keep the log literal. Tool names, paths and numbers are never renamed. The metaphor lives in the chrome only.

## Relation to tui-surfaces.md

Mission Control is the next proposal on top of [tui-surfaces.md](tui-surfaces.md) (the "previous proposal" on the page). **Mission Control always wins** where it and tui-surfaces differ (Q1).

Still valid from tui-surfaces:

- Phase U0 (the bugs), all steps.
- Circles (C22): `○ ◐ ● ⊗ ◉ ⊘`. Shapes differ, so they read under NO_COLOR.
- Hint grammar (C3): `key action · key action`, keys in ink, actions dim.
- No spinner. A running mark is a still `◐`. Only the elapsed time moves.
- NO_COLOR is shapes only.

Superseded:

| Topic | tui-surfaces (2026-10-17) | Mission Control |
|---|---|---|
| Scrollback | Native scrollback. No alternate screen | Full screen on the alternate screen. The app owns history: REWIND and SCAN (Q2) |
| Border state (C7) | Prompt frame top border: `◐ working · 12s · esc stop`, `done · 3.1s`, `⊗ failed · 2.0s` | The comms frame title carries the state: READY, IN FLIGHT, LANDED, DID NOT LAND, ABORTED, COMMS LOST, CLEARANCE |
| Footer (C14) | Today's two-row footer | Replaced by the HUD (one row) and the comms foot (view and tokens) |
| Always-on chrome rows | "No new always-on chrome rows." Footer 2 rows and prompt frame 3 rows | Four rows: HUD 1 and comms 3. One row fewer than today |
| Notice row (C9) | One row above the prompt frame | Gone. Notices are transmissions: a round toast at top right |
| Task panel (C10) | Plan line above the notice row. `ctrl+g` panel in a blue frame | The plan is a row in the trajectory. `ctrl+g` opens the flight plan console |
| Thinking (C13) | Folded: ` ✻ thought for 4s · ctrl+t` | A live Think row in the `··` slot while running (header only). Folds to `·· ● Think  ✻ thought 4s · ctrl+t` when done. `ctrl+t` expands the text (Q4) |
| Radar (C21) | One row per call (peek headers), runs folded into counts | One row per mission: steps, hits, time, credits. Only hits, holds and live calls get a row of their own |
| Prompt glyph | `❯` | `▸` (spec §3.2) |
| Colours | Fixed hex: done `#b5bd68`, failed `#d58181`, now `#5cc8bb`, needs you `#ffff00`, keys `#6e93ff`, prompt frame `#757575` | Deep-space tokens (spec §2.3): `--ok #7ee081`, `--err #ff5c7a`, `--acc #5ce1e6`, `--warn #ffb547`, `--you #9b8cff`, `--faint #3a4462`, `--dim #7d87a6` (Q5) |
| Queued notes (C8) | Inside the prompt frame, on the user band, "sends after this step" | Kept, inside the comms frame. `○` then the text. Right label `queued · sends after this step` or `queued · becomes M2 after landing` |
| Attach view (C15) | Header row `◂ explore · …`. Prompt frame empty with `read-only` | Header row kept. HUD says INSIDE AGENT. Comms frame is read-only, titled `◂ READ-ONLY · this agent's own log` |
| Model errors (C18) | One Model block. Retries in the notice row | Six hails on a time axis under one Model block. Title COMMS LOST |
| Questions (C12) | Blue frame picker | Amber card (warn) that replaces the comms frame. Four numbered answers, the recommended one tagged |
| Lists (C11) | Blue frame, `› name` selected band. Slash and `@` drop-downs have no frame | Heavy console replaces the comms frame, with the command in its title, a match count and a right-aligned description. Slash and `@` lists are consoles too |
| Start (C17) | Wordmark, one blank row, resources line | Mark on three rows with name, version and folder. Ship, skills, agents and commands line. A key line. The start block scrolls away |
| `!` command (C19) | ` ! printf …` on the user band | ` ▲ $  printf …` on the user band, with `exit 0 · 4ms · no step, it's yours` |
| Compaction (C20) | Dim divider `── compacted · 38k → 6k tokens ──` | A JETTISONED row: `≋≋≋ JETTISONED M1–M3 into one summary stage · 120 → 144 tokens` |
| Scroll view (C16) | Rewind hints in the footer's place | `◂◂ REWIND 43–61 of 78 · 55%` bar above the bottom area, with its hints on the same row |
| Subagent rail (C5) | Grey `│` rail at column 6 | Violet `┃` rail. Sub-steps numbered `1.1`, `1.2` under the parent step |

Still open from tui-surfaces: the `⊗` and `◉` glyph font check (its open question 2) still applies at M2, where the comms frame and the cards use them.

## Decisions

### Vocabulary

Compressed from spec §1.3 to the words that show in the chrome. The full map is spec §1.3.

| Word | On screen |
|---|---|
| mission `M1` | Header `▲ M1  prompt` in peek. One row in radar. A queued note becomes the next mission |
| ship | The model. `SHIP mock-large` in the HUD. `/model` opens the hangar |
| step `01` | One tool call, with a gold number. Agent calls are `1.1`, `1.2` |
| hit | A failed call: an error, or a non-zero exit. Red `HIT` chip, `⊗`, `N✕` in radar |
| hail | A provider retry (529, timeout). A `⊗` tick on the time axis. After the last one, COMMS LOST |
| fuel · credits | Context used (`FUEL ▰▱…` with %). Money spent (`CR $0.000`) |
| agent `◈`, skill `✦`, satellite `⌁` | Keep their names. An agent's steps sit on a violet `┃` rail |
| comms | The 3-row heavy frame at the bottom. Its title is the state. It holds the input row |
| clearance | Amber card asking to run a Shell command: the command, a REACH line, keys `y a n e` |
| console | A heavy-framed list that replaces the comms frame |
| transmission | A toast at top right, with a draining bar |
| hull, hull paint | The theme (`/theme`). The session colour (`/color`) |
| hangar, docking, mission log | `/model`, `/login`, `/resume` |
| jettison, stage separation, flight path | `/compact`, `/fork`, `/tree` |
| scan, rewind | `ctrl+r` searches the transcript. PgUp, PgDn or the wheel rewinds, with a `◂◂ REWIND` bar |
| flight plan | The mission's waypoints, `FLIGHT PLAN ●●◐○○ 2/5` in the trajectory. `ctrl+g` or `/plan` opens its console |
| BUILD, PLAN | Modes. BUILD may change files and asks clearance for Shell. PLAN changes nothing. shift+tab toggles |
| end of shift | `/quit`, or ctrl+c twice |

### Layout (Q2)

The TUI is a full-screen app on the alternate screen. Top to bottom:

1. HUD: row 1 of the screen.
2. One blank row.
3. Transcript area: today's blocks, drawn into a scrollable region. Running blocks sit at its bottom.
4. Rewind or scan bar: one row, only when active.
5. Bottom area: the comms frame (3 rows), or a card, or a console.

The app owns the history. REWIND moves with PgUp, PgDn or the mouse wheel. SCAN opens with ctrl+r. The terminal's native scrollback is not used.

Trade-off: when the app exits, the transcript is not left in the terminal. A later option could print a summary on exit. That is a follow-up, not a decision.

Toasts sit at top right, under the HUD, over the transcript area.

### HUD (spec §5.1)

Left: `◭ HOOCODE`, the mode chip (`BUILD` accent, `PLAN` violet), the folder in dim text.

Right, in order: `◈ N agent` (violet, while agents run), `⌁ N sat` (accent, while satellites are up), `SHIP` and the ship name, `FUEL` with 8 cells and the percent, `CR` and the spend.

- Fuel colour: accent below 70%, warn below 90%, err from 90%.
- Narrow (80 columns, spec r10): drop the folder first, then the ship. Keep mode, fuel and credits.
- Other cuts: the left part is cut first, with `…` at its end. The right part is cut from the right.

### Comms frame (spec §5.2)

| State | Title (left) | Frame | Input placeholder |
|---|---|---|---|
| READY | `READY` dim | faint | `name your mission · / commands · @ files · ! shell` |
| IN FLIGHT | `◐ IN FLIGHT · step N · time · esc abort` | acc | `type to queue the next mission` |
| COMMS LOST | `⊗ COMMS LOST · enter hail again · /model switch ship` | err | `press enter to hail again` |
| LANDED | `● LANDED · time · N steps`. Only the title is green | faint | suggestion and `tab`, else READY |
| DID NOT LAND | `⊗ DID NOT LAND · time · N steps` | faint | suggestion and `tab`, else READY |
| ABORTED | `⊘ ABORTED · READY` dim | faint | suggestion and `tab`, else READY |

- Right label on the top border: `app-edit` by default, or the hull paint name after `/color`.
- Foot, left: `peek` or `radar · flight log`. In PLAN, `PLAN · nothing gets changed` in violet.
- Foot, right: token totals `↑in ↓out`, dim.
- Input glyph `▸`: accent during a mission, err after a lost one, ink otherwise.

### Trajectory (peek, spec §4 b1 to b7, c5, c7)

- Mission header `▲ M1  prompt`, the `▲` and `M1` in the `you` colour. A skill chip `✦ review` sits at the right.
- Each step: gold number, status circle, tool name, target, and stats right-aligned. Steps sit on a `┊` rail.
- Up to three output lines per step on the rail. A fold reads `+3 more`.
- A failed step gets the `HIT` chip in place of its success stats.
- A new file gets the `NEW` chip.
- Edits show only the changed lines, with `+1 −1` on the header.
- Thinking: a Think row in the `··` slot, header only (Q4). It takes no step number. While it runs it reads `·· ◐ Think  ✻ 4s`. When done it reads `·· ● Think  ✻ thought 4s · ctrl+t`. `ctrl+t` shows the text, dim italic, on the `┊` rail.
- Skill findings use `▲` in warn.
- A satellite: `⌁ sat job-1 · in orbit` on its step row.
- The landing line ends every mission: `● LANDED · 2 steps · 1 hit · 0.5s · 0.8k tokens · $0.002`.

### Flight log (radar, spec §4 r1 to r11)

- One row per mission: mark, prompt, steps, hits (`0✕`), time, cost.
- A hit gets its own line under the mission: `⊗ 04 Shell  echo boom; exit 3  → boom  HIT`.
- A mission in flight shows `◐` and its live step.
- Thinking shows as `✻ thought 3s` under its mission. No gist line (Q4).
- Stats are never dropped. The label is cut with `…` first.

### Clearance card (spec §4 c1, r5)

- Replaces the comms frame. Warn frame.
- Title `◉ CLEARANCE NEEDED · step NN`. Right label `BUILD asks for Shell`.
- The command, with `$`.
- REACH line: `■ read  □ write  □ network  ■ pipe`. `■` on, `□` off.
- Keys: `y go once · a always … · n abort + say why · e edit first`.
- Typed text works as a reply.

### Question card (spec §4 c2)

- Title `◉ MISSION CONTROL ASKS`. Right label `sets the default for every call`.
- Four numbered answers. The recommended one is tagged. Number keys answer.
- Last answer: `4 something else  type it`.

### Consoles (spec §4 d1 to d10)

- A thick (heavy) block that replaces the comms frame. Title is the console name with a count.
- Rows: `›` marks the selected row, on the selection band. Optional query row `▸ text`.
- Hint row last.
- Consoles: commands, `@` files, hangar (`/model`), settings (`/settings`), hull (`/theme`), mission log (`/resume`), flight path (`/tree`), stage separation (`/fork`), controls (`/hotkeys`, a drawn keyboard), docking (`/login`, with a real QR code), flight plan (`ctrl+g`).
- Page-only screens for commands the app already has get this look. No new commands are added.

### Toasts (spec §5.3)

- Round frame (`╭ ╮ ╰ ╯`), top right, 46 cells wide, title `● TRANSMISSION`.
- A second row `▔` drains as the toast times out.
- The newest toast shows.

### Rewind and scan bars

- One row, above the comms frame, only when active.
- REWIND: `◂◂ REWIND  43–61 of 78 · 55%  ↓ 3 new`, with its label in the `vio` colour.
- SCAN: `⌕ SCAN  query  1 contact  enter next · esc close`, with its label in warn. Matches show as `◂ contact` in the transcript.

### Jettison, end of shift, hull paint

- Jettison: `≋≋≋ JETTISONED …` row, in `vio`.
- End of shift: an amber card. Keys `enter` ends the shift, `esc` stays. The debrief lists missions, landed, steps, hits, time and cost.
- Hull paint (`/color`): cycles none, parity (gold), cyan, violet, lime, red. It tints the HUD and the comms frame.

### Themes (spec §2)

- Deep-space is the default dark theme: navy void, the tokens in spec §2.3 (Q5).
- Daylight is the light theme: paper white, the same token names.
- NO_COLOR: one ink colour (`#e6e6e6`) and no hues. Shapes and frame titles carry the state (spec §2.5).
- Today's real themes (dark, light, solarized-dark, colorsafe, vox-cutout) keep working. Each maps onto the same token names.
- `/theme` is the HULL console. Moving the selection previews live. `enter` keeps. `esc` reverts.
- The page's "terminal theme" is not a theme (spec §8.19).

### Glyphs (spec §3)

- Status circles: `○ ◐ ● ⊗ ◉ ⊘`, as in tui-surfaces C22.
- Kinds and marks: `▲` mission, `✦` skill, `◈` agent, `⌘` commands, `✻` thinking, `⌁` satellite, `◭` mark, `≋` jettison, `⌕` scan, `✂` fork, `◂` back or contact, `◂◂` rewind, `✎` editing, `▸` input, `›` selected, `…` cut or fold, `·` separator.
- Diff and counts: `+ −`, `✕` hits, `↑ ↓` tokens.
- Bars and rails: `┊` trajectory, `┃` agent, `━ ┄` flight-plan track, `░` progress track (not built; see Q6, item 7), `▰ ▱` fuel, `▔` toast timer.
- Frames: heavy (`┏ ┓ ┗ ┛`) for the comms frame and every console. Round for toasts.
- Hints: keys in ink, actions dim, separated by `·`. Example: `enter next · esc close`.
- Every mark is one terminal cell. The QR code uses half blocks (`▀ ▄`).

### Keys

Spec §6.1 and §6.2 are the reference. Letter keys work with ctrl or alt. Today's bindings are checked against §6 at M1, and any clash comes back to the user.

## ratatui architecture (decided by the orchestrator)

### Crates

| Crate | Depends on | Holds |
|---|---|---|
| `hoocode-tui-ratatui` (new) | `hoocode-tui-render`, `ratatui-core`, `ratatui-widgets` | Generic widgets, the buffer-to-lines adapter and the screen backend. No agent knowledge |
| code-tui crates | `hoocode-tui-ratatui` | The Mission Control surfaces: steps, missions, clearance, consoles |

- Not the `ratatui` facade. The facade 0.30 pulls crossterm 0.29 beside our crossterm 0.28.1 (`Cargo.lock`).
- Versions in `Cargo.lock` today: `ratatui-core` 0.1.2 and `ratatui-widgets` 0.3.2. Check the lock again when a step adds them, and read the API from `cargo doc` for those exact versions ([CLAUDE.md](../../CLAUDE.md)).
- Firewall: `scripts/ci/dep-firewall.json` gets `ratatui-core` and `ratatui-widgets` as owners, naming `hoocode-tui-ratatui` only. If a code-tui crate needs ratatui types, it goes through `hoocode-tui-ratatui`, or M0 adds it to the owner list with a reason. Run `python3 scripts/ci/check_dep_firewall.py` when deps change.
- Working tree on 2026-10-17: `crates/hoocode-tui-ratatui/` already exists, uncommitted, with `Cargo.toml` and `Cargo.lock` changed. It was not made by this card. The M0 brief checks `git status` before it starts, and builds on that skeleton if it fits.

### Component work (M0, M2 to M8): ratatui inside today's renderer

- ratatui renders into a `Buffer`. An adapter (`RatatuiComponent`) turns the buffer into today's `Vec<String>` lines with SGR. It implements the existing `Component` trait (`crates/hoocode-tui-render/src/component.rs`).
- The existing `Tui` differential writer and the component goldens keep working.
- Each surface moves to ratatui widgets when it changes to the Mission Control look.

### Full-screen shell (M1)

- The app enters the alternate screen. The frame is a ratatui `Terminal` with `Viewport::Fullscreen`.
- A custom `Backend` writes through `hoocode-tui-terminal`'s `Terminal::write_frame`, on the output thread `hoocode-term-out`.
- Layout: HUD on row 1 (placeholder content is fine at M1), a blank row, the transcript area (today's component lines drawn into a scrollable region, with app-owned scroll that replaces the pinned scroll view), and the bottom area (today's prompt frame lines).
- Replaces the custom pinned scroll view and the `\x1b[3J` full redraw.
- Trade-off: the transcript is not left in the terminal on exit (see Layout).

### Theme

- The Mission Control tokens (ink, dim, faint, acc, ok, err, warn, you, vio, gold and the bands) become a ratatui `Style` palette.
- The palette is built from `hoocode-code-tui-theme`, so `/theme` and NO_COLOR keep working.
- The palette moves from M0 to M2 (the first step that draws a visible surface in ratatui). Deep-space is the default dark theme (Q5).

### Width

- The spec is drawn at 100 columns. r10 shows 80.
- Narrow: the HUD drops folder, then ship (r10). Other rows cut with `…`, left part first (spec §5.1).
- Stats are never dropped. Labels are cut with `…` first.

## What we build

### Build order

| Step | Work | Surfaces (spec) | Size |
|---|---|---|---|
| M0 Foundation | `hoocode-tui-ratatui` crate. Buffer-to-lines adapter (`RatatuiComponent`). Golden test of the adapter. No visible change. **Done 2026-10-17** (ratatui-core 0.1.2, ratatui-widgets 0.3.2, no second crossterm). The palette moves to M2 | none | S/M |
| U0 Bug fixes | tui-surfaces Phase U0, steps U0.1 to U0.10, on today's renderer. Haiku subagents. Runs before M1 (Q3) | none (bugs) | S to M |
| M1 Full-screen shell | Alternate screen. `Viewport::Fullscreen` ratatui `Terminal`. Backend through `Terminal::write_frame` (`hoocode-term-out`). Layout: HUD row 1, transcript area with app-owned scroll, bottom area. REWIND and SCAN are app-owned. Folds in the former MR step | none (frame) | L |
| M2 Comms frame | Heavy frame, state titles, right label, foot (view and tokens), `▸` input row, queued notes inside. Replaces the prompt frame and its border state. Palette from the theme tokens | a1 to a4, c4 | M |
| M3 HUD | One row replacing the two-row footer. Fields and truncation. Hull paint tint | a1, r10, e5 | M |
| M4 Trajectory (peek) | Mission header `▲ M1`, numbered steps with gold numbers, `┊` rail, up to 3 output lines, HIT chip, NEW chip, landing line, Think row, agent `┃` sub-steps `1.1`, skill chip, satellite row | b1 to b7, a2, a3 | L |
| M5 Flight log (radar) | One row per mission. Plan row | r1 to r11 | M |
| M6 Clearance and question cards | Warn frame replacing the comms frame. End-of-shift card. Flight plan console | c1 to c3, c5 to c8 | M |
| M7 Consoles | The list pattern in a thick block: commands, `@` files, hangar, settings, hull, mission log, flight path, docking, fork, keys | d1 to d10 | L |
| M8 History and chrome | COMMS LOST model error with hail ticks, rewind bar, scan bar, jettison row, toasts, session colour | e1 to e6 | M |

- Replaces tui-surfaces: M2 to M8 cover the same surfaces as its U1 to U7, with the Mission Control look. Its UR (the Inline viewport port) is replaced by M1, the full-screen shell.
- U0 (the bugs) runs after M0 and before M1 (Q3).

### Rules for each step

- Haiku subagents implement, review and test. The main session briefs, reviews and merges ([CLAUDE.md](../../CLAUDE.md), 2026-10-09; repeated 2026-10-17).
- After an edit: `cargo check -p <crate> --message-format=short`. Lints: `cargo clippy --fix --allow-dirty -p <crate>` first, then the rest by hand.
- Each step updates [maps/ui.md](../maps/ui.md). A crate change also updates [maps/packages.md](../maps/packages.md). Both go in the same commit as the change.
- Component goldens: `UPDATE_GOLDENS=1` accepts a deliberate change. Screen goldens (`python3 scripts/tui/goldens.py`) are a pre-release check, not per step.
- Done bar: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo nextest run -p <touched crates>`.
- Errors: the hand-written error enums. No `anyhow` or `thiserror`.

## Not doing

- No transcript printed on exit. A summary on exit is a follow-up, not decided.
- No native terminal scrollback for the transcript. The app owns history while it runs.
- No animation. No spinner. Only elapsed time moves.
- No renaming of tool names, paths or numbers. The metaphor is in the chrome only.
- No images and no Canvas. Every glyph is one cell.
- Simulator-only features are not product behaviour: random outcomes, the scripted scenarios, `/race`, ship crash rolls, the mock token counts, the starter chips list.
- No progress bar in radar (Q6, item 7).
- No launch suggestion chips (spec §8.16).
- No new commands. Only the commands the app already has get the new look.

## Questions (answered 2026-10-17)

The user answered each question on 2026-10-17. The answer is in bold under each question. The options tables stay as the record.

### 1. Does Mission Control win over tui-surfaces and decisions-2026-10-17 where they differ?

| Option | Trade-off |
|---|---|
| (a) recommended: yes | One look, one card to build from. The table in "Relation to tui-surfaces.md" applies |
| (b) keep today's look, take only the HUD and the comms frame | Smaller change. The trajectory and flight log stay as tui-surfaces describes. Two designs to keep in step |

**Answer: (a). Mission Control always wins** over tui-surfaces.md and the earlier 2026-10-17 decisions where they differ. The table in "Relation to tui-surfaces.md" applies.

### 2. HUD placement with native scrollback

| Option | Trade-off |
|---|---|
| (a) recommended: the HUD is the top row of the live region, directly above the comms frame | Four chrome rows at the bottom. Scrollback stays native. The page draws the HUD at the top of the screen, which the terminal cannot do without the alternate screen |
| (b) HUD pinned at the top of the screen | Needs the alternate screen. Contradicts "the terminal is home" (decisions-2026-10-17, rule 4) |

**Answer: (b). The HUD goes at the top of the screen, as in the Mission Control design.** The TUI becomes a full-screen app on the alternate screen (ratatui `Viewport::Fullscreen`), like the simulator. History is owned by the app: REWIND with PgUp, PgDn or the wheel, and SCAN with ctrl+r. The terminal's native scrollback is not used. Trade-off: on exit the transcript is not left in the terminal. A later option could print a summary on exit. That is a follow-up, not a decision. See Layout and M1.

### 3. U0 bugs first, or M steps first?

| Option | Trade-off |
|---|---|
| (a) recommended: M0 now (no visible change), then U0, then M1 onward | The bugs are fixed before any visible change lands. U0 runs on the renderer M0 wraps |
| (b) M steps straight through, U0 after them | The bugs (retry rows, the `1h60m` rounding, the duplicate plan) stay visible for longer. M1 and M3 touch the same code as U0 |

**Answer: (a). The order is M0 (done), then tui-surfaces Phase U0 (steps U0.1 to U0.10, by Haiku subagents), then M1 onward.**

### 4. Thinking: live Think row, or folded with `ctrl+t`?

| Option | Trade-off |
|---|---|
| (a) recommended: a live Think row while running (`··`, `✻ 4s`). Folds to `✻ thought 4s` when done. `ctrl+t` still expands | Changes decision 5 of 2026-10-17 for the running state only. Matches spec a2 and r11 |
| (b) keep decision 5: folded from the start, no live row | No live thinking row in the trajectory. Spec a2 is not built as drawn |

**Answer: Thinking is folded by default** (this keeps decision 5). While it runs, one Think row sits in the `··` slot, header only: `·· ◐ Think  ✻ 4s`, with no gist line. When done, it folds to `·· ● Think  ✻ thought 4s · ctrl+t`. `ctrl+t` expands it to show the text, dim italic, on the `┊` rail. Radar shows `✻ thought 3s`.

### 5. Colours

| Option | Trade-off |
|---|---|
| (a) recommended: the deep-space hexes (spec §2.3) become the default dark theme tokens | The palette changes from the tui-surfaces hexes. Needs-you becomes amber `#ffb547`, not `#ffff00` |
| (b) keep today's hexes, use Mission Control for layout only | Cards and the HUD do not match the page. Two palettes to keep |

**Answer: Yes.** The deep-space hexes from spec §2.3 become the default dark theme tokens. Daylight is the light theme. NO_COLOR is shapes only.

### 6. Spec discrepancies that need a call

Spec §8 lists 22 items. The items below need a call. Each has a proposed default, which the build uses.

| # | Item (spec §8) | Proposed default |
|---|---|---|
| 1 | LANDED frame colour (§8.4). The page says green, the mock and simulator keep the frame faint | Frame stays faint. Only the title is green (simulator) |
| 2 | Question card `esc` (§8.5). The mock says `esc skip`. The simulator picks option 1 | `esc` picks option 1, the recommended answer (simulator). Footer reads `esc pick 1` (my wording; the simulator has no footer text) |
| 3 | Fuel scale (§8.6). The simulator divides by 128,000 for every ship | Fuel uses each ship's real context window: 200k, 128k, 32k |
| 4 | Footers in e1, e2, e3 (§8.9, §8.10) | e1: `/model switch ship` (simulator). e2: `↓ N new` as the page note says, with `esc live`. e3: `enter next · esc close` (simulator) |
| 5 | Clearance REACH line (§8.11). The mock says `read src/wrap.rs`, the simulator `read repo` | The mock's real path. Paths are never renamed |
| 6 | Launch suggestion chips (§8.16). The mock shows `1` and `2` | Not built. The start block shows the key line only |
| 7 | Radar progress bar (§8.15, r4) | Not built. A running row shows its elapsed time only, for example `◐ 01 Shell  sleep 6; echo done  3s` |
| 8 | Signed-out `SHIP —` (§8.17). The simulator always has a ship | Shown only when no model is configured. The simulator never reaches it |
| 9 | Satellite landing (§8.12). The mission lands when its steps end, and the job lands later | Simulator behaviour. The HUD count drops when the job lands, with its own toast |

**Answer: All proposed defaults are accepted** as listed above, for every item 1 to 9.

Other §8 items need no call. Items 1, 2 and 3 are settled in the body above. Items 7 and 8 use the real app's count and themes. Item 13 (80 columns) follows the r10 rule. Item 14 (layout quirks such as `~/src/hoocode◈`) is not copied: one space before every mark. Items 18 to 22 are unused mock marks, dead code, or simulator-only.

## Details

- Spec §4 holds the 46 surfaces, with the "this" view of each. Use it as the test for M1 to M8 goldens.
- Spec §6.9 holds the simulator's clearance allowlist (`cargo`, `wc`, `ls`, `cat`, `grep`, `git status`, `git diff`, `echo`, `sleep`). It is a simulator rule. The real app's permission model decides clearance.
- The page stores the chosen theme in `localStorage` (`hoo-term`). That is a page feature. The TUI keeps its own theme setting.

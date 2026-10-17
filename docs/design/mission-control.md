# TUI: Mission Control (the cockpit) on ratatui

Status: **design agreed 2026-10-17 (user answers recorded below); M0 to M8 built on the integration branch `agent/mc-int` (2026-10-17), not yet merged to main.** U0 is done. Per milestone: M1 is in [M1 in detail](#m1-in-detail), with its as-built notes and gaps. Known gaps are listed in the build-order table and in the milestone notes; screen goldens (`color-chrome`, changed by M8 hull paint) are accepted at the pre-release check.

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
- Stats stay whole. The label is dropped when it does not fit beside them. Stats are cut only when they fit nowhere.

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

- The app enters the alternate screen. `Tui` draws the frame itself in a full-screen mode. It is not a ratatui `Terminal`. Each frame is one string through `Terminal::write_frame`, on the output thread `hoocode-term-out`. The reasons and the files are in [M1 in detail](#m1-in-detail), section (1). This replaces the earlier `Viewport::Fullscreen` plan.
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
| U0 Bug fixes | tui-surfaces Phase U0, steps U0.1 to U0.10, on today's renderer. Haiku subagents. Runs before M1 (Q3). **Done 2026-10-17** (commits 8a4c231 to d9954f5; as built in [tui-surfaces.md](tui-surfaces.md)) | none (bugs) | S to M |
| M1 Full-screen shell | Alternate screen. Full-screen mode in `Tui`, one frame string through `Terminal::write_frame` (`hoocode-term-out`). Layout: HUD row 1, transcript area with app-owned scroll, bottom area. REWIND and SCAN are app-owned. Folds in the former MR step. Detail: [M1 in detail](#m1-in-detail) | none (frame) | L. **Done 2026-10-17** (M1a to M1e: commits cfe54c4, 2ae76b9, 174ebc2, 7f58fed, and the M1e commit). As built and known gaps: [M1 in detail](#m1-in-detail), section "As built" |
| M2 Comms frame | Heavy frame, state titles, right label, foot (view and tokens), `▸` input row, queued notes inside. Replaces the prompt frame and its border state. Palette from the theme tokens | a1 to a4, c4 | M | **Done** (ratatui; `comms_frame.rs`).
| M3 HUD | One row replacing the two-row footer. Fields and truncation. Hull paint tint | a1, r10, e5 | M | **Done** (ratatui; `hud.rs`, hull tint).
| M4 Trajectory (peek) | Mission header `▲ M1`, numbered steps with gold numbers, `┊` rail, up to 3 output lines, HIT chip, NEW chip, landing line, Think row, agent `┃` sub-steps `1.1`, skill chip, satellite row | b1 to b7, a2, a3 | L | **Done** (ratatui port in M5). Agent sub-steps `1.1` not built; see M4 notes in `ui.md`.
| M5 Flight log (radar) | One row per mission. Plan row | r1 to r11 | M | **Done** (ratatui). Plan waypoint row not built (no data); OSC-8 links dropped.
| M6 Clearance and question cards | Warn frame replacing the comms frame. End-of-shift card. Flight plan console | c1 to c3, c5 to c8 | M | **Done** (ratatui cards). Shell "always" is session-only, program-scoped, fail-closed. Flight plan console moved to M7.
| M7 Consoles | The list pattern in a thick block: commands, `@` files, hangar, settings, hull, mission log, flight path, docking, fork, keys | d1 to d10 | L | **Done** (ratatui; shared `Console`). Consoles built as listed in `ui.md`. No QR code for `/login` (URL only, no new dependency).
| M8 History and chrome | COMMS LOST model error with hail ticks, rewind bar, scan bar, jettison row, toasts, session colour | e1 to e6 | M | **Done** (ratatui). Hull paint saved per session. Single-attempt errors never enter COMMS LOST.

- Replaces tui-surfaces: M2 to M8 cover the same surfaces as its U1 to U7, with the Mission Control look. Its UR (the Inline viewport port) is replaced by M1, the full-screen shell.
- U0 (the bugs) runs after M0 and before M1 (Q3).

### Rules for each step

- Haiku subagents implement, review and test. The main session briefs, reviews and merges ([CLAUDE.md](../../CLAUDE.md), 2026-10-09; repeated 2026-10-17).
- After an edit: `cargo check -p <crate> --message-format=short`. Lints: `cargo clippy --fix --allow-dirty -p <crate>` first, then the rest by hand.
- Each step updates [maps/ui.md](../maps/ui.md). A crate change also updates [maps/packages.md](../maps/packages.md). Both go in the same commit as the change.
- Component goldens: `UPDATE_GOLDENS=1` accepts a deliberate change. Screen goldens (`python3 scripts/tui/goldens.py`) are a pre-release check, not per step.
- Done bar: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo nextest run -p <touched crates>`.
- Errors: the hand-written error enums. No `anyhow` or `thiserror`.

### M1 in detail

**As built (M1a to M1e, 2026-10-17).** Full-screen is on by default; `HOOCODE_FULLSCREEN=0` turns it off for a run.

- M1a: row arithmetic in `tui-render/src/fullscreen.rs` (commit cfe54c4). M1b: full-screen mode in `Tui` (cfe54c4), with review fixes (174ebc2). M1c: HUD row, gap, tree and setting (2ae76b9). M1d: scroll on the body, `N new`, shift+arrows, jump to user message in the body (7f58fed). M1e: lifecycle and checks (the M1e commit).
- M1e checks, by hand under tmux with the mock LLM (the real binary): ctrl+z, then `fg`, returns to the normal screen while stopped and re-enters the alternate screen with a repaint. Alt+e (the external editor, `$EDITOR`) and `!echo` (bash mode, which runs inline and never leaves the screen) keep the alternate screen. Ctrl+C twice exits and restores; the tty is back in cooked mode. SIGTERM restored only after the fix in M1e (see the gaps below).
- M1e automated: `tui-terminal/tests/it/crash_restore.rs` runs a child process. A UI-thread panic writes the restore sequence (`?1049l` and the rest) to stdout. SIGTERM restores and exits 143. `tui-render` has an ignored timing test, `perf_long_transcript_frame_time`.
- M1e fixes found by these checks: SIGTERM and SIGHUP had no handler, so a TUI killed by SIGTERM left the alternate screen on (the normal screen was not restored). Now `interrupt::install_terminate_restore` (installed from `ProcessTerminal::start`) restores and exits. Full-screen compose strips the cursor marker only on the drawn rows, not on the whole transcript, while live (5,000 lines: 2.8 ms to 1.3 ms per frame in a debug build).
- Mouse: with reporting on (the default), a plain drag does not select text. Selection is shift+drag, a terminal behaviour the app does not change. `/copy` copies the last reply with no selection.
- Review fixes on M1 (2026-10-17): `/new`, `/resume` and the thinking toggle go live; ctrl+o keeps the pinned message on top; plain arrows belong to the prompt while it holds text; Esc during an agent run also aborts it; `↓ N new` counts only growth since the last rebuild or resize; the HUD refreshes on model switch; `HOOCODE_FULLSCREEN` reads 0/false/no/off and 1/true/yes/on (`hoocode_code_paths::parse_env_bool`). The repaint check after stray stderr was removed: every interactive write now goes through the sink, so it could never fire.
- M1e goldens: the two scrollback checks now read the visible screen (`message-queue.json` `delivered`, `subagent-task.json` `screen`; the answers are on screen). All 11 screen goldens are accepted after review: the HUD and gap rows, the tail view (the header scrolls off when the transcript is longer than the body), no flex spacer (`screen_fill`) in full-screen, and the footer path without its leading ellipsis. Two platform labels are normalized (`alt`/`Option`, `hints.rs`), so the goldens hold on macOS and Linux. The U0 rounding fix shows in `attached-running` (`4h00m left`, not `1h60m`); that golden was stale before M1.
- Frame timing (debug build, `/perf` on a 2,000-line answer): frame build p50 about 27 to 33 ms in both modes, from transcript component rendering. Full-screen adds about 0.5 ms per frame at 5,000 lines in the harness, and the composition is not the bottleneck.

Known gaps (not fixed in M1):

- No debug trigger for a UI-thread panic in the `hoocode` binary. The panic path is tested in a child process at the terminal level, not from the app.
- Composition still asks each body component for all of its lines each frame (O(transcript)). A windowed render API on the chat component would make it O(screen). Not simple, so a follow-up.
- Frame build p50 of about 30 ms during a 2,000-line stream is the component render (debug build). Release numbers are not measured.
- A summary on exit is not printed (a decided follow-up, not a decision).
- The inline path (`HOOCODE_FULLSCREEN=0`) stays until the next release, then goes.
- The M1 keys keep today's step (view height minus 2), not spec §6.1's step.


Read on 2026-10-17 at commit 9638ed8. File names are the anchor. Line numbers are as read and may drift. M1 is the full-screen shell only. The real HUD fields, the comms frame, the trajectory and the rest come in M2 to M8.

#### Today's render path

The alternate screen and a full-screen painter already exist. They run only while the view is pinned. `set_scroll_offset` enters the alternate screen (`tui.rs:609`), and `scroll_to_live` leaves it (`tui.rs:574`). M1 makes that the normal state for the whole session.

| Piece | Where | What it does today |
|---|---|---|
| `Tui` fields | `hoocode-tui-render/src/tui.rs:285` | `root` container, `previous_lines` (last inline frame), `scroll_offset` (`Some` = pinned), `scroll_total_lines`, `scroll_prompt`, `flex_spacer` |
| `Tui::new` | `tui.rs:360` | Inline. No alternate screen |
| `start`, `stop` | `tui.rs:854`, `tui.rs:939` | `stop` leaves the alternate screen only if pinned (`:945`). Then it moves the cursor below the content and writes `\r\n` (`:951-958`) |
| `request_render`, `do_render` | `tui.rs:965`, `tui.rs:1292` | Renders now, or defers inside input. Waits while the terminal still holds the last frame (`frame_pending`) |
| `paint` | `tui.rs:1323` | Pinned: `render_scroll_view`. Otherwise `paint_live` |
| `paint_live` | `tui.rs:1342` | Full redraw on a size change (`full_redraw_clear`, `:1434`). Else `paint_differential` (`:1600`) or `full_render` (`:1687`, which clears with `ESC[2J ESC[H ESC[3J`) |
| `render_scroll_view` | `tui.rs:1761` | The pinned painter. Sync output `?2026` (`:1782`). One `ESC[r;1H ESC[2K` per row. Scrollbar in the last column. Picker bottom-aligned. Status row last |
| Scroll moves | `tui.rs:527-600` | `scroll_by_lines` (`:527`), `scroll_by_pages` (`:549`, step = view height − 2), `scroll_to_live` (`:567`), `scroll_to_row` (`:584`) |
| Row split | `tui.rs:497` `pinned_split` | Rows = screen − 1, shared by transcript and picker. Transcript keeps at least 1 row |
| Transcript width | `tui.rs:421` | One column less while pinned, for the scrollbar |
| Mouse input | `tui.rs:1040-1131` | `consume_mouse_reports` (`:1040`). Wheel = `WHEEL_LINES` 3 (`:141`). Scrollbar press pages (`:1102`). Link click through `hyperlink_at_screen_cell` (`:1134`) |
| `Terminal` trait | `hoocode-tui-terminal/src/lib.rs:44-92` | `write_frame` (`:53`), `frame_pending` (`:58`), `mouse_reporting` (`:84`), `set_alternate_screen` (default no-op, `:90`) |
| `ProcessTerminal` | `lib.rs` | Mouse on in `start` unless `HOOCODE_MOUSE=0` or `TERM=dumb` (`:411-418`). `set_alternate_screen` (`:628`). `restore_screen_modes` leaves the alternate screen and turns mouse off (`:346`) |
| Output thread | `hoocode-tui-terminal/src/output.rs` | One pending frame. `write_frame` (`:67`). `notify_when_drained` wakes the loop |
| Crash restore | `hoocode-tui-terminal/src/interrupt.rs:31` | `RESTORE_SEQUENCE` already contains `\x1b[?1049l`. `restore_terminal` at `:148` |
| Panic hook | `hoocode-code-tui-app/src/mode/mod.rs:495` | Calls `restore_terminal` |
| App tree | `mode/mod.rs:762-792` | In `add_child` order: header container, `screen_fill` (a `FlexSpacer`, set as the flex child at `:775`), attach slot, chat, pending messages, status, widget-above, tasks, notifications, footer transient lines, editor container, widgets-below, footer slot |
| Scroll keys | `hoocode-code-tui-app/src/scroll_view.rs:255` (`handle_live`), `:283` (`handle_input`), `:379` (`install_scroll_view`) | Key routing, search, status text |
| Status row | `scroll_view.rs:100` `format_scroll_status` | Position and keys |
| Jump to user message | `scroll_view.rs:43-70` | Finds the chat among the root's children (`ui.children()`) |
| Run loop | `mode/mod.rs:1004`; `tui.start()` at `:1017` | |
| Suspend and external editor | `mode/commands.rs:333-336`, `mode/input.rs:722-736` | Call `Tui::stop`, then `Tui::start`. M1 must keep both working |
| Shutdown | `mode/shutdown.rs:69` | `run_ordered`. The terminal restore is the last step |

#### (1) Where full-screen lives

**Recommendation: a full-screen mode inside `Tui` (option A).** Not a ratatui `Terminal` with a custom `Backend` (option B).

| | A: `Tui` full-screen mode | B: ratatui `Terminal` and custom `Backend` |
|---|---|---|
| The frame | `H` rows of ANSI strings, from the same `Component::render` calls as today | A `Buffer` of cells |
| The transcript | Written as it is today | Each line parsed back from SGR into cells, every frame. Or the transcript is ported to ratatui first |
| What can break | Nothing new | OSC 8 links, Kitty image placements, `CURSOR_MARKER` and wide-character handling all need redoing |
| Diff | Our own, per row (the pattern of `render_scroll_view`) | ratatui's cell diff. Our backend then turns it into a string |
| Output | One string through `write_frame` | `Backend::draw` writes a string, then `write_frame`. The same pipe, one layer more |
| Frame gating, cursor | Existing `Tui` code | Duplicated. ratatui's `Terminal` also tracks the cursor and the viewport |
| Cost | An estimate: about 300 lines in `tui.rs`, plus one module | A backend (ratatui-core 0.1.2 `backend.rs:160-331`), plus the transcript port or an SGR parser |
| Later | Ratatui widgets come in as `RatatuiComponent` lines (M0) | Cleaner once every surface is a `Buffer`, after M5 and M8 |

Why A:

- Every transcript component returns ANSI strings (`Component::render`, `component.rs:14`). A cell buffer does not hold them.
- The output thread takes one frame string. Option A produces exactly that.
- The pinned painter already does a full-screen job row by row. Option A reuses its method.

Revisit B when the transcript and the cards are ratatui widgets. Nothing in M2 to M8 depends on the choice, because they draw through `RatatuiComponent`.

Files for A:

- `crates/hoocode-tui-render/src/tui.rs`: the full-screen flag, `compose_fullscreen`, the paint branch, `start` and `stop`, resize.
- `crates/hoocode-tui-render/src/fullscreen.rs` (new): row arithmetic and scroll anchor, as pure functions.
- `crates/hoocode-tui-render/src/lib.rs`: export.
- `crates/hoocode-tui-terminal`: no new API. `set_alternate_screen` and `restore_screen_modes` exist.
- `crates/hoocode-tui-ratatui`: no change. The HUD uses `RatatuiComponent`.

#### (2) Layout and heights

Rows, top to bottom. `H` = screen rows, `W` = columns.

| Region | Rows | Content |
|---|---|---|
| HUD | 1 | `hud.rs` (new). Placeholder content |
| Gap | 1 | A blank row |
| Attach header | `A`: 0, or 1 in the attached view | `attach_slot` (`mode/mod.rs:776`) |
| Body | `T` | Header container and chat: the transcript |
| Bar | 0 or 1 | The scroll status row. Pinned only |
| Bottom | `B_shown` | Today's bottom children, in tree order: pending messages through footer slot |

Rules:

- `B` is the natural height of the bottom children at width `W`.
- `fixed = 2 + A`. That is the HUD and the gap.
- `B_shown = min(B, H − fixed − bar − 1)`. The bottom always keeps one row.
- `T = H − fixed − bar − B_shown`. The body always keeps one row.
- If `B_shown < B`, drop rows from the top of the bottom. The task panel and notices go first. If that would cut the cursor row, drop from the bottom instead, so the footer goes first.
- Short screens. `H` 6 or more: all rows above. `H` 4 or 5: no gap row. `H` 2 or 3: no HUD, no gap, no body. Only the bottom shows, with the cursor row. (Chrome already starts compact below 25 rows: `SMALL_TERMINAL_ROWS` in `chrome_layout.rs`.)

Worked numbers. Bottom natural height `B` = 7, unless noted. `bar` is 1 when pinned.

| Screen | State | Fixed | Bar | `B_shown` | Body `T` | Body width | Check (sums to `H`) |
|---|---|---|---|---|---|---|---|
| 100×30 | live | 2 | 0 | 7 | 21 | 100 | 2 + 21 + 7 = 30 |
| 100×30 | pinned | 2 | 1 | 7 | 20 | 99 | 2 + 20 + 1 + 7 = 30 |
| 80×24 | live | 2 | 0 | 7 | 15 | 80 | 2 + 15 + 7 = 24 |
| 80×24 | live, attach view | 3 | 0 | 7 | 14 | 80 | 3 + 14 + 7 = 24 |
| 80×24 | live, autocomplete open (`B` = 30) | 2 | 0 | 21 | 1 | 80 | 2 + 1 + 21 = 24 |
| 80×12 | live | 2 | 0 | 7 | 3 | 80 | 2 + 3 + 7 = 12 |
| 80×6 | live | 2 | 0 | 3 | 1 | 80 | 2 + 1 + 3 = 6 |

A bottom taller than the screen: the bottom is trimmed first, from its top. The body is squeezed to one row. Only when the screen is under 6 rows does the HUD go. The prompt is cut only when even the prompt alone does not fit.

Width: the body is `W` wide live and `W − 1` pinned. Decision: always reserve the last column in full-screen. It is blank while live. That keeps the text width the same when a user pins, so the transcript does not reflow on every wheel notch.

Cursor: its row is `fixed + T + bar` plus its row inside the bottom, minus the rows dropped from the top. Escape rows are 1-based. Show the cursor only when the view is live and the editor has focus. Hide it while pinned, as today (`tui.rs:609`).

Composing one frame, `compose_fullscreen(width, height)`:

1. Lines of the HUD, the gap and the attach slot.
2. Lines of the body (header container and chat), laid out at the body width. Live: the last `T` lines. Pinned: from `scroll_offset`, clamped to `0 … total − T` (the same clamp as `render_scroll_view`). A short transcript starts at the top, and blank rows follow it. No flex spacer. The body is top-aligned.
3. The bar: pinned only. `format_scroll_status` (`scroll_view.rs:100`).
4. The bottom lines, windowed to `B_shown`.
5. Exactly `H` rows. Each row is cut to `W` (over-width guard: test `over_width_line_on_diff_path_is_truncated_not_panicked`, `tui.rs:2274`).

#### (3) App-owned history

- The body holds the whole transcript. Nothing goes to native scrollback. The alternate screen has no scrollback of its own.
- Live: the body shows the newest lines and follows output.
- Pinned: the window holds still while output grows below it.

Today's keys. Keep all of them, with their steps (`keybindings.rs:148-231`, `scroll_view.rs`):

| Key | Today | Where |
|---|---|---|
| PgUp, PgDn | Page up or down, pinning at the prompt. Step = view height − 2 | `keybindings.rs:148-155`; `tui.rs:549` |
| Ctrl+Home, Ctrl+End | Top, and live | `keybindings.rs:158-165` |
| Ctrl+R | Search: pins at the bottom and opens the query | `keybindings.rs:167` |
| Ctrl+Up, Ctrl+Down | Previous and next user message | `keybindings.rs:169-177`; `scroll_view.rs:43` |
| ↑, ↓ | Pinned: one line. At the live prompt: input history | `keybindings.rs:206-213`; `scroll_view.rs:283` |
| Esc | Leave the pinned view. Clears the search first | `keybindings.rs:216` |
| `/`, `n`, `p` | Search in the pinned view. Next and previous match | `keybindings.rs:221-231` |
| Mouse wheel | 3 rows per notch. Wheel up pins. Reaching the bottom returns to live | `tui.rs:141`, `:1068`, `:527` |
| Scrollbar click | Above the thumb: page up. Below: page down | `tui.rs:1102` |
| Any other key while pinned | Back to live, then the key does its usual job | `scroll_view.rs`, end of `handle_input` |

Changes for M1:

- Add `shift+↑` and `shift+↓` as aliases of PgUp and PgDn. Nothing binds `shift+up` in the app today. `tui-keys/src/constants.rs:180` only maps the sequence. Spec §6.1 uses the same keys.
- Keep the step at view height − 2. Spec §6.1 says view height − 12, at least 3. That is open (see Decisions).
- New-output indicator: store `seen_total`, the body's line count when the user last pinned or scrolled. New lines = `total − seen_total`. The bar shows `↓ N new` while pinned and N > 0. Going live clears it. Keep this state in `Tui`, beside `scroll_offset`.
- The bar keeps today's text (position and keys) and adds `↓ N new`. The MC rewind bar layout comes in M8.
- Jump to user message: `scroll_view.rs:43-70` looks for the chat among the root's children. With the body as a container of header and chat, the lookup must go through the body. Fix it in M1d, with a test.
- Search (`tui.rs:620-760`) works on the body lines, as now.
- Link clicks: `hyperlink_at_screen_cell` (`tui.rs:1134`) maps screen rows through the last painted lines. In full-screen, keep that map for body rows only.

#### (4) Resize, sync output, cursor, mouse

- Resize: the terminal's callback sends `TuiEvent::Resize`, which schedules a render (`tui.rs:885`). In full-screen, a width or height change clears once (`ESC[2J ESC[H`) and repaints every row. It does not diff against the old size.
- Pinned on resize: keep the same fraction of the transcript. `top' = round(top × total' ÷ total)`, clamped. Live stays at the tail.
- Sync output: each frame is wrapped in `?2026h` and `?2026l`, as `render_scroll_view` does (`tui.rs:1782`). Autowrap off (`?7l`) around the frame. Terminals without `?2026` ignore it.
- Diff: keep `previous_screen`, a `Vec<String>` of `H` rows, separate from `previous_lines`. Write only the rows that changed: `ESC[r;1H`, `ESC[2K`, the line, `ESC[0m`. Typing changes one row, so a keystroke writes little.
- Cursor: the marker (`CURSOR_MARKER`, `tui.rs:53`) is found by `extract_cursor_position` (`tui.rs:1258`), run on the composed frame.
- Kitty images in the body: clear the old placements before a repaint, as the pinned path does (`clear_scroll_images`, `tui.rs:1840`; `emit_scroll_image`, `tui.rs:1871`). Test with images on.
- Mouse: keep reporting on. See Decisions.
- Trade-off: with reporting on, a plain drag does not select text in most terminals. Users use shift+drag. `/copy` copies the last reply with no selection (`code-resources/src/slash_commands.rs:56`).

#### (5) Exit

- Print nothing. In full-screen, `Tui::stop` skips the cursor move and `\r\n` (`tui.rs:951-958`). It shows the cursor. `ProcessTerminal::stop` then runs `restore_screen_modes`, which writes `?1049l` and turns mouse off (`lib.rs:346`). The normal screen returns as it was before launch.
- Crash: `RESTORE_SEQUENCE` already leaves the alternate screen (`interrupt.rs:31`). The panic hook (`mode/mod.rs:495`) keeps working. Shutdown restores last (`mode/shutdown.rs:69`).
- Suspend (Ctrl+Z, `mode/commands.rs:333-336`) and the external editor (`mode/input.rs:722-736`): `stop` leaves the alternate screen. `start` enters it again and repaints every row. This round trip is a test in M1b.
- Follow-up, not decided: print a summary on exit.

#### (6) Opt-in and default

- A setting `fullscreen` (boolean, default `true`) beside `chromeDensity` in `hoocode-code-settings`. The env var `HOOCODE_FULLSCREEN=0` turns it off for one run. The env var wins over the setting.
- Read in `InteractiveMode::new` (`mode/mod.rs:538`). Set on the `Tui` before `tui.start()` (`mode/mod.rs:1017`).
- Off: today's inline path stays. That is `paint_live`, `screen_fill`, and the tree without the HUD. Delete it one release later.

#### (7) Goldens and screen checks

| What | Effect |
|---|---|
| Component goldens (`tests/golden/<crate>/`, `render_golden` in `tui-render/src/golden.rs`) | Unaffected. They render components, not `Tui` |
| `Tui` unit tests (`tui.rs` test module) | Existing tests run with full-screen off, so they do not change. New tests run with it on |
| Screen goldens (`scripts/tui/scenarios/*.json`, 11 scenarios) | All change, because the HUD and gap rows move everything down. Expected. Accept at pre-release with `goldens.py update <scenario>`. Not per step ([CLAUDE.md](../../CLAUDE.md)) |
| Two checks that read tmux scrollback: `message-queue.json:202`, `subagent-task.json:56` (`history: true`) | They change. The alternate screen has no scrollback. Make them visible-screen checks, or add PgUp steps first |
| Last-screen capture (`goldens.py:554`, `capture(history=True)`) | After exit it shows the pre-launch screen. Do not read it as the transcript |
| Sizes | 10 scenarios at 100 columns, one at 140. Heights from 12 to 40. The 12-row scenario tests the short-screen rules |

Check before updating screen goldens: in tmux, `capture-pane -p` returns the alternate screen while it is active. M1e checked that by hand (2026-10-17), and it holds.

#### (8) Sub-steps

Each step is one commit. Each updates its map (`docs/maps/ui.md`, and `docs/maps/packages.md` when a crate changes). Each is testable on its own.

| Step | Work | Files | Size | Done when |
|---|---|---|---|---|
| M1a | Pure arithmetic: row split, body window, bottom window, anchor on resize, new-line count. No visible change | `hoocode-tui-render/src/fullscreen.rs` (new), `lib.rs` | S | Unit tests below pass |
| M1b | `Tui` full-screen mode: compose, paint per row, sync output, cursor, alternate screen on `start` and off on `stop`, resize clear. Off path untouched | `hoocode-tui-render/src/tui.rs` (and `MockTerminal` at `:1939`) | M to L | Frame and lifecycle tests below. Old tests unchanged |
| M1c | App tree and HUD: HUD placeholder row, gap, attach slot on top, body = header container + chat, no `screen_fill` in full-screen, bar slot. Setting and env var | `hoocode-code-tui-app/src/mode/mod.rs` (tree at `:762-796`, `new` at `:538`, `start` at `:1017`), `hud.rs` (new), `hoocode-code-settings`, `docs/maps/ui.md`, `docs/maps/packages.md` | M | HUD and tree tests below |
| M1d | Scroll on the body: live follows, pinned holds, `↓ N new`, `shift+↑`/`shift+↓`, jump to user message through the body, reserved scrollbar column | `hoocode-tui-render/src/tui.rs`, `hoocode-code-tui-app/src/scroll_view.rs`, `hoocode-code-tui-keybindings/src/keybindings.rs`, `ui.md` (scroll section) | M | Scroll tests below |
| M1e | Lifecycle and checks: crash restore test, suspend round trip, the two scrollback checks changed, a manual tmux capture, `/perf` on a long session | `hoocode-tui-terminal` tests, `scripts/tui/scenarios/message-queue.json`, `subagent-task.json`, `docs/maps/ui.md` | S | Manual capture works. Frame timing noted |

Tests, by step:

- M1a (unit, in `fullscreen.rs`):
  - `layout_100x30_live`: body 21, bottom 7.
  - `layout_100x30_pinned`: body 20, bar 1.
  - `layout_80x24_attached`: body 14.
  - `layout_bottom_taller_than_screen`: at 24 rows with `B` = 30, bottom 21 and body 1.
  - `layout_short_heights`: `H` 6 gives body 1 and bottom 3. `H` 4 and 5 drop the gap. `H` 2 and 3 show the bottom only.
  - `body_window_is_tail_when_live`: total 50, `T` 20 gives offset 30.
  - `body_window_holds_when_pinned`: top 10 stays 10 when total grows to 60.
  - `anchor_keeps_fraction_on_resize`: top 10 of 100 becomes 20 of 200.
  - `new_lines_since_seen`: seen 50, total 58 gives 8.
- M1b (in `tui.rs` tests, with `MockTerminal` recording `set_alternate_screen`):
  - `fullscreen_start_enters_alternate_screen_stop_leaves_it`.
  - `fullscreen_stop_writes_no_newline_below_content`.
  - `fullscreen_frame_100x30` and `fullscreen_frame_80x24`: composed frame text against a golden (`UPDATE_GOLDENS=1`). Use a stub HUD line, a 40-line body and a bottom with `CURSOR_MARKER`. Check row 0 is the HUD, row 1 is blank, the body tail follows, the bottom is last, and the cursor position is right.
  - `fullscreen_repaints_only_changed_rows`: one bottom row changes, so the write holds one row move.
  - `fullscreen_resize_clears_then_repaints_all_rows`.
  - `fullscreen_restart_repaints_everything` (the suspend and editor round trip).
  - `fullscreen_never_writes_more_rows_than_screen`.
  - The `MockTerminal` is private to `tui.rs`. These are in-crate unit tests, not `tests/it` files. The rule for `tests/it` applies only to integration tests.
- M1c (`crates/hoocode-code-tui-app/tests/it/hud.rs`, registered in `tests/it/main.rs`):
  - `hud_100_cols_shows_mode_folder_ship`.
  - `hud_80_cols_drops_folder_first`.
  - `tree_order_hud_gap_body_bottom` (a test accessor on the root's order).
  - `off_switch_keeps_inline_tree`.
- M1d (scroll tests, with the existing scroll test style):
  - `pinned_view_holds_while_output_grows`.
  - `live_view_follows_output`.
  - `new_count_resets_on_scroll`.
  - `page_down_past_end_returns_live`.
  - `shift_arrows_page_like_pageup_pagedown`.
  - `jump_to_user_message_finds_chat_inside_body`.
  - `wheel_up_pins_and_wheel_to_bottom_returns_live`.
- M1e:
  - `restore_sequence_leaves_alternate_screen` (in `hoocode-tui-terminal`).
  - By hand: a tmux `capture-pane -p` of the alternate screen. `/perf` frame timing on a 2,000-line session. Flattening the whole transcript each frame is today's cost too, so a cache is a follow-up if `/perf` shows it.

Decisions I made. Say if you want any changed:

1. Full-screen is on by default. The env var turns it off for one release.
2. Mouse reporting stays on. The wheel is the only mouse scroll. Selection is shift+drag. Keys only would drop the wheel that users have today. With mouse off, many terminals in alternate-scroll mode turn the wheel into ↑ and ↓. At the prompt, ↑ recalls input history.
3. Keep today's scroll step (view height − 2). The spec §6.1 step (view height − 12) is open.
4. Always reserve the scrollbar column in full-screen, so pinning does not reflow.
5. Option A for the architecture. This replaces the earlier `Viewport::Fullscreen` plan in the "Full-screen shell (M1)" bullet above.

Not in M1: the comms frame and queued notes (M2), the palette (M2), the real HUD fields and narrow rules (M3), the trajectory (M4), toasts, the rewind bar and the scan bar (M8), a summary on exit, and any animation.

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

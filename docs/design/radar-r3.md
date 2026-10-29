# Radar R3: grouped flight log

Date: 2026-10-29
Status: **approved by the user (2026-10-29), not built.** Decisions are in
[decisions-2026-10-29.md](decisions-2026-10-29.md) (section "Radar (R3)").
Supersedes the radar parts of [tool-calls-peek-radar.md](tool-calls-peek-radar.md) and
[mission-control.md](mission-control.md) (flight log section).
Card: [tui-activity.md](tui-activity.md), Phase R.

## Problem (measured on a live 100x30 render)

Capture: the real debug binary, mock LLM, tmux 100x30, `toolOutputView=radar`. Source:
`scratchpad/radar/before.txt` (scratchpad of the session that ran it).

- **No turn end.** Peek and full close each mission with a landing line
  (`● LANDED`, `⊗ DID NOT LAND`, `⊘ ABORTED`). Radar shows only a small mark on the mission
  row. The only `LANDED` text on the screen is the comms frame title, which covers the last
  mission only. The user cannot tell when the agent's turn ended.
- **Prompt and stats share one row and wrap.** `M1 ●  fix the escaped-quote test … 2 steps 1✕`.
- **One chain line per whole mission.** `● Ran echo · read notes.txt   2 calls · 1 failed · 7 lines`
  covers two batches of work in one line.
- **Hit snippets are truncated.** `⊗ 01 Shell  echo "error[E0308]: mismatched typ…  → error[E0308]: mismatched types  HIT`.
- **The answer is unmarked.** Three lines of prose follow the hit with no rail and no mark.
- **State is shown twice.** The mission mark `●` and the chain line's `●` say the same thing.

Current radar block, copied from `before.txt` (the mission rows only; the HUD and the
comms frame are unchanged by R3):

```
 M1 ●  fix the escaped-quote test                                       2 steps  1✕   <DUR>  $0.000
        ● Ran echo · read notes.txt                                   2 calls · 1 failed · 7 lines
            ⊗ 01 Shell  echo "error[E0308]: mismatched typ…  → error[E0308]: mismatched types  HIT
        The escaped-quote case still fails. The lexer builds the token from a &str slice, but the
        parser expects an owned String, so the fix has to change the token type in lex.rs. I have
        not changed any file yet.
 M2 ●  what do the notes say                                             1 step  0✕   <DUR>  $0.000
        ● Read notes.txt                                                           1 call · 1 line
        The notes say the build uses edition 2021.
```

## Approved design (R3, grouped like peek and full)

Mockup (approved; the text is the design):

```
▲ M1  fix the escaped-quote test
   ┊ ● Edited lex.rs, token.rs, parser.rs · ran cargo test                          9 calls
   ┊
   ┊ Implemented the escaped-quote fix: Token now owns a String, so the parser
   ┊ accepts it. cargo test passes.
   ╰ ● LANDED · 9 steps · 1 hit · 42s · 2.1k tokens · $0.031

▲ M2  add a regression test for escaped quotes
   ┊ ✻ put it next to the existing lexer tests
   ┊ ● Read lex.rs, tests/lexer.rs                                                    2 calls
   ┊ ✻ cover \" inside a raw string too
   ┊ ⊗ Edited tests/lexer.rs · ran cargo test                                         3 calls
   ┊   └ 05 Shell  cargo test -p hoocode-parser  → 1 failed: raw_string_escape        HIT
   ┊ ✻ raw strings skip escapes, so fix the expectation
   ┊ ◐ … Edit › Shell › Edit › Shell                                          20 calls · 14s
   ╰ ◐ IN FLIGHT · 25 steps · 1m 12s
```

Other turn ends (same row):

```
   ╰ ⊗ DID NOT LAND · 2 steps · 1 hit · 5.0s · Request failed: connection reset by peer
   ╰ ⊘ ABORTED · 12 steps · 30s
   ╰ ◆ WAITING · approve Edit src/lex.rs
```

### Rules

1. **Mission grouping reuses peek and full.** The header is `▲ Mn  prompt`, rows hang on the
   `┊` rail, and the turn ends with the same landing-line vocabulary and stats as peek and
   full (`trajectory.rs` `landing_spans`). Radar differs only in one line per tool batch
   instead of per-step bodies. No step-number gutter (a step number appears only on a hit
   row, see rule 4).
2. **Turn end is mandatory.** It is always the mission's last row and closes the rail with
   `╰`: `● LANDED`, `⊗ DID NOT LAND`, `⊘ ABORTED`, `◐ IN FLIGHT` (live, with an elapsed clock)
   or `◆ WAITING` (paused for the user's approval; it names what is awaited). The live row
   updates in place into the final state.
3. **Batch line.** A batch is the tool calls between two thinking blocks or two assistant
   messages.
   - Settled: one phrase naming what changed, edits first, then reads and commands, with
     file names (reuse and extend `turn_phrase`). The call count sits flush right as
     `N calls` only.
   - Live: one line that updates in place, `◐ … ` followed by the last 4 tools joined by
     ` › `, and on the right `N calls · elapsed`. It never wraps, however many calls.
4. **Failures.** The batch mark becomes `⊗`. Each failed call gets one row,
   `└ NN Tool  subject  → snippet   HIT`, nested two columns under its batch. Do **not** put
   `N failed` on batch or summary lines: the `⊗` and the `HIT` snippet already say it. The
   mission's hit total appears only on the turn-end row.
5. **Thinking.** One line, `✻ gist`, dim, no duration, truncated to one line.
6. **Narration and answer.** Narration text between tool calls is hidden in radar. The
   final answer is shown in full (ink) on the rail, after a blank rail line.
7. **Open vs folded.** The live mission and the latest finished mission are open (all rows).
   Older finished missions fold to: header, one summary line for the whole mission, blank
   rail, the full answer, and the turn-end row. The summary line's mark follows the mission
   mark (rule 9): it stays `●` even when a hit was recovered from.
8. **Colour restraint.** Ink for batch lines and the answer. Dim for thinking and stats.
   Faint for the rail and borders. Mark colours as today: green `●`, red `⊗`, accent `◐`,
   dim `⊘`. The `HIT` chip is red. One text column after the rail.
9. **Mark decision.** A mission that had a hit but landed keeps `●`, as peek and full do.
   Red is only for `DID NOT LAND`.

## Waiting state (verified 2026-10-29)

Question: is a held turn (waiting for the user's approval) represented in the code, and
does `◆ WAITING` need a new `FlightMark` variant?

**What exists:**

| Piece | Where |
|---|---|
| The gate asks the UI and blocks inside the tool call. No event goes out on the agent stream | `hoocode-code-permissions/src/lib.rs:819` `HooPermissionGate::check`; ask at `:843` `ui.select_clearance(&info)` |
| UI trait with the card hook (default: legacy three-choice prompt) | `hoocode-code-permissions/src/lib.rs:345` `PermissionUi`, `:353` `select_clearance` |
| Request payload: tool, detail, mode, step, reach, `from` (subagent) | `hoocode-code-permissions/src/lib.rs:302` `ClearanceInfo` |
| Answer type (`Once`, `Always`, `Deny`, `Revise`) | `hoocode-code-permissions/src/lib.rs:321` `ClearanceChoice` |
| Subagent children send holds up to the parent | `hoocode-code-permissions/src/lib.rs:381` `ClearanceRelay`; `hoocode-code-tui-app/src/dialog_bridge.rs:163-181` `TuiClearanceRelay` |
| TUI request and blocking wait | `hoocode-code-tui-app/src/dialog_bridge.rs:47` `DialogRequest::Clearance`, `:110` `ask_clearance` |
| Card open: marks the held block | `hoocode-code-tui-app/src/mode/dialogs.rs` `open_clearance_card`, `:273` `set_waiting_for_clearance(true)` |
| Card close: clears the mark | `hoocode-code-tui-app/src/mode/dialogs.rs:292` (`set_waiting_for_clearance(false)`) |
| Queue of requests behind the open card | `hoocode-code-tui-app/src/clearance_queue.rs:19` `PendingClearance`, `:37` `ClearanceQueue`, `:85` `waiting()` |
| Mode state: open card and queue | `hoocode-code-tui-app/src/mode/mod.rs:570`, `:572` |
| Block flag: a call with no result that is not frozen | `hoocode-code-tui-widgets/src/tool_execution.rs:431` field, `:618` setter, `:609` `is_held_by`, `:761` `is_waiting_for_clearance` |
| Today's rendering of a held step: `◉ NN Tool … holding for clearance`, the head `◉`, queued steps `○ next` | `hoocode-code-tui-widgets/src/radar.rs:783` `waiting_row`, `:814` `waiting_line`; used at `tool_chain.rs:361` |
| Subagent held step under its agent | `radar.rs:796` `sub_waiting_row`; imported at `tool_chain.rs:22` |
| Mission mark today: an open chain is `InFlight`, so a held mission reads `◐ IN FLIGHT` | `hoocode-code-tui-app/src/mode/transcript.rs:337` `mission_mark` (`:342`) |

**What does not exist:**

- No `WAITING` or `Held` variant in `FlightMark` (`radar.rs:72`) or `CommsState`
  (`hoocode-code-tui-app/src/comms_frame.rs:47`, with its match at `:87-90`).
- No waiting event in the agent session: `hoocode-code-agent-session/src` has no
  clearance reference.
- Headless modes (`Headless { fail_closed }`, `permissions/src/lib.rs`) have no card, so
  they have no WAITING state. The rpc approval dialogs are dropped
  ([rpc-approvals.md](rpc-approvals.md)).

**Verdict: derive it; do not add a `FlightMark` variant.**

- The state can come from what already exists. A mission is waiting when an open chain
  holds a block with `is_waiting_for_clearance()` true (`tool_execution.rs:761`). The awaited
  call is that block's own tool name and subject (`chain_entry()`, the same fields
  `is_held_by` matches).
- A new `FlightMark` variant is the wrong place. The mark is the mission's outcome. WAITING
  is a live state of an open mission, so it would leak into the comms title match
  (`comms_frame.rs:87-90`) for nothing. Put it on `FlightStats` instead, as an optional
  `awaiting` field (tool and subject). Include it in `FlightStats::render_key`
  (`radar.rs:139`).
- Accuracy: the held mark exists only while the card is up (set at `dialogs.rs:273`,
  cleared at `:292`), so the row reads WAITING exactly while the user is asked.

**Caveats (for the implementer):**

- A subagent's hold does not sit on the parent's block. It shows under the subagent's
  Agent row (`sub_waiting_row`). Whether the parent's turn-end row should read WAITING for
  it was not traced end to end. See open question 4.
- The queue (`ClearanceQueue::waiting()`) lives in the mode, not the transcript. The row
  names the head only.
- The turn's elapsed clock keeps running while the call is held. That matches today's
  `IN FLIGHT` clock and is intended.
- The glyph `◆` is not in the status module (`hoocode-tui-glyphs`). Its glyph notes say
  some geometric glyphs (`◐◑◒◓`) are missing from the bundled font, yet the radar already
  uses `◐`. Check `◆` in the font before the build (open question 5).

## Implementation notes

Effort: **S-M.** The data change in `transcript.rs` is the largest part. Rendering is one
file. Per [tui-activity.md](tui-activity.md), a Haiku subagent does the work.

| File | Change |
|---|---|
| `hoocode-code-tui-widgets/src/radar.rs` | Mission header `▲ Mn  prompt` (mark moves to the turn end). New batch row (settled and live), hit rows as `└` nested two columns, thinking as one dim line with no duration (`think_rows` wraps today), summary row for folded missions, turn-end row (reuse `landing_spans`, add the reason piece for `DID NOT LAND` and the `awaiting` text for WAITING). `FlightStats` gains ordered flight items and `awaiting`. `render_key` covers both. `chain_rows` and the row builders it uses are replaced. |
| `hoocode-code-tui-widgets/src/trajectory.rs` | Reuse `landing_spans` (`:685`), `landing_line` (`:717`), `short_dur` (`:290`), `steps_text`, `hits_text`. No change to peek or full. |
| `hoocode-code-tui-widgets/src/tool_chain_summary.rs` | Reuse `turn_phrase`. Do **not** change `chain_stats` in place: it writes `N failed` and line counts that peek may still use. Add a radar-only call-count formatter. Check callers first. |
| `hoocode-code-tui-widgets/src/tool_chain.rs` | A chain closes only on assistant text (`speaking_ends_chain`, `:37`). Thinking is a lead-in to the next chain (`transcript.rs:1223`, `place_trace_in_chain`). R3 needs thinking to be a batch boundary in radar. Keep the chain model for peek. Radar reads the batch boundaries from the flight items below. `ends_in_hit` (`:311`) and the `waiting_row` use (`:361`) stay. |
| `hoocode-code-tui-app/src/mode/transcript.rs` | Per mission, an ordered list of flight items: `Batch` (a range of chain entries), `Thinking` (seconds and gist), and the final answer. Built as blocks arrive, refreshed in `refresh_flight_marks` (`:409`). `MissionRange` (`:130-140`) holds it. `awaiting` comes from the mission's open chain's held block (`mission_mark` area, `:337`). `sync_radar_prose` (`:376`) already shows only the final answer once a mission ends (rule 6 holds today); R3 changes how it renders, not when. |
| `hoocode-code-tui-app/src/comms_frame.rs` | No change. The foot label and the `CommsState` match stay as they are. |
| `hoocode-code-tui-widgets/tests/it/radar.rs` | Rewrite the tests that assert the old one-row-per-mission chain: `:212` (chain row, flying and landed), `:255` (wide chain row wraps; becomes "a batch line never wraps"), `:283` (thinking wraps; becomes one line), `:312` (prose moves to the hit text). Keep the prompt-wrap, hit-indent, stepless and sub-step tests, adjusting their expected text. Add: batch split at thinking, folded summary, WAITING row, DID NOT LAND reason. |
| `hoocode-code-tui-app/src/mode/transcript.rs` tests (`:1853-2094`) | Update the flight-stats tests for the new items. |
| Goldens | `tests/golden/hoocode-code-tui-app/layers_{aborted,agent,failed,landed,stepless}_radar_{80,120}.txt` (10 files, at the repo root). Regenerate with `UPDATE_GOLDENS=1` under nextest. Screen goldens (`scripts/tui/goldens.py`) are a pre-release check only. |
| `docs/maps/ui.md` | Update the radar rows (the flight log row at line 68 and the trajectory row at line 67 if its wording changes) in the implementing commit. |
| `docs/user/` | Grep for radar. Change any page that describes the radar rows in the same commit. |

Done bar (CLAUDE.md): `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo nextest run -p hoocode-code-tui-widgets -p hoocode-code-tui-app`.

## Options considered and rejected

The options A to E are mocked up in the scratchpad (`radar/options.md`, `A.txt` to `E.txt`).
R1 and R2 have no mockup file; they are described here from the design brief.

| Option | Idea | Why rejected |
|---|---|---|
| A. Mission cards | Each mission is a rounded box with collapsible steps and the answer under a rule | Box chrome and collapse state add a new width model. Peek and full use the rail, not boxes |
| B. Mobile timeline rail | One thread of event rows with a time gutter; prompt cut to one line | Drops the prompt's wrap and the batch and answer structure. The time gutter is new |
| C. Two-pane tree | Missions and steps in a tree on the left, the selected detail on the right | Needs a selection cursor, keys and a new pane (effort L). Fixes truncation, but is a larger change than the problem needs |
| D. Heat strip | One line per mission, one cell per step | Hides the batches and the answer. Its folded one-line idea is kept as the folded summary (rule 7) |
| E. Grouped by state | Missions under NEEDS YOU, IN FLIGHT and LANDED headers | Reorders missions out of story order. A held mission would move groups, not stay in its place |
| R1. Step log | One row per tool call, the peek step list in radar | Too dense. It keeps the density problem and has no batch line |
| R2. Chain and outcome | Today's shape: one chain line per mission plus a landing row | Only one chain line per whole mission, so batches are lost. It is the current state the problem describes |

## Not doing

- No thinking duration.
- No `N failed` on batch or summary lines.
- No step-number gutter.
- No selection cursor or two-pane tree (option C).
- No new keys. Open and folded are automatic (rule 7).
- No change to peek or full, the comms frame, or `chain_stats` in peek.
- No new `FlightMark` variant for WAITING.

## Open questions

Each has the recommended answer first.

1. **Time format on the turn-end row.** The mockup mixes `5.0s` and `42s`, `30s` and `1m 12s`.
   Recommended: one rule, the same one the landing line uses in peek and full, so the two
   views match. Alternative: a new radar format. The landing line is formatted by its caller,
   so this is a formatter choice, not a layout one.
2. **Hits on the live row.** The mockup's `◐ IN FLIGHT` row shows no hit count. Rule 4 says
   the hit total appears on the turn-end row, and the live row is that row. Recommended: show
   the count live as well (the row updates in place). Alternative: as the mockup, count only on
   landing.
3. **Step number on hit rows.** Rule 1 says no step-number gutter; rule 4 keeps `NN` on hit
   rows (`05 Shell`). Recommended: keep `NN` on hit rows only, as the mockup shows, since a
   hit row names its step. Alternative: drop it.
4. **Subagent holds.** Should the parent's turn end read `◆ WAITING` when a subagent's child
   is held? Recommended: yes, read from the Agent row's held child. Alternative: the Agent row
   shows the hold and the turn end reads `◐ IN FLIGHT`. Needs an end-to-end trace before build.
5. **Glyph `◆`.** Check font coverage for `◆` before the build. If it is missing, the
   candidates are `◉` (used for the held step today) or `◇`. Recommended: `◆` only if the font
   has it.

## Before and after

- **Before:** the live capture above (`before.txt`, mission rows only).
- **After:** the approved mockup above. Layout is verified by the component goldens listed in
  Implementation notes, which the implementing commit regenerates.

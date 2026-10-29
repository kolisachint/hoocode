# Decisions, 2026-10-29

Made with the user while planning performance. Where an earlier page disagrees, this page wins.

## Performance (short sessions)

The card is [perf-short-sessions.md](perf-short-sessions.md).

| # | Question | Answer |
|---|---|---|
| 1 | What to optimise for | **Sessions of 10 to 15 turns.** At that length the parent is already lean (about 33 to 36 MiB RSS, frame p50 about 0.7 ms). |
| 2 | Long-session items (whole-transcript re-render, single copy of rendered lines, lazy catalog, regex LRU, allocator swap) | **Deferred**, with their measured evidence and reasons in the card's "Not doing". |
| 3 | P1: `[profile.release]` with `lto = "thin"`, `codegen-units = 1`, `strip = "symbols"`; keep `panic = "unwind"` | **Yes, approved.** Build in a separate session. |
| 4 | P2: idle wakeups (perf sampler only on `--perf-log` or `/perf`; watchdog tick 250 ms; git HEAD poll 1 s; UI loop cap 1 s) | **Yes, approved.** Build in a separate session. |
| 5 | M3: subagent child footprint | **Measure first.** Propose a lean child only if it saves 8 MiB or more per child. Changing the default concurrency needs the user's decision. |
| 6 | M4: streaming markdown re-lex | **Measure first.** Propose incremental lexing only if lexing is 25% or more of streaming CPU. |

## Radar (R3)

The design is [radar-r3.md](radar-r3.md). Where an earlier radar page disagrees, this page wins.

| # | Question | Answer |
|---|---|---|
| 1 | Layout | **R3: grouped like peek and full.** Header `▲ Mn  prompt`, `┊` rail, one line per tool batch. |
| 2 | Turn end | **Mandatory.** Always the mission's last row, closing the rail with `╰`: `● LANDED`, `⊗ DID NOT LAND`, `⊘ ABORTED`, `◐ IN FLIGHT` (live clock) or `◆ WAITING` (names what is awaited). |
| 3 | Thinking duration | **None.** Thinking is `✻ gist`, one dim line, no duration. |
| 4 | `N failed` | **Not on lines.** The `⊗` mark and the `HIT` snippet say it. The hit total is on the turn-end row only. |
| 5 | Folded missions | **Full answer shown.** A folded mission is header, one summary line, the full answer and the turn-end row. |
| 6 | WAITING | **Shown.** Recommended (not yet answered by the user): derive it from the held block, not a new `FlightMark` variant (radar-r3.md, Waiting state). |
| 7 | Open vs folded | **Live and latest finished missions are open.** Older finished ones fold. |
| 8 | Mark for a landed mission with a hit | **`●`,** as in peek and full. For the mission mark, red is only for `DID NOT LAND`. |
| 9 | Step gutter | **None.** A step number appears only on a hit row. |
| 10 | Build | **Not yet** (no user answer on build order).  The card is Phase R in [tui-activity.md](tui-activity.md), after Phase S. Open questions in radar-r3.md (time format, hits on the live row, `◆` glyph, subagent holds) are for the user. |

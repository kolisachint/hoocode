# Decisions, 2026-10-17: TUI surfaces

Made with the user while reviewing every TUI screen before the ratatui move. Where an earlier page disagrees, this page wins.

| # | Question | Answer |
|---|---|---|
| 1 | Philosophy first: five rules | **Yes.** The five rules: attention is the budget; three layers, never mixed (You, Agent, Frame); everything is a block; the terminal is home; modern underneath, plain on top. |
| 2 | Keep today's look and change only what the rules cut | **Yes.** The user preferred today's design to the v1 to v3 proposals. |
| 3 | The prompt frame's top border carries the session state | **Yes.** |
| 4 | Task panel as one plan line; full panel only on ctrl+g | **Yes.** |
| 5 | Thinking folded by default; ctrl+t opens it | **Yes.** |
| 6 | Turn the review's bugs into steps in the TUI plan | **Yes.** Phase U0 in [tui-surfaces.md](tui-surfaces.md). |
| 7 | Status marks are circles: ○ ◐ ● plus ⊗ failed, ◉ needs you, ⊘ cancelled | **Yes.** |
| 8 | Spinner | **No spinner.** Running is a still `◐`; only the elapsed time moves. |
| 9 | Radar | **Radar = peek's headers**; runs of calls fold into one row of counts. |

Review page (private): https://claude.ai/artifact/EytpgWc9LPyjPscFAckrZ5

## Later on 2026-10-17: Mission Control

The user supplied the Mission Control design (`mission-control/index.html` and `simulator.html`) and asked for it to be documented, stored and built. The card is [mission-control.md](mission-control.md). The user answered its questions the same day (rows 15 to 20).

| # | Question | Answer |
|---|---|---|
| 10 | Document the design first, and store the artefacts | **Yes.** Card: [mission-control.md](mission-control.md). Extracted spec and both HTML sources in [mission-control/](mission-control/). |
| 11 | Build it | **Yes.** Build order M0 to M7, then MR. Each step is built and reviewed by Haiku subagents. |
| 12 | UI library | **ratatui**, through a generic crate `hoocode-tui-ratatui` that depends on `ratatui-core` and `ratatui-widgets`, not the facade. |
| 13 | Who does the work | **Haiku subagents for everything**: implement, review and test. The main session briefs and merges. |
| 14 | Where Mission Control and tui-surfaces differ | Open. Recommended: Mission Control wins (open question 1 in the card). Awaits the user. |
| 15 | Q1 of the card: where Mission Control and tui-surfaces differ | **Mission Control always wins** over [tui-surfaces.md](tui-surfaces.md) and the earlier 2026-10-17 decisions where they differ. |
| 16 | Q2 of the card: HUD placement | **The HUD goes at the top of the screen,** as in the Mission Control design. The TUI becomes a full-screen app on the alternate screen (`Viewport::Fullscreen`). History is app-owned (REWIND with PgUp, PgDn or the wheel; SCAN with ctrl+r), not the terminal's scrollback. Trade-off: the transcript is not left in the terminal on exit. A summary on exit is a follow-up, not decided. This supersedes "no fullscreen" and native scrollback in tui-surfaces. |
| 17 | Q3 of the card: order | **M0 is done. Then tui-surfaces Phase U0 (U0.1 to U0.10, by Haiku subagents). Then M1 onward.** |
| 18 | Q4 of the card: thinking | **Folded by default** (keeps decision 5). While it runs, one Think row in the `··` slot, header only (`·· ◐ Think  ✻ 4s`), no gist line. When done it folds to `·· ● Think  ✻ thought 4s · ctrl+t`. `ctrl+t` expands the text. Radar: `✻ thought 3s`. |
| 19 | Q5 of the card: colours | **Yes.** The deep-space hexes from spec §2.3 are the default dark theme tokens. Daylight is the light theme. NO_COLOR is shapes only. |
| 20 | Q6 of the card: spec discrepancies | **All proposed defaults accepted** as listed in the card's table (items 1 to 9). |

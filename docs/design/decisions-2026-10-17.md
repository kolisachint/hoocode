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

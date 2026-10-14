# Decisions, 2026-10-14: closing plan step 0

Made with the user while closing steps 0a–0d of [README.md](README.md). Where an earlier
page disagrees, this page wins.

| # | Question | Answer |
|---|---|---|
| 1 | Delete `hoocode-agent-mcp`? | **No. Keep it.** It is the home of the MCP client that card 9.1 ([mcp.md](mcp.md)) rebuilds on rmcp. The delete row for `agent-mcp` in [decisions-2026-10-08.md](../../archive/docs/decisions-2026-10-08.md) no longer applies. |
| 2 | Migration task 13.4 (TS test port ledger) | **Dropped**, not moved. Its `moved_to` pointed at `docs/design/ts-to-rust-migration.md`, which does not exist, and it has no design card. The ledger status is now `dropped`. |

### TUI open decisions (answered 2026-10-14)

The 11 open decisions of [tui-activity.md](tui-activity.md) ("Open decisions (2026-10-09)"),
numbered on from 2 above. Each row points at the same item in that list.

| # | Question | Answer |
|---|---|---|
| 3 | Expired OAuth token on model menus (2026-10-09 #34; open decision 1) | **Marker only.** Show an `expired` marker from the cached token state. No background refresh, no redraw, no I/O in the menu. Reason: the menu never blocked, and a background refresh would hide a model that a request-time refresh fixes. Supersedes 2026-10-09 #34. Closes the N14 UI remainder once built. |
| 4 | N6: unterminated ESC P, ESC _ or ESC ] (open decision 2) | **Confirmed.** Read as Alt+P, Alt+_ and Alt+]. Reason: that is how the parser reads them today, and its tests pin it. |
| 5 | Task panel key hint (open decision 3) | **Show every key** of `app.tasks.cycleForward`, joined with " or ". Reason: the same format as the mode dial (N7). Today only the first key shows. Card X18. |
| 6 | Edit call-slot diff (open decision 4) | **Keep the `peek_block` trim** (`edit.rs:45-48`, `:73`). Reason: decision 36 puts Edit on the shared peek dial. |
| 7 | Cleanups: `js_trim` wrappers (open decision 5) | **Finish.** Remove the wrappers in tui-widgets `lib.rs:35` and `assistant_message.rs:33`, and call `js_trim` from tui-components. Reason: one copy of the rule (N13); the wrappers add nothing. |
| 8 | Shell and Schedule `TaskKind` variants (open decision 6) | **Wait.** Each lands with its feature (Schedule with T4.2, Shell with T5). Reason: a variant with no producer is dead code. T1.3 stays open until then. |
| 9 | Flaky goldens (open decision 7) | **Fix and prune, quarantine nothing.** `scroll-view` waits for its final text, not a fixed `wait_stable 1.0`. Drop `mode-cycle` (no scenario exists) and `print-tool-bash` (only a replay fixture) from the flaky list. Reason: a fixed quiet period guesses at timing, and a quarantined test hides regressions. |
| 10 | Abort text, X12 (open decision 8) | **One shared const** for every provider: "Request was aborted", no period. Reason: one text for users; OpenAI's period variant was the odd one out. The const moves to a crate that the providers can depend on, checked with `scripts/ci/check_dep_firewall.py`. |
| 11 | Colour when colour is off, X15 (open decision 9) | **Both follow the switch.** `color_enabled()` reads `NO_COLOR` (any non-empty value means off, per no-color.org). Theme `fg` and `bg` respect chalk's switch. Reason: a user who turns colour off should get none. |
| 12 | Dead render branches (open decision 10) | **Remove** the `extension_diagnostics` and `custom_themes` fields, their render branches and their tests (N12 follow-up). Reason: nothing produces either field. |
| 13 | Lone ESC not flushed after resubscribe (open decision 11) | **New card X17** (Phase S, Next, size S). Add a self-pipe that wakes the unix poll loop on replay. Reason: a held ESC waits for the next key, and then ESC followed by `a` reads as Alt+a. |

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
| 18 | `FORCE_COLOR` vs `NO_COLOR` precedence (open decision 1 in [tui-activity.md](tui-activity.md)) | **`FORCE_COLOR` wins** when both are set. `NO_COLOR` still turns colour off when `FORCE_COLOR` is unset. Reason: the user's call; an explicit force beats a general opt-out. Built with X15 (#11). NO_COLOR also turns off fill chips (user, 2026-10-14). |
| 19 | `spawn_refresh` after the N14 marker (open decision 2 in [tui-activity.md](tui-activity.md)) | **Delete it** if nothing calls it. Nothing does after the marker-only change, so it is removed with its in-flight set and its test. Built with N14 (#3). |
| 20 | X18, the task panel key hint (open decision 3; #5 above) | **Not built.** Marked *won't build by default* in [tui-activity.md](tui-activity.md). The default config binds only `alt+l` to `app.tasks.cycleForward`, so the change would show nothing today. Reason: the user's call. |
| 21 | `scroll-view` search match (open decision in [tui-activity.md](tui-activity.md) T0.3) | **The scenario picks its match explicitly.** It types the unique query `beta item 25` in one step, so the search selects that row (1/1) whatever the view position. The old `25` query picked `alpha item 25` or `beta item 25` depending on where the view sat after the earlier `2` keystroke. Golden `search-typing` and `search-kept` accepted; the search-dropped and live snapshots are unchanged. Reason: the user's call, 2026-10-14; a golden must not depend on incidental view position. |

### Reliability step 1 (answered 2026-10-14)

Made while closing [reliability.md](reliability.md) (step 1). The answers are in that page.

| # | Question | Answer |
|---|---|---|
| 14 | MCP and plugin tools in rpc (1.1, decision c) | **Gated in rpc (fail-closed).** `mcp_*` tools and the plugin lifecycle tools are blocked unless `auto_allow` lists them. A UI and print still run them without asking. Reason: a hoobot user could run an MCP tool unchecked. Prompting for MCP tools in the UI is not part of this. |
| 15 | macOS job on pull requests (1.3) | **Decided: the `test-macos` job runs on pull requests too.** Its `pull_request` exclusion is removed in `archive/migration/ci/step1-workflows.patch`. The session token cannot push workflows, so the user applies and pushes it. Reason: macOS failures are bugs, so a PR must show them. |
| 16 | Staged CI: guards and TUI parity (0d, T0.6) | **Staged, not applied.** The `guards` job (`no_cortex.sh`, `packages.py --check`) and the `tui-parity` workflow (manual and nightly goldens) are in `archive/migration/ci/step1-workflows.patch`. The session token lacks workflow scope, so the user runs `git apply archive/migration/ci/step1-workflows.patch` and pushes. |
| 17 | `.agents` scope (1.2) | **Kept, and documented as a cross-vendor scope.** `~/.agents` and `<project>/.agents` are read for skills, commands, agents, AGENTS.md and MCP config, the layout other tools share. hoocode writes there only the scheduled-task store (`<project>/.agents/scheduled_tasks.json`), so "read-only" has that one exception. |

### Subagent orchestration (answered 2026-10-15)

Made with the user on 2026-10-15 for [subagent-orchestration.md](subagent-orchestration.md) Q5 to Q7 and the target-dir risk.

| # | Question | Answer |
|---|---|---|
| 22 | Where subagent worktrees live (subagent-orchestration Q5; D5, §3) | **Outside the repo**, at `~/.hoocode/worktrees/<repo>/<label>`, where `<repo>` is the repo's folder name (one folder per repo). Branch stays `agent/<label>`. Reason: search and build never see a worktree twice. Replaces the earlier `.hoocode/worktrees/<label>` inside the repo. |
| 23 | Should the harness enforce the worktree (subagent-orchestration Q6; D3) | **Never.** Worktree isolation stays instructions-only. No later code card. Reason: the user's call. |
| 24 | Uncommitted parent changes (subagent-orchestration Q7; §3) | **Warn only.** The parent warns in its report when the tree is dirty. It commits only if the user asks. Reason: the user's call. |
| 25 | Build output per worktree ("New risk from Q2" in subagent-orchestration) | **Build output is per project, not per protocol.** The worktree protocol is language-neutral and does not touch build output. Each worktree uses its language's default build/cache location (e.g. its own `target/`, `node_modules/`, `.venv/`), and that output goes away when the worktree is removed. Sharing a build cache across worktrees is a per-project choice, written in that project's own rules (AGENTS.md / CLAUDE.md), not in hoocode's protocol. Example: a Rust project may set `CARGO_TARGET_DIR` to share one target dir. Watch disk use and build times; no harness code for this. Corrected 2026-10-15: the first version of this answer was cargo-specific. |

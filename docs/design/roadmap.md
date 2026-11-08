# Roadmap

Status (2026-11-08): one plan for all areas. It replaces the core, TUI, and themes and keybindings plans. 110 open items: 43 next, 21 need a user decision, 46 deferred behind a trigger or gate.

Rule: when the user asks to implement, ask which item first. The top of Next is the recommendation.

## Built

| Area | What is built | Doc |
|---|---|---|
| Engineering | Done bar and agent loop. CI jobs and guards. nextest profiles. Release pipeline with four channels (archives, curl installer, npm, PyPI). SessionStart hook (nextest, prune, warm-up). 2026-10 | [README.md](README.md) |
| Names and migration | Milestones 0a to 0d: trimmed crates, rename to hoocode, `~/.hoocode`, `HOOCODE_` only, `.agents` read-only. 2026-10 | [README.md](README.md) |
| Reliability | Milestone 1: rpc fails closed, `text_slice`, panic audit, 9 fuzz targets, macOS test job, in-process `@file` walk. 2026-10 | [runtime.md](runtime.md) |
| Runtime | One io runtime, three lanes, caps, watchdog, ordered shutdown, panic hook, memory limits, release profile, idle budget. Milestones 2, 5 and 10. 2026-10 | [runtime.md](runtime.md) |
| Agent tools | Read, Shell, Edit, Write, CodeSearch, AskUserQuestion, DocSearch, TodoWrite, background shell. Web tools are a flag only. Milestone 3. 2026-10 | [agent.md](agent.md) |
| Models | `scopedModels` with effort, categories and aliases. Subagents use scoped models. 2026-10 | [agent.md](agent.md) |
| Scheduler and `/loop` | Cron store, `/loop` forms, `/loop auto`, late-fire rule. Milestone 7. 2026-10 | [agent.md](agent.md) |
| Extras | Daily version check, `hoocode update`, turn chime. Milestone 8. 2026-10 | [agent.md](agent.md) |
| MCP client | rmcp, stdio and Streamable HTTP, folder trust, OAuth with PKCE. Milestone 4. 2026-10 | [agent.md](agent.md) |
| Plugins | Agent Plugins 1.0 load and install, `/plugin`, opt-in model tools. Milestone 6. 2026-10 | [agent.md](agent.md) |
| App server and rpc | Codex app-server wire format, approvals, scoped model and effort rules. 2026-10 | [agent.md](agent.md) |
| Subagents | Child processes, background by default, writer worktrees, caps, tree kill, ledger, 22 eval scenarios. Milestone 9. 2026-10 | [subagents.md](subagents.md) |
| TUI look | Declutter: plain words, status line, panes, outline, peek and full views, full screen by default. 2026-11 | [tui.md](tui.md) |
| TUI plan | T0 goldens, T2 subagent storage, T3 live attach, T4 panel tabs, T5 background shell. Phase S replaced by the declutter. 2026-10 | [tui.md](tui.md) |
| Themes | Five built-in themes, JSON tokens, user themes, hot reload. Plan T-1 to T-6. 2026-11 | [tui.md](tui.md) |
| Keybindings | Leader Ctrl+X, bare-letter lists, no Alt in defaults, grouped `keybindings.json`. Plan K-0 to K-10. 2026-11 | [tui.md](tui.md) |
| Single view | One row per tool call, click or Enter to expand in place; one pane grammar; yellow only for blocking. UI-21 + UI-22, phases P0–P6 on branch `feat/u21-u22`. 2026-11-07 | [tui.md](tui.md) |
| Subagent nudge | The loop header names the run's resume label; notice heads in `●`/`⊗` with dim words; the attach header takes `›` with `esc back`; a skill block is its bold name only. UI-1, 2026-11-08 | [tui.md](tui.md) |

## Next

Ordered. Items with no trigger, or a met one. The approved performance order is kept (RT-1, RT-2 to RT-9, RT-10 and RT-11, rule debt RT-12 to RT-15, then RT-16). Small items from other areas are placed between them.

### Now

1. [RT-1](runtime.md#open) Responses tool-call routing fix (M). Correctness. First in the approved order.
2. [AG-11](agent.md#open) Verify the unchecked MCP, plugin and scope rules (S)
3. [RT-2](runtime.md#open) `--continue` picks the newest valid session (S)
4. [RT-3](runtime.md#open) `message_count()` and `has_messages()` (S)
5. [RT-4](runtime.md#open) `SessionManager` `by_id` index (S)
6. [RT-5](runtime.md#open) One process-wide HTTP client (S)
7. [SA-4](subagents.md#open) Re-run `--matrix` and `--soak 600` after the cancel and tree-kill rounds (S)
8. [UI-6](tui.md#open) Component goldens for every dialog and notice (M). The dialog and notice mockups were deleted, so pin them first

### Then

9. [RT-6](runtime.md#open) CodeSearch: compile term regexes once (S)
10. [RT-7](runtime.md#open) UserMessageComponent: render stamp at construction (S)
11. [RT-8](runtime.md#open) `ShellOutput` wait on a Condvar (S)
12. [RT-9](runtime.md#open) `MessageUpdate` sets dirty (S)
13. [RT-10](runtime.md#open) Approvals stop freezing the batch (L)
14. [RT-11](runtime.md#open) Tool progress: latest per call id (M)
15. [RT-12](runtime.md#open) SSE 16 MiB cap in provider decoders (S). Rule debt.
16. [RT-13](runtime.md#open) Cap unbounded channels outside the runtime (M). Rule debt.
17. [RT-14](runtime.md#open) LLM first-byte and idle deadline (S). Rule debt.
18. [RT-15](runtime.md#open) Compaction off io and tools (M). Rule debt.
19. [RT-16](runtime.md#open) Step 0: measured 10 and 15 turn baseline (S). Gates RT-17 to RT-20.
20. [RT-28](runtime.md#open) Input-thread panic restarts the reader (S)
21. [RT-33](runtime.md#open) Clear render caches on memory shed (S)
22. [RT-34](runtime.md#open) Test the hard-limit turn abort (S)
23. [RT-35](runtime.md#open) One tool output truncation default (S)
24. [RT-36](runtime.md#open) One `home_dir` (S)
25. [EN-8](README.md#open) and [UI-3](tui.md#open) Run the plain-words check in CI `guards`. Same item (S)
26. [EN-1](README.md#open) Wire the Stop and PostToolUse hooks (S)
27. [EN-2](README.md#open) `[workspace.lints]` (S)
28. [EN-3](README.md#open) Hygiene job: `cargo machete` and `cargo deny` (S)
29. [EN-7](README.md#open) MSRV job on Rust 1.88 (S)
30. [AG-3](agent.md#open) Delete the dead `packages` setting (S)
31. [AG-4](agent.md#open) Drop Copilot and `.agents-plugin` plugin markers (S)
32. [AG-5](agent.md#open) Rewrite the plugin-authoring skill text (S)
33. [AG-6](agent.md#open) Fix the TUI plugin tool list (S)
34. [AG-17](agent.md#open) Remove the `/learn` tip. The decision says no (S)
35. [AG-15](agent.md#open) Cron batch 2: one-shot persistence, busy guards, `/cd` split (M)
36. [UI-2](tui.md#open) Delete the orphan `color-chrome` goldens (S)
37. [UI-4](tui.md#open) Drop the `○●` pulse loader, or keep it for compaction only (S)
38. [UI-5](tui.md#open) Refuse a printable leader key (S)
39. [UI-8](tui.md#open) Add `dusk`, `high-contrast` and `paper` to the contrast test (S)
40. [UI-9](tui.md#open) Fix docs drift in maps and user pages (S)
41. [UI-20](tui.md#open) Rename the `*_ts*.rs` test files (S, optional)
42. [EN-14](README.md#open) Point code comments and `clippy.toml` at the new design docs (S)
43. [EN-15](README.md#open) Drop dead `archive/` lines from `no_cortex.sh` (S)
45–46. [UI-21 + UI-22](tui.md#proposed-single-view-ui-21-ui-22) Single view + pane grammar: done 2026-11-07, phases P0–P6 on branch `feat/u21-u22` (see Built).

## Needs a user decision

| ID | Question | Recommended answer |
|---|---|---|
| [EN-6](README.md#open) | Add a doctest step to CI, or drop the doctest rule? | Add the step for the crates that have doctests |
| [EN-9](README.md#open) | Fix `hoocode update` to print the command for the install method now? | Yes, now |
| [EN-11](README.md#open) | Build for Windows? | No. Wait until a user asks |
| [EN-13](README.md#open) | Run subagent fast evals on PRs that touch `hoocode-code-subagents`? | Yes |
| [RT-32](runtime.md#open) | Fix the frame-build tail from the bash block now? | No. Wait until the user asks |
| [AG-1](agent.md#open) | Register `WebFetch` and `WebSearch`, or drop the names? | Register both. The helpers exist and the flag already promises them |
| [AG-8](agent.md#open) | Enforce `allowed-tools`, or delete the approval record? | Delete the record. Keep the warning |
| [AG-12](agent.md#open) | Wire `/goal` to `/loop auto`? | Not yet. Do it when someone asks for `/goal` |
| [AG-19](agent.md#open) | Remove `DocSearch` instead of indexing docs? | Decided 2026-11-07: yes, remove (with UI-21 decision 7); code removal pending |
| [AG-20](agent.md#open) | Build semantic code search? | No. Keep the "not available" message |
| [AG-27](agent.md#open) | Add a CI step for the app-server schema check? | Yes, nightly and non-blocking |
| [SA-6](subagents.md#open) | Remove the unused `known_issue` mechanism? | Yes, with its report branch |
| [UI-7](tui.md#open) | OSC 11 auto theme: in scope, or drop? | In scope. Short timeout, then fall back to `COLORFGBG` |
| [UI-10](tui.md#open) | Confirm before a session is deleted? | Yes. Add a `y` and `n` step |
| [UI-11](tui.md#open) | Bind yank-pop and redo, or drop them? | Drop. Redo needs an undo stack |
| [UI-14](tui.md#open) | Remove the inline screen in the next release? | Yes |
| [UI-15](tui.md#open) | Add a one-row exit summary? | No. Keep exit silent |
| [UI-16](tui.md#open) | Pinned page scroll step: view height minus 2, or minus 12? | Minus 2, as built. Write it in the code and on the page |
| [UI-18](tui.md#open) | Keep the replay fixture? | Delete it if no test reads it |
| [UI-19](tui.md#open) | Take the held behaviour changes? | Not now. One commit each, when asked |
| [UI-21, UI-22](tui.md#proposed-single-view-ui-21-ui-22) | The 9 single-view decisions: one mode, tool-row cut order, failed reason colour, running Shell tail, expand-all scope, live click pins, Tool output setting, pane clicks, yellow for blocking only | Decided 2026-11-07 in [tui.md](tui.md#decisions-needed): one mode; cut order; dim reason; Shell tail line; no expand-all; click pins; Tool output row removed (+ `DocSearch` removed); click-selects / `Enter`-opens; yellow blocking only |

## Deferred (trigger)

| ID | Item | Trigger |
|---|---|---|
| [EN-4](README.md#open) | Highlight opt-level experiment | Measure with EN-5 |
| [EN-5](README.md#open) | Re-measure the build, decide on mold | After EN-2 and EN-4 |
| [EN-10](README.md#open) | Remove the dead `publish_crates` input | Next edit to `release.yml` |
| [EN-12](README.md#open) | macOS signing | Users hit the browser warning, and an Apple Developer ID exists |
| [RT-17](runtime.md#open) | `parse_streaming_json`: repair once, byte scan | Measure first (RT-16) |
| [RT-18](runtime.md#open) | Responses text: drop the `current_item` mirror | Measure first (RT-16) |
| [RT-19](runtime.md#open) | Child JSON: text deltas to the coalescer | Measure first (RT-16) |
| [RT-20](runtime.md#open) | `transform_context` takes a slice | Measure first (RT-16): transcript copies at peak |
| [RT-21](runtime.md#open) | Speculative read-only start | Measure first: build only if it pays |
| [RT-22](runtime.md#open) | P1 follow-ups: CPU A/B, build time, `--version` | Measure first |
| [RT-23](runtime.md#open) | `hoocode-io` and `hoocode-bg` wakeup trace | Measure first: wakeup trace |
| [RT-24](runtime.md#open) | `MALLOC_ARENA_MAX=2` in subagent children | Measure first: CPU A/B and evals |
| [RT-25](runtime.md#open) | Session reads off the caller | Measure first: caller stall in a long load |
| [RT-26](runtime.md#open) | Streaming markdown re-lex: close-out | Measure first: callgrind (M4 gate) |
| [RT-27](runtime.md#open) | Remaining non-test `block_on` sites | After the runtime items close |
| [RT-29](runtime.md#open) | Panic hook output goes to the debug log | Real-tty check |
| [RT-30](runtime.md#open) | Real-TUI checks: UI stall, panic hook, terminal restore | Before the runtime items count as done |
| [RT-31](runtime.md#open) | Keystroke p99 under 16 ms: unexplained misses | A repeat of the miss |
| [RT-37](runtime.md#open) | macOS baseline and lane QoS | A macOS machine |
| [RT-38](runtime.md#open) | Windows `Shell` priority | Windows ships (not planned) |
| [AG-2](agent.md#open) | Decide MCP schema deferral | Measured token cost of MCP schemas |
| [AG-7](agent.md#open) | Live reload of plugin tools | Users hit a stale tool list |
| [AG-9](agent.md#open) | Publish the OAuth Client ID Metadata document | An OAuth MCP server needs CIMD |
| [AG-10](agent.md#open) | MCP conformance run and manual runs | Before MCP counts as done |
| [AG-13](agent.md#open) | `/grill` skips its questions in the loop | With AG-12 |
| [AG-14](agent.md#open) | Stale-lock fix with `flock` | Two processes take over one stale lock |
| [AG-16](agent.md#open) | Off switch for the three Cron tools | Measured token cost |
| [AG-18](agent.md#open) | Thinking escalation | A way to measure its effect |
| [AG-21](agent.md#open) | Un-defer MCP Apps | A tool the user relies on ships a `ui://` resource |
| [AG-22](agent.md#open) | app-server: WebSocket on TCP with a token | Remote clients |
| [AG-23](agent.md#open) | app-server: `item/tool/requestUserInput` | A client needs it |
| [AG-24](agent.md#open) | app-server: thread fork, archive and name | Client need |
| [AG-25](agent.md#open) | app-server: daemon, idle timeout, launchd | Headless use |
| [AG-26](agent.md#open) | app-server: profiles and worker processes | Multi-profile use |
| [SA-1](subagents.md#open) | Warm-pool evals: cap, reuse, cancel, stop | Next change to `warm.rs`, or before warm pool goes on by default |
| [SA-2](subagents.md#open) | Real-model trial of the brief and writer protocol | Before relying on writers in real work |
| [SA-3](subagents.md#open) | Review-agent Shell filter | A review agent runs another command in a real run |
| [SA-5](subagents.md#open) | Re-run queue saturation at root cap 12 | Next `--include-slow` run |
| [SA-7](subagents.md#open) | Evals for progress-stall, tool-stall and RSS limits | Before changing their defaults |
| [SA-8](subagents.md#open) | In-process child mode | A measured need for lower per-dispatch cost |
| [SA-9](subagents.md#open) | Group sweep misses a nested subagent's group | A leaked process seen in a run or soak |
| [SA-10](subagents.md#open) | Re-parented grandchildren are not found | A change to the re-parented jobs policy |
| [SA-11](subagents.md#open) | `maxOutputTokens` read only by Anthropic | A subagent truncates on a non-Anthropic model |
| [UI-12](tui.md#open) | Check the VS Code terminal does not take Ctrl+X | Manual check |
| [UI-13](tui.md#open) | Check `⊗` and `◉` on common fonts | Manual check |
| [UI-17](tui.md#open) | Windowed render API for the chat component | `/perf` shows the cost |

## Not doing

- Engineering: [README.md#not-doing](README.md#not-doing)
- Runtime: [runtime.md#not-doing](runtime.md#not-doing)
- Agent: [agent.md#not-doing](agent.md#not-doing)
- Subagents: [subagents.md#not-doing](subagents.md#not-doing)
- TUI: [tui.md#not-doing](tui.md#not-doing)
- Dropped: Copilot canvas, Skills over MCP, plugin update, package and publish tools, writing Claude plugin formats, `/mcp import`, rpc approval dialogs, semantic search part B, crates.io, the WASM crate, Azure, `fd` and `rg`, the external-tools pane, and the extras marked No.

# TUI: activity panel, subagent attach, and hoocode-only goldens

Status: **design, agreed 2026-10-09** ([decisions-2026-10-09.md](decisions-2026-10-09.md)).
Input: eight read-only reviews of the TUI, subagent and parity code (summarised in Details).

**T0 done 2026-10-09** (T0.1 to T0.6). The hoocode-ts parity gate is retired: the done bar is
L1, and screen goldens as a pre-release check (`python3 scripts/tui/goldens.py check all`, about 10 scenarios). `ledger.py` is read-only. The phase
tables below stay as the record. **Phase S, tier Now is done 2026-10-09** (N1 to N13 and X11; N10 with T1.1; N12 with T1.4; N11 with the Edit and AgentOutput follow-up, decision 36), except two parts: N14's exit-time abort (waits on the ordered shutdown, concurrency.md §4) and T1.2 partial (the event handlers still take `&mut Mode`). Decision 34 is answered 2026-10-14: marker only (N14 remainder). All 11 open TUI decisions are answered (decisions-2026-10-14.md #3 to #13); their build work sits in N12, N13, N14, X12, X15, T0.3, and the new X17 and X18. **Status 2026-10-14 (second session, user: build everything at once):** X5–X10, X16, the X2/X3 leftovers and T2 are built on branch `claude/all-next-2026-10-14`; see each row for what landed. X18 is not built (#20).

This is the **TUI plan**. It runs side by side with the core plan in
[README.md](README.md); neither is ahead of the other. Before implementing any step, ask the
user which plan and which step to take next.

## Next session (2026-10-14)

Status: **proposed, not confirmed by the user.** Ask which step to start with before any build.
Haiku subagents implement. The main session orchestrates, briefs and merges.

| Order | Card | Note |
|---|---|---|
| 1 | N12 and N13 (deletions) | N12 follow-up (#12): remove `extension_diagnostics` and `custom_themes`. N13 (#7): finish the `js_trim` wrappers. **Done 2026-10-14.** |
| 2 | X17 | Lone ESC: self-pipe in `stdin_hub.rs`. **Done 2026-10-14.** |
| 3 | X12 | Shared abort const (#10). **Done 2026-10-14.** |
| 4 | X15 | Colour follows the switch (#11). **Done 2026-10-14**: `color_enabled()` reads `NO_COLOR` and FORCE_COLOR wins (#18); theme `fg` and `bg` follow chalk. Fill is not changed (see the X15 row). |
| 5 | N14 | Marker only (#3). **Done 2026-10-14**: the `/model` picker shows `expired`; `spawn_refresh` deleted (decisions-2026-10-14.md #19). |
| 6 | X18 | Task panel key hint (#5). **Won't build by default** (#20): the default config binds one key, so it changes nothing. |
| 7 | T0.3 | `scroll-view` waits for its final text (#9) and picks its match explicitly (#21). **Done 2026-10-14.** The startup wait is `wait_for` the footer `mock-model`. The search types the unique query `beta item 25` in one step, so the match is `beta item 25` at 1/1 whatever the view position (user, 2026-10-14). Golden accepted; three checks pass. |

**Open questions** (the user has not decided; each row has a proposed default):

| # | Question | Proposed default |
|---|---|---|
| 1 | `FORCE_COLOR` vs `NO_COLOR` precedence (X15). `hoocode-code-cli/src/lib.rs:81` reads `FORCE_COLOR`. | `FORCE_COLOR` wins. **Answered 2026-10-14: `FORCE_COLOR` wins** (decisions-2026-10-14.md #18). |
| 2 | `spawn_refresh` (`hoocode-code-auth/src/lib.rs:517`) has no UI caller after the N14 marker-only change. | Delete it at build time if nothing else calls it. **Deleted 2026-10-14** (decisions-2026-10-14.md #19). |
| 3 | Does the default config bind more than one key to `app.tasks.cycleForward`? Checked 2026-10-14: no. Only `alt+l` (`hoocode-code-tui-keybindings/src/keybindings.rs:121`). | X18 changes nothing by default. **Answered 2026-10-14: X18 is not built** (decisions-2026-10-14.md #20). |

## Goal

1. Save each subagent's transcript next to its parent session, so `/resume` brings them back
   and an interrupted one can be resumed.
2. List all background work (plan items, subagents, background shell jobs, scheduled jobs,
   MCP) in the task panel, one tab per kind.
3. Attach the screen to a running or finished subagent's (or shell job's) log, read-only,
   and return to the main session.
4. Replace the L1/L2 parity gates with a hoocode-only bar: L1 plus committed screen goldens,
   with a fast LLM visual review of the screens that changed.
5. Simplify every TUI component first (Phase S), right after T0 and before any feature phase.

## Decisions

| Topic | Decision |
|---|---|
| Subagent storage | Sibling dir: `<session>.jsonl` plus `<session>/subagents/<task_id>.jsonl`. The child header's `parent_session` is set. The parent gets a `custom` entry with `customType: "subagent_run"` (task_id, agent type, description, child file, outcome). Not a new entry type: hoocode-ts reads the same `~/.hoocode` sessions and must not choke. Its session listing reads `*.jsonl` only, so the sibling dir is invisible to it. |
| Recovery | View and resume. After a restart a child transcript is browsable read-only. An interrupted child can be resumed on request (existing `resume_task_id` path). No auto-resume. |
| Retention | Child files live as long as the parent session. Success no longer deletes them (this changes [subagents.md](subagents.md) §2, where a clean success deletes its dispatch dir). `.hoocode/dispatch/<task>/` holds only scratch (pid, result.json, output.json). The 24h sweep touches scratch only. |
| Attach | **True transcript swap** (not an overlay). The main transcript is parked and the child's is shown full screen. Header: `◂ explore · <description> · running · Esc back`. |
| Attached input | Read-only. The prompt is disabled with a hint. Esc detaches. |
| Picking a run | `/agents` lists this session's subagent runs (running, done, interrupted) and attaches to the chosen one. Needed when the panel is collapsed. |
| Live log source | The child's full stdout events are forwarded to the UI (`AppEvent::Subagent(task_id, event)`). On attach, history is backfilled from the child's `session.jsonl`. |
| Panel content | Tabs: **Plan · Agents · Shell · Schedule · MCP**. Empty tabs are hidden. Unfocused, the panel shows the current tab plus a one-line count per tab. |
| Panel keys | **Ctrl+G** focuses the panel (Ctrl+T stays thinking). Tab / Shift+Tab switch tabs, Up/Down pick a row, Enter attaches or opens, `x` cancels (with confirm), Esc returns to the prompt. All keys are rebindable. |
| Teams lens | Delete it (about 300 lines with no production producer). Focus is rebuilt on the new row model. |
| Background shell | In scope. `Shell` gets `run_in_background`, a job registry, output to a file, kill, and `ShellOutput`/`ShellKill` tools. Jobs appear in the Shell tab and are attachable. Quitting with jobs running asks once; "kill all" is the default. |
| Done bar | **L1** (fmt, clippy, nextest, dep firewall) **plus hoocode goldens**. Text diffs fail. LLM review is advisory. (Removed 2026-10-10: the hoocode-ts setup hooks and CI.) |
| Golden tiers | (a) In-process component goldens: vt100 at a fixed width, run under nextest, in milliseconds. (b) tmux end-to-end scenarios on the real binary and the mock LLM. |
| Visual review | Text grid plus a compact style legend by default. A PNG is rendered only for changed screens or when the text review is unsure. It is run by a Haiku subagent in-session (no API key, not in CI). |
| Cleanups | **Simplification first** (decision 22): after T0, tier **Now** of Phase S lands before T2. **Next** and **Later** may run alongside the features where files don't overlap. No idea is dropped to narrow the list; ideas move between tiers instead. |
| Who works | **Haiku subagents for all work** (decision 23): review, implement, test, visual review. The main session orchestrates and merges. |
| Replay fixtures | Keep `hoocode-0.5.89/` and `replay.json` as regression tests until T0 has run green for a week; then decide. |
| Migration ledger | Freeze `ledger.py` read-only at T0.6. New work is tracked in this card's phase tables. |

## What we build

### Phase T0: hoocode-only goldens (first, so later phases have a safety net) (done 2026-10-09)

Needs: nothing. Start here.

| Step | Work | Size |
|---|---|---|
| T0.1 | `hoocode-tui-render` test support: `render_golden(component, width) -> String` (vt100 screen plus style runs) and `assert_golden!(name, text)` writing to `tests/golden/<crate>/<name>.txt`. `UPDATE_GOLDENS=1` rewrites. | S |
| T0.2 | Component goldens for the widgets the features touch: task panel, tool block (Agent, Shell), session chip, footer, user and assistant messages. About 30 snapshots. | S |
| T0.3 | `scripts/tui/goldens.py` (forked from `harness.py`, which is deleted 2026-10-10; the hoocode-ts half was dropped): `run <scenario>`, `check` (diff against `tests/golden/tui/<scenario>/*.txt`), `update`. It reuses the scenario JSON, `mockllm.py`, the tmux driver and the `normalize.json` rules (tool-name rules dropped). | M |
| T0.4 | (retired 2026-10-10) `scripts/tui/review_bundle.py`: for changed screens only, writes `target/tui-review/<scenario>/<snap>/{before,after}.txt`, `diff.txt`, a style legend and an optional `after.png` (`grid_to_html` → `render_png.mjs`). Plus `index.md` listing the bundles. | S |
| T0.5 | (retired 2026-10-10) `.claude/skills/tui-review/SKILL.md`: how a Haiku subagent reviews a bundle (checklist: alignment, truncation, colour roles, overflow at 80 and 120 columns, empty and error states). It writes `review.md` per screen with ok / issue / unsure. On unsure, it asks for the PNG. | S |
| T0.6 | Retire parity: done = L1 + `goldens.py check`. Freeze `ledger.py` (read-only, `verify` prints a pointer here). Remove `setup_hoocode.sh` from SessionStart and CI (done 2026-10-10). Update CLAUDE.md ("Done =" line, commands), build-speed.md, ui.md ("Rules for UI changes") and the continue-migration skill. Move the mock and scenarios from `migration/tui-parity/` to `scripts/tui/`. The replay fixtures stay (Decisions). | S |

**Outcome (2026-10-09)**

- **T0.1:** the `hoocode-tui-render` feature `golden` adds `render_golden` and `assert_golden!`. Goldens live in `tests/golden/<crate>/`. `UPDATE_GOLDENS=1` rewrites them.
- **T0.2:** 36 component goldens: 31 in `hoocode-code-tui-widgets`, 5 for the footer in `hoocode-code-tui-app`. Not covered, because they depend on the wall clock: the Shell and Agent elapsed lines, subagent rows in the task panel, and the footer git branch. Covering them needs a clock seam in production code.
  - Follow-up: add an injectable clock when T1 or T3 touches those widgets.
- **T0.3:** `scripts/tui/goldens.py` ran 67 scenarios (165 screens). Cut on 2026-10-10 to about 10 key interactive screens, run once before a release. Print, json, rpc and changelog scenarios are dropped.
  - Flaky fix, **answered 2026-10-14** (decisions-2026-10-14.md #9; open decision 7), size S: `scripts/tui/scenarios/scroll-view.json:22` waits for its final text, not `wait_stable: 1.0`. Drop `mode-cycle` (no scenario exists) and `print-tool-bash` (only a replay fixture) from the flaky list. Quarantine nothing.
  - **Outcome (2026-10-14):** line 22 now waits for `mock-model`. Not verified as a fix. `goldens.py check scroll-view` run 7 times on the real binary: the three search snapshots (`search-typing`, `search-kept`, `search-dropped`) differ in every run since, and the original scenario differs the same way (3 of 3). The difference is the selected match: the golden has `beta item 25` at `2/2`, the binary shows `alpha item 25` at `1/2`. Not a startup race, so it is not fixed here. Open: accept the new screen (`goldens.py update scroll-view`) or fix the search order. Needs the user's call.
- **T0.4 and T0.5:** retired 2026-10-10. `review_bundle.py` and the `tui-review` skill are removed. Screen goldens are a pre-release check of about 10 key screens, not per change.
- **T0.6:** parity is retired. The done bar is L1 plus `goldens.py check all`. `ledger.py` is read-only. Open items:
  - The TUI parity workflow (`tui-parity.yml`) and the guards CI job (naming check and `packages.py --check`) are staged in `archive/migration/ci/step1-workflows.patch`, not in `.github/workflows/`. The user applies it with `git apply archive/migration/ci/step1-workflows.patch` and pushes, because the session token lacks workflow scope.
  - Done 2026-10-10: the fixture fetch is removed. `scripts/ci/fetch_hoocode_fixtures.sh` is deleted, and SessionStart fetches nothing for the TS pin.
- **Replay fixtures** (`hoocode-0.5.89/`) are frozen. `replay.json` moved to `crates/hoocode-code-main/tests/fixtures/` on 2026-10-10. No re-recording path remains.

### Phase S: simplification

Needs: T0. Tier **Now** lands before T2. **Next** and **Later** may run alongside the features
where files don't overlap. All ideas from 15 reviews plus 8 verification passes
(2026-10-09) are kept. None was dropped to narrow the list; only the order differs. Each row
was checked against the code once (V = verified, P = partly true, corrected below). The
subagent that takes a row still confirms it before changing code.

**Now** (bugs, feature prerequisites, cheap wins), in order:

| # | Work | Was | Size |
|---|---|---|---|
| N1 | UTF-8 split across stdin reads becomes U+FFFD. A single read byte >127 is treated as Meta (`stdin_hub.rs:58-61`). Carry the incomplete tail. Add an "every split point" stdin test first. **Done 2026-10-09** (`314d505`). | TM3 V + new | S |
| N2 | The Kitty flag never reaches `tui-keys` (setter only in tests), so a custom Shift+Enter acts as Alt+Enter. Share one flag. **Done 2026-10-09** (`338ef2b`). | KY1 V | S |
| N3 | An over-width line is checked only on the diff paint path. Truncate it in `emit_line` on every path, instead of panicking. **Done 2026-10-09** (`23644d1`). | RN1 V | XS |
| N4 | Toggling thinking display mid-stream collapses the segmented blocks (`refresh` drops `streaming`). The resize half of the claim was wrong. **Done 2026-10-09** (`cff7c47`). | MS1 P | S |
| N5 | Up to 1 s UI stall when a compaction ends or on quit. The progress keepalive sleeps 1 s and is joined. Wake it with a Condvar. **Done 2026-10-09** (`b08109f`, in `hoocode-tui-terminal`). | TM1 V | S |
| N6 | Alt+P / Alt+_ / Alt+] start a string sequence, and the flush emits one dead blob. Split off the ESC and re-feed the rest. **Done 2026-10-09** (`9461fc1`). | TM4 P | S |
| N7 | Small fixes: "Full output: undefined" sent to the model when no temp file exists (`tool-bash/tool.rs:157`). CSI scan ends at any final byte (`ansi.rs:18`). An Esc just before a paste is lost. "/" is ambiguous as the hint separator. Remove the unreachable "undefined" fallbacks. **Done 2026-10-09** (`1c4596e`, `13f667e`, `33fc95c`, `944172f`, `53f4a07`). The hotkey banner now joins dial keys with " or " (not "/"). | new, UT1 P, TM2 V, KY2 P, TB1 P | S |
| N8 | Delete the overlay subsystem (~560 LOC plus 3 test files, no caller outside `tui-render`). Do it before any other `paint` edit. **Done 2026-10-09** (`248c829`): no picker used an overlay, so nothing replaces them; also removed tui-util `extract_segments`, which only overlay compositing used. | RN2 P | M |
| N9 | Delete the teams lens, roster, focus code, `app.team.*` bindings, team hotkey hint and ~17 tests. The `--team` flag is removed too (decision 27). **Done 2026-10-09** (`179b541`): ten team-only tests removed (the row said ~17) and six adapted. Also removed: `TaskAgentKind::Role` (no producer), the `TaskPanelEvent` path, the three `app.team.*` keybindings and the `/hotkeys` rows. The parity fixture `keybindings-gold.json` is updated to match. `TaskAgent.role`, `state` and `handoff` are no longer read by the panel; the store still writes them, so they stay for now. | TP1 V, AP12 hint | M |
| X11 | Task panel inside T1.3/T4 (promoted from Next 2026-10-09; do it with N9): `Task::is_plan_item()` shared with `todo.rs`, the `Plan` rename, `kind` added with the Shell/Schedule rows, and one row model per store version. **Done 2026-10-09** (`2e5dafa`): `is_plan_item()`; `kind()` returns `TaskKind` Plan, Subagent or Mcp (Shell and Schedule come with T4.2 and T5); `TaskPanelView::Flat` is `Plan` (the tab label stays "tasks"); a `RowModel` is rebuilt only when `store.version()` moves, with rows keyed by store id. Rendering is unchanged. | TP3 V, TP4 P, TP2 P | S |
| N10 | Split `interactive_mode.rs` into `mode/*`. Then `Transcript` = the 12 fields `reset_transcript_view` clears, plus `chat.clear()`. **Done 2026-10-09** (`b776ca4` split, `ec1c524` Transcript). The split is 17 files under `src/mode/` (see T1.1); no behaviour change, goldens 67/67 match. Open: the handler signature (T1.2). | IM1 V, IM2 P | M |
| N11 | Tool blocks: one `peek_block` for 6 copies (~70 LOC), with search and web merged first. Agent and Edit respect the peek dial. Agent shows `· <description>`, and `description` ranks above `prompt`. **Done 2026-10-09** (`1f9e4c4` the helper, `8eb437a` the Agent line). `peek_block` (in `tool_output_view.rs`) serves WebFetch, WebSearch, CodeSearch, the plugin tools and Read. The Agent call line shows `· <description>`, else the first line of `prompt`. The Agent result already follows the dial through the fallback. **Follow-up done 2026-10-09** (decision 36, commit `4b9984a`, goldens `2d7ed09`): Edit (its diff body in the call preview and the result, and its error text) and AgentOutput (the card body and the roster) now go through `peek_block`. The one golden that moved is `tool-edit`; the banner rows above the Edit block return because the shorter block gives the bottom-anchored viewport room. The `tool_execution` fallback and Bash keep their own bodies (the fallback's hint styles the space and `)` differently, so its bytes would move; Bash trims by visual line). | TB2 P, TB3 V, TB4 V | S |
| N12 | Dead-code sweep, verified only (tests that use the code are edited too): resource Canvases/Plugins/Extensions sections, `RAIL`, the `sort` argument; `cancellable_loader.rs`; `Loader.text`; `truncate_primary` and its parameters; the local `impl Clone` in `settings_list` → `derive`; `lex_inline`; the JSON dumpers → `tests/it/common`; RN3 APIs + `cursor_row` + `on_debug` + `wants_key_release`; `list_languages`, `highlight_auto_tree`, `once_cell`, private `engine`/`value`; Sixel (decision 24) with WT → no images (Sixel path only; Kitty and iTerm2 image rendering stay); `delete_all_kitty_images`; `is_key_repeat`, `event_type`; `get_effective_config`; `get_color_mode`, `ThemeJson::color`/`to_value`, `regex_escape`, `has_active_codes`; `LEGACY_TOOL_OUTPUT_VIEWS`, `_cwd`, `let _ = js_trim`; `js_line_count`; the no-op `CustomMessageComponent::set_expanded`; `get_padding_x`, `get_autocomplete_max_visible`, `is_focused`, `with_bg_fn`; `notification_panel::dismiss`; `chrome_layout::layout`; `ExpandableText::is_expanded`; `footer::active_subagent_count` with one `list()` pass. **Done 2026-10-09** (`eb962eb`, `b54de43`, `16be6b5`). Sixel: no Sixel variant, decoder hook, `sixel` override or save/restore wrapper; Windows Terminal keeps truecolor with no images; `ImageRenderOptions.mime_type` (Sixel-only) goes too. The rest is removed as listed (grep and the compiler as the check), plus two unlisted items that were provably unused: `Category::Marketplaces` and `SessionEvent::event_type` (now a helper in the one test that used it). Also: the private `engine`/`value` modules expose the `illegal` field and two helpers, all removed. "RN3 APIs" is read as the unused or write-only renderer items: `wants_key_release`, `on_debug`, `cursor_row`, `set_child`, `clear_children`, `get_show_hardware_cursor`, `get_clear_on_shrink`. The startup listing has no Extensions or Canvases cells (production never filled them). Open: `archive/migration/ts-tests.json` still cites the deleted Sixel test port; the generator needs the TS pin, so it is regenerated later. `ResourceListing.custom_themes` and `extension_diagnostics` have no production producer either. **Follow-up 2026-10-14, done** (decisions-2026-10-14.md #12, size S): removed both fields (`crates/hoocode-code-tui-app/src/resource_display.rs:56`, `:58`), their render branches (`:434`, `:475-476`, `:642-644`, `:670`) and their tests. The startup listing loses its Themes cell; `theme_diagnostics` stays. | AP1 AP2 AP4 AP5 AP6 AP11 MC3 MC4 MC5 MC6 MC11 RN3 HI2 HI3 HI5 KY6 KY9 UT7 TB5 MS8 ED2 | M |
| N13 | One copy of each helper: `js_trim` (3 identical copies; the editor copy differs, decide U+0085), `group_digits` (2) → `agent-session/format.rs`, one `SEGMENT_SEP` (agent-session is the lower layer), `text_slice` (tool-api re-exports tui-util), one `js_round` (6 copies) in tui-util, the `resume_picker` doc comment. **Done 2026-10-09** (`044a60b` js_trim, js_round, text_slice; `9a436f4` Cargo.lock; `1b0a3af` group_digits, SEGMENT_SEP, `resume_picker`). `js_trim` is `js_regex::js_trim` in tui-util. The editor and tool-search copies trimmed U+0085; they now use the shared JS rule (open question 1). `js_round` is `tui-util::js_math::js_round`, seven copies in all (the row said six; `code-tools-fs` context GC was the seventh). tool-search, tool-api, code-media and code-tools-fs now depend on tui-util. **Left as they are:** the u16 `js_trim` in `code-tools-fs` edit_diff (it trims UTF-16 units); the `is_js_space` copies in tui-widgets (`lib.rs`, `assistant_message.rs`), agent-harness (`messages.rs`, `output_compression.rs`) and `jsdiff`'s `is_ws`. Those are the same rule under other names; merging them is a follow-up. **Follow-up 2026-10-14, done** (decisions-2026-10-14.md #7, size XS): deleted the `is_js_space` wrappers in `crates/hoocode-code-tui-widgets/src/lib.rs` and `assistant_message.rs`. Callers use `hoocode_tui_util::js_regex::is_js_space` (the same rule), not `js_trim`, because `js_trim` is a string trim and the callers pass a char predicate to `trim_start_matches` and `all`. | UT2 P, UT4 V, IM9, UT11 V, AP7 P, PK8 P | S |
| N14 | Move the 9 UI-thread `block_on` session ops (resume, new, fork, tree, `/cd`, reload, import, `/mode`) to the runtime, one op at a time starting with `/mode`, each behind a loader. **Partly done 2026-10-09**: all of them now run off the UI thread through `AppEvent::SessionOp` (`mode/session_op.rs`); `/new` also covers a command's `newSession`. While one runs, a loader shows and the prompt refuses input (the text returns to the editor). Left, each with a `TODO(N14)` comment: (a) the exit-time `session.abort()` in `mode/mod.rs` `dispose`, which must finish before the session flush (needs the ordered shutdown of concurrency.md §4); (b) auth, **closed 2026-10-09 without a UI change**: the menu path never refreshed. `get_available_models` uses `has_auth`, which does no I/O; the test `menu_path_shows_an_expired_token_without_network_refresh` pins that. `code-auth` now has `AuthStorage::readiness` (Ready, NeedsRefresh, Missing; no I/O) and `spawn_refresh` (single-flight per provider, on its own thread), with tests. Decision 34's UI (an unavailable marker, a background refresh, a menu redraw) was not wired. **Answered 2026-10-14: marker only** (decisions-2026-10-14.md #3). Remainder, size S: the model menu (`crates/hoocode-code-tui-app/src/mode/models.rs`) reads `AuthStorage::readiness` (`crates/hoocode-code-auth/src/lib.rs:505`, no I/O) and shows an `expired` marker on NeedsRefresh. No background refresh, no redraw. **Built 2026-10-14**: the marker is in; `spawn_refresh` is deleted (no caller). This closes the N14 UI remainder. `get_api_key_blocking` stays for non-UI callers. Its last UI-reachable caller, `uses_anthropic_subscription_auth`, returns early for OAuth, so no refresh runs there. The clippy `disallowed-methods` rule was not added: it would also hit 16 sites in 9 other crates. It comes with core concurrency phase 1 (decision 35). Until then a test (`every_ui_thread_block_on_is_a_marked_n14_exception`) fails on any `block_on(` in `mode/` without a `TODO(N14)` above it. Fixes UI freezes during slow session loads. Pulled into Now by the user (formerly X1). Under this row only: the UI-thread `block_on` removal in [concurrency.md](concurrency.md) phase 1 (its runtime and caps parts stay there). | IM5 V | L |

**Next** (worth doing, larger or visible):

| # | Work | Was | Size |
|---|---|---|---|
| X2 | Bash streaming: `update_display` joins and truncates the whole buffer on every chunk (O(N²), unbounded memory). Use a bounded tail. Compute tool result text once and hold `ToolResult` in an `Rc`. **Partly done 2026-10-14** (`a7a3129`): Bash `!` output keeps a bounded tail (`OutputTail`, ≤801 lines / 2×max bytes) and renders byte-identical. `ToolResult` is held in `Rc` (tool block, `ToolSignalInput`). Left: compute the result text once (needs the renderer API changed for ~6 renderers). | TB7 P, TB8 V | S |
| X3 | `read.rs` highlights the whole file on every render. Highlight the visible prefix and memo it, reusing the write tool's `HighlightCache` pattern. Memo the Edit diff. **Partly done 2026-10-14** (`a7a3129`): Read preview memo (total line count and the first peek lines) and Edit diff memo. Both rebuild when the output, language or theme changes (`Arc::ptr_eq` on the theme). Left: highlighting only the visible prefix. The highlighter adds a closing colour line for an open token at end of input, which changes the "N more lines" count. | TB6 P, HI6 corrected | S |
| X4 | Images: decode only the header for dimensions. Cache the encoded sequence on `Image`. **Done 2026-10-14** (`a7a3129`): only the header is decoded for dimensions (`decode_prefix`). JPEG scans a prefix first, then falls back to the full decode. `Image` caches its rendered lines under a `RenderKey` (protocol, width, cell size, id). | HI7 V | S |
| X5 | Grammar: a small name/alias index so `supports_language` doesn't parse the 1.1 MB JSON, then per-language load. Measure the first-code-block stall first. **Done 2026-10-14**: `supports_language` reads a generated index (`src/grammar_index.rs`: 191 languages and 175 aliases, in the dump's order). Each language loads its own chunk (`data/grammars/<name>.json`: the nodes it reaches; a node shared by two languages stays one object per thread). `index_matches_the_dump` fails on drift; `grammar_index::regenerate` (ignored) rewrites both. Stall, fresh process, debug build, median of 10 (`tests/it/first_block.rs`): before, `supports_language` 91 ms plus first highlight 24 ms, total 115 ms; after, 0.015 ms plus 22 ms, total 22 ms. The highlight itself is unchanged (grammar compile dominates). Gold output byte-identical. Review 2026-10-14: the shared node table, alias lookup and `supports_language` (answered from the same index as `highlight`) hold; a one-word edit to a chunk fails `index_matches_the_dump`. The dump is test-only. | HI8 V | M–L |
| X6 | `PickerCursor` in tui-components: window (7 copies), `step` (**clamp at both ends, every picker**, decision 29), the "(n/len)" footer (5 copies), and `set_query` for the 3 fuzzy pickers. `/model`, `/tree`, the user-message picker and scoped models stop wrapping (visible). PgUp/PgDn and Home/End jump. | PK1 P, PK2 corrected, PK3 corrected, PK4 P | S |
| X7 | One `home_dir` + `shorten_path` policy in a std-only leaf (7 copies, 6 behaviours). Rule: HOME if set and non-empty, else the passwd entry, never "/" (decision 28). Paths shown when HOME is unset change. | UT5 P | M | **Done 2026-10-14**: new leaf `hoocode-home` (`home_dir`, `shorten_path`). Passwd fallback via `dirs` 5, which the workspace already uses (libc is not a workspace dep, and std has no passwd lookup). The 6 shorten helpers and the home lookups now call it. `home_dir` is `None` when there is no home, never `/`. `shorten_path` matches whole components only, so `/home/al` leaves `/home/alice` alone. `code-paths::home_dir` keeps a `.` fallback for path-returning callers. Changed when HOME is unset or empty: `~` uses the passwd home (was `/`), and `/cd` with no home warns. USERPROFILE is no longer read (dirs covers Windows). Review 2026-10-14: `shorten_path` takes `\` as a separator on Windows (it had only `/`, so Windows paths lost their `~`). On Windows with HOME set, the resource and config paths now prefer HOME as the footer and tools already did (they used the profile folder before). |
| X8 | One `ansi_runs()` iterator for the 10 ANSI scan loops. Keep two named tab widths (render 3, editor and input 4); both are deliberate. **Done 2026-10-14**: nine of the ten loops now walk `ansi_runs` (`crates/hoocode-tui-util/src/ansi.rs`, items `AnsiRun::Ansi` and `AnsiRun::Text`; `visible_width` now shares the strip in `text.rs`). The tenth, `repair_nested_bg_resets`, stays hand-written: it matches SGR-shaped text inside OSC payloads and after non-SGR CSI, which the shared scanner skips, so moving it would change bytes. The tab widths are `RENDER_TAB_WIDTH` (3) and `EDITOR_TAB_WIDTH` (4) in tui-util, used at six sites (the render sites: `visible_width`, the markdown and text components, and the tool-renderer `replace_tabs`; and the editor and input paste). Review 2026-10-14 found the `replace_tabs` site still literal and changed it. The markdown `lexer.rs` 4-space expansion is marked's own tab stop, not a surface width: it is `MARKED_TAB` there, and the render paths expand tabs to 3 before lexing. | UT3 P | M |
| X9 | Renderer after N8: one frame copy on full redraw (RN5); `Option` sentinels plus an explicit force-redraw flag (RN6); `move_rows` shared with `Terminal::move_by` (RN4); then split `paint` (RN9). **Done 2026-10-14** (`eedda2a`, `6b258e7`, `658125f`, `34a0926`): a full redraw copies the frame once (`full_render` takes it by value; it made two). `previous_width` and `previous_height` are `Option` (0 still means unset), the diff keeps an `Option` row range, and `request_render(true)` sets `force_redraw` instead of writing -1 sizes. `move_rows` in `hoocode-tui-terminal` serves the renderer and `Terminal::move_by`. `paint` is split into named steps with the same bytes written. Left: the scroll search `index` keeps -1, because -1 is a real position (before the first match), not a sentinel. The RN4 to RN9 IDs are not in this doc's review section, so each finding was checked against the code. | RN4–RN6 V/P, RN9 P | M |
| X10 | Loop state: an `ActivePicker` enum for the picker slots only (not the `pending_*` receivers); the 5 `try_recv` pollers → `AppEvent`; one subagent dispatch helper; `EditorChanged` only on a `!` transition. | IM3 P, IM4 P, IM6 P, IM7 V | M |
| X12 | Messages: `summary_sheet` for branch and compaction (2 copies); `BlockKey` enum and no deep clone per delta; a shared `ABORTED_MESSAGE` const (it misses the OpenAI "aborted." variant today); `after_indent` returns an `Option`. **Partly done 2026-10-14** (`a7a3129`): `summary_sheet` is shared; `BlockKey` is an enum; `after_indent` returns `Option`. The streaming message is an `Rc`, so there is no deep clone per delta. Also fixed: the newest delta is stored before the throttled render runs. `ABORTED_MESSAGE` is a const. **Decided 2026-10-14** (decisions-2026-10-14.md #10): one shared const, "Request was aborted", no period. **Done 2026-10-14:** `ABORTED_MESSAGE` lives in `hoocode-ai-types` (`lib.rs`). Every provider uses it (anthropic, openai, openai-responses, openai-codex, google, google-gemini-cli, faux); the OpenAI pre-response branch and Gemini's local copy are gone. The TUI widget uses it too. Google's cancel text "This operation was aborted" also moved to the shared text (not named in the card, read as "all providers"). Users see the new text. The dep firewall passes. | MS2 P, MS3 V, MS4 V, MS6 P, MS5 P | S |
| X13 | Terminal: one paste-end helper searching only the new tail (with a split-marker overlap); one `terminated_by`; `parse_mouse_report -> (event, len)`; one ESC-deadline clock, with the test-only methods under `cfg(test)`; drop the unused `_now`. **Done 2026-10-14** (`a7a3129`): one paste-end helper searches only the new tail (split-marker tests cover it). `string_sequence_status` replaces the three terminator checks. `parse_mouse_report` returns `Option<(Option<MouseEvent>, len)>`: the inner `None` means a malformed X10 report, which is consumed. `Feed::arm_flush` is the one writer of the ESC deadline. Test-only methods are under `cfg(test)`, and `_now` is dropped. N6 is unchanged. | TM5 V, TM6 V, TM7 V, TM9 P, TM11 P | S |
| X14 | Keys: a pre-parsed `KeySpec` plus a `\x1b[` fast path; a pre-lowered fuzzy matcher, also used by the session search; a `OnceLock` for `keybindings()`; the dead `kitty_active` parameter and an args struct for `matches_left_right`. **Done 2026-10-14** (`a7a3129`): `KeySpec` is parsed once (the keybindings manager keeps `specs_by_id`). About 30 literal `matches_key` sites still parse per call. Also: a `\x1b[` fast path; `FuzzyQuery` (pre-lowered, shared with the session search); a `OnceLock` keybinding table; `kitty_active` removed from `matches_arrow`; `LeftRightArgs`. | KY3 V, KY4 V, KY10 V, KY9 P, KY5 P | S |
| X15 | Theme and util: wire `chalk::set_enabled(color_enabled())`; pass the bg opener once per band; skip the tab copy when there are no tabs; `JsRegex::search` returns `Option` (still UTF-16). **Done 2026-10-14** (`a7a3129`), with one open point: `chalk::set_enabled(color_enabled())` in the CLI `main`; `band_opener` once per band (Text, Markdown); `visible_width` copies only when there are tabs; `JsRegex::search` returns `Option`. Open, now decided 2026-10-14 (decisions-2026-10-14.md #11), size S: both follow the switch. **Built 2026-10-14** (#18 for precedence; `fill` chips still emit colour with the switch off, not in the decision). `theme.fg/bg` still emit colour when colour is off, because only bold, italic and the like honour chalk. Fix in `crates/hoocode-code-tui-theme/src/theme.rs:222-227` (chalk at `chalk.rs:12`). `color_enabled()` (`crates/hoocode-code-cli/src/lib.rs:80`) must read `NO_COLOR`: any non-empty value means off. | UT6 V, UT8 P, UT10 P | S |
| X16 | Small structure: `startup_progress` → a single slot, removing the dead Download/Work branches; one scroll-key table; camelCase TS names → Rust names (file provenance stays); j/k as named bindings in `extension_selector`; frame sizing helpers shared with editor and box; the shared undo-coalescing predicate; `apply_completion` helper (3 tails); one autocomplete item builder, with `CmdView` → `AutocompleteItem`; a keyed `file_search` slot; drop the unused bold/underline/strike flags. | AP8 V, AP9 V, AP13 P, PK7 V, ED3 P, ED4 P, MC7 P, MC8 V, MC9 V, MC2 V | M |
| X17 | Terminal: a lone ESC is not flushed after a resubscribe. `subscribe` (`crates/hoocode-tui-terminal/src/stdin_hub.rs:200-215`) replays held chunks, but the reader (`:227-260`) stays in `poll(-1)`, so ESC waits for the next byte, and ESC then `a` reads as Alt+a. Add a self-pipe that wakes the unix poll loop, so the reader re-reads `flush_at`. Regression tests: a replayed lone ESC flushes as Escape within the timeout, and ESC followed later by `a` is not Alt+a. **Done 2026-10-14** (`stdin_hub::wake_tests`; the test fails with the wake call removed). **New 2026-10-14** (decisions-2026-10-14.md #13; open decision 11). | new | S |
| X18 | **Won't build by default** (decisions-2026-10-14.md #20). Task panel key hint: show every key of `app.tasks.cycleForward` joined with " or ", the same format as the mode dial (N7). Today `app_key_label` (`crates/hoocode-code-tui-keybindings/src/hints.rs:101`) returns only the first key, and `crates/hoocode-code-tui-widgets/src/task_panel.rs:304` uses it. Use `get_keys` instead. A golden moves only when the key has a second binding. **New 2026-10-14** (decisions-2026-10-14.md #5; open decision 3). | new | XS |

**Later** (keep; do only after `/perf` shows the cost, or when the file is touched anyway):

| # | Work | Was |
|---|---|---|
| L1 | Render caches: a markdown version counter (no full-text compare); an editor layout cache or a whole-editor render cache; `Component::render_into`; a per-child line cache behind the debug "diff equals full render" assertion | MC1, ED1, RN8, RN10 |
| L2 | I/O: batch output writes (never across a frame); stdin on `&str`; fold the Kitty timer thread into the runtime work | TM8, TM10, TM12 |
| L3 | Theme lookup without a Mutex and an Arc clone per call; enum-indexed colour table | UT9 |
| L4 | Types: unsigned sizes, signed deltas; direct slicing for `find` offsets | RN11, RN7 |
| L5 | Readability: `format_parsed_key` match; capability table; one ctx builder in `tool_execution`; one-pass `chain_phrase`; tree glyph constants; `Editor::new` `mem::replace`; `derive(Default)` in perf; inline the `ThemeFactory` alias | KY7, HI4, TB9, TB10, PK6, ED6, AP10, MS7 |
| L6 | Tests: merge the duplicate fuzzy suites and the `_ts` case-for-case suites into named tests | KY11 |
| L7 | Behaviour changes held until asked: `@file` ranking via tui-fuzzy (changes which files appear); editor cursor in bytes (only with a real emoji/paste bug); a generic `PickerView` wrapper; tips catalog trimming; `get_conflicts` surfaced in `/hotkeys` | MC10, ED5, PK5, AP3, KY8 |

**Checked, not an issue** (kept for the record): HI1 (an empty `subLanguage` already
auto-detects); IM8 (already one keybinding pass); MS7's "unused" claim (`ThemeFactory` is
used); PK2 as first stated (`/login`, `/logout` and `/resume` already agree; the real split is
across pickers → X6); PK3 as first stated (those lists use `SelectedRowList` → X6); HI6 as
first stated (only one `highlight_code` → X3).

### Phase T1: prerequisites (refactors, no visible change; goldens must stay identical)

Done in Phase S: T1.1 and T1.2 = N10 (T1.2 except the handler signature, see its row), T1.4 = N12 (footer). T1.3 = X11 is only partly done: the row model, `kind` for Plan, Subagent and Mcp, and the teams removal are in. The Shell and Schedule kinds wait for T4.2 and T5, so T1.3 stays open until then.

| Step | Work | Size |
|---|---|---|
| T1.1 | Split `interactive_mode.rs` (6,624 LOC, 85 fields) into `mode/{mod,events,transcript,prompt_queue,session_ops,commands,dialogs,subagents,...}.rs`. Mechanical: several `impl Mode` blocks. **Done 2026-10-09** with N10 (`b776ca4`): `mode/` holds `mod` (struct, setup, loop) and 16 concern modules (`input`, `transcript`, `events`, `prompt_queue`, `session_ops`, `tree`, `models`, `auth`, `settings`, `dialogs`, `subagents`, `bash`, `clipboard`, `mcp`, `chrome`, `commands`). The crate module is `mode` (was `interactive_mode`). | M |
| T1.2 | `Transcript` struct: owns the chat `Container` and the ~20 parallel fields `reset_transcript_view` clears (`chains`, `open_chain`, `streaming`, `pending_tools`, `tool_output_view`, ...). `handle_session_event` and `handle_agent_event` write through a target `&mut Transcript`. Reset is a replace. **Partly done 2026-10-09** with N10 (`ec1c524`): the struct holds the chat and the 12 fields reset clears (`tool_output_view` is not one of them; reset never cleared it, so it stays on `Mode`); reset replaces it. Open: the two handlers still take `&mut Mode`, because they call Mode-wide methods, so a `&mut Transcript` target needs a design decision. | M |
| T1.3 | Task panel: delete the teams/roles path. Give `Task` a `kind` (Plan, Subagent, Shell, Schedule, Mcp). Build one row model per store `version()`, keyed by a stable row id, not an index. Stop cloning on every frame. | S |
| T1.4 | `footer.rs:subagent_counts`: one pass, delete `active_subagent_count`. **Done 2026-10-09** with N12 (`b54de43`). | XS |

### Phase T2: subagent storage and recovery

Needs: Phase S tier Now.


| Step | Work | Size |
|---|---|---|
| T2.1 | Pool: the child's `--session` points at `<parent>/subagents/<task_id>.jsonl`. `Header.parent_session` is set. `dispatch-log.json` and ledger lines carry the parent session id. | S |
| T2.2 | Parent session: a `subagent_run` custom entry at dispatch, and a second one with the outcome at settle (append-only; the last one wins). Success no longer deletes the transcript. The sweep spares `subagents/`. Deleting a session deletes its dir. | S |
| T2.3 | `/resume`: subagent runs nest under their parent session in the threaded view. Search matches the parent row only. | S |
| T2.4 | After restart, `subagent_run` runs with no outcome entry are shown as `interrupted`. Enter on such a row offers View / Resume (the existing `SubagentPool::resume`). | S |

### Phase T3: live events and attach

Needs: T1.2 and T2.


| Step | Work | Size |
|---|---|---|
| T3.1 | Forward every child stdout event: a pool listener sends `AppEvent::Subagent(task_id, event)`. The child also emits `message_update` deltas, throttled to the existing streaming render rate. | S |
| T3.2 | The Agent tool emits a partial at dispatch carrying `task_id`, so the tool block knows its run. The block shows `Agent explore · <description>` and a 5-line tail of the live activity. | S |
| T3.3 | Attach: park the main `Transcript`, build the child's from its jsonl (read on a worker thread, per concurrency.md: no file I/O on the UI thread; `render_session_context` on arrival), and route that task's live events into it. Main-session events keep updating the parked view. Esc restores it with its scroll offset. The prompt is disabled while attached. The header row shows state. | M |
| T3.4 | Attach from three places: the panel (Enter), the Agent tool block (a key on the selected block), and `/agents` (a picker of runs in this session). Add `/agents` to `BUILTIN_SLASH_COMMANDS` and `ui.md`. | S |

### Phase T4: activity panel tabs

Needs: T1.3. Attach from the panel needs T3.


| Step | Work | Size |
|---|---|---|
| T4.1 | Panel tabs over the T1.3 row model. The unfocused view shows the current tab plus counts. Ctrl+G focuses it. Keys are as in Decisions. The tick runs while any row is running. | M |
| T4.2 | Schedule tab: the scheduler publishes fire, run and finish plus next run time into the store. The store and tick exist today; `/loop` itself is core milestone 7 ([scheduler-and-loop.md](scheduler-and-loop.md)), and the tab shows its jobs once it lands. | S |
| T4.3 | MCP tab: `McpHub` publishes per-server state (connecting, ready, auth needed, failed) and in-flight calls longer than 1 s. Enter on an auth-needed row runs `/mcp login`. `x` aborts a call. | S |

### Phase T5: background shell

Needs: T4.1 for the tab and T3.3 for attach. The registry and tools (T5.1, T5.2) can start after tier Now.


| Step | Work | Size |
|---|---|---|
| T5.1 | Job registry (`hoocode-code-tool-bash`): spawn with output to `<session>/jobs/<id>.log`, a kill handle, exit status. Session-scoped. On quit with jobs running, ask once ("kill all" default, or leave running). | M |
| T5.2 | `Shell` gets `run_in_background`. New opt-in tools `ShellOutput(id, since?)` and `ShellKill(id)`. Completion is a notice to the model at the next turn. | S |
| T5.3 | Shell tab rows plus attach (a tail of the log file, the same swap view as T3.3 with a plain-text transcript). | S |

### How Haiku subagents do the work

- Every piece of work, including reviews, tests and visual checks, goes to a Haiku subagent.
  One phase step or inventory row = one Haiku subagent in its own worktree, with a precise brief: files, the
  acceptance test, and the goldens that must stay unchanged or the ones it may update.
- An inventory row is a claim from a read-only review. The subagent confirms it first and
  drops it if it doesn't hold.
- Each one runs L1 for its crates and `goldens.py check`. On a visual change, it produces a
  screen golden diff and reads it. The orchestrator (main session) merges.
- Parallel only where files don't overlap: T0.1/T0.3 together, Phase S rows by crate, T4.2/T4.3 together, T5.1/T5.2 with T3. T1.1 → T1.2 → T3.3 are serial.
- Before starting any step, the orchestrator asks the user which plan (core or TUI) and step.

## Not doing

- Prompting or steering a subagent while attached (read-only by decision; T1.2 keeps it
  possible later).
- Auto-resuming interrupted subagents on `/resume`.
- An LLM review that gates the build, or that runs in CI.
- Split view of main and subagent side by side.
- Parity with hoocode-ts screens. New screens may differ freely.
- In-process subagents (still processes, per decisions-2026-10-08).

## Open questions

1. **Editor `js_trim` (N13):** the editor's copy also trims U+0085. Decided 2026-10-09: the shared
   JS-whitespace rule (drops U+0085) is used, matching the rest of the TUI. The lexical copy in tool-search
   changed the same way.

## Open decisions (2026-10-09)

**All 11 answered 2026-10-14.** The answers are in [decisions-2026-10-14.md](decisions-2026-10-14.md) (#3 to #13). Each item keeps its text as the record, with its status in front.

1. **ANSWERED 2026-10-14** (#3: marker only). **Decision 34, expired-token menu UI.** Recommended: drop it. Menus never blocked (see 82c684b).
   The unavailable marker and background redraw would hide a model that a request-time refresh
   would fix.
2. **ANSWERED 2026-10-14** (#4: confirmed). **N6, unterminated ESC P, `_` or `]`.** The parser reads it as Alt+key. Confirm that reading.
3. **ANSWERED 2026-10-14** (#5: every key, joined with " or "; card X18). **Banner wording.** The panel key hint says "shift+tab or tab". Confirm the wording.
4. **ANSWERED 2026-10-14** (#6: keep the trim). **Edit peek.** It also trims the pending call-slot diff. Confirm that stays.
5. **ANSWERED 2026-10-14** (#7: finish the wrappers; the dead fields are #12). **Cleanups (partly done 2026-10-14):**
   - Duplicates, whitespace part: **done 2026-10-14** (`a7a3129`). One copy in `hoocode-tui-util`
     js_regex, including `is_js_space_unit` and `js_trim_units`. `hoocode-agent-harness` keeps one
     crate-local copy, because agent-* must not depend on tui-*. Left: the wrappers in tui-widgets
     `lib.rs:35` and `assistant_message.rs:33` over the tui-components markdown `js_trim`. `jsdiff.rs`'s `is_ws`
     and the u16 `js_trim` in `edit_diff.rs` now use the tui-util copies.
   - `TaskAgent.handoff`: **removed 2026-10-14** (`a7a3129`). `role` stays (one tui-widgets test
     reads it). `state` stays (tests read it).
   - `--team`: **fixed 2026-10-14** (`a7a3129`) in `docs/design/extras.md`.
   - Sixel entry: **removed 2026-10-14** (`a7a3129`) from `archive/migration/ts-tests.json` by hand.
     Regenerate the file when the TS pin returns.
   - `ResourceListing.custom_themes` and `extension_diagnostics`: **removed 2026-10-14** (decision 12). Nothing produced them, so their render branches went too.
6. **ANSWERED 2026-10-14** (#8: wait; each lands with its feature). **Shell and Schedule `TaskKind` variants.** Deferred to T5 and T4.2. T1.3 stays open until then.
7. **ANSWERED 2026-10-14** (#9: fix and prune; quarantine nothing; work in T0.3). **Flaky tests** (fix or quarantine):
   - Fixed 2026-10-14 (`244ab05`): races `hoocode-code-rpc` `fork_branches` and `a_closed_agent`.
   - Fixed 2026-10-14 (`50efc28`): `hoocode-ai-oauth-anthropic` tests use an ephemeral port (production stays 53692).
   - Pruned 2026-10-14: `mode-cycle` (no scenario exists) and `print-tool-bash` (only a replay fixture) are off the list.
   - Still open: `scroll-view` (search snapshots, see T0.3 outcome).
8. **ANSWERED 2026-10-14** (#10: one shared const, no period). **OpenAI abort text (X12).** `ABORTED_MESSAGE` is one const now. The OpenAI path says "Request was
   aborted." Decide whether every provider uses that text. It changes visible text.
9. **ANSWERED 2026-10-14** (#11: both follow the switch). **Colour when colour is off (X15).** `theme.fg/bg` still emit colour when colour is off; only bold,
   italic and the like honour chalk. `color_enabled()` ignores `NO_COLOR`. Decide whether both follow
   the switch.
10. **ANSWERED 2026-10-14** (#12: remove the fields, branches and tests, N12). **Dead render branches (cleanup 5).** `ResourceListing.custom_themes` and `extension_diagnostics`
    are read by render code, and nothing produces them. Remove the branches and the fields, or keep
    them for a planned producer?
11. **ANSWERED 2026-10-14** (#13: card X17). **Known bug: lone ESC not flushed (`hoocode-tui-terminal` stdin_hub).** A pre-existing race. After
    `Terminal::stop()` and `start()`, a lone ESC read while unsubscribed is replayed on subscribe, but
    the reader stays in `poll(-1)`. So ESC is not flushed until the next byte, and ESC then `a` reads as
    Alt+a. The fix needs a self-pipe to wake the unix poll loop. Suggested as a card; not started.

## Details

### Review findings that shaped the plan

| Area (LOC) | Finding used here |
|---|---|
| `interactive_mode.rs` (6,624) | The transcript is a re-rendered component tree, not terminal scrollback (`ui.md` said otherwise; fixed in the same commit as this card). A swap needs `Transcript` (T1.2). Nine UI-thread `block_on` calls break concurrency.md §4 (backlog B1). |
| Task panel (1,100) | Teams/focus is unreachable. The store is memory-only. Shell, scheduler and MCP are absent. `render` clones the store every frame. |
| Subagents | Success deletes the transcript (`pool.rs` settle). Nothing links the child to its parent. Progress is collapsed to one string in `instance.rs`. stdin is null. |
| Tool blocks (4,384) | The Agent block can't learn its task id until it finishes. It hides `description`. AgentOutput and Edit ignore the peek dial. |
| Parity harness | Scenario JSON, `mockllm.py`, the tmux driver, `grid_to_html` and `render_png.mjs` are reusable as is. vt100 is already a dev-dep. |

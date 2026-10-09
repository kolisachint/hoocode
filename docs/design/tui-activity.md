# TUI: activity panel, subagent attach, and hoocode-only goldens

Status: **design, agreed 2026-10-09** ([decisions-2026-10-09.md](decisions-2026-10-09.md)).
No code yet. Input: eight read-only reviews of the TUI, subagent and parity code
(summarised in Details).

## Goal

1. Save each subagent's transcript next to its parent session, so `/resume` brings them back
   and an interrupted one can be resumed.
2. List all background work (plan items, subagents, background shell jobs, scheduled jobs,
   MCP) in the task panel, one tab per kind.
3. Attach the screen to a running or finished subagent's (or shell job's) log, read-only,
   and return to the main session.
4. Replace the L1/L2 parity gates with a hoocode-only bar: L1 plus committed screen goldens,
   with a fast LLM visual review of the screens that changed.
5. Do the TUI simplifications these features need first. Keep the rest as a ranked backlog.

## Decisions

| Topic | Decision |
|---|---|
| Subagent storage | Sibling dir: `<session>.jsonl` plus `<session>/subagents/<task_id>.jsonl`. The child header's `parent_session` is set. The parent gets a `subagent_run` entry (task_id, agent type, description, child file, outcome). |
| Recovery | View and resume. After a restart a child transcript is browsable read-only. An interrupted child can be resumed on request (existing `resume_task_id` path). No auto-resume. |
| Retention | Child files live as long as the parent session. Success no longer deletes them. `.hoocode/dispatch/<task>/` holds only scratch (pid, result.json, output.json). The 24h sweep touches scratch only. |
| Attach | **True transcript swap** (not an overlay). The main transcript is parked and the child's is shown full screen. Header: `◂ explore · <description> · running · Esc back`. |
| Attached input | Read-only. The prompt is disabled with a hint. Esc detaches. |
| Live log source | The child's full stdout events are forwarded to the UI (`AppEvent::Subagent(task_id, event)`). On attach, history is backfilled from the child's `session.jsonl`. |
| Panel content | Tabs: **Plan · Agents · Shell · Schedule · MCP**. Empty tabs are hidden. Unfocused, the panel shows the current tab plus a one-line count per tab. |
| Panel keys | **Ctrl+G** focuses the panel (Ctrl+T stays thinking). Tab / Shift+Tab switch tabs, Up/Down pick a row, Enter attaches or opens, `x` cancels (with confirm), Esc returns to the prompt. All keys are rebindable. |
| Teams lens | Delete it (about 300 lines with no production producer). Focus is rebuilt on the new row model. |
| Background shell | In scope. `Shell` gets `run_in_background`, a job registry, output to a file, kill, and `ShellOutput`/`ShellKill` tools. Jobs appear in the Shell tab and are attachable. |
| Done bar | **L1** (fmt, clippy, nextest, dep firewall) **plus hoocode goldens**. Text diffs fail. LLM review is advisory. hoocode-ts setup leaves hooks and CI. |
| Golden tiers | (a) In-process component goldens: vt100 at a fixed width, run under nextest, in milliseconds. (b) tmux end-to-end scenarios on the real binary and the mock LLM. |
| Visual review | Text grid plus a compact style legend by default. A PNG is rendered only for changed screens or when the text review is unsure. It is run by a Haiku subagent in-session (no API key, not in CI). |
| Cleanups | Only the prerequisites first. The rest is a ranked backlog for later Haiku batches. |

## What we build

### Phase T0: hoocode-only goldens (first, so later phases have a safety net)

| Step | Work | Size |
|---|---|---|
| T0.1 | `hoocode-tui-render` test support: `render_golden(component, width) -> String` (vt100 screen plus style runs) and `assert_golden!(name, text)` writing to `tests/golden/<crate>/<name>.txt`. `UPDATE_GOLDENS=1` rewrites. | S |
| T0.2 | Component goldens for the widgets the features touch: task panel, tool block (Agent, Shell), session chip, footer, user and assistant messages. About 30 snapshots. | S |
| T0.3 | `scripts/tui/goldens.py` (forked from `harness.py` with the hoocode-ts half dropped): `run <scenario>`, `check` (diff against `tests/golden/tui/<scenario>/*.txt`), `update`. It reuses the scenario JSON, `mockllm.py`, the tmux driver and the `normalize.json` rules (tool-name rules dropped). | M |
| T0.4 | `scripts/tui/review_bundle.py`: for changed screens only, writes `target/tui-review/<scenario>/<snap>/{before,after}.txt`, `diff.txt`, a style legend and an optional `after.png` (`grid_to_html` → `render_png.mjs`). Plus `index.md` listing the bundles. | S |
| T0.5 | `.claude/skills/tui-review/SKILL.md`: how a Haiku subagent reviews a bundle (checklist: alignment, truncation, colour roles, overflow at 80 and 120 columns, empty and error states). It writes `review.md` per screen with ok / issue / unsure. On unsure, it asks for the PNG. | S |
| T0.6 | Retire parity: `ledger.py verify` = L1 + `goldens.py check <scenarios>`. Remove `setup_hoocode.sh` from SessionStart and CI. Update CLAUDE.md, build-speed.md, ui.md and the continue-migration skill. Move `migration/tui-parity/` to `scripts/tui/` (keep mock and scenarios). The replay fixtures stay until the user says so. | S |

### Phase T1: prerequisites (refactors, no visible change; goldens must stay identical)

| Step | Work | Size |
|---|---|---|
| T1.1 | Split `interactive_mode.rs` (6,624 LOC, 85 fields) into `mode/{mod,events,transcript,prompt_queue,session_ops,commands,dialogs,subagents,...}.rs`. Mechanical: several `impl Mode` blocks. | M |
| T1.2 | `Transcript` struct: owns the chat `Container` and the ~20 parallel fields `reset_transcript_view` clears (`chains`, `open_chain`, `streaming`, `pending_tools`, `tool_output_view`, ...). `handle_session_event` and `handle_agent_event` write through a target `&mut Transcript`. Reset is a replace. | M |
| T1.3 | Task panel: delete the teams/roles path. Give `Task` a `kind` (Plan, Subagent, Shell, Schedule, Mcp). Build one row model per store `version()`, keyed by a stable row id, not an index. Stop cloning on every frame. | S |
| T1.4 | `footer.rs:subagent_counts`: one pass, delete `active_subagent_count`. | XS |

### Phase T2: subagent storage and recovery

| Step | Work | Size |
|---|---|---|
| T2.1 | Pool: the child's `--session` points at `<parent>/subagents/<task_id>.jsonl`. `Header.parent_session` is set. `dispatch-log.json` and ledger lines carry the parent session id. | S |
| T2.2 | Parent session: a `subagent_run` entry at dispatch, and its outcome at settle. Success no longer deletes the transcript. The sweep spares `subagents/`. Deleting a session deletes its dir. | S |
| T2.3 | `/resume`: subagent runs nest under their parent session in the threaded view. Search matches the parent row only. | S |
| T2.4 | After restart, `subagent_run` entries whose outcome is missing are shown as `interrupted`. Enter on such a row offers View / Resume (the existing `SubagentPool::resume`). | S |

### Phase T3: live events and attach

| Step | Work | Size |
|---|---|---|
| T3.1 | Forward every child stdout event: a pool listener sends `AppEvent::Subagent(task_id, event)`. The child also emits `message_update` deltas, throttled to the existing streaming render rate. | S |
| T3.2 | The Agent tool emits a partial at dispatch carrying `task_id`, so the tool block knows its run. The block shows `Agent explore · <description>` and a 5-line tail of the live activity. | S |
| T3.3 | Attach: park the main `Transcript`, build the child's from its jsonl (`render_session_context`), and route that task's live events into it. Main-session events keep updating the parked view. Esc restores it with its scroll offset. The prompt is disabled while attached. The header row shows state. | M |
| T3.4 | Attach from three places: the panel (Enter), the Agent tool block (a key on the selected block), and `/agents` (a picker of runs in this session). | S |

### Phase T4: activity panel tabs

| Step | Work | Size |
|---|---|---|
| T4.1 | Panel tabs over the T1.3 row model. The unfocused view shows the current tab plus counts. Ctrl+G focuses it. Keys are as in Decisions. The tick runs while any row is running. | M |
| T4.2 | Schedule tab: the scheduler publishes fire, run and finish plus next run time into the store. | S |
| T4.3 | MCP tab: `McpHub` publishes per-server state (connecting, ready, auth needed, failed) and in-flight calls longer than 1 s. Enter on an auth-needed row runs `/mcp login`. `x` aborts a call. | S |

### Phase T5: background shell

| Step | Work | Size |
|---|---|---|
| T5.1 | Job registry (`hoocode-code-tool-bash`): spawn with output to `<session>/jobs/<id>.log`, a kill handle, exit status. Session-scoped, and killed on exit unless the user detaches it. | M |
| T5.2 | `Shell` gets `run_in_background`. New opt-in tools `ShellOutput(id, since?)` and `ShellKill(id)`. Completion is a notice to the model at the next turn. | S |
| T5.3 | Shell tab rows plus attach (a tail of the log file, the same swap view as T3.3 with a plain-text transcript). | S |

### How Haiku subagents do the work

- One phase step = one Haiku subagent in its own worktree, with a precise brief: files, the
  acceptance test, and the goldens that must stay unchanged or the ones it may update.
- Each one runs L1 for its crates and `goldens.py check`. On a visual change, it produces a
  review bundle and a second Haiku subagent reviews it (T0.5). The orchestrator (main
  session) reads the review and merges.
- Parallel only where files don't overlap: T0.1/T0.3 together, T1.3/T1.4 together, T4.2/T4.3
  together. T1.1 → T1.2 → T3.3 are serial.

## Not doing

- Prompting or steering a subagent while attached (read-only by decision; T1.2 keeps it
  possible later).
- Auto-resuming interrupted subagents on `/resume`.
- An LLM review that gates the build, or that runs in CI.
- Split view of main and subagent side by side.
- Parity with hoocode-ts screens. New screens may differ freely.
- In-process subagents (still processes, per decisions-2026-10-08).

## Open questions

1. `/agents`: a new slash command (T3.4), or attach only from the panel and tool block?
   Recommended: add it. It is the only way in when the panel is collapsed.
2. Background shell jobs at exit: kill silently, or ask when jobs are running?
   Recommended: ask once, with "kill all" as the default.
3. Replay fixtures (`hoocode-0.5.89/`, `replay.json`): keep as regression tests, or delete
   with the parity harness? Recommended: keep until T0 has run green for a week.
4. Migration ledger: keep `ledger.py` at all once parity is gone? Recommended: freeze it
   read-only. New work is tracked in this card's phase tables.

## Details

### Review findings that shaped the plan

| Area (LOC) | Finding used here |
|---|---|
| `interactive_mode.rs` (6,624) | The transcript is a re-rendered component tree, not terminal scrollback (`ui.md` was wrong; fixed). A swap needs `Transcript` (T1.2). Nine UI-thread `block_on` calls break concurrency.md §4 (backlog B1). |
| Task panel (1,100) | Teams/focus is unreachable. The store is memory-only. Shell, scheduler and MCP are absent. `render` clones the store every frame. |
| Subagents | Success deletes the transcript (`pool.rs` settle). Nothing links the child to its parent. Progress is collapsed to one string in `instance.rs`. stdin is null. |
| Tool blocks (4,384) | The Agent block can't learn its task id until it finishes. It hides `description`. AgentOutput and Edit ignore the peek dial. |
| Parity harness | Scenario JSON, `mockllm.py`, the tmux driver, `grid_to_html` and `render_png.mjs` are reusable as is. vt100 is already a dev-dep. |

### Backlog (after T5, ranked by value/effort)

| # | Item | Visible |
|---|---|---|
| B1 | Move the nine UI-thread `block_on` session ops to the runtime (`AppEvent::SessionOp`) | no |
| B2 | One `ActivePicker` enum in place of ~14 `Option` slots; the `poll_*` functions become `AppEvent`s | no |
| B3 | Shared picker helpers: `visible_window`, `step`, `Filtered<T>`, `PickerView` (~200 LOC saved); one wrap-vs-clamp policy | wrap only |
| B4 | One `peek_block` helper for the 7 copies of the peek trim; AgentOutput and Edit respect the dial | yes |
| B5 | Render cost: bash output styled and wrapped in full every frame; Read highlights the whole file; markdown cache clones its lines; assistant message clones on every delta | no |
| B6 | Per-keystroke cost: `EditorChanged` clones the full text; bindings re-parsed on each `matches` | no |
| B7 | Dead code and TS-isms: `js_line_count`, `LEGACY_TOOL_OUTPUT_VIEWS`, MultiEdit/NotebookEdit names, "(exit undefined)", duplicated `js_trim`/`home_dir`/`group_digits`/`shorten_path` | "undefined" text only |
| B8 | `custom_message.rs`: one summary-sheet helper for three copies | spacing |
| B9 | `Container` per-child line cache keyed by version, with a debug check against an uncached render | no |
| B10 | Editor cursor in byte offsets instead of UTF-16 (needs emoji and paste tests first) | no |

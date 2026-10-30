# UI map

Where each part of the interactive screen lives. Snapshot of 2026-10-17 (full-screen, M1e). Crate names are `hoocode-*` since step 0c
([naming-and-paths.md](../../archive/docs/naming-and-paths.md)). Crates are
described in [packages.md](packages.md). Names, not line numbers: line numbers drift.

**Keep this current.** Add, move or remove a screen part, picker or slash command →
update this page in the same commit.

## The screen, top to bottom

Full-screen is the default (M1c, `hud.rs`, `mode/mod.rs`). The app runs on the alternate screen and `Tui` draws the frame. Rows top to bottom:

```
HUD ─ row 1: ◭ HOOCODE  <mode>  <folder> │ [◈ N agent] [⌁ N sat] SHIP <model>  FUEL <8-cell bar> <pct>  CR <cost>   (M3; no footer in full-screen)
  (gap: one blank row)
  attach slot ─ the attached subagent's header, when attached (top chrome)
┌ body ─ header, then transcript (chat). Live: follows the tail. Pinned: holds still ─┐
│ ...                                                                                 │
└─────────────────────────────────────────────────────────────────────────────────────┘
  bar ─ pinned only: position, keys, search. Drawn by Tui between body and bottom
  pending messages, status, widgets above
  task panel ─ TodoWrite plan items, subagent runs
  notification band ─ tips, transient notices, progress bar
  ┏ comms frame (3 rows at rest) ─ state title + app-edit │ ▸ input row │ foot: view dial, PLAN note, ↑in ↓out ┓
  ┗ grows with input; cards and consoles replace it ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
  (no footer in full-screen: the HUD replaces it. Inline `HOOCODE_FULLSCREEN=0` keeps the two-row footer: folder, branch, path │ model, context, cost)
```

`HOOCODE_FULLSCREEN=0` turns full-screen off for one run (the env var wins over the `fullscreen` setting, default `true`). Off, the screen is the inline tree with no HUD and no gap: the rows of `mode/mod.rs` `tree_plan(false)` in `hud.rs`. The tree order for both modes is `hud.rs` `tree_plan`; the test that pins it is `tests/it/hud.rs`.

The inline tree (off) from top to bottom: the header, a flex spacer (`screen_fill`), the attach slot, the chat, then the rest as above. The full-screen tree (on): the HUD, a gap, the attach slot (top chrome), the body (header and chat), then the rest as above. Root children before the body are the top chrome, those after it the bottom (`Tui::set_fullscreen_body`).

Exit leaves the alternate screen and prints nothing (`Tui::stop`). SIGTERM and SIGHUP restore the terminal first, then exit 128 plus the signal (`tui-terminal/src/interrupt.rs`, `install_terminate_restore`, M1e). A UI-thread panic restores it too (the panic hook).

| Region | Component | File |
|---|---|---|
| Whole screen, event loop | `InteractiveMode` (`Mode`) | `hoocode-code-tui-app/src/mode/mod.rs` (struct, setup, loop); the rest is split by concern into `mode/*.rs`: `input` (editor and keys), `transcript` (`Transcript`: the chat container and the components that track it; reset replaces it), `events` (agent and session events), `prompt_queue`, `session_ops`, `session_op` (the session operations that run off the UI thread: `SessionOpDone`, `SessionOutcome`, `finish_session_op`), `tree`, `models`, `auth`, `settings`, `dialogs`, `subagents`, `bash`, `clipboard`, `mcp`, `chrome` (footer, title, colour), `commands` (slash commands), `shutdown` (the exit steps after the loop, each with a deadline; concurrency.md §4) |
| Transcript scroll-back view | scroll view | `hoocode-code-tui-app/src/scroll_view.rs` |
| Full-screen frame: the alternate screen, the per-row diff, the body window, the bar | `Tui` full-screen mode | `hoocode-tui-render/src/tui.rs` (`paint_fullscreen`, `compose_fullscreen`), row arithmetic in `fullscreen.rs` |
| HUD row: `◭ HOOCODE`, mode chip (`BUILD` accent, `PLAN` violet), folder dim · right, in order: `◈ N agent` (violet, while agents run), `⌁ N sat` (accent, while satellites are up), `SHIP` + model, `FUEL` 8-cell bar + percent (acc below 70%, warn below 90%, err from 90%), `CR` + spend · hull paint name (`/color`, M8 e5) ahead of the ship, row background takes the hull tint, paint persisted per session (`SessionInfo` hull, restored on render) · narrow (80 cols): drops the folder first, then the ship; mode, fuel and credits stay · other cuts trim the left part first with `…` at its end, the right part from the right · the setting read, the tree plan | HUD | `hoocode-code-tui-app/src/hud.rs` (`hud_text`, `hud_runs`, `hud_component`, `read_hud`, `fuel_bar`, `fuel_token`, `fullscreen_enabled`, `tree_plan`, paint in `hull_paint.rs` over `footer_data.rs`); drawn with `hoocode-tui-ratatui` |
| Startup banner | wordmark | `hoocode-code-tui-app/src/wordmark.rs` |
| Startup resource listing (skills, commands, agents, MCP, context; no themes, extensions or canvases cells, N12) | resource display | `hoocode-code-tui-app/src/resource_display.rs`, `expandable_text.rs` |
| Task panel: tabs Plan · Agents · Shell · Schedule · MCP (a tab shows while it has rows), a count per tab; focused (Ctrl+G) it is the FLIGHT PLAN console (M7, ratatui): `FLIGHT PLAN done/total`, agents/sats on the right, the waypoint track over the plan rows, the tab strip, spec hint foot; unfocused it draws nothing in radar (the flight log carries the work), keys and events unchanged in [Task panel](#task-panel) | task panel | `code-tui-widgets/src/task_panel.rs` (tabs, rows, keys, `render_flight_plan`); focus and events in `hoocode-code-tui-app/src/mode/task_panel.rs` |
| Notification band (inline) and transmission toasts (fullscreen, M8 e4) | notification panel, tips, startup update notice | `hoocode-code-tui-app/src/notification_panel.rs` (newest first; `set_overlay` quiets the band, `toast_lines` feeds the float), `tips.rs`; the update notice is polled by `poll_update_notice` in `mode/events.rs` from `hoocode-code-update`. The toast is a 46-wide round frame titled `● TRANSMISSION` with a `▔` drain row (`transmission.rs`, ratatui), floated top-right over the transcript by `tui-render/src/tui.rs` (`set_toast_provider`, `float_toast_line`) |
| Completion bell (`chimeOnTurnComplete`): one `\x07` when a turn over 10 s ends (not after an abort) or when `AskUserQuestion` opens (not MCP elicitation forms); repeats within 5 s dropped | `hoocode-code-tui-app/src/chime.rs` (pure timing, `TurnTimer` set at prompt submit); rung by `ring_chime` and `finish_turn_chime` in `mode/events.rs`, and at the `AskOptions` dialog in `mode/mod.rs` when its request has `chime` set (`dialog_bridge.rs`) |
| Progress bar | progress bar, startup progress | `hoocode-code-tui-app/src/progress_bar.rs`, `startup_progress.rs` |
| Prompt editor | `CustomEditor` around a borderless `Editor` (`▸` prefix, per-state placeholder) | `mode/input.rs` (`CustomEditor`); `tui-components/src/editor/` (`placeholder`, `placeholder_color`) |
| Comms frame (M2) | `CommsFrame` around the prompt: heavy `┏┓┗┛` edges, state titles (READY dim, `◐ IN FLIGHT · step N · time · esc abort` acc, `⊗ COMMS LOST` err, `● LANDED · time · N steps` ok, `⊗ DID NOT LAND · …` err, `⊘ ABORTED · READY` dim; the last settled mission's outcome holds until the next mission opens, `MissionOutcome` in `comms_frame.rs`, set by `record_mission_outcome` in `mode/transcript.rs`), foot with the view dial, the PLAN note and `↑in ↓out` token totals | `hoocode-code-tui-app/src/comms_frame.rs` (`CommsState`, `InputStyle`, `GlyphStyle`); read from the session (`is_streaming`/`is_retrying` for READY vs IN FLIGHT, the `comms_lost` flag for COMMS LOST — set when the last retry gives up, cleared by the next hail or mission — `TurnTimer::elapsed` for the time) and `FooterSource::usage_totals`; hull paint (`hull_paint.rs`, `/color` cycles none/parity/cyan/violet/lime/red) is the right label and takes the title and border colour |
| Prompt frame | frame, input frame (dialogs; the main prompt uses the comms frame now) | `tui-components/src/frame.rs`; `code-tui-widgets/src/input_frame.rs` |
| Attached subagent view (T3.3): a header row above the chat (`◂ <type> · <description> · <state> · Esc back`), and the prompt's place with a read-only hint; Esc goes back. The chat shows the run's transcript while attached; the session's is parked and kept up to date | attached view | `code-tui-widgets/src/attach_view.rs` (header, hint). The swap, the routing and the run's events: `hoocode-code-tui-app/src/mode/attach.rs`; the header's slot is `Mode::attach_slot` (above the chat) |
| Autocomplete (`/`, `@file`) | autocomplete | `tui-components/src/autocomplete/`; commands fed by `mode/input.rs` `setup_autocomplete_provider`. `@file` uses the in-process finder `hoocode-code-tools::file_finder`, injected by `mode/input.rs` `at_file_finder` (no `fd`) and run on a worker thread (`autocomplete/file_search.rs`): typing never waits, and the editor re-asks when a walk finishes (`take_ready`). The `/` and `@` popups draw as Mission Control consoles (M7): `CONSOLE · commands N of M` and `CARGO · @ files N matches`, with `▸` query row and hint row, from `editor.rs` `console_popup`; plain path completion keeps the legacy list. |
| Footer (inline mode only) | footer, footer data. Always two rows: row 1 folder, branch, session, path, then mode chip and view dial; row 2 model, context gauge, tokens, cost. Compact keeps both rows. Transient messages show above the prompt, not in the footer: a warning line (memory shedding, UI stall), the auto-retry notice (`retrying · attempt 2 of 6 · …`, set from `AutoRetryStart` and cleared by `AutoRetryEnd` in `mode/events.rs` through `set_retry_notice`), extension statuses and startup progress, read with `FooterComponent::transient_lines`. The warning is set by `mode/mod.rs` `sync_runtime_health`. Drawn with the ratatui `Palette` (M3): every piece `paint`ed from `pal` ink/dim/faint/accent styles | `hoocode-code-tui-app/src/footer.rs`, `footer_data.rs`; styles from `hoocode-tui-ratatui` |
| Session chip | session chip | `code-tui-widgets/src/session_chip.rs` |
| How much room chrome gets (two stops: full, compact; `/chrome <full\|compact>`, `Alt+Z` / `Shift+Alt+Z` toggle) | chrome layout | `hoocode-code-tui-app/src/chrome_layout.rs` |

## Transcript items

| Item | File (`code-tui-widgets/src/`) |
|---|---|
| User message | `user_message.rs` |
| Assistant message (markdown, thinking; the abort or error line shows with or without tool calls, `end_reason` is the one text radar and peek share) | `assistant_message.rs` → `tui-components/src/markdown/` |
| Custom and branch-summary messages | `custom_message.rs` |
| Compaction summary (M8 e6): folded is the JETTISONED row (`≋≋≋ JETTISONED M1–M3 into one summary stage · A → B tokens`, vio, ratatui`; range from `Transcript::jettisoned_range`, `earlier missions` fallback), expanded is the summary sheet | `custom_message.rs` (`jettison_text`, `jettison_row`), range plumbed as `mission_range` on `CompactionSummaryMessage` and `FileEntry::Compaction` (serde default: old files load; threaded through `append_compaction` from `/compact` time) |
| One tool call's block | `tool_execution.rs` |
| A run of tool calls. Radar: a mission's chains draw nothing (its header draws the calls, see the flight log below); orphan chains (before the first mission) keep the old summary line. Peek and full draw each step from `ToolExecutionComponent::step_parts` (lead, header, body, images): the header wraps beside the step number and stats (`trajectory::step_header_rows`), the body folds to 3 rows at peek and not at all at full (a Shell step folds to its last 3 rows with `… N earlier lines` above them; `StepParts.tail`/`footer`, set through `BodyLayout` by the Shell renderer, keep `Took 8.2s` and truncation notes whole and uncounted), images sit after the fold; a failed Shell step's header shows `HIT exit N · time`; a step the user's abort cut short (`set_interrupted`) reads `⊘ … interrupted`, is no hit and shows no body (the aborted message prints the reason once); a settled Agent step lists its own calls on the violet rail (`live::settled_steps`: failed ones at peek, all at full) | `tool_chain.rs`, `tool_chain_summary.rs` |
| Mission Control trajectory (M4, peek and full): mission headers, numbered steps on the `┊` rail, Think rows, landing lines. The landing line takes the mission's outcome (`FlightMark`, the one the radar row and the comms title read): `● LANDED` ok, `⊗ DID NOT LAND` err, `⊘ ABORTED` dim; a turn with no steps lands too (no step counts on it), and one dim line right after it sums the turn up (`Edited 3 files · ran 2 commands`, `tool_chain_summary::turn_phrase`; a command is named by its program and first subcommand, `ran cargo test`, several distinct ones count; the chain line keeps the whole command). The Think row wraps its gist at full (`ThinkHeader::set_wrap`, via `AssistantMessageComponent::set_view`) and a message's opening blank row is plain unless a step sits above it M5 port: every row draws through a ratatui `Buffer` + the palette (`header_parts` span layout; block bodies parse back via `tui-ratatui` `ansi_spans`) | `trajectory.rs` (`MissionHeader`, step/rail/fold/chip/landing helpers); peek numbering in `tool_chain.rs`, Think row + prose rail in `assistant_message.rs` |
| Mission Control flight log (R3, radar): one mission block, drawn by its header from the mission's `FlightStats`: `▲ Mn  prompt` (wraps, no stats), then the body on the `┊` rail, then the turn end closing it with `╰`. The body is the mission's items in story order: one line per tool batch (settled `● Edited … · ran cargo test` with `N calls` flush right, `⊗` when a call failed; live `◐ … Edit › Shell › …` with `N calls · clock`, never wrapping), a failed call nested under its batch as `└ NN Tool  subject  → snippet  HIT`, a held call as `◉ … holding for clearance`, a background job and a subagent's own calls nested under its Agent call, one dim `✻ gist` line per thinking block that has text (a batch splits there; a message without thinking text adds no split; the opening message's thinking comes first, and the final answer's thinking follows the last batch), and the final answer whole after a blank rail line. The turn end is mandatory: `● LANDED`, `⊗ DID NOT LAND · reason` (the reason stays on the row when it fits, else on one continuation line under it), `⊘ ABORTED` (the landing line's words and stats, `trajectory::landing_spans`), `◐ IN FLIGHT` with its clock while it flies, or `◆ WAITING · approve <tool> <subject>` while a call waits for approval (its own or a subagent's). An older finished mission folds to one summary line, then the answer and its turn end. The live mission and the latest finished one are open. Peek-only rows hide via `PeekOnly`; orphan thinking and prose keep their own rows | `radar.rs` (`mission_rows`, `FlightStats`, `FlightItem`, `FlightStats::awaiting`, `calls`, the batch, call and turn-end rows; `think_rows` and `prose_rows` for orphans); `MissionHeader` radar branch in `trajectory.rs` (a blank row above every mission but the first); `code-tui-app` `mode/transcript.rs` (`mission_items` splits chains at thinking, `refresh_flight_marks` sets `items` and `folded`, `land_mission` sets tokens, `sync_radar_prose` sets the answer and reason). The mission's `N✕` also counts the calls its subagents' own runs saw fail (`hoocode-code-subagents` `live::failed_total`, read by `tally_range`; `mode/subagents.rs` refreshes on a run's call end and settle). A mission's outcome is its chains' reading overridden by how the turn ended (`settle_mark`: error → `⊗`, abort → `⊘` whatever the steps did; the call the abort cut short is no hit), so a stepless turn ends consistently; `land_mission` always lands it. An `AgentEnd` with a message still streaming settles it (`mode/events.rs`) |
| One call's signal row (a block drawn alone in radar); the call subject helpers `tool_subject`, `chain_subject` | `tool_signal.rs` |
| Dial: radar → peek → full (Ctrl+O steps forward and wraps, Alt+O / Shift+Alt+O cycle either way; persisted as `toolOutputView` `radar`/`peek`/`full`; footer and comms foot print the layer name). Full is peek with nothing cut: no line budgets, no `… N more lines` folds, no width ellipsis on content (rows wrap), whole thinking, tool output and subagent result; the `expanded` flag renderers get is the full stop, and `transcript.full_active` shares it with the `!` rows. `peek_block` is the shared peek body | `tool_output_view.rs` |
| The one `… N more lines` hint (dim, `… 1 more line`; `… N earlier lines` above an output that shows its last lines) and the `…` glyph; `fold_rows` / `fold_rows_tail` fold a block's body rows for the chain, adds the renderer's own hint to the count, and never folds image rows or `[Image: …]` placeholders | `fold.rs` |
| `<skill …>` block of a `/skill:name` message (radar row / peek body, 5 lines / full body) | `skill_block.rs` |
| Background-run notice (radar row / peek body; full wraps the summary instead of cutting it) | `background_notice.rs` |
| `!` bash command typed by the user (radar: header only, `set_radar_flag` shares the transcript's radar flag; peek keeps the last 20 output lines; full, via `set_full_flag`, all of them; the command header wraps, never cut) | `bash_execution.rs` |
| Diffs | `diff.rs`, `jsdiff.rs` |
| File content with line numbers | `read_output.rs` |
| Notice rows (`showRecord`) | `hoocode-code-tui-app/src/record_row.rs` |
| Failed Model block (`⊗ Model`, one per retry streak that gave up or was cancelled; the failed attempts' own rows are taken out of the chat). A streak that ran out of retries is COMMS LOST: the block carries the hail axis (`⊗` per failed request, gaps doubling, `backoff doubles`), and empty enter hails the last prompt again | `hoocode-code-tui-app/src/mode/retry.rs` (`hail_axis`, built on the chat from `AutoRetryStart` / `AutoRetryEnd` in `mode/events.rs`; the `comms_lost` flag in `mode/mod.rs`, the last prompt in `Transcript`) |

### Tool renderers (`code-tui-widgets/src/tools/`)

| Tool | File |
|---|---|
| `Shell` | `bash.rs` |
| `ShellOutput`, `ShellKill` (opt-in) | none: the default block. Their jobs show in the Shell tab (T5.3) |
| `Read` (and skills) | `read.rs` |
| `Edit` | `edit.rs` |
| `Write` | `write.rs`: the content previews under the header while there is no result; the result (numbered content, or the error) is then the one copy |
| `TodoWrite` (opt-in) | `todo.rs`: one header row (`TodoWrite  3 items · 1 done`). The plan shows in the task panel |
| `CodeSearch` | `search.rs` |
| `Agent`, `AgentOutput` (subagents) | `subagent.rs`. An Agent block's result slot shows its run's live tail (the newest 5 rows, peek) while the run goes, else its result. The tail and the run lookup are `hoocode-code-subagents` `live.rs` (`tail`, `is_running`, `run_for_tool_call`); the block is fed by `mode/subagents.rs` `apply_subagent_event`. `app.subagent.attach` (Ctrl+N, at the prompt) attaches to the newest Agent block's run (`mode/attach.rs` `attach_newest_agent_block`) |
| Plugin tools | `plugins.rs` |
| `WebFetch`, `WebSearch` | `web.rs` (web tools are deferred) |
| Registry: tool name → renderer | `mod.rs` |
| Shared helpers | `../render_utils.rs` |

## Pickers and dialogs

They replace the prompt frame while open. All in `code-tui-selectors/src/` unless noted.

Every picker moves its cursor with `PickerCursor` (`tui-components/src/picker_cursor.rs`): up and down, PgUp and PgDn by a window, Home and End to the ends. The ends clamp; nothing wraps (decision 29). The visible window and the `(n/len)` footer come from the same type. The `hoocode config` list skips its group headers and counts resources in its footer. In a picker with a query line (`/model`, scoped models, `/login`, `/resume`, `hoocode config`), Home and End stay with the query's text cursor, and only PgUp and PgDn jump the list (`handle_query_key`). The `/tree` query has no text cursor, so there Home and End jump the list. Ask options: Home and End edit the custom answer while it has focus.

| Opened by | Picker | File |
|---|---|---|
| `/settings` | SYSTEMS console (M7, ratatui): `SYSTEMS · /settings  N tok/turn`, `▸` query row, one row per setting with accent `›` on the selection band, description + per-turn surface below, hint foot `↑↓ move · space toggle · enter open · esc close` (literal keys). The theme submenu is the HULL console (`HULL · /theme  N themes`, `● in use` on the selection, `↑↓ preview live · enter keep · esc back`; live preview, enter keeps, esc reverts, as before). Other submenus keep their framed lists. Keys, filtering and events unchanged. | `settings_selector.rs` (`SettingsSelectorComponent`; `setting_row`, `surface_row`, `query_row` from the console helper; `SelectSubmenu::with_console`). Goldens `systems_80/100`, `hull_80/100`. Uses `tui-components/src/settings_list.rs` (added `query_text`, `selected`, `window`, `scroll_footer`, `has_submenu`, `open_submenu` accessors) and `select_list.rs` (`console_window`, `footer`). No External tools category: the external-tools layer went with `fd`/`rg` (reliability 1.4), so the top level is the tool rows, then the categories. Tool rows are sorted by tool name (`tool_row_sort_key`: Shell, Edit, Read, Write). The top-level **Models** row shows `all models` or `N models` (from `scopedModels`); Enter closes the pane and opens `/scoped-models` (`SettingsChange::OpenScopedModels`, handled in `hoocode-code-tui-app/src/mode/settings.rs`). |
| `/model` | HANGAR console (M7, ratatui): `HANGAR · /model  N ships`, scope lines, `▸` query row, ships with accent `›` on the selection band, hint foot `↑↓ ship · enter fly it · tab scope · esc close`; shared `Console` chrome in `code-tui-widgets/src/console.rs`. A model whose provider's stored OAuth token has expired shows an `expired` marker (`set_expired_providers`, read by `mode/models.rs` `show_model_selector` via `AuthStorage::readiness`; no refresh). | `model_selector.rs` (`ModelSelectorComponent`; `ship_row`, `query_row` from the console helper). Goldens `hangar_80/100`. |
| `/scoped-models` | scoped models | `scoped_models_selector.rs`. Loads `scopedModels` (`hoocode-code-tui-app/src/mode/models.rs`, `concrete_scoped_entries`), one row per model with an effort column and a category column, a header line above the rows. Keys: enter toggles, alt+a/alt+x all/clear, alt+g provider, alt+up/alt+down reorder (order is priority), **tab** cycles the row's effort through the model's supported levels then unset, **alt+j** cycles its category none, cheap, fast, standard, capable then none, alt+s saves full entries (`Persist(Vec<ScopedModel>)`, validated first). Alias is kept, not edited. Typing filters the list. |
| thinking level, theme | one-list consoles (M7, ratatui): the title names the console, one row per option with accent `›` on the selection band, hint foot `↑↓ navigate · enter select · esc cancel`; shared `Console` chrome in `code-tui-widgets/src/console.rs` | `small_selectors.rs`, `framed_list.rs` (`FramedSelectList`; `/color` cycles the hull paint since M8 e5 instead of opening swatches) |
| `/agents` | subagent runs of this session (running, done, interrupted), one row each; Enter attaches to the chosen run | `show_selector` (`ExtensionSelectorComponent`); the rows are read on the tools pool by `mode/attach.rs` `list_runs`, then `agent_runs_listed` opens the picker and `AppEvent::AttachPicked` attaches; drawn as a console (M7, ratatui) |
| `/resume` | MISSION LOG console (M7, ratatui): `MISSION LOG · /resume  N sessions` (`V of T` while searching), scope tabs + search legend on top, `▸` query row, one row per session with accent `›` on the selection band, hint foot with literal keys. Rename draws as a console too. Keys, filtering and events unchanged. | `session_selector.rs`, `session_selector_search.rs`. Subagent runs (`<session>/subagents/`) are rows under their parent in the threaded view, added by the loaders in `hoocode-code-tui-app/src/session_picker.rs` (`with_subagent_runs`, from `list_subagent_sessions` in `code-session`). A row reads `<type> · <complete\|failed\|…>`, `running` or `interrupted`. A search, or a non-threaded sort, lists the parent rows only. Enter on an `interrupted` run opens a choice: `v` views its transcript, `r` resumes it (`SubagentRunRef` to `pool.resume` in `hoocode-code-tui-app/src/mode/subagents.rs`; in-session picker only), esc backs out. Deleting a session deletes its `subagents/` dir. Goldens `mission_log_80/100`. |
| `--resume` on the command line | session picker on its own TUI | `hoocode-code-tui-app/src/session_picker.rs` |
| `/tree` | FLIGHT PATH console (M7, ratatui): `FLIGHT PATH · /tree  N nodes` (`V of T` while searching), `▸` query row, one row per entry with accent `›` on the selection band, position footer, hint foot with literal keys (leading run that fits). Label editor draws in the console while open. No fork action yet (no `f fork` hint). | `tree_selector.rs` (`TreeSelectorComponent`; `entry_line`, `query_row` from the console helper). Goldens `flight_path_80/100`. |
| `/fork` | STAGE SEPARATION console (M7, ratatui): `STAGE SEPARATION · /fork`, explanation on top, one row per user message with accent `›` on the selection band, position and scroll rows below, hint foot `↑↓ move the line · enter fork · esc close`; shared `Console` chrome in `code-tui-widgets/src/console.rs`. Keys and events unchanged (enter forks before the message, esc cancels, empty list auto-cancels). | `user_message_selector.rs` (`UserMessageSelectorComponent`; `message_row`, `query_row` from the console helper — no query row here). Goldens `stage_separation_80/100`. |
| MCP trust prompt (once per process, at startup, when a project or plugin declares MCP servers) | the permission selector (`TuiPermissionUi`, `DialogRequest::Select`); asked on a thread by `mode/mcp.rs` `ask_mcp_trust`, answered as `MCP_TRUST_YES` / `MCP_TRUST_NO`; the grant runs `McpHub::grant` (trust store, then start) |
| `/login`, `/logout` | provider pickers and login dialog, drawn as the DOCKING console (M7, ratatui): `DOCKING · /login` with provider counts, `▸` query row, `›` on the selection band, spec hint foot; the dialog shows the URL/code and notes the missing QR | `oauth_selector.rs`, `login_dialog.rs` (shared `Console` chrome in `code-tui-widgets/src/console.rs`); flow in `hoocode-code-tui-app/src/login_controller.rs` |
| `AskUserQuestion` tool | question card (M6): amber `┏┓┗┛` card replacing the comms frame, `◉ MISSION CONTROL ASKS`, numbered answers with the recommended one tagged, last row `N something else · type it`; number keys answer (the custom row's number only moves there), `↑↓` move, `enter` confirms, esc picks option 1 (MCP elicitation forms pass `esc_answers_first: false` and keep esc-as-skip) | `ask_options.rs` (`AskOptionsComponent`, `AskOptionsOptions::esc_answers_first`) |
| Shell/tool approval prompt | clearance card (M6): amber card replacing the comms frame, `◉ CLEARANCE NEEDED[ · step NN]`, right label `MODE asks for Tool`, the command, a REACH line (`■`/`□ read/write/network/pipe from the call's real data), keys `y go once · a always … · n abort + say why · e edit first`; card keys fire while no reason is typed, then letters type the abort reason (`enter` sends, esc clears first); `e` edits, `enter` denies the original and sends the revised text as the next message | `hoocode-code-tui-app/src/clearance_card.rs` (`ClearanceCard`); REACH in `hoocode-code-permissions/src/lib.rs` (`reach_for`); gate answers in `ClearanceChoice`, asked via `PermissionUi::select_clearance` (`dialog_bridge.rs` `DialogRequest::Clearance`). Only one card shows; a request that arrives while one is up (the main agent's, or a subagent's) waits in `clearance_queue.rs` (`ClearanceQueue`) and shows when the card closes, in arrival order, and quitting denies the queue. A subagent's gated call is relayed by its child (`hoocode-code-subagents` `clearance.rs`) to `TuiClearanceRelay` (`dialog_bridge.rs`, set in `mode/mod.rs`), which decides with this session's own gate (config rules, session grants, one `a`); the card's right label then names the subagent (`BUILD · explore asks for Shell`, `ClearanceInfo::from`) and goes away if the subagent ends (`DialogRequest::HideClearance`). The subagent's Agent block shows the hold as a `◉ NN.n Tool … holding for clearance` sub-step (`task_clearance` pool event, `live::sub_steps` `held`, `sub_waiting_row`). `a` on Shell remembers the program for this session only (`always_allows`, in memory per cwd+mode, never config); a later command auto-allows only if every segment's program is approved, with no wrapper/builtin (`sudo`, `cd`, … — `a` then runs once and hides), no slash program, no assignment, no expansion/substitution, no file redirect, and ≤32 KiB (fail-closed to a prompt; never consulted in PLAN, whose card hides `a`); other tools persist their name in `auto_allow` |
| `/quit`, ctrl+c twice, ctrl+d | end-of-shift card (M6): amber card with the shift's debrief (missions, steps, tokens, spend), `enter` ends, `esc` stays; with satellites up, `◉ END SHIFT WITH N SATELLITE(S) UP?` lists them (`k` brings them down, `l` leaves them, `enter` is bring-down, `esc` stays). User quits go through `request_user_quit`; fatal session errors exit without asking | `hoocode-code-tui-app/src/end_shift.rs` (`EndShiftCard`, `EndShiftJobsCard`); flow in `mode/task_panel.rs` (`EndShift`, `ready_to_quit`) |
| `hoocode config` | RESOURCES console (M7, ratatui): `RESOURCES · config  N resources` (`V of T` while searching), `▸` query row, one row per entry with accent `›` on the selection band (group headers plain bold, type headers dimmed, items as state checkbox + name + dimmed path), scroll position over resources, hint foot `↑↓ move · space toggle · esc close`; shared `Console` chrome in `code-tui-widgets/src/console.rs`. Keys, grouping, filtering and `+`/`-` toggle persistence unchanged. | `config_selector.rs` (`ConfigSelectorComponent`; `entry_row`, `query_row` from the console helper) |
| `hoocode update` (CLI, no TUI): prints the update command for the install method; never replaces the binary | `hoocode-code-update/src/lib.rs` (`run_update_command`); dispatched in `hoocode-code-cli/src/lib.rs` |
| `/hotkeys` | CONTROLS console (M7, ratatui): drawn keyboard with the ctrl-layer keys lit at 100+ columns, key list below; both from the real keybindings | `code-tui-widgets/src/controls.rs`; shown by `handle_hotkeys_command` in `hoocode-code-tui-app/src/mode/commands.rs` |
| `/changelog` | changelog | `hoocode-code-tui-app/src/changelog.rs` |
| Questions from code off the UI thread | select as a console (M7, ratatui) / editor dialogs: the select title (with the countdown while timed) names the console, one row per option with accent `›` on the selection band, hint foot `↑↓ navigate · enter select · esc cancel`; shared `Console` chrome in `code-tui-widgets/src/console.rs` | `hoocode-code-tui-app/src/dialog_bridge.rs`, `extension_selector.rs` (`ExtensionSelectorComponent`), `extension_editor.rs` |

## Slash commands

| Part | Where |
|---|---|
| The list (names, descriptions, order) | `code-resources/src/slash_commands.rs` `BUILTIN_SLASH_COMMANDS`. List only commands that run: an entry with no `BuiltinCommand` arm becomes `Pending` and answers "not available yet". `/export` writes JSONL only; there is no `/share` |
| Name → `BuiltinCommand` | `mode/commands.rs`, `enum BuiltinCommand` and its parser |
| What each does | `mode/commands.rs` `run_builtin_command` (`/compact`: `handle_compact_command`) |
| `/mcp`: each MCP server with source, state and tool count (a text block, not a picker). `AuthNeeded` servers show `run /mcp login <server>`. Typed, not in the autocomplete list, so the slash menu does not change for users without MCP | `handle_mcp_command` in `mode/mcp.rs`; listing in `hoocode-code-tui-app/src/mcp_listing.rs` (`format_listing`); states from `code-agent-session/src/mcp.rs` (`McpHub::servers`) |
| `/mcp login <server>`: starts the OAuth login of an `AuthNeeded` HTTP server. The browser opens and the login link is a chat record (the fallback when it does not open); the redirect is awaited off the UI thread, then the server reconnects and its tools appear at the next turn | `start_mcp_login` in `mode/mcp.rs`; `McpHub::login` (`code-agent-session/src/mcp.rs`) runs `begin_login` and `finish` on `hoocode-io` |
| MCP elicitation (a server asks for input mid-call) in the TUI: a selector Answer / Decline / Cancel, then the options pane with one question per form field (choices for enums and booleans, free text otherwise); a URL request shows the link and asks Accept / Decline / Cancel | `hoocode-code-tui-app/src/mcp_elicitation.rs` (`TuiElicitation`), on a `run_blocking` thread; the answers map to the MCP accept / decline / cancel. Print and rpc decline (`DeclineElicitation`) and say so on stderr |
| `/perf`: threads, RSS, frame and keystroke timing, stalls (Phase 0 counters) | `handle_perf_command` in `mode/commands.rs`; collector and report in `hoocode-code-tui-app/src/perf.rs` (`format_report`) |
| `/chrome`: `/chrome` names the stop; `/chrome full` or `/chrome compact` sets it and saves `chromeDensity`. `bare` is refused with an error that names the two stops. A saved `bare` loads as compact | `handle_chrome_command` in `mode/chrome.rs`; the dial is `hoocode-code-tui-app/src/chrome_layout.rs` |
| Mode commands (`/mode`, `/plan`, `/grill`, `/goal`, `/approve`) | `code-modes` |
| `/plugin`: `list`, `marketplace add <src> [--trust]|list|refresh`, `install <name[@marketplace]> [--scope user|project]`, `remove <name> [--scope ...]`, `update [name]`, `trust`. Text blocks, not a picker. A change makes the plugins live (MCP servers restart with the new set, then the `/reload` path), or waits for the turn to end. Its notice shows after that reload | `handle_plugin_command` in `mode/plugin.rs` (`reload_plugins`); the parser and replies are `code-plugins` `command.rs` (`command::run`, shared with the CLI's `hoocode plugin`, `install`, `remove`/`uninstall` and `list` in `code-cli` `plugin_command.rs`); the work in `code-plugins` `install.rs`, `marketplace.rs` |
| `/agents`: the subagent runs of this session as a picker (see Pickers); Enter attaches to the run's transcript, read-only (T3.4). Listed on the tools pool (`mode/attach.rs` `open_agent_runs`) | `mode/attach.rs`; the entry is `BuiltinCommand::Agents` in `mode/commands.rs` |
| `/loop`: scheduled prompts (`/loop "<cron>"` or `<N>m\|h\|d` with a prompt, `once`, `list`, `delete <id>`, `stop`) and `/loop auto [--max-turns N] <task>`. Forms parsed in `code-scheduler` `loop_command.rs`; the handler and the auto-loop turn flow are in `mode/loop_command.rs` (`PromptDone` and `MessageEnd` call into it; `tick_scheduler` reports late and skipped prompts). The auto-loop flag and halt, read by the `AskUserQuestion` host, are `auto_loop.rs` | `code-tui-app` `mode/loop_command.rs`, `auto_loop.rs`; `code-scheduler` `loop_command.rs` |
| MCP tools in the transcript | `mcp_<server>_<tool>` tools use the generic tool block (no renderer); progress reports arrive as partial results and show there |
| Skill and prompt-template commands | `code-resources` |

To add one: add it to `BUILTIN_SLASH_COMMANDS`, add a `BuiltinCommand` variant and
its parse arm, handle it in `run_builtin_command`, add a test, update this table.

## Scroll-back (pinned view)

Live: the view follows the newest output. Pinned: the view holds its row while output arrives.
The TUI owns the mechanism; the app owns which key does what.

**Behaviour**

- Scrollbar: last column, only while pinned and the transcript is taller than the view.
- The transcript renders one column narrower while the bar shows.
- `┃` is the thumb, `░` the track. The shapes differ, not only the colour.
- Click above the thumb: page up. Click below: page down. On the thumb: nothing.
- A picker or ask-options pane with focus paints its lines at the bottom, above the status row.
  The transcript keeps at least one row.
- Pinning works from any focus. Picker keys never un-pin.
- PgDn past the bottom, or the wheel reaching the bottom, returns to live.
- Status row (bottom): the REWIND bar while pinned (`◂◂ REWIND  43–61 of 78 · 55%  ↓ 3 new`, label in vio, then the keys: line, `PgUp/PgDn page`, top, live). The SCAN bar while searching (`⌕ SCAN  query  N contact(s)`, label in warn; typing: `enter lock · esc close`; locked: `enter next · esc close`).
  The line keys are `↑/↓` over an empty prompt and `shift+↑/shift+↓` while the prompt holds text.
- Full-screen and mouse reporting on (the default): a plain drag does not select text. Selection is shift+drag.
- The view pinned on a message keeps that message on top when the tool output view (ctrl+o) changes rows.
  A rebuild of the transcript (`/new`, `/resume`, the thinking toggle, ctrl+t) goes live.
- Full-screen (M1d, the default): the body is the transcript (header and chat). The bar row sits above the bottom,
  and carries `↓ N new` while pinned: lines that arrived since the view last pinned or moved. A shorter transcript
  (a clear) starts the count again. The scrollbar column is always reserved (blank while live, so pinning does not
  reflow), and its glyphs are drawn per body row while pinned. Search marks its matches in the body (`◂ contact` on each matched row) and steps with enter/`n`/`p`. A focused
  picker replaces the bottom rows while pinned. A body image whose picture would reach above the body is named
  `[image]`.

**Files**

| Concern | Where |
|---|---|
| Pin, clamp, page, live, search, wheel, scrollbar click, picker split, paint | `tui-render/src/tui.rs`: `scroll_by_lines`, `scroll_by_pages`, `scroll_to_top`, `scroll_to_live`, `scroll_to_row`, `transcript_width`, `pinned_split`, `press_scrollbar`, `render_scroll_view`, `handle_mouse_event` |
| Full-screen scroll (M1d): body window, bar with `N new`, scrollbar column, picker rows, search marks, image clip | `tui-render/src/tui.rs`: `compose_fullscreen`, `body_row`, `region_row`, `fullscreen_regions`, `fullscreen_measure` (before the first frame) |
| Scrollbar glyphs (pure), `SCROLLBAR_THUMB`, `SCROLLBAR_TRACK` | `tui-render/src/scrollbar.rs` |
| Full-screen mode (M1): alternate screen for the run, one frame composed from regions, per-row diff, no scrollback writes. Off by default (`Tui::set_fullscreen`); the app's default and `HOOCODE_FULLSCREEN` come in M1c | `tui-render/src/tui.rs`: `set_fullscreen`, `set_fullscreen_body`, `paint_fullscreen`, `compose_fullscreen`, `start`, `stop` |
| Stray stderr text (a TLS warning, child output, a reload warning, an extension handler error) while the TUI owns the terminal: it goes to the debug log. Every such write goes through `write_stderr` | `hoocode-runtime/src/diagnostics.rs` (`write_stderr`); sink installed in `run` (`mode/mod.rs`) |
| Full-screen row arithmetic (pure): `split_rows`, `body_start`, `bottom_window`, `anchor_on_resize`, `body_width`, `new_lines_since` | `tui-render/src/fullscreen.rs` |
| Prompt focus: `Tui.scroll_prompt`. Any other focus is a picker (`scroll_pane`) | `tui-render/src/tui.rs`; set in `install_scroll_view` |
| Key routing, search, status text, keys at the prompt | `hoocode-code-tui-app/src/scroll_view.rs`: `handle_live`, `handle_picker`, `handle_input`, `format_scroll_status` (REWIND/SCAN bars as ratatui spans via `BarRun`/`paint_bar`), `install_scroll_view` |
| Keeping the pinned top on its message across a change of rows | `scroll_view.rs`: `capture_top`, `reanchor_top` (called from `apply_tool_output_view` in `mode/transcript.rs`) |
| Jump to user message (full-screen: offsets inside the body; inline: among the root's children) | `scroll_view.rs` `jump_to_user_message`, at `transcript_width()` |

**Key routing**

| Key | Prompt focused | Picker focused (live or pinned) |
|---|---|---|
| PgUp / PgDn | Page the transcript (pins) | Same |
| Ctrl+Home / Ctrl+End | Top / live | Same |
| Wheel | Scroll; wheel up pins | Same |
| Scrollbar click | Pinned only: page toward the click | Same |
| ↑ / ↓ | Pinned: one line over an empty prompt; with text, the prompt. Live: the prompt | Picker |
| Shift+↑ / Shift+↓ | Live: Shift+↑ pins one line up; Shift+↓ does nothing. Pinned: one line | Picker |
| Ctrl+↑ / Ctrl+↓ | Previous / next user message | Picker |
| Ctrl+R | Pin at bottom, open search | Picker |
| `/` | Pinned: search in view. Live: types `/` | Picker |
| `n` / `p` | Search open: next / previous match. Else the prompt | Picker |
| Enter | Search locked: next contact. Typing: locks the query. Else the prompt | Picker |
| Esc | Search open: clear it. Else live; during an agent run, also aborts it | Picker |
| Enter, typed text | Search open: the query (Enter commits). Else pinned: back to live, then the prompt | Picker, never un-pins |

A key that is not a scroll key un-pins at the prompt, then does its usual job.

## Task panel

One tab per kind of task: **Plan** (TodoWrite items, each linked run nested under its item), **Agents** (the subagent forest), **Schedule** (scheduled jobs, one row each, with `next in …`), **Shell** (background shell jobs, one row each: running with Abort, then done, failed with its exit status, or killed; the rows are `shell:<id>`, published by `hoocode-code-tool-bash` `jobs.rs`), **MCP** (MCP servers and calls). A tab shows only while it has rows. Unfocused, the panel shows the current tab's rows and one count per tab on the strip (`Plan 1/3  Agents 1/1`). In the compact density (`Summary`, one row) it shows the plan line on the left (`Plan 3/5  ◉ <running item>`, cut with `…`) and the other tabs' counts on the right; the plan is rebuilt from the session's last successful TodoWrite on resume, reload and tree navigation (`restore_plan` in `hoocode-code-tools-optin`, called from `render_initial_messages`). Adding a kind means a `TaskPanelView` variant, its label, and a rule in `lens_rows`.

| Concern | Where |
|---|---|
| Tabs, rows, pick, strip, hint line, cancel arming, the tick (`ticking`, while any row in any tab is running) | `code-tui-widgets/src/task_panel.rs` (`TaskPanelComponent`, `RowModel`, `PanelEvent`) |
| Focus (Ctrl+G), what the app does with events (`Open`, `Cancel`, `Exit`), the pool cancel | `hoocode-code-tui-app/src/mode/task_panel.rs`; events drained in `mode/mod.rs` `run` |
| Keys (`app.panel.*`) | `hoocode-code-tui-keybindings/src/keybindings.rs`; listed on `/hotkeys` (`code-tui-widgets/src/controls.rs`) |

**Keys.** Ctrl+G at the prompt focuses the panel (with no rows it does nothing and says so). Once focused, the panel takes the keyboard (`Tui::set_focus`).

| Key | Focused panel |
|---|---|
| `app.panel.focus` (Ctrl+G) | Back to the prompt (a toggle) |
| `app.panel.nextTab` (Tab) / `app.panel.prevTab` (Shift+Tab) | Next / previous tab with rows (wraps) |
| `app.panel.up` / `app.panel.down` (arrows) | Pick the row above / below (ends clamp). The pick is kept by row id across rebuilds |
| `app.panel.open` (Enter) | A row with an action (MCP server `Login`) queues `Perform`, which `TaskStore::perform` runs (`/mcp login` path). A subagent row queues `Open`, which attaches to its run (`attach_run`, T3.3). A shell row queues `Open`, which attaches to the job's output (`attach_job`, T5.3). Other rows do nothing on Enter |
| `app.panel.cancel` (`x`) | First press arms the row (the right column says `press x again to cancel`). Second: an Abort row (MCP call, running shell job) queues `Perform(Abort)`; a running subagent run queues `Cancel`, through `SubagentPool::cancel`. Other rows ignore it. Any other key disarms |
| `app.panel.exit` (Esc) | Disarms a pending cancel, else back to the prompt |

**Scroll and density.** Focus shows the strip even with one tab, the pick marker `›`, and a hint line, and shows full rows even in the summary density.

## Attached view (subagent transcript or shell output)

Attach shows one subagent run's transcript, or one shell job's output (T5.3), on the chat area, read-only, with the header row above it and the prompt's place taken by a hint. Esc goes back, and the session's transcript and scroll come back as they were. Ctrl+C and Ctrl+D go back too (the view takes the keyboard). A session switch (`/new`, `/resume`, `/fork`, `/import`, `/cd`) detaches first (`take_session_runtime`).

**Shell output (T5.3).** Enter on a Shell row shows the job's log (`<session>/jobs/<id>.log`): the last 32 KiB at first, as plain text (escape sequences and carriage returns dropped), then followed once a second on the tools pool while the job runs. Once the job has ended, its output is closed (no process of its group is alive, so a background child that still writes keeps it open: `jobs.rs` `output_open`) and its log is read to the end, the view stops polling. The header reads `◂ shell · <command> · running | done | exit N | killed · Esc back`; the hint says it is a shell job's output. A read that started from an earlier position is dropped, so no text shows twice. Each tick's read (`jobs.rs` `follow_log`) catches up to the end of the log in bounded chunks (up to 256 KiB a tick); a follower more than 64 KiB behind skips to the tail, and the view marks the gap (`… output skipped`). Files: `mode/job_tail.rs` (`JobTail`, the view's text and `accepts`, `state_word`), `mode/attach.rs` (`attach_job`, `poll_shell_tail`, `shell_tail_read`), `hoocode-code-tool-bash` `jobs.rs` (`follow_log`, `read_log`). Off the UI thread: the read runs on the tools pool and returns as `AppEvent::ShellTail`; if the pool cannot run it, the view gets the error and stops following instead of freezing.

**Where the output goes.** A job's stdout and stderr are the log file itself (no pipe), so the job keeps writing after hoocode exits (Leave running). The log is created new (`create_new`), so a job never appends to an older job's log of the same number.

## Quit with shell jobs running (T5.3)

A quit (Ctrl+D, Ctrl+C twice, `/quit`, or a fatal session error) with shell jobs running asks once: "N background shell job(s) still running. Quit?" with **Kill all** (the default) and **Leave running**. Kill all signals each job's process group (`jobs::kill_all_jobs`) before the app exits. Esc keeps the app open, and the next quit asks again. A quit asked again while the prompt is up does nothing. Files: `mode/task_panel.rs` (`QuitJobs`, `ready_to_quit`, `quit_jobs_answered`); the event loop in `mode/mod.rs` `run` asks `ready_to_quit` where it used to break on `exit_requested`. After the loop, `mode/mod.rs` `run` calls `mode/shutdown.rs` (abort the agent, kill the tool process groups, flush the session, close MCP, release, then restore the terminal; each step has a deadline, concurrency.md §4).

| Concern | Where |
|---|---|
| Header row and hint (pure components, goldens) | `code-tui-widgets/src/attach_view.rs` (`AttachHeader`, `AttachPrompt`) |
| Attach, detach, the run's history (read on the tools pool), the run's live events, `/agents` rows | `hoocode-code-tui-app/src/mode/attach.rs` |
| Keys: `app.subagent.attach` (Ctrl+N, at the prompt: newest Agent block) and `app.subagent.detach` (Esc, in the attached view) | `hoocode-code-tui-keybindings/src/keybindings.rs`; listed on `/hotkeys` |
| Screen check | `scripts/tui/scenarios/subagent-attach.json` (tmux, the mock LLM; the attach, live and finished, and the way back) |

**How it works.** The chat container is shared by the session and the attached run. Two halves exist, and `Attached::main_in` says which one is on screen; `flip_view` swaps them. The session is on screen by default. The run's half is shown only in three windows: the run's own events (`apply_attached_event`), a key to the screen (`accept_terminal_event`), and the render at the end of the loop. A session handler therefore runs against the transcript it always wrote, and the run's view is never changed by it. While attached, a session event does not set the dirty flag (its change is drawn on the next one-second tick).

## Events and the loop

| What | Where |
|---|---|
| Main loop (input, app events, ticks, render). Up to 64 keys per pass, then app events, then one render (`accept_terminal_event`, `flush_scheduled_render`) | `mode/mod.rs` `run` |
| Events from other threads | `enum AppEvent` (session events, prompt done, bash chunks, dialogs, login…; the attach ones: `AttachLoaded`, `AgentRunsListed`, `AttachPicked`) |
| Agent session events → screen | `handle_session_event` |
| Live events of subagent runs (`AppEvent::Subagent`) → the Agent blocks that dispatched them, and the attached view if it shows that run | `apply_subagent_event` in `mode/subagents.rs` (sink set in `mode/mod.rs`; forwarding in `hoocode-code-subagents` `instance.rs` `wire_live_events`); the attached view's part is `mode/attach.rs` `apply_attached_event` |
| Key and UI actions | `enum Action`, `handle_action` |
| Submitting a prompt | `submit`, `prompt`, `prompt_with_images` |
| Streaming render throttle | `schedule_streaming_render`, `run_streaming_render` |
| Ctrl+Z | `hoocode-code-tui-app/src/suspend.rs` |
| Perf counters (`/perf`, `--perf-log <file>`) | `hoocode-code-tui-app/src/perf.rs`. The loop calls `Perf::key_arrived` for each key (stamped by the terminal's reader thread: `TuiEvent::Input(data, arrived)`) and `Perf::end_iteration` after each frame. Frame build and write times come from `Tui::set_frame_observer`. The UI thread only records into fixed rings; a sampler thread (`hoocode-perf`) reads `/proc` and writes the log once a second. |

## TUI library (`tui-*`)

| Concern | Crate / file |
|---|---|
| Differential renderer, frames, pinned scroll view, scrollbar | `tui-render/src/tui.rs`, `component.rs`, `scrollbar.rs` |
| Terminal: raw mode, stdin reader, resize (SIGWINCH), mouse, SIGTERM and SIGHUP restore | `tui-terminal/src/lib.rs`, `stdin_buffer.rs`, `mouse.rs`, `interrupt.rs` |
| Terminal output: every write, in order; one pending frame | `tui-terminal/src/output.rs` (`hoocode-term-out`) |
| Key parsing (Kitty, modifyOtherKeys), matching | `tui-keys` |
| App key map, `keybindings.json`, hint text | `code-tui-keybindings` |
| Components: text, input, select list, picker cursor, loader, box, image | `tui-components/src/` |
| Markdown (`marked` v15 rules) | `tui-components/src/markdown/` |
| Syntax highlighting (highlight.js 10.7.3 grammars) | `tui-highlight` |
| Images (Kitty, iTerm2) | `tui-images` |
| ANSI-aware width, wrap, truncate | `tui-util` |
| Kill ring, undo | `tui-editing` |
| Fuzzy match | `tui-fuzzy` |
| Themes (built-in and JSON), colours | `code-tui-theme` |

## Rules for UI changes

- Screen goldens: about 10 key interactive screens, a pre-release check. `python3 scripts/tui/goldens.py check all`
  runs the real binary in tmux against the mock LLM (`scripts/tui/mockllm.py`) and diffs each screen with
  `tests/golden/tui/<scenario>/`. Scenarios are in `scripts/tui/scenarios/`. A deliberate
  change is accepted with `goldens.py update <scenario>` and shows in the diff. Component goldens
  (`tests/golden/<crate>/`) run in nextest; accept them with `UPDATE_GOLDENS=1`. The screens
  are hoocode's own.
- Concurrency ([concurrency.md](../design/concurrency.md)): the UI thread never does
  file or network I/O. Work off the UI thread reports back through `AppEvent`.
- Attached view: code that writes to the transcript (`self.transcript`, `add_to_chat`) runs
  with the session on screen, which is the loop's default. Only the attached run's events and
  the render go through `show_attached`; a new window needs the same care.

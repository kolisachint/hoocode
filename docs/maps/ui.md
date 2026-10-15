# UI map

Where each part of the interactive screen lives. Snapshot of 2026-10-08. Crate names are `hoocode-*` since step 0c
([naming-and-paths.md](../../archive/docs/naming-and-paths.md)). Crates are
described in [packages.md](packages.md). Names, not line numbers: line numbers drift.

**Keep this current.** Add, move or remove a screen part, picker or slash command →
update this page in the same commit.

## The screen, top to bottom

```
┌ transcript (re-rendered component tree) ─ messages, tool blocks, notices ────┐
│ task panel ─ TodoWrite plan items, subagent runs                             │
│ notification band ─ tips, transient notices, progress bar                    │
│ ┌ prompt frame ─ editor, autocomplete; pickers and dialogs replace it ─────┐ │
│ └──────────────────────────────────────────────────────────────────────────┘ │
└ footer ─ 2 rows: folder, branch, path │ model, context, cost ────────────────┘
```

| Region | Component | File |
|---|---|---|
| Whole screen, event loop | `InteractiveMode` (`Mode`) | `hoocode-code-tui-app/src/mode/mod.rs` (struct, setup, loop); the rest is split by concern into `mode/*.rs`: `input` (editor and keys), `transcript` (`Transcript`: the chat container and the components that track it; reset replaces it), `events` (agent and session events), `prompt_queue`, `session_ops`, `session_op` (the session operations that run off the UI thread: `SessionOpDone`, `SessionOutcome`, `finish_session_op`), `tree`, `models`, `auth`, `settings`, `dialogs`, `subagents`, `bash`, `clipboard`, `mcp`, `chrome` (footer, title, colour), `commands` (slash commands) |
| Transcript scroll-back view | scroll view | `hoocode-code-tui-app/src/scroll_view.rs` |
| Startup banner | wordmark | `hoocode-code-tui-app/src/wordmark.rs` |
| Startup resource listing (skills, commands, agents, MCP, context; no themes, extensions or canvases cells, N12) | resource display | `hoocode-code-tui-app/src/resource_display.rs`, `expandable_text.rs` |
| Task panel: tabs Plan · Agents · MCP (a tab shows while it has rows), a count per tab; focused (Ctrl+G) it takes the keys in [Task panel](#task-panel) | task panel | `code-tui-widgets/src/task_panel.rs` (tabs, rows, keys); focus and events in `hoocode-code-tui-app/src/mode/task_panel.rs` |
| Notification band | notification panel, tips, startup update notice | `hoocode-code-tui-app/src/notification_panel.rs`, `tips.rs`; the update notice is polled by `poll_update_notice` in `mode/events.rs` from `hoocode-code-update` |
| Completion bell (`chimeOnTurnComplete`): one `\x07` when a turn over 10 s ends (not after an abort) or when `AskUserQuestion` opens (not MCP elicitation forms); repeats within 5 s dropped | `hoocode-code-tui-app/src/chime.rs` (pure timing, `TurnTimer` set at prompt submit); rung by `ring_chime` and `finish_turn_chime` in `mode/events.rs`, and at the `AskOptions` dialog in `mode/mod.rs` when its request has `chime` set (`dialog_bridge.rs`) |
| Progress bar | progress bar, startup progress | `hoocode-code-tui-app/src/progress_bar.rs`, `startup_progress.rs` |
| Prompt editor | `CustomEditor` around `Editor` | `mode/input.rs` (`CustomEditor`); `tui-components/src/editor/` |
| Prompt frame | frame, input frame | `tui-components/src/frame.rs`; `code-tui-widgets/src/input_frame.rs` |
| Attached subagent view (T3.3): a header row above the chat (`◂ <type> · <description> · <state> · Esc back`), and the prompt's place with a read-only hint; Esc goes back. The chat shows the run's transcript while attached; the session's is parked and kept up to date | attached view | `code-tui-widgets/src/attach_view.rs` (header, hint). The swap, the routing and the run's events: `hoocode-code-tui-app/src/mode/attach.rs`; the header's slot is `Mode::attach_slot` (above the chat) |
| Autocomplete (`/`, `@file`) | autocomplete | `tui-components/src/autocomplete/`; commands fed by `mode/input.rs` `setup_autocomplete_provider`. `@file` uses the in-process finder `hoocode-code-tools::file_finder`, injected by `mode/input.rs` `at_file_finder` (no `fd`) and run on a worker thread (`autocomplete/file_search.rs`): typing never waits, and the editor re-asks when a walk finishes (`take_ready`). |
| Footer | footer, footer data. Always two rows: row 1 folder, branch, session, path, then mode chip and view dial; row 2 model, context gauge, tokens, cost. Compact keeps both rows. Transient messages show above the prompt, not in the footer: a warning line (memory shedding, UI stall), extension statuses and startup progress, read with `FooterComponent::transient_lines`. The warning is set by `mode/mod.rs` `sync_runtime_health` | `hoocode-code-tui-app/src/footer.rs`, `footer_data.rs` |
| Session chip | session chip | `code-tui-widgets/src/session_chip.rs` |
| How much room chrome gets (two stops: full, compact; `/chrome <full\|compact>`, `Alt+Z` / `Shift+Alt+Z` toggle) | chrome layout | `hoocode-code-tui-app/src/chrome_layout.rs` |

## Transcript items

| Item | File (`code-tui-widgets/src/`) |
|---|---|
| User message | `user_message.rs` |
| Assistant message (markdown, thinking) | `assistant_message.rs` → `tui-components/src/markdown/` |
| Custom and branch-summary messages | `custom_message.rs` |
| One tool call's block | `tool_execution.rs` |
| A run of tool calls on one line; radar view | `tool_chain.rs`, `tool_chain_summary.rs`, `tool_signal.rs` |
| Dial: radar ↔ peek (Ctrl+O toggles, persisted). `peek_block` is the shared peek body | `tool_output_view.rs` |
| `<skill …>` block of a `/skill:name` message (radar row / peek body) | `skill_block.rs` |
| Background-run notice (radar row / peek body) | `background_notice.rs` |
| `!` bash command typed by the user | `bash_execution.rs` |
| Diffs | `diff.rs`, `jsdiff.rs` |
| File content with line numbers | `read_output.rs` |
| Notice rows (`showRecord`) | `hoocode-code-tui-app/src/record_row.rs` |

### Tool renderers (`code-tui-widgets/src/tools/`)

| Tool | File |
|---|---|
| `Shell` | `bash.rs` |
| `ShellOutput`, `ShellKill` (opt-in) | none: the default block (no renderer yet; T5.3 adds the Shell tab) |
| `Read` (and skills) | `read.rs` |
| `Edit` | `edit.rs` |
| `Write` | `write.rs` |
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
| `/settings` | settings pane | `settings_selector.rs` (uses `tui-components/src/settings_list.rs`). No External tools category: the external-tools layer went with `fd`/`rg` (reliability 1.4), so the top level is the tool rows, then the categories. Tool rows are sorted by tool name (`tool_row_sort_key`: Shell, Edit, Read, Write). The top-level **Models** row shows `all models` or `N models` (from `scopedModels`); Enter closes the pane and opens `/scoped-models` (`SettingsChange::OpenScopedModels`, handled in `hoocode-code-tui-app/src/mode/settings.rs`). |
| `/model` | model picker | `model_selector.rs`. A model whose provider's stored OAuth token has expired shows an `expired` marker (`set_expired_providers`, read by `mode/models.rs` `show_model_selector` via `AuthStorage::readiness`; no refresh). |
| `/scoped-models` | scoped models | `scoped_models_selector.rs`. Loads `scopedModels` (`hoocode-code-tui-app/src/mode/models.rs`, `concrete_scoped_entries`), one row per model with an effort column and a category column, a header line above the rows. Keys: enter toggles, alt+a/alt+x all/clear, alt+g provider, alt+up/alt+down reorder (order is priority), **tab** cycles the row's effort through the model's supported levels then unset, **alt+j** cycles its category none, cheap, fast, standard, capable then none, alt+s saves full entries (`Persist(Vec<ScopedModel>)`, validated first). Alias is kept, not edited. Typing filters the list. |
| thinking level, theme, `/color` | one-list pickers | `small_selectors.rs`, `framed_list.rs` |
| `/agents` | subagent runs of this session (running, done, interrupted), one row each; Enter attaches to the chosen run | `show_selector` (`ExtensionSelectorComponent`); the rows are read on the tools pool by `mode/attach.rs` `list_runs`, then `agent_runs_listed` opens the picker and `AppEvent::AttachPicked` attaches |
| `/resume` | session picker | `session_selector.rs`, `session_selector_search.rs`. Subagent runs (`<session>/subagents/`) are rows under their parent in the threaded view, added by the loaders in `hoocode-code-tui-app/src/session_picker.rs` (`with_subagent_runs`, from `list_subagent_sessions` in `code-session`). A row reads `<type> · <complete\|failed\|…>`, `running` or `interrupted`. A search, or a non-threaded sort, lists the parent rows only. Enter on an `interrupted` run opens a choice: `v` views its transcript, `r` resumes it (`SubagentRunRef` to `pool.resume` in `hoocode-code-tui-app/src/mode/subagents.rs`; in-session picker only), esc backs out. Deleting a session deletes its `subagents/` dir. |
| `--resume` on the command line | session picker on its own TUI | `hoocode-code-tui-app/src/session_picker.rs` |
| `/tree` | session tree | `tree_selector.rs` |
| `/fork` | pick a user message | `user_message_selector.rs` |
| MCP trust prompt (once per process, at startup, when a project or plugin declares MCP servers) | the permission selector (`TuiPermissionUi`, `DialogRequest::Select`); asked on a thread by `mode/mcp.rs` `ask_mcp_trust`, answered as `MCP_TRUST_YES` / `MCP_TRUST_NO`; the grant runs `McpHub::grant` (trust store, then start) |
| `/login`, `/logout` | provider pickers, login dialog | `oauth_selector.rs`, `login_dialog.rs`; flow in `hoocode-code-tui-app/src/login_controller.rs` |
| `AskUserQuestion` tool | options pane | `ask_options.rs` |
| `hoocode config` | resource list | `config_selector.rs` |
| `hoocode update` (CLI, no TUI): prints the update command for the install method; never replaces the binary | `hoocode-code-update/src/lib.rs` (`run_update_command`); dispatched in `hoocode-code-cli/src/lib.rs` |
| `/hotkeys` | shortcuts page | `hoocode-code-tui-app/src/hotkeys.rs` |
| `/changelog` | changelog | `hoocode-code-tui-app/src/changelog.rs` |
| Questions from code off the UI thread | select / editor dialogs | `hoocode-code-tui-app/src/dialog_bridge.rs`, `extension_selector.rs`, `extension_editor.rs` |

## Slash commands

| Part | Where |
|---|---|
| The list (names, descriptions, order) | `code-resources/src/slash_commands.rs` `BUILTIN_SLASH_COMMANDS` |
| Name → `BuiltinCommand` | `mode/commands.rs`, `enum BuiltinCommand` and its parser |
| What each does | `mode/commands.rs` `run_builtin_command` (`/compact`: `handle_compact_command`) |
| `/mcp`: each MCP server with source, state and tool count (a text block, not a picker). `AuthNeeded` servers show `run /mcp login <server>`. Typed, not in the autocomplete list, so the slash menu does not change for users without MCP | `handle_mcp_command` in `mode/mcp.rs`; listing in `hoocode-code-tui-app/src/mcp_listing.rs` (`format_listing`); states from `code-agent-session/src/mcp.rs` (`McpHub::servers`) |
| `/mcp login <server>`: starts the OAuth login of an `AuthNeeded` HTTP server. The browser opens and the login link is a chat record (the fallback when it does not open); the redirect is awaited off the UI thread, then the server reconnects and its tools appear at the next turn | `start_mcp_login` in `mode/mcp.rs`; `McpHub::login` (`code-agent-session/src/mcp.rs`) runs `begin_login` and `finish` on `hoocode-io` |
| MCP elicitation (a server asks for input mid-call) in the TUI: a selector Answer / Decline / Cancel, then the options pane with one question per form field (choices for enums and booleans, free text otherwise); a URL request shows the link and asks Accept / Decline / Cancel | `hoocode-code-tui-app/src/mcp_elicitation.rs` (`TuiElicitation`), on a `run_blocking` thread; the answers map to the MCP accept / decline / cancel. Print and rpc decline (`DeclineElicitation`) and say so on stderr |
| `/perf`: threads, RSS, frame and keystroke timing, stalls (Phase 0 counters) | `handle_perf_command` in `mode/commands.rs`; collector and report in `hoocode-code-tui-app/src/perf.rs` (`format_report`) |
| `/chrome`: `/chrome` names the stop; `/chrome full` or `/chrome compact` sets it and saves `chromeDensity`. `bare` is refused with an error that names the two stops. A saved `bare` loads as compact | `handle_chrome_command` in `mode/chrome.rs`; the dial is `hoocode-code-tui-app/src/chrome_layout.rs` |
| Mode commands (`/mode`, `/plan`, `/grill`, `/goal`, `/approve`) | `code-modes` |
| `/plugin`: `list`, `marketplace add <src> [--trust]|list|refresh`, `install <name[@marketplace]> [--scope user|project]`, `remove <name> [--scope ...]`, `update [name]`, `trust`. Text blocks, not a picker. A change makes the plugins live (MCP servers restart with the new set, then the `/reload` path), or waits for the turn to end. Its notice shows after that reload | `handle_plugin_command` in `mode/plugin.rs` (`run_plugin_command` is the parser and the work; `reload_plugins`); the work in `code-plugins` `install.rs`, `marketplace.rs` |
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
- Status row (bottom): position, then the keys: line, `PgUp/PgDn page`, top, live. Search has its own text.

**Files**

| Concern | Where |
|---|---|
| Pin, clamp, page, live, search, wheel, scrollbar click, picker split, paint | `tui-render/src/tui.rs`: `scroll_by_lines`, `scroll_by_pages`, `scroll_to_top`, `scroll_to_live`, `scroll_to_row`, `transcript_width`, `pinned_split`, `press_scrollbar`, `render_scroll_view`, `handle_mouse_event` |
| Scrollbar glyphs (pure), `SCROLLBAR_THUMB`, `SCROLLBAR_TRACK` | `tui-render/src/scrollbar.rs` |
| Prompt focus: `Tui.scroll_prompt`. Any other focus is a picker (`scroll_pane`) | `tui-render/src/tui.rs`; set in `install_scroll_view` |
| Key routing, search, status text, keys at the prompt | `hoocode-code-tui-app/src/scroll_view.rs`: `handle_live`, `handle_picker`, `handle_input`, `format_scroll_status`, `install_scroll_view` |
| Jump to user message | `scroll_view.rs` `jump_to_user_message`, at `transcript_width()` |

**Key routing**

| Key | Prompt focused | Picker focused (live or pinned) |
|---|---|---|
| PgUp / PgDn | Page the transcript (pins) | Same |
| Ctrl+Home / Ctrl+End | Top / live | Same |
| Wheel | Scroll; wheel up pins | Same |
| Scrollbar click | Pinned only: page toward the click | Same |
| ↑ / ↓ | Pinned: one line. Live: the prompt | Picker |
| Ctrl+↑ / Ctrl+↓ | Previous / next user message | Picker |
| Ctrl+R | Pin at bottom, open search | Picker |
| `/` | Pinned: search in view. Live: types `/` | Picker |
| `n` / `p` | Search open: next / previous match. Else the prompt | Picker |
| Esc | Search open: clear it. Else live | Picker |
| Enter, typed text | Search open: the query (Enter commits). Else pinned: back to live, then the prompt | Picker, never un-pins |

A key that is not a scroll key un-pins at the prompt, then does its usual job.

## Task panel

One tab per kind of task: **Plan** (TodoWrite items, each linked run nested under its item), **Agents** (the subagent forest), **Schedule** (scheduled jobs, one row each, with `next in …`), **MCP** (MCP servers and calls). A tab shows only while it has rows. Unfocused, the panel shows the current tab's rows and one count per tab on the strip (`Plan 1/3  Agents 1/1`). Adding a kind means a `TaskPanelView` variant, its label, and a rule in `lens_rows`. Shell goes in with T5.

| Concern | Where |
|---|---|
| Tabs, rows, pick, strip, hint line, cancel arming, the tick (`ticking`, while any row in any tab is running) | `code-tui-widgets/src/task_panel.rs` (`TaskPanelComponent`, `RowModel`, `PanelEvent`) |
| Focus (Ctrl+G), what the app does with events (`Open`, `Cancel`, `Exit`), the pool cancel | `hoocode-code-tui-app/src/mode/task_panel.rs`; events drained in `mode/mod.rs` `run` |
| Keys (`app.panel.*`) | `hoocode-code-tui-keybindings/src/keybindings.rs`; listed on `/hotkeys` (`hotkeys.rs`) |

**Keys.** Ctrl+G at the prompt focuses the panel (with no rows it does nothing and says so). Once focused, the panel takes the keyboard (`Tui::set_focus`).

| Key | Focused panel |
|---|---|
| `app.panel.focus` (Ctrl+G) | Back to the prompt (a toggle) |
| `app.panel.nextTab` (Tab) / `app.panel.prevTab` (Shift+Tab) | Next / previous tab with rows (wraps) |
| `app.panel.up` / `app.panel.down` (arrows) | Pick the row above / below (ends clamp). The pick is kept by row id across rebuilds |
| `app.panel.open` (Enter) | A row with an action (MCP server `Login`) queues `Perform`, which `TaskStore::perform` runs (`/mcp login` path). A subagent row queues `Open`, which attaches to its run (`attach_run`, T3.3). Other rows do nothing on Enter |
| `app.panel.cancel` (`x`) | First press arms the row (the right column says `press x again to cancel`). Second: an Abort row (MCP call) queues `Perform(Abort)`; a running subagent run queues `Cancel`, through `SubagentPool::cancel`. Other rows ignore it. Any other key disarms |
| `app.panel.exit` (Esc) | Disarms a pending cancel, else back to the prompt |

**Scroll and density.** Focus shows the strip even with one tab, the pick marker `›`, and a hint line, and shows full rows even in the summary density.

## Attached view (subagent transcript)

Attach shows one subagent run's transcript on the chat area, read-only, with the header row above it and the prompt's place taken by a hint. Esc goes back, and the session's transcript and scroll come back as they were. Ctrl+C and Ctrl+D go back too (the view takes the keyboard). A session switch (`/new`, `/resume`, `/fork`, `/import`, `/cd`) detaches first (`take_session_runtime`).

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
| Terminal: raw mode, stdin reader, resize (SIGWINCH), mouse | `tui-terminal/src/lib.rs`, `stdin_buffer.rs`, `mouse.rs` |
| Terminal output: every write, in order; one pending frame | `tui-terminal/src/output.rs` (`hoocode-term-out`) |
| Key parsing (Kitty, modifyOtherKeys), matching | `tui-keys` |
| App key map, `keybindings.json`, hint text | `code-tui-keybindings` |
| Components: text, input, select list, picker cursor, loader, box, image | `tui-components/src/` |
| Markdown (port of `marked` v15) | `tui-components/src/markdown/` |
| Syntax highlighting (port of highlight.js 10.7.3) | `tui-highlight` |
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
  are hoocode's own (there is no TS reference).
- Concurrency ([concurrency.md](../design/concurrency.md)): the UI thread never does
  file or network I/O. Work off the UI thread reports back through `AppEvent`.
- Attached view: code that writes to the transcript (`self.transcript`, `add_to_chat`) runs
  with the session on screen, which is the loop's default. Only the attached run's events and
  the render go through `show_attached`; a new window needs the same care.

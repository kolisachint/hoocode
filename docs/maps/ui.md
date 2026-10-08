# UI map

Where each part of the interactive screen lives. Snapshot of 2026-10-08. Crate names are `hoocode-*` since step 0c
([naming-and-paths.md](../design/naming-and-paths.md)). Crates are
described in [packages.md](packages.md). Names, not line numbers: line numbers drift.

**Keep this current.** Add, move or remove a screen part, picker or slash command →
update this page in the same commit.

## The screen, top to bottom

```
┌ transcript (terminal scrollback) ─ messages, tool blocks, notices ───────────┐
│ task panel ─ TodoWrite plan items, subagent runs                             │
│ notification band ─ tips, transient notices, progress bar                    │
│ ┌ prompt frame ─ editor, autocomplete; pickers and dialogs replace it ─────┐ │
│ └──────────────────────────────────────────────────────────────────────────┘ │
└ footer ─ model, cwd, git branch, session chip, cost, context use ────────────┘
```

| Region | Component | File |
|---|---|---|
| Whole screen, event loop | `InteractiveMode` (`Mode`) | `code-tui-app/src/interactive_mode.rs` |
| Transcript scroll-back view | scroll view | `code-tui-app/src/scroll_view.rs` |
| Startup banner | wordmark | `code-tui-app/src/wordmark.rs` |
| Startup resource listing | resource display | `code-tui-app/src/resource_display.rs`, `expandable_text.rs` |
| Task panel | task panel | `code-tui-widgets/src/task_panel.rs` |
| Notification band | notification panel, tips | `code-tui-app/src/notification_panel.rs`, `tips.rs` |
| Progress bar | progress bar, startup progress | `code-tui-app/src/progress_bar.rs`, `startup_progress.rs` |
| Prompt editor | `CustomEditor` around `Editor` | `interactive_mode.rs` (`CustomEditor`); `tui-components/src/editor/` |
| Prompt frame | frame, input frame | `tui-components/src/frame.rs`; `code-tui-widgets/src/input_frame.rs` |
| Autocomplete (`/`, `@file`) | autocomplete | `tui-components/src/autocomplete/`; commands fed by `interactive_mode.rs` `setup_autocomplete_provider`. `@file` uses the in-process finder `hoocode-code-tools::file_finder`, injected by `interactive_mode.rs` `at_file_finder` (no `fd`) and run on a worker thread (`autocomplete/file_search.rs`): typing never waits, and the editor re-asks when a walk finishes (`take_ready`). |
| Footer | footer, footer data | `code-tui-app/src/footer.rs`, `footer_data.rs` |
| Session chip | session chip | `code-tui-widgets/src/session_chip.rs` |
| How much room chrome gets | chrome layout | `code-tui-app/src/chrome_layout.rs` |

## Transcript items

| Item | File (`code-tui-widgets/src/`) |
|---|---|
| User message | `user_message.rs` |
| Assistant message (markdown, thinking) | `assistant_message.rs` → `tui-components/src/markdown/` |
| Custom and branch-summary messages | `custom_message.rs` |
| One tool call's block | `tool_execution.rs` |
| A run of tool calls on one line; radar view | `tool_chain.rs`, `tool_chain_summary.rs`, `tool_signal.rs` |
| Radar / peek / full dial | `tool_output_view.rs` |
| `!` bash command typed by the user | `bash_execution.rs` |
| Diffs | `diff.rs`, `jsdiff.rs` |
| File content with line numbers | `read_output.rs` |
| Notice rows (`showRecord`) | `code-tui-app/src/record_row.rs` |

### Tool renderers (`code-tui-widgets/src/tools/`)

| Tool | File |
|---|---|
| `Shell` | `bash.rs` |
| `Read` (and skills) | `read.rs` |
| `Edit` | `edit.rs` |
| `Write` | `write.rs` |
| `CodeSearch` | `search.rs` |
| `Agent`, `AgentOutput` (subagents) | `subagent.rs` |
| Plugin tools | `plugins.rs` |
| `WebFetch`, `WebSearch` | `web.rs` (web tools are deferred) |
| Registry: tool name → renderer | `mod.rs` |
| Shared helpers | `../render_utils.rs` |

## Pickers and dialogs

They replace the prompt frame while open. All in `code-tui-selectors/src/` unless noted.

| Opened by | Picker | File |
|---|---|---|
| `/settings` | settings pane | `settings_selector.rs` (uses `tui-components/src/settings_list.rs`). No External tools category: the external-tools layer went with `fd`/`rg` (reliability 1.4), so the top level is the tool rows, then the categories. Tool rows are in hoocode-ts's order (sorted by its tool names, `tool_row_sort_key`: Shell, Edit, Read, Write). |
| `/model` | model picker | `model_selector.rs` |
| `/scoped-models` | scoped models | `scoped_models_selector.rs` |
| thinking level, theme, `/color` | one-list pickers | `small_selectors.rs`, `framed_list.rs` |
| `/resume` | session picker | `session_selector.rs`, `session_selector_search.rs` |
| `--resume` on the command line | session picker on its own TUI | `code-tui-app/src/session_picker.rs` |
| `/tree` | session tree | `tree_selector.rs` |
| `/fork` | pick a user message | `user_message_selector.rs` |
| `/login`, `/logout` | provider pickers, login dialog | `oauth_selector.rs`, `login_dialog.rs`; flow in `code-tui-app/src/login_controller.rs` |
| `AskUserQuestion` tool | options pane | `ask_options.rs` |
| `hoocode config` | resource list | `config_selector.rs` |
| `/hotkeys` | shortcuts page | `code-tui-app/src/hotkeys.rs` |
| `/changelog` | changelog | `code-tui-app/src/changelog.rs` |
| Questions from code off the UI thread | select / editor dialogs | `code-tui-app/src/dialog_bridge.rs`, `extension_selector.rs`, `extension_editor.rs` |

## Slash commands

| Part | Where |
|---|---|
| The list (names, descriptions, order) | `code-resources/src/slash_commands.rs` `BUILTIN_SLASH_COMMANDS` |
| Name → `BuiltinCommand` | `interactive_mode.rs`, `enum BuiltinCommand` and its parser |
| What each does | `interactive_mode.rs` `run_builtin_command` (`/compact`: `handle_compact_command`) |
| `/perf`: threads, RSS, frame and keystroke timing, stalls (Phase 0 counters) | `handle_perf_command` in `interactive_mode.rs`; collector and report in `code-tui-app/src/perf.rs` (`format_report`) |
| Mode commands (`/mode`, `/plan`, `/grill`, `/goal`, `/approve`) | `code-modes` |
| Skill and prompt-template commands | `code-resources` |

To add one: add it to `BUILTIN_SLASH_COMMANDS`, add a `BuiltinCommand` variant and
its parse arm, handle it in `run_builtin_command`, add a test, update this table.

## Events and the loop

| What | Where |
|---|---|
| Main loop (input, app events, ticks, render). Up to 64 keys per pass, then app events, then one render (`accept_terminal_event`, `flush_scheduled_render`) | `interactive_mode.rs` `run` |
| Events from other threads | `enum AppEvent` (session events, prompt done, bash chunks, dialogs, login…) |
| Agent session events → screen | `handle_session_event` |
| Key and UI actions | `enum Action`, `handle_action` |
| Submitting a prompt | `submit`, `prompt`, `prompt_with_images` |
| Streaming render throttle | `schedule_streaming_render`, `run_streaming_render` |
| Ctrl+Z | `code-tui-app/src/suspend.rs` |
| Perf counters (`/perf`, `--perf-log <file>`) | `code-tui-app/src/perf.rs`. The loop calls `Perf::key_arrived` for each key (stamped by the terminal's reader thread: `TuiEvent::Input(data, arrived)`) and `Perf::end_iteration` after each frame. Frame build and write times come from `Tui::set_frame_observer`. The UI thread only records into fixed rings; a sampler thread (`hoocode-perf`) reads `/proc` and writes the log once a second. |

## TUI library (`tui-*`)

| Concern | Crate / file |
|---|---|
| Differential renderer, frames, overlays | `tui-render/src/tui.rs`, `component.rs`, `overlay.rs` |
| Terminal: raw mode, stdin reader, resize (SIGWINCH), mouse | `tui-terminal/src/lib.rs`, `stdin_buffer.rs`, `mouse.rs` |
| Terminal output: every write, in order; one pending frame | `tui-terminal/src/output.rs` (`hoocode-term-out`) |
| Key parsing (Kitty, modifyOtherKeys), matching | `tui-keys` |
| App key map, `keybindings.json`, hint text | `code-tui-keybindings` |
| Components: text, input, select list, loader, box, image | `tui-components/src/` |
| Markdown (port of `marked` v15) | `tui-components/src/markdown/` |
| Syntax highlighting (port of highlight.js 10.7.3) | `tui-highlight` |
| Images (Kitty, iTerm2, Sixel) | `tui-images` |
| ANSI-aware width, wrap, truncate | `tui-util` |
| Kill ring, undo | `tui-editing` |
| Fuzzy match | `tui-fuzzy` |
| Themes (built-in and JSON), colours | `code-tui-theme` |

## Rules for UI changes

- The L2 parity harness (`migration/tui-parity/harness.py`) compares the screen with
  hoocode v0.6.0. A deliberate visual difference needs a harness mask or a recorded
  reason, not a silent failure.
- Concurrency ([concurrency.md](../design/concurrency.md)): the UI thread never does
  file or network I/O. Work off the UI thread reports back through `AppEvent`.

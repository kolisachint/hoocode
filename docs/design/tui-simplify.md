# TUI simplification inventory

Status: **design, 2026-10-09.** Every TUI component, with its simplification and improvement
ideas. In the TUI plan ([tui-activity.md](tui-activity.md)) this work comes **right after T0**
(goldens, the safety net) and **before** the feature phases (decision 22). All work here is
done by Haiku subagents (decision 23).

Source: 15 read-only Haiku reviews covering every TUI crate and file, about 63,000 LOC.
**Each finding is a claim, not a fact.** The reviews used grep and reading. Nothing was built
or profiled. The subagent that takes a row must first confirm it (callers, line numbers,
behaviour) and drop it if it does not hold.

## How to read and work this page

- **ID** is stable. Batches and PRs cite it.
- **Vis**: `no` = no visible change, so goldens must stay byte-identical. `yes` = the
  golden changes, and the change goes through the visual review (T0.5).
- **Risk** L/M/H. **Size** XS (<30 min) · S (a session) · M (a few) · L (many).
- Batches (bottom of the page) group rows that touch the same files, so parallel Haiku
  subagents don't collide.

## Coverage

| Area | Crate / files | LOC | Rows |
|---|---|---|---|
| Event loop, screen | `code-tui-app/interactive_mode.rs` | 6,624 | IM |
| App chrome and misc | `code-tui-app/` other files | 3,430 + footer 524 | AP |
| Transcript messages | `code-tui-widgets/` messages, `render_utils`, `brand` | 932 | MS |
| Tool blocks | `code-tui-widgets/` tool_*, diff, bash, read, `tools/*` | 4,384 | TB |
| Task panel, chip, footer | `code-tui-widgets/task_panel.rs`, `session_chip.rs`, `footer.rs` | ~1,700 | TP |
| Pickers and dialogs | `code-tui-selectors/`, app pickers | 9,586 | PK |
| Editor, input | `tui-components/editor/`, `input.rs`, `frame.rs`, `tui-editing` | ~3,800 | ED |
| Markdown, autocomplete, small components | `tui-components/` rest | 6,040 | MC |
| Renderer | `tui-render` | 2,960 | RN |
| Terminal I/O | `tui-terminal` | 2,487 | TM |
| Keys, keybindings, fuzzy | `tui-keys`, `code-tui-keybindings`, `tui-fuzzy` | 3,086 | KY |
| Highlight, images | `tui-highlight`, `tui-images` | 3,222 | HI |
| Util, theme | `tui-util`, `code-tui-theme` | 4,583 | UT |

## 1. Bugs found by the reviews (fix first)

These change behaviour, so each one needs a failing test before the fix.

| ID | Where | Bug | Vis | Risk | Size |
|---|---|---|---|---|---|
| TM1 | `tui-terminal/lib.rs:set_progress`, `stop` | The progress keepalive thread sleeps 1 s and is then joined, so turning progress off or quitting can stall the UI for up to 1 s. Wake it with a Condvar instead. | yes | L | S |
| TM2 | `stdin_buffer.rs:process_inner` (~295) | An incomplete escape before a paste marker is discarded, so an Esc pressed just before a paste is lost. | yes | L | XS |
| TM3 | `stdin_hub.rs:Feed::chunk` (~61) | Each read is decoded with `from_utf8_lossy` on its own, so a multi-byte character split across reads becomes U+FFFD. Keep the incomplete tail between reads. | yes | L | S |
| TM4 | `stdin_buffer.rs:is_complete_sequence` | ESC P / ESC _ / ESC ] (Alt+Shift+P, Alt+_, Alt+]) wait for a terminator with no cap, so keys typed right after them are swallowed. Treat them as complete at the flush deadline. | yes | M | S |
| KY1 | `tui-keys/state.rs:set_kitty_protocol_active` | Never called in production; the terminal crate keeps its own flag. On Kitty terminals, a custom shift+enter can resolve to alt+enter or enter. | yes | M | S |
| UT1 | `tui-util/ansi.rs:extract_ansi_code` (~19) | The CSI scan stops only at m/G/K/H/J, so a cursor-move code swallows the text after it and wrap/truncate miscount the width. End at any final byte 0x40–0x7E. | yes | M | XS |
| RN1 | `tui-render/tui.rs:paint` (~1835) | The over-width line check runs only on the diff path. `full_render` and the scroll view skip it. Do the check once, in `emit_line`. | yes | L | XS |
| MS1 | `assistant_message.rs:refresh` (~291) | `refresh` passes `streaming=false`, so a resize or thinking toggle mid-stream collapses the segmented blocks until the next delta. Store the flag. | yes | L | S |
| HI1 | `tui-highlight/engine.rs:process_sub_language` (~966) | An empty `subLanguage` array gives plain text instead of auto-detection. | yes | M | XS |
| KY2 | `code-tui-keybindings/hints.rs:format_keys` | It joins with "/" and then splits on "/", so a binding that includes the "/" key shows as `//n`. | yes | L | XS |
| TB1 | `bash_execution.rs:update_display`; `read.rs:135`; `bash.rs:134` | Shows "(exit undefined)" and "undefined" (a JS leftover). | yes | L | XS |

## 2. Inventory by component

### IM: `interactive_mode.rs` (6,624 LOC, 85 fields, 172 fns)

| ID | Idea | Vis | Risk | Size |
|---|---|---|---|---|
| IM1 | Split into `mode/{mod,events,transcript,prompt_queue,session_ops,commands,dialogs,autocomplete,clipboard,tree,models,login,settings,subagents,footer_session}.rs`. Mechanical: several `impl Mode` blocks. (= T1.1) | no | L | M |
| IM2 | A `Transcript` struct owns the chat `Container` and the ~20 parallel fields that `reset_transcript_view` clears. Reset becomes a replace. (= T1.2, needed for attach) | no | M | M |
| IM3 | One `ActivePicker` enum in place of ~14 `Option<…>` picker slots, with one `close`. | no | M | M |
| IM4 | Replace the 10 `poll_*` `try_recv` calls per loop pass with `AppEvent::PickerDone(kind, value)`. | no | M | S |
| IM5 | Move the 9 UI-thread `block_on` session ops (`/mode`, new, tree, fork, `/cd`, reload, import, resume) to the runtime as `AppEvent::SessionOp`, behind a loader. This breaks a concurrency.md §4 rule today. | yes (loader) | H | M |
| IM6 | One `dispatch_subagent` helper for `handle_subagent_command` and `retry_newest_subagent`. | no | L | XS |
| IM7 | `on_editor_change` clones the full text on every keystroke only to check for a leading `!`. Signal only when that changes. | no | L | XS |
| IM8 | Put the `ThinkingBackward` extra in the `EDITOR_ACTIONS` table. Do one keybindings lookup per key, not two. | no | L | XS |
| IM9 | `group_digits` copy (~6524) → tui-util (UT4). | no | L | XS |

### AP: app chrome and misc (`code-tui-app/`)

| ID | Where | Idea | Vis | Risk | Size |
|---|---|---|---|---|---|
| AP1 | `resource_display.rs` canvases | Delete the canvas path: never populated, and it names commands that don't exist (~45 LOC). | no | L | S |
| AP2 | `resource_display.rs` | Delete `RAIL` (empty string, 4 uses) and the always-true `sort` parameter. Merge `display_source_info` and `scope_group` into one scope function. Check whether the Plugins section is reachable. | no | L | S |
| AP3 | `tips.rs` | Stop building the 6 unshipped tips only to filter them out. | no | L | XS |
| AP4 | `notification_panel.rs` | Delete `dismiss` and `pending` (test-only). | no | L | XS |
| AP5 | `chrome_layout.rs` | Delete `layout()` (no caller). Simplify the applied/Hidden bookkeeping in `apply`. | no | L | XS |
| AP6 | `expandable_text.rs` | Delete `is_expanded`. Fold `refresh` into `set_expanded`. | no | L | XS |
| AP7 | `progress_bar.rs`, `embsearch_progress.rs`, `scroll_view.rs` | One shared `js_round`. Drop the always-None `cells` parameter. | no | L | XS |
| AP8 | `startup_progress.rs` + `embsearch_progress.rs` | Replace the generic keyed store and listener bus (one producer, one subscriber, ~220 LOC) with a single-slot index status. | no | M | M |
| AP9 | `scroll_view.rs` | One helper for the six scroll keys repeated in live and pinned modes. One helper for the status-row layout. | no | L | S |
| AP10 | `perf.rs` | `derive(Default)`, drop `Ring::new`. Cosmetic. | no | L | XS |
| AP11 | `footer.rs:subagent_counts` | One pass over the store, not two clones. Delete `active_subagent_count`. (= T1.4) | no | L | XS |
| AP12 | `hotkeys.rs` | A table of (label, binding, kind) rows in place of 68 `let` lines (~60 LOC). Drop the team-focus hint (TP1). | yes (hint) | L | S |
| AP13 | all of `code-tui-app` | Remove TS names from doc comments (~40 sites, e.g. `tips.ts`, `resolveChrome`, `formatStatus`). | no | L | XS |

### MS: transcript messages

| ID | Where | Idea | Vis | Risk | Size |
|---|---|---|---|---|---|
| MS2 | `custom_message.rs` | One `summary_sheet(label, headline, body)` helper for the three copies (~90 LOC). Pick one spacing rule. | yes (spacing) | L | S |
| MS3 | `assistant_message.rs` | Stop deep-cloning the message on every streaming delta (use `Rc` or a move). | no | L | S |
| MS4 | `assistant_message.rs` | Typed `BlockKey` enum for the cache, not `"{i}:text"` strings. Evict stale keys. | no | L | S |
| MS5 | `assistant_message.rs:38-93` | Replace ~56 lines of hand-emulated regex (and a `"\u{0}"` sentinel) with `JsRegex` statics or an `Option`. | no | M | M |
| MS6 | `assistant_message.rs` | A `status_line` helper for abort/error. The "Request was aborted" literal moves to its producer. | no | L | XS |
| MS7 | widgets | A `themed_style(key)` helper for the repeated `DefaultTextStyle { color: Box::new(…) }`. Delete the unused `ThemeFactory`. | no | L | XS |
| MS8 | `render_utils.rs` | Delete `js_line_count` (no callers). Make `set_expanded` a trait default. | no | L | XS |

### TB: tool blocks

| ID | Where | Idea | Vis | Risk | Size |
|---|---|---|---|---|---|
| TB2 | `tools/*`, `tool_execution.rs` | One `peek_block(lines, expanded, style)` in place of 7 copies (~150 LOC). | no | L | S |
| TB3 | `tools/subagent.rs`, `edit.rs` | AgentOutput and Edit respect the peek dial. | yes | L | S |
| TB4 | `subagent.rs:format_task_call`, `tool_signal.rs:SUBJECT_KEYS` | Show `Agent explore · <description>`. Rank `description` above `prompt`. | yes | L | XS |
| TB5 | several | Dead code: `LEGACY_TOOL_OUTPUT_VIEWS`, unused `_cwd`, `let _ = js_trim`, MultiEdit/NotebookEdit/SuggestPluginInstall names, the stale `Task` comment. Move `js_number` to `js_json.rs`. Bash `now_ms` duplicates the task store's. | no | L | XS |
| TB6 | `read.rs`, `write.rs`, `edit.rs` | Highlight only the visible prefix. Reuse the write cache. Drop the serde round-trip per render. Memoize the diff. | no | M | S |
| TB7 | `bash.rs`, `bash_execution.rs`, `visual_truncate.rs` | Slice to the last N lines before styling and wrapping. Streaming output is O(N²) today. | no | L | S |
| TB8 | `render_utils::get_text_output` callers | Compute the result text once in `update_result`. Hold `ToolResult` (with image base64) in an `Rc`. | no | L | S |
| TB9 | `tool_execution.rs` | Use `RenderShell` directly, one ctx builder, per-slot dirty flags, typed per-tool state. | no | M | M |
| TB10 | `tool_chain_summary.rs:chain_phrase` | One pass over the families, not two. | no | L | XS |

### TP: task panel and session chip

| ID | Idea | Vis | Risk | Size |
|---|---|---|---|---|
| TP1 | Delete the teams/roles lens, roster and focus code (~300 LOC, no production producer). (decision 9) | yes (hint) | L | S |
| TP2 | One row model per store `version()`, with a stable row key. No per-frame clones; `available_views` and the per-tab filter run once. `ticking` comes from the snapshot, not from render. (= T1.3) | no | L | S |
| TP3 | Rename the `Flat` lens to `Plan` and `is_main_task` to `is_plan_item`. | no | L | XS |
| TP4 | Give `Task` a `kind` (Plan, Subagent, Shell, Schedule, Mcp), not "source == None". (= T1.3) | no | L | S |

### PK: pickers and dialogs

| ID | Idea | Vis | Risk | Size |
|---|---|---|---|---|
| PK1 | `visible_window(sel, len, max)` and `step(sel, len, delta, wrap)` helpers in place of 8 copies (~60 LOC). | no | L | S |
| PK2 | One wrap-vs-clamp policy (recommended: wrap) for `/resume`, `/login`, `/logout`. | yes | L | XS |
| PK3 | Extend `SelectList` with a row closure and an "(n/len)" footer. The model, oauth and scoped-model lists use it. | no | M | M |
| PK4 | A shared `Filtered<T>` for 5 copies of query filtering (~80 LOC). | no | L | M |
| PK5 | A `PickerView` trait and one generic wrapper for the `Rc<RefCell<State>>` + events shell (~15 LOC per picker). | no | L | M |
| PK6 | One tree-prefix helper for `/resume` and `/tree`. | no | L | S |
| PK7 | `extension_selector.rs` hard-codes `j`/`k`/`\n`. Route through keybindings and keep j/k as extras. | yes (remaps) | L | XS |
| PK8 | Duplicates: `js_trim` ×2 → UT2. `home_dir` ×4 and `shorten_path` → UT5. Fix the `resume_picker` doc comment. | no | L | XS |

### ED: editor and input

| ID | Idea | Vis | Risk | Size |
|---|---|---|---|---|
| ED1 | Cache the editor layout by (text generation, width). `segment(after)` takes only the first grapheme. The paste-id set becomes a field. | no | L | S |
| ED2 | Delete dead editor APIs: `get_padding_x`, `get_autocomplete_max_visible`, `is_focused`, `text.rs:with_bg_fn`. `get_lines` and `get_border` become `cfg(test)`. `input.rs:handle_input_owned` (~40 LOC). | no | L | XS |
| ED3 | Editor uses `frame.rs` sizing helpers instead of copies (`is_box`, `inner_width`, padding clamp). | no | M | S |
| ED4 | A shared `BracketedPaste` helper and undo-coalescing rule in `tui-editing` for `editor.rs` and `input.rs`. One `delete_word_backwards`. | no | M | S |
| ED5 | Editor cursor in byte offsets, not UTF-16 (~85 call sites). Emoji and paste-marker tests first. | no | H | L |
| ED6 | `Editor::new`: move `theme.border_color` in directly. | no | L | XS |

### MC: markdown, autocomplete, small components

| ID | Where | Idea | Vis | Risk | Size |
|---|---|---|---|---|---|
| MC1 | `markdown/mod.rs:render` | A version counter in place of 3 copies of the source text and a full string compare per frame. Cached lines in `Rc<[String]>`, not cloned per frame. | no | L–M | S |
| MC2 | `markdown/render.rs` | Drop the never-set bold, strikethrough and underline style flags. | no | L | XS |
| MC3 | `markdown/lexer.rs` | Delete `lex_inline` (no callers). Move the JSON dumpers (~163 LOC) to test support. | no | L | S |
| MC4 | `cancellable_loader.rs` | Delete it (147 LOC, no callers; its `AbortSignal` name clashes with hoocode-ai's). | no | L | XS |
| MC5 | `loader.rs` | Delete the unread `text` field. | no | L | XS |
| MC6 | `select_list.rs` | Delete the `truncate_primary` hook (never `Some`). | no | L | XS |
| MC7 | `autocomplete/mod.rs:apply_completion` | One helper for the four identical tails (~30 LOC). | no | L | S |
| MC8 | `autocomplete/mod.rs` | One item builder for file and fuzzy-file suggestions. Drop the ignored `is_directory` option. `CmdView` becomes `AutocompleteItem`. | no | L | S |
| MC9 | `autocomplete/file_search.rs` | Merge `ready` into `last` (two clones per walk). `forget` must also clear `wanted`. | no | M | S |
| MC10 | `autocomplete/mod.rs:score_entry` | Rank `@file` with `tui-fuzzy`, like slash commands. | yes (order) | M | M |
| MC11 | `settings_list.rs` | `#[derive(Clone)]` in place of the local `impl Clone`. | no | L | XS |

### RN: renderer (`tui-render`)

| ID | Idea | Vis | Risk | Size |
|---|---|---|---|---|
| RN2 | Delete the overlay subsystem (~590 LOC, no production callers). Attach uses a transcript swap (decision 3), so nothing needs it. It also fixes a latent z-order bug. | no | L | S |
| RN3 | Delete zero-caller APIs (`get_show_hardware_cursor`, `get_clear_on_shrink`, `clear_children`, `on_debug`, `wants_key_release`) and the write-only `cursor_row`. | no | L | XS |
| RN4 | One `move_rows(delta)` helper for ~13 hand-built cursor-move escapes. | no | L | XS |
| RN5 | One copy of the frame per paint, not three whole-frame clones. | no | L | S |
| RN6 | `Option` in place of the -1/0 sentinels for size, search index and measured-at. | no | L | S |
| RN7 | Slice directly at the 23 `text_slice` calls whose offsets come from `str::find`. | no | M | S |
| RN8 | `Component::render_into(&mut Vec<String>)` default, so `Container` appends into one buffer. | no | L | M |
| RN9 | Split `paint` (315 lines, 11 returns) into a decision step plus `write_diff`, `write_tail_clear` and `write_flex_window`. | no | M | M |
| RN10 | A per-child line cache keyed by a revision counter, with a debug check against an uncached render. | no | H | L |
| RN11 | `usize`/`u16` for rows and columns (75 `as i64` casts). | no | M | L |

### TM: terminal I/O (`tui-terminal`)

| ID | Idea | Vis | Risk | Size |
|---|---|---|---|---|
| TM5 | One helper for the paste-end block (2 copies). Search only the new tail, since chunked paste is O(N²) today. | no | L | XS |
| TM6 | One `terminated_by` for the OSC/DCS/APC checks. Drop the duplicate arms and the unreachable branch. | no | L | XS |
| TM7 | `mouse.rs`: one parse returning `(event, len)`. Delete `is_mouse_sequence`. | no | L | S |
| TM8 | `output.rs`: coalesce queued writes and flush once per batch. | no | L | S |
| TM9 | One clock for the ESC deadline (move it into `Feed`). Delete the test-only `destroy` and `buffer_contents`. | no | L | S |
| TM10 | `stdin_buffer` on `&str` byte offsets, not `Vec<char>` (~3 allocations per byte today). | no | M | M |
| TM11 | `raw_write` without a `Writer` per call. Drop the unread `last_cols`/`last_rows` and the `_now` parameter. | no | L | XS |
| TM12 | Fold the 150 ms Kitty fallback thread into the reader's poll deadline (open in concurrency.md). | no | M | M |

### KY: keys, keybindings, fuzzy, editing

| ID | Idea | Vis | Risk | Size |
|---|---|---|---|---|
| KY3 | Pre-parse each binding once in `rebuild()` (a `KeySpec`). Today `matches` lowercases and splits per call, and there are ~36 calls per key in the editor. | no | L | S |
| KY4 | Fast-path out of the 5 Kitty regexes unless the input starts with `\x1b[`. | no | L | XS |
| KY5 | One table drives both `legacy_key` and `legacy_sequence_key_id`. Fold the four modifier ladders. Drop the dead `kitty_active` parameter and the 9-argument `matches_left_right`. | no | M | M |
| KY6 | `decode_kitty_printable` reuses `parse_kitty_sequence`. Delete `is_key_repeat` and `event_type`. | no | L | XS |
| KY7 | `format_parsed_key`: one `match` in place of a 25-branch ladder. | no | L | XS |
| KY8 | Delete `get_conflicts` (computed, never read). | no | L | XS |
| KY9 | Delete `migrate_keybindings_config_file` and `get_effective_config` (test-only). Cache `keybindings()` in a `OnceLock`. Check first that hoocode-ts doesn't rely on the rewrite of the shared file. | no | M | XS |
| KY10 | `tui-fuzzy`: lowercase the query once per filter and each item once. Sort with `total_cmp`. | no | L | S |
| KY11 | Merge the duplicate fuzzy test modules and the `_ts` case-for-case suites into one set of named tests. | no | L | S |

### HI: highlight and images

| ID | Idea | Vis | Risk | Size |
|---|---|---|---|---|
| HI2 | Delete `list_languages`, `highlight_auto_tree` and `Highlighted` (no callers). Make `engine` and `value` private. Drop `once_cell` and the duplicate dev-dependency. Drop the always-true `ignore_illegals`. | no | L | XS |
| HI3 | Sixel: there is no production rasterizer, so it always falls back to text. Delete it (235 LOC) or add a decoder (user call). | yes if built | M | M |
| HI4 | `detect_terminal_capabilities`: a table in place of 7 copied struct literals. Delete the unread `true_color`. | no | L | XS |
| HI5 | Kitty encode via `chunks()`. Delete `delete_all_kitty_images` and the unused iTerm2 options. | no | L | XS |
| HI6 | Memo the highlight output by (lang, code). Merge the two `highlight_code` copies in the theme. | no | L | S |
| HI7 | Cache an image's encoded sequence. Read dimensions from the header prefix. | no | L | S |
| HI8 | Don't parse the 1.1 MB grammar JSON into a `Value` graph on the first highlight. Generate it at build time, or load per language. | yes (stall) | M | L |

### UT: util and theme

| ID | Idea | Vis | Risk | Size |
|---|---|---|---|---|
| UT2 | One `js_trim` (tui-util) for the 4 TUI copies. The tool-search and tools-fs versions differ and stay. | no | L | XS |
| UT3 | One `ansi_runs()` iterator for the strip loop and 4 scan loops. `TAB_WIDTH` and `expand_tabs` live in tui-util (replaces `replace_tabs`). | no | L–M | S |
| UT4 | `group_digits` → tui-util. One `SEGMENT_SEP` (brand), re-exported. | no | L | XS |
| UT5 | One `home_dir` and one `shorten_path`, in `hoocode-code-paths` (7 `home_dir` copies with 4 behaviours). | yes (HOME unset) | M | S |
| UT6 | `chalk::set_enabled` has no caller. Wire it to colour detection or delete it. | maybe | L | XS |
| UT7 | Delete zero-caller theme and util APIs: `get_color_mode`, `ThemeJson::color`/`to_value`, `has_active_codes`, identity `regex_escape`. Derive `REQUIRED_COLOR_TOKENS`. | no | L | XS |
| UT8 | The background opener is passed in, not recovered per line. Skip the tab copy in `visible_width` when there is nothing to replace. | no | L | S |
| UT9 | Theme lookup: an enum-indexed ANSI table and `RwLock`/`ArcSwap`, not a `Mutex` + `Arc` clone + `format!` per span. | no | M | M |
| UT10 | `JsRegex::search` returns byte offsets as `Option<usize>`, not UTF-16 with -1. | yes (minor) | M | S |
| UT11 | One `text_slice` in a leaf crate, not two copies kept in step by a comment. | no | L | S |

## 3. Batches (order of work)

Each batch is one PR. Rows in a batch are spread over parallel Haiku subagents only where
files don't overlap. Every batch: L1 + `goldens.py check`. `Vis: yes` rows add a review bundle.

| Batch | Rows | Why this order |
|---|---|---|
| S0: bugs | TM1–TM4, KY1, UT1, RN1, MS1, HI1, KY2, TB1 | Real defects. Each gets a failing test first. |
| S1: dead code | AP1–AP6, AP10, AP13, MS8, TB5, TP1, ED2, MC2–MC6, RN2, RN3, KY6, KY8, KY9, HI2, HI4, HI5, UT6, UT7 | Cheapest and lowest risk. Shrinks what later batches read. |
| S2: one copy of each helper | UT2–UT5, UT11, IM9, PK1, PK6, PK8, AP7, AP9, MS2, MS6, MS7, TB2, MC7, MC8, MC11, RN4, TM5–TM7, KY7 | Dedupe before restructuring. |
| S3: structure (feature prerequisites) | IM1, IM2, TP2, TP4, TP3, AP11, IM3, IM4, IM6, PK3–PK5, ED3, ED4, TB9, RN6, RN9 | IM1 → IM2 serial; the rest in parallel by crate. Includes T1 of the TUI plan. |
| S4: per-keystroke and per-frame cost | IM7, IM8, KY3, KY4, KY10, ED1, MC1, MC9, MS3, MS4, TB6–TB8, TB10, RN5, RN8, TM8, TM9, TM11, UT8, UT9, HI6, HI7 | Measure with `/perf` and the load scenario before and after. |
| S5: visible changes | TB3, TB4, PK2, PK7, AP12, MC10 | Each one goes through the visual review. |
| S6: larger and riskier | IM5, MS5, RN7, RN10, RN11, TM10, TM12, ED5, KY5, KY11, UT10, AP8, HI3, HI8 | One at a time. Some need a user call (HI3). |

After S3 the feature phases (T2–T5) can start. S4–S6 can run alongside them where files don't
overlap.

## 4. Open question

1. **HI3 Sixel:** delete it, or add a decoder so Windows Terminal shows images?
   Recommended: delete. Kitty and iTerm2 cover most users, and a decoder is a new dependency.

# Backlog and handover

Status (2026-11-11): handover from the TUI consistency and stabilization session. This page says what was done, what was checked, and what is left. The plan stays in [design/roadmap.md](design/roadmap.md); the rules stay in [design/tui.md](design/tui.md). When this page and those disagree, they win and this page is stale.

How to use it: read "Start here", then do "Next session". "Left to do" holds the detail for its rows. Update the tables in the same commit as the work, and delete rows that are finished and recorded in the design docs.

## Start here

| Need | Where |
|---|---|
| TUI rules (keys, colours, panes, transcript) | [design/tui.md](design/tui.md) |
| Where each screen part lives in code | [maps/ui.md](maps/ui.md) |
| What each colour means, for agents | `crates/hoocode-code-tui-theme/color-semantics.json` (checked by `tests/it/color_semantics.rs`) |
| Keys for users | [user/keybindings.md](user/keybindings.md) |
| Done bar | `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo nextest run -p <touched crates>` |

Environment notes for the next session:

- The cloud container's pinned toolchain (1.94.1) was installed without `cargo` and `clippy`. `rustup toolchain uninstall 1.94.1 && rustup toolchain install 1.94.1 --profile minimal -c rustfmt -c clippy` fixes it. The SessionStart warm-up fails until then (`target/warmup.log`).
- Do not share one `CARGO_TARGET_DIR` between git worktrees that build at the same time. The workspace crates get the same artifact names, so one worktree's tests can run against the other's code. Use one worktree at a time, or a target directory each.

## Next session (the last one): do all of this

The user said on 2026-11-11 that the next session is the last TUI session and should close every item on this page. Work top to bottom. Each row is one commit by a Haiku subagent, with tests. Where a row needs a decision, the answer in the row is the default: take it unless the user says otherwise at the start.

| Order | ID | Do | Size |
|---|---|---|---|
| 1 | Screen check | Run `python3 scripts/tui/goldens.py check all`. Read the report. Accept `scroll-view`, `subagent-task` and any other scenario whose only change is the new pinned hint, the task strip or a joined tip. Then look at the real screen once in `dark` and `light` (tmux capture to a file is enough) and fix what reads badly. | S |
| 2 | UI-6 | Component goldens for every dialog and notice: permission, question, quit, models, settings, sessions, theme, tree, fork, log in, scoped models, keys, slash, files, task panel variants; notices info, info with value, usage, warning; a notice joined to the task pane. One light and one mono each. | M |
| 3 | UI-30 | Remove the write-only override map in `TranscriptFocus` (`expanded_override`, `clear_expanded`). | S |
| 4 | UI-24 | Add `ctrl+x o`: open all tool rows, or close all when any is open. Frozen rows have released their output: either skip them and say so in the status slot, or reload from the session. Record the decision in `tui.md`. | S |
| 5 | UI-25 | Let the keyboard focus walk every block that toggles (thinking, summaries, `!` blocks, skill, background notice), not only tool calls. Wire thinking's `set_expanded`. A click on these already toggles; make both paths share one flag as tool rows do. | M |
| 6 | UI-28 | Move the hard-coded keys into the keymap: `e` and `1`–`3` (permission card), `v` and `r` (session picker), Space (settings). Hints follow from the keymap. | S |
| 7 | UI-27 | Settings search: Space types into a non-empty query; it toggles only when the query is empty. | S |
| 8 | UI-26 | `/hotkeys`: group by context (the five rows of "Where a key acts" in `tui.md`), show every meaning of a bare key, and show non-letter keys. | S |
| 9 | UI-29 | Go through the warning-colour list below. Move each to ink bold or dim, or add it as a named exception in `color-semantics.json` with a test. After this the JSON and the code agree. | S |
| 10 | Schema drift | `schema.rs` `COLOR_PROPERTIES` lists `dim` twice and leaves out `mcp`; the `toolBandBg` text says "peek view"; decide whether the uncalled `thinkingOff`…`thinkingXhigh` tokens stay. | S |
| 11 | AU-1 | User page for subscription logins: the three providers, the Gemini client variables, `ANTHROPIC_OAUTH_TOKEN`, the callback ports, and what to do when a port is taken. | S |
| 12 | AU-2, AU-5 | Live tests fail when run with `--ignored` and no token. Warn when callback port 1455 is taken. | S |
| 13 | AU-3, AU-4 | Mock tests: 401 and 403 on the three stream paths; refresh on expiry through `AuthStorage::get_api_key` with the real provider ids; an Anthropic stream test with an `sk-ant-oat` token. | M |
| 14 | AU-6, AU-7 | Decide and record in `agent.md`: how the pinned client versions are refreshed; whether the PKCE verifier stays as the OAuth `state`. Change code only if the provider's own client differs. | S |
| 15 | Roadmap TUI rows | Close what is left in `tui.md` Open with the roadmap's recommended answers: UI-16 (record "minus 2"), UI-18 (delete the replay fixture if no test reads it), UI-20 (rename the `*_ts*.rs` files), UI-15 (no exit summary: record it), UI-11 text. Leave UI-7, UI-14, UI-17 and UI-19 as written unless the user asks: they are larger or gated. UI-12 and UI-13 need a person at a real terminal. | S |
| 16 | Live logins | Only if the session has the three accounts and a browser: the four steps under "Subscription logins". Otherwise say plainly that they were not run. | — |
| 17 | Close out | Update `tui.md`, `maps/ui.md`, `roadmap.md` and the user pages in the same commits. Then cut this page down to what is truly left, or delete it and remove its line from `CLAUDE.md` if nothing is. | S |

Prompt for that session (paste as the first message):

```text
This is the last TUI session. Read CLAUDE.md, then docs/backlog.md, then
docs/design/tui.md. Do every row of the table "Next session (the last one)"
in docs/backlog.md, top to bottom, and finish all of them.

How to work:
- You orchestrate. Haiku subagents review, implement and test, one row per
  commit. Use a stronger model only for a row a Haiku agent fails twice.
- Run agents one at a time in the main checkout, or give each worktree its own
  target directory. Never share one CARGO_TARGET_DIR between worktrees that
  build at the same time.
- First fix the toolchain if cargo is missing:
  rustup toolchain uninstall 1.94.1 && rustup toolchain install 1.94.1 --profile minimal -c rustfmt -c clippy
- Keep the design. No redesign, no new colours, no new conventions. Colours
  follow crates/hoocode-code-tui-theme/color-semantics.json. Keys follow the
  rule in tui.md: at the prompt a bare key always types.
- Where a row names a default answer, take it and record it as a dated row in
  the Decisions table. Ask me only if a row cannot be done as written.
- Done bar for every commit: cargo fmt --all, cargo clippy --workspace
  --all-targets -- -D warnings, cargo nextest run -p <touched crates>. Before
  the PR: the full workspace suite, the CI guards and
  python3 scripts/tui/goldens.py check all.
- Do not claim a subscription login works unless you ran it with a real
  account. If you cannot, say so and leave the steps in place.
- Finish by updating the design docs, maps and user pages, cutting
  docs/backlog.md down to what is truly left (or deleting it), adding a
  CHANGELOG entry, and opening one PR labelled rust:patch. Report what is
  done, what is not, and why.
```

## Done in this session (2026-11-11)

| Item | What changed | Where |
|---|---|---|
| Bare keys type at the prompt | In the pinned view a typed character always goes to the prompt. Before, `j k q [ ] { } /` were taken as pager keys over an empty prompt, so "jump" became "ump" and `/model` became a transcript search. Transcript letters now act only while a tool block is focused (`ctrl+x u`). | `crates/hoocode-code-tui-app/src/scroll_view.rs` (`handle_input`), tests `tests/it/scroll_view_keys.rs`, `transcript_focus_keys.rs` |
| Click and Enter agree | Both read and write the block's own `expanded` flag. A click moves the keyboard focus only when a block is already focused. `q` and any unbound letter clear a focus. | `mode/transcript/focus.rs` (`set_block_expanded`, `toggle_focused`, `move_focus_on_click`), `mode/mod.rs` (`drain_body_clicks`) |
| Pinned hints | `ctrl+x u focus · esc live`; with a block focused `enter open · y copy · { } move · esc clear`. | `scroll_view.rs` (`scroll_slot`, `focused_hints`), golden `scroll_rewind_100x1.txt` |
| Colour semantics for agents | One JSON file: rules, roles, states, surfaces and a meaning for all 86 tokens. A test fails when the schema and the file drift. | `crates/hoocode-code-tui-theme/color-semantics.json`, `tests/it/color_semantics.rs` |
| Task-tab hints | The unfocused strip ends with `ctrl+x l next tab · ctrl+x g hide`. Hints drop before tab labels when narrow. The focused foot leaves out `←→ tab` with one tab. | `crates/hoocode-code-tui-widgets/src/task_panel.rs` (`tab_strip`, `pick_hints`), goldens `task_panel_*` |
| Tips join the task pane | With the task panel drawn, tips and longer notices are painted on the pane colour at indent 3 with no blank row. Without the panel, and in monochrome, they render as before. | `crates/hoocode-code-tui-app/src/notification_panel.rs` (`set_joined_to_pane`, `render_joined`), set each pass in `mode/mod.rs` |
| Typed keys leave the task panel | A printable key the focused panel does not use returns the keyboard to the prompt and types there. | `task_panel.rs` (`PanelEvent::Type`), `mode/task_panel.rs` (`type_into_prompt`) |
| New tip | "Tasks have tabs". | `crates/hoocode-code-tui-app/src/tips.rs` |
| Running is accent | Subagent rows and the working status tone use accent, not yellow. The MCP "auth needed" glyph follows; its words stay yellow (UI-29). | `code-tui-widgets/src/text_block.rs`, `tools/subagent.rs`, `code-tui-app/src/mcp_listing.rs` |
| Orphan goldens (UI-2) | `tests/golden/tui/color-chrome/` deleted. | — |
| Plain-words guard in CI (UI-3, EN-8) | Added to the `guards` job. | `.github/workflows/ci.yml` |
| Dangling doc pointer | The Gemini missing-client error no longer names `docs/providers.md`. | `crates/hoocode-ai-oauth-google/src/lib.rs` |
| User pages | Pinned-transcript section rewritten; the stale "View dial (phasing out)" bullet removed. | `docs/user/keybindings.md`, `docs/user/configuration.md` |
| Design docs | `tui.md` folded to as-built; maps and roadmap updated. | `docs/design/tui.md`, `docs/maps/ui.md`, `docs/design/roadmap.md` |

Open items found already done (closed in `tui.md`): UI-5 (a printable leader is refused), UI-8 (contrast test covers five themes), UI-10 (session delete has a confirm), UI-11 (`ctrl+x y` and `ctrl+x z` are bound).

## Subscription logins: what was checked

No real account, token or browser exists in the cloud container, so no live login and no live model call was made. Nothing below is an end-to-end pass.

Checked here (2026-11-11): `cargo nextest run` on `hoocode-ai-oauth`, `-oauth-anthropic`, `-oauth-openai-codex`, `-oauth-google`, `hoocode-ai-provider-anthropic`, `-openai-codex`, `-google-gemini-cli` and `hoocode-code-auth`: 163 passed, 8 skipped (the live tests).

| Stage | Claude (`anthropic`) | ChatGPT (`openai-codex`) | Gemini (`google-gemini-cli`) |
|---|---|---|---|
| Authorize URL and PKCE | Mock test | Mock test (challenge value not asserted) | Mock test (challenge value not asserted) |
| Code exchange | Mock test | Mock test | Mock test |
| Token refresh | Mock test | Mock test | Mock test |
| Saved to `auth.json` and read back | Generic test only | Generic test only | Generic test only |
| Refresh on expiry through `AuthStorage::get_api_key` | Generic test only (fake provider id) | Generic test only | Generic test only |
| Request headers and body for a subscription token | Unit test | Mock server test | Mock server test |
| Streamed reply decoded | Mock server, API-key token only | Mock server, with the subscription token | Mock server, with the subscription token |
| 401 and 403 handling | Not covered | Refresh only | Not covered |
| Live login | Not run | Not run | Not run |
| Live model call | Not run | Not run | Not run |

Blockers, exactly:

- `claude.ai/oauth/authorize` and `auth.openai.com/oauth/authorize` answer 403 with a Cloudflare challenge from this container. The API and token hosts (`api.anthropic.com`, `platform.claude.com`, `chatgpt.com`, `oauth2.googleapis.com`, `cloudcode-pa.googleapis.com`) are reachable.
- Login is interactive only. There is no `hoocode login` command. `hoocode-code-cli::auth::login` has no production caller.
- Gemini: hoocode ships no OAuth client. `/login` fails until `HOOCODE_GEMINI_CLI_CLIENT_ID` and `HOOCODE_GEMINI_CLI_CLIENT_SECRET` are set. No user page says so. Only `crates/hoocode-ai-oauth-google/README.md` does.
- The live tests return early when their variable is missing, so `--ignored` with no token reports a pass that tested nothing.

To finish the check on a machine with a browser and the three accounts:

1. Claude: `/login` → Anthropic. Send one prompt with a Claude model. Then `ANTHROPIC_OAUTH_TOKEN=<token> cargo test -p hoocode-ai-provider-anthropic --test it -- --ignored oauth_tool_names_round_trip`.
2. ChatGPT: `/login` → ChatGPT (port 1455 must be free; when it is taken the login silently falls back to paste). Send one prompt with a `gpt-5.x-codex` model. Then `OPENAI_CODEX_OAUTH_TOKEN=<token> cargo test -p hoocode-ai-provider-openai-codex -- --ignored live_handles_sse`.
3. Gemini: set the two client variables, `/login` → Google, send one prompt with a `gemini-*` model. Paid tiers also need `GOOGLE_CLOUD_PROJECT`.
4. For each: restart hoocode and send a prompt again (the saved credential is read back). After the token's expiry, send again (refresh).

Follow-ups worth a roadmap item (not started):

| ID | Item | Size |
|---|---|---|
| AU-1 | Document the Gemini client variables, `ANTHROPIC_OAUTH_TOKEN` and the callback ports (53692, 1455, 8085) on a user page | S |
| AU-2 | Make the live tests fail, not pass, when run with `--ignored` and no token | S |
| AU-3 | Mock tests for 401 and 403 on the three stream paths, and for refresh-on-expiry with the real provider ids | M |
| AU-4 | A mock-server stream test for Anthropic with an `sk-ant-oat` token (headers, system prefix and tool renames on the wire) | S |
| AU-5 | Warn when the ChatGPT callback port 1455 is taken (today the bind error is dropped) | S |
| AU-6 | `CLAUDE_CODE_VERSION` (`2.1.280`) and the Antigravity version are constants that go stale; decide how they are refreshed | S |
| AU-7 | Anthropic and Google use the PKCE verifier as the OAuth `state`. Check this against each provider's own client before changing it | S |

## Left to do (TUI)

IDs are the ones in `tui.md` Open. Order is the recommendation.

| ID | Item | Size | Notes |
|---|---|---|---|
| UI-6 | Component goldens for every dialog and notice | M | Still the top TUI item. Dialogs and notices have no pinned text. |
| UI-24 | Expand-all and collapse-all key for tool rows | S | Needs a user decision. Recommended: `ctrl+x o` toggles all. Check frozen rows first (they release their output). |
| UI-25 | Keyboard focus walks tool blocks only | M | Thinking, summaries and `!` blocks open by click only. Thinking's `set_expanded` has no production caller. |
| UI-26 | `/hotkeys` shows one meaning per bare key and leaves out non-letter keys | S | `code-tui-widgets/src/controls.rs` (`layer_cap`, `layer_actions`). |
| UI-27 | Settings search: Space toggles the row before the query sees it | S | `code-tui-selectors/src/config_selector.rs`. |
| UI-28 | Hard-coded keys outside the keymap: `e`, `1`–`3`, `v`, `r`, Space | S | K-P5 gap. |
| UI-29 | Non-blocking uses of the warning colour | S | List below. |
| UI-30 | `TranscriptFocus` override map is write-only now | S | Remove `expanded_override` and `clear_expanded` (`scroll_view.rs`). |

Known behaviour kept on purpose:

- Permission card: `y` and `n` answer at once while the reply is empty (decision 2026-11-05). The first letter of a typed reason can therefore answer. Change only by a new decision.
- Frozen rows (older than the newest 50 blocks) keep the old palette after a theme change until they scroll out.
- At 100 columns the pinned hints push the cost and the session name off the status line. The model and context stay.

### UI-29: warning colour outside blocking dialogs

The rule is "yellow marks only the thing that blocks the user". These still use the `warning` token. Each needs a look: move to ink bold or dim, or record it as an allowed exception in `color-semantics.json`.

| Where | What |
|---|---|
| `code-tui-widgets/src/text_block.rs` (`DetailTone::Warn`) | Status words such as MCP `auth needed` |
| `code-tui-widgets/src/tools/read.rs` (3 sites), `tools/web.rs`, `tools/bash.rs` | Truncation notes |
| `code-tui-app/src/resource_display.rs` | Context warnings in the startup listing |
| `code-tui-widgets/src/task_panel.rs` | `press x again to cancel`, and the task note cue |
| `code-tui-widgets/src/bash_execution.rs` | `(cancelled)` |
| `code-tui-app/src/transient_lines.rs` | Retry notice |
| `code-tui-selectors/src/model_selector.rs` | `expired` |
| `code-tui-widgets/src/controls.rs` | Key-file problems in `/hotkeys` |
| `code-tui-widgets/src/tool_signal.rs` | Draws nothing in production since the dial went |

Also found by the colour pass, not changed: `mdHeading` is yellow in `dark` (accepted as M4, 2026-11-07); the tree picker paints the `assistant` role label green; the login and settings pickers paint `configured` and `[x]` green; `schema.rs` `COLOR_PROPERTIES` lists `dim` twice and leaves out `mcp`; the schema text for `toolBandBg` still says "peek view"; the `thinkingOff`…`thinkingXhigh` tokens have no caller.

## Tests run in this session

Run on the merged branch, toolchain 1.94.1, 2026-11-11:

| Check | Result |
|---|---|
| `cargo fmt --all --check` | Clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | Clean |
| `cargo nextest run --workspace --no-fail-fast` | 5,524 passed, 0 failed, 66 skipped |
| `scripts/ci/no_literal_keys.py`, `plain_words.sh`, `no_cortex.sh` | OK |
| `scripts/maps/packages.py --check` | OK after regenerating the line counts |

Not checked by eye: no one looked at the real screen in a terminal. The task strip, the joined tip and the pinned hints are pinned by component goldens (text and colours), not by a person. Look once in `dark` and `light` before a release.

Not run: the screen goldens (`python3 scripts/tui/goldens.py check all`). They are a pre-release check. The pinned hint and the task strip changed, so expect `scroll-view` and `subagent-task` to need `update` after a look at the report.

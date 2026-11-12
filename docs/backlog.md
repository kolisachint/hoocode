# Backlog and handover

Status (2026-11-12): handover from the TUI session that worked the UI-25 to UI-34 rows and closed the roadmap TUI rows UI-15, UI-16, UI-18 and UI-20. `docs/wip/` is empty (the folder is absent). This is not the last TUI session. The plan stays in [design/roadmap.md](design/roadmap.md); the rules stay in [design/tui.md](design/tui.md). When this page and those disagree, they win and this page is stale.

How to use it: read "Start here", then do "Next session" top to bottom. Delete rows as they land.

## Start here

| Need | Where |
|---|---|
| TUI rules (keys, colours, panes, transcript) | [design/tui.md](design/tui.md) |
| Where each screen part lives in code | [maps/ui.md](maps/ui.md) |
| What each colour means, for agents | `crates/hoocode-code-tui-theme/color-semantics.json` |
| Keys for users | [user/keybindings.md](user/keybindings.md) |
| Subscription logins for users | [user/login.md](user/login.md) |
| Unfinished work, as patches | `docs/wip/` (delete each file when its work lands) |
| Done bar | `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo nextest run -p <touched crates>` |

Environment notes:

- The cloud container's pinned toolchain (1.94.1) may come without `cargo`. Fix: `rustup toolchain uninstall 1.94.1 && rustup toolchain install 1.94.1 --profile minimal -c rustfmt -c clippy`.
- Disk is about 30 GB free. One target directory is about 5 GB warm and grows to 15 GB. Two worktrees with a target directory each fit; three do not. Prune with `scripts/dev/prune_target.sh --apply --if-over 8 --target <dir>`.
- Never share one `CARGO_TARGET_DIR` between worktrees that build at the same time.
- One test fails only in a checkout with a long path (`tool_execution_component::parity::a_collapsed_read_shows_only_its_call_and_the_toggle_opens_it_whole`: the path wraps at 120 columns). It passes in `/home/user/hoocode`.
- Remote branch deletion is blocked for agents in the cloud session. The user deletes branches (list below).

How to spend less in the next session (the user asked for this):

- Two usage limits were hit in the 2026-11-11 session. The cause was long agent briefs, three-commit agents, and agents that each re-read the design docs and ran the workspace clippy.
- Give one agent one row. Name the files. Let it run `cargo clippy -p <crate>` and `cargo nextest run -p <crate>` only. Run the workspace checks once, at the end, in the main session.
- Do not start a parallel lane for fewer than three rows.
- Start from the patch. It holds real work.

## Next session: in this order

| Order | ID | Do | Size |
|---|---|---|---|
| 1 | Live logins | Only with the accounts and a browser: the steps under "Subscription logins". | — |
| 2 | Non-tool blocks toggle | Check on the real binary in tmux, with mouse sequences, that thinking, notice and other non-tool blocks open and close. Only tool rows were checked (UI-25, UI-31). | S |
| 3 | Token `mcp` | No reader. The theme registry fills it, the schema lists it, and `color-semantics.json` marks it `legacy`, but nothing draws it. Decide: keep it or delete it. | S |
| 4 | Open TUI rows | UI-7, UI-12, UI-13, UI-14, UI-17 and UI-19 are still open in [design/tui.md](design/tui.md), each with its trigger or gate. | — |

## Done in this session (2026-11-12)

On branch `claude/great-maxwell-n95s34`. Not pushed, and no PR yet.

| Item | What changed |
|---|---|
| UI-25, UI-31 | One toggle for every block: a click, `enter` or `ctrl+x o` opens and closes it. Each block shows an open or closed indicator. The startup listing has a `Resources` header. (7a5905e) |
| UI-28 | The permission card, the interrupted-run row, settings and scoped models read their keys from the keymap. (a683043) |
| UI-27 | Space types into a non-empty settings search. (bd99349) |
| UI-26 | `/hotkeys` groups keys by where they act. (21a3f8a) |
| UI-32 | The selected row is bold in every pane, with no stray blank rows. (1484fe7) |
| UI-29 | Yellow only on what blocks the user. (1d105c1) |
| Schema drift | The theme schema lists every colour token once. `thinkingOff` to `thinkingXhigh` are removed; old themes still load. (613240f) |
| UI-34 | The golden-versus-design differences are worked through and the dialogs and notices follow the design. Decisions are dated rows in `tui.md`. (c17e401, 3024e3c) |
| UI-33 | A failed tool row names its reason for every tool. The screen goldens are refreshed. (37a1cca, 8c8cd38) |
| Roadmap rows | UI-15 (no exit summary), UI-16 (minus 2), UI-18 (the replay fixture stays: `crates/hoocode-code-main/tests/it/replay.rs` reads it), UI-20 (the 30 `*_ts*.rs` test files lost the `ts`). Recorded in `tui.md`. |

Known behaviour kept on purpose:

- Permission card: `y` and `n` answer at once while the reply is empty (decision 2026-11-05).
- Frozen rows keep the old palette after a theme change until they scroll out.
- At 100 columns the pinned hints push the cost and the session name off the status line.

## Subscription logins: what was checked

No real account, token or browser exists in the cloud container. No live login and no live model call was made in either session. Nothing here is an end-to-end pass.

| Stage | Claude (`anthropic`) | ChatGPT (`openai-codex`) | Gemini (`google-gemini-cli`) |
|---|---|---|---|
| Authorize URL, code exchange, token refresh | Mock test | Mock test | Mock test |
| Refresh on expiry through `AuthStorage::get_api_key`, real provider id | Mock test | Mock test | Mock test |
| Request headers and body for a subscription token, on the wire | Mock server | Mock server | Mock server |
| 401 and 403 | Mock server: fails at once with status and body | Mock server: one SSE attempt, `ChatGPT API error (401): …` | Mock server: one request, `Cloud Code Assist API error (401): …` |
| Callback port taken | Fails with the bind error | Warns, then paste | Fails with the bind error |
| Live login, live model call | Not run | Not run | Not run |

To finish on a machine with a browser and the accounts:

1. Claude: `/login` → Anthropic. Send one prompt with a Claude model. Then `ANTHROPIC_OAUTH_TOKEN=<token> cargo test -p hoocode-ai-provider-anthropic --test it -- --ignored oauth_tool_names_round_trip`.
2. ChatGPT: `/login` → ChatGPT (port 1455 free). Send one prompt with a `gpt-5.x-codex` model. Then `OPENAI_CODEX_OAUTH_TOKEN=<token> cargo test -p hoocode-ai-provider-openai-codex -- --ignored live_handles_sse`.
3. Gemini: set `HOOCODE_GEMINI_CLI_CLIENT_ID` and `HOOCODE_GEMINI_CLI_CLIENT_SECRET`, `/login` → Google, send one prompt with a `gemini-*` model. Paid tiers also need `GOOGLE_CLOUD_PROJECT`.
4. For each: restart hoocode and send a prompt again. After the token's expiry, send again.

Open auth notes: the pinned Claude Code version is `2.1.280` and the published one was `2.1.322` on 2026-11-11 (refresh rule in `agent.md`). `docs/user/login.md` is not in the site's sidebar until the site repo adds it. The ChatGPT retry loop still retries other 4xx errors such as 400.

## Branches the user can delete

Agents cannot delete remote branches in the cloud session. Checked against `main` on 2026-11-11.

- Merged, nothing unmerged: `claude/design-docs-unified`, `claude/ecstatic-archimedes-ej8zwn`, `claude/haiku-agents-concurrency-72r3pt`, `claude/themes-keybindings-design-nwqgis`, `claude/upbeat-pasteur-n4rk4m`, `tui-visual-overhaul`, `dependabot/cargo/rand-0.10.3`, and, after this PR merges, `claude/festive-gauss-q4m0lg` and `claude/great-maxwell-n95s34`.
- Superseded by this PR or by `main`: `agent/responses-parallel-args` (#171, fixed on `main` in 888eccac), `agent/shell-exit-chip` (#172, the single view replaced it), `fix/tui-goldens-version-mask` (#170, harness fix taken by hand), `bot/models-sync` (#167), `dependabot/cargo/cargo-minor-patch-1575fb4005` (#180), `dependabot/cargo/sha2-0.11.1`, `dependabot/cargo/jsonwebtoken-11.1.0`, `dependabot/cargo/jsonschema-0.60.2`.
- From before the history restart (no common base with `main`, last commit 2026-10-09 to 2026-10-29): `agent/muse-integrate`, `agent/ui-prompt-fixes`, `ci/models-sync-use-gh-token`, `claude/all-next-2026-10-14`, `claude/core-step1-reliability`, `claude/design-system-extraction-6t4r9r`, `claude/fix-agent-loop-registry-flake`, `claude/fix-bash-idle-tick-flake`, `claude/friendly-wozniak-ek34vd`, `claude/tui-next-2026-10-14`, `claude/website-who-code-updates-t75o9t`, `dependabot/cargo/cargo-minor-patch-5854e3a6c1`, `dependabot/cargo/crossterm-0.29.0`, `dependabot/cargo/fancy-regex-0.19.2`, `dependabot/github_actions/actions/setup-python-7`, `docs/readme-landscape-demo`, `docs/v2-trailer`, `feat/footer-toolcalls-cleanup`, `fix/app-server-scoped-models`, `fix/fuzz-build-target`, `fix/ui-row-reclaim`. Look once before deleting the two `docs/` branches: they hold README media.
- Keep: `main`, `dependabot/cargo/windows-sys-0.61.2` (#163, open).

## Prompt for the next session

```text
Continue the TUI work. Read CLAUDE.md, then docs/backlog.md. Do the table
"Next session: in this order", top to bottom, as far as the budget allows.
docs/wip/ is empty, so no row starts from a patch.

How to work:
- Spend little. One Haiku agent per row, a short brief that names the files,
  crate-level clippy and nextest only. Run the workspace checks once at the
  end yourself. Use a stronger model only for row 1 or after two failures.
- One checkout unless three or more independent rows are ready.
- Keep the design. Colours follow color-semantics.json. At the prompt a bare
  key always types.
- Record each decision as a dated row in tui.md. Delete each docs/wip patch
  and each backlog row when its work lands.
- Finish with CHANGELOG, one PR labelled rust:patch, merge, and watch CI and
  the release until green.
```

# Backlog and handover

Status (2026-11-11, evening): handover from the second TUI session of the day. It closed part of the list and was wound up early at the user's request. This is not the last TUI session. The plan stays in [design/roadmap.md](design/roadmap.md); the rules stay in [design/tui.md](design/tui.md). When this page and those disagree, they win and this page is stale.

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

- Two usage limits were hit in this session. The cause was long agent briefs, three-commit agents, and agents that each re-read the design docs and ran the workspace clippy.
- Give one agent one row. Name the files. Let it run `cargo clippy -p <crate>` and `cargo nextest run -p <crate>` only. Run the workspace checks once, at the end, in the main session.
- Do not start a parallel lane for fewer than three rows.
- Start from the patch. It holds real work.

## Next session: in this order

| Order | ID | Do | Size |
|---|---|---|---|
| 9 | UI-33 | Refresh the screen goldens. Notes below. | M |
| 10 | Roadmap TUI rows | UI-16 (record "minus 2"), UI-18 (delete the replay fixture if no test reads it), UI-20 (rename the `*_ts*.rs` files), UI-15 (no exit summary: record it). | S |
| 11 | Live logins | Only with the accounts and a browser: the steps under "Subscription logins". | — |

### UI-33: screen goldens

`python3 scripts/tui/goldens.py check all` on 2026-11-11: `subagent-task` passes, nine differ. None was accepted. The harness got two fixes (one style across the version mask; the random session name in the status line is masked).

| Scenario | Difference | Likely action |
|---|---|---|
| `chat-basic`, `session-resume` | Status line lost the view word; picker rows and hints changed | Accept after a read |
| `permission-prompt` | Option rows and hint changed | Accept in UI-33 (the hint change is done in UI-34) |
| `tool-bash`, `tool-edit` | Tool rows are the single-view rows now (`exit 0 · 3 lines`) | Fix the waits, accept |
| `slash-commands` | `/session` has no "Session Info" header | Accept after a read |
| `subagent-attach` | The run finishes before the `running` wait looks | Fix the scenario timing |
| `scroll-view` | "first question" not in the `previous-message` snapshot | Look: timing or a real change |
| `message-queue` | Queue order changed and "Restored 2 queued messages to editor" is gone | Look first: may be a real bug |

Also seen on the real screen: a tool call that fails its own input validation shows `⊗ Read notes.txt` with no reason. Failed rows should carry one.

## Done in this session (2026-11-11, second session)

All on `main` through one PR, label `rust:patch`.

| Item | What changed |
|---|---|
| UI-6 | 78 component goldens: every dialog and notice in dark, light and monochrome at 100 columns. |
| UI-30 | The write-only override map in `TranscriptFocus` is gone. |
| UI-24 | `ctrl+x o` (`app.scroll.toggleAll`) opens or closes all tool rows. Frozen rows are skipped with a notice. |
| Screen check | Harness fixes only (see UI-33). |
| AU-1 | New user page `docs/user/login.md`. |
| AU-2 | Live tests fail when run with `--ignored` and no credential, in the provider crates, `hoocode-ai-registry` and `hoocode-agent-compaction`. |
| AU-3, AU-4 | Mock tests: 401 and 403 on the three stream paths; refresh on expiry through `AuthStorage::get_api_key` with the real provider ids; an Anthropic OAuth stream test. |
| AU-5 | The ChatGPT login says when port 1455 is taken. Claude and Google already fail with the bind error. |
| AU-6, AU-7 | Decisions in `design/agent.md`: pinned client versions are refreshed by hand; the OAuth `state` stays as built. |
| Found by AU-3 | A rejected token (401, 403) is no longer retried for 7 s on the ChatGPT and Gemini paths. The ChatGPT error names the status. |
| Issues | #107, #153, #154, #157, #106, #155 fixed. #152 (the `migrate` command is gone) and #156 (the HUD is gone) are obsolete. |
| Dependencies | #180 (minor and patch group), #167 (model catalog), #164 (sha2 0.11), #162 (jsonwebtoken 11), #161 (jsonschema 0.60) folded in. #163 (windows-sys 0.61) not taken: no Windows target to compile it. |

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

- Merged, nothing unmerged: `claude/design-docs-unified`, `claude/ecstatic-archimedes-ej8zwn`, `claude/haiku-agents-concurrency-72r3pt`, `claude/themes-keybindings-design-nwqgis`, `claude/upbeat-pasteur-n4rk4m`, `tui-visual-overhaul`, `dependabot/cargo/rand-0.10.3`, and `claude/festive-gauss-q4m0lg` after this PR merges.
- Superseded by this PR or by `main`: `agent/responses-parallel-args` (#171, fixed on `main` in 888eccac), `agent/shell-exit-chip` (#172, the single view replaced it), `fix/tui-goldens-version-mask` (#170, harness fix taken by hand), `bot/models-sync` (#167), `dependabot/cargo/cargo-minor-patch-1575fb4005` (#180), `dependabot/cargo/sha2-0.11.1`, `dependabot/cargo/jsonwebtoken-11.1.0`, `dependabot/cargo/jsonschema-0.60.2`.
- From before the history restart (no common base with `main`, last commit 2026-10-09 to 2026-10-29): `agent/muse-integrate`, `agent/ui-prompt-fixes`, `ci/models-sync-use-gh-token`, `claude/all-next-2026-10-14`, `claude/core-step1-reliability`, `claude/design-system-extraction-6t4r9r`, `claude/fix-agent-loop-registry-flake`, `claude/fix-bash-idle-tick-flake`, `claude/friendly-wozniak-ek34vd`, `claude/tui-next-2026-10-14`, `claude/website-who-code-updates-t75o9t`, `dependabot/cargo/cargo-minor-patch-5854e3a6c1`, `dependabot/cargo/crossterm-0.29.0`, `dependabot/cargo/fancy-regex-0.19.2`, `dependabot/github_actions/actions/setup-python-7`, `docs/readme-landscape-demo`, `docs/v2-trailer`, `feat/footer-toolcalls-cleanup`, `fix/app-server-scoped-models`, `fix/fuzz-build-target`, `fix/ui-row-reclaim`. Look once before deleting the two `docs/` branches: they hold README media.
- Keep: `main`, `dependabot/cargo/windows-sys-0.61.2` (#163, open).

## Prompt for the next session

```text
Continue the TUI work. Read CLAUDE.md, then docs/backlog.md. Do the table
"Next session: in this order", top to bottom, as far as the budget allows.
Rows 1 and 2 start from the patches in docs/wip/.

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

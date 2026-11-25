# Backlog and handover

Status (2026-11-12): handover from the TUI session that worked the UI-25 to UI-34 rows and closed the roadmap TUI rows UI-15, UI-16, UI-18 and UI-20. `docs/wip/` is empty (the folder is absent). This is not the last TUI session. The plan stays in [design/roadmap.md](design/roadmap.md); the rules stay in [design/tui.md](design/tui.md). When this page and those disagree, they win and this page is stale.

Plugins v1 (2026-11-18): done and released. PL-1 to PL-12 shipped in hoocode v0.2.45 to v0.2.47 (PRs #189, #191, #192), webtools 0.7.0 from `kolisachint/hoo-web-plugin` (`plugin/`), listed in the pre-trusted `kolisachint/hoo-plugins`. Verified on the released binary against real sites: `hoocode install webtools@hoo-plugins`, HTTPS fetch and search (DuckDuckGo, Tavily) with `--ca-cert` or `NODE_EXTRA_CA_CERTS`, SSRF refusals, plugin debug lines only with `HOOCODE_DEBUG_AGENT_LOG=1`. Open for the user: tag `wit-v0.1.0` on 0102f060; apply hoo-web-plugin `ci/workflow-updates.patch` (needs `workflow` scope) and make its `release.yml` bump `plugin/plugin.json`. Deferred: PL-16 (plugin HTTP via explicit `HTTPS_PROXY`), PL-13 to PL-15. Notes: cargo 1.100 puts test binaries under `target/debug/build/<crate>/<hash>/`, so tests must not derive paths from `current_exe()` parents; abort tests on the faux provider must set `tokens_per_second` (unpaced streams race the abort on slow runners).

Plugins design session (2026-11-17, docs only): added [design/plugins.md](design/plugins.md). Decided: Agent Plugins 1.0 format; internal WASM tier (wasmtime 48 LTS, in-process, gate enforced) and external MCP tier; v1 is the webtools plugin, PL-7 to PL-12, gated by PL-7 (measure trimmed wasmtime and the toolchain bump; stop rule in plugins.md). Repos involved: hoocode, `kolisachint/webtools`, `web-plugins` and `hoocode-official-plugins` (the last two do not exist yet). Scratch measurement code is gone with the container; the numbers are in plugins.md.

Browser plugin design session (2026-11-19, docs only): decided the plugin tiers (A static future, B WASM, C native helper, C+ later), the tier C contract (MCP over Streamable HTTP on a Unix socket or Windows named pipe, one helper per session, per-agent tokens, declared tools, per-platform binaries) and the browser plugin (agent-browser as engine, one shared browser, a window and tabs per agent, Chromium headless default). All in [design/plugins.md](design/plugins.md): Tiers, Native tier contract, Browser plugin (with the locked plan B0 to B4), Decisions, PL-18 to PL-22. Next: PL-20 (agent-browser fixes in `kolisachint/agent-browser`) and PL-18 can start in parallel. Scratch measurement scripts are gone with the container; the numbers are in plugins.md.

Tier C native helpers docs (2026-11-20, docs only, branch `agent/pl18-core`): PL-18 is built on the branch; this session wrote only the docs. [design/plugins.md](design/plugins.md) Native tier contract now matches the build (reqwest `unix_socket` + rmcp, manifest `server` field, `binaryPath` > download > mcp.json `command`, tool origin and permission hints, EnsureHelper relay, lifecycle notifications; Windows not shipped), with dated 2026-11-20 Decisions rows D1–D6, tier C marked Built, PL-18 marked Done and B1 marked Done. [design/roadmap.md](design/roadmap.md) PL-18 marked done in the roadmap style. [user/configuration.md](user/configuration.md) documents `nativeHelpers.<plugin>.binaryPath`, the download dir, the hints and the shared-only subagent rule. CHANGELOG has an Unreleased tier C entry. `docs/maps/packages.md` regenerated. What remains: B0/PL-20 agent-browser fixes, B2 hoo-browser helper. Local test caveat: the user's `~/.hoocode/hoo-config.json` with skip_permissions true breaks 3 hoocode-code-permissions relay tests locally (tests do not isolate the config dir); pre-existing order-dependent flakes in hoocode-code-cli lib tests and hoocode-agent-mcp stdio_deadline_ends_a_slow_call, seen on base 861d5e11.

Detached sessions design (2026-11-25, docs only): added SA-12 to [design/subagents.md](design/subagents.md) (section Detached sessions, Decisions row, Open row) and roadmap Now 0b. Next session implements phase 1. Read the section first; open items for that session are listed at its end.

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

- The cloud container's pinned toolchain (1.100.0) may come without `cargo`. Fix: `rustup toolchain uninstall 1.100.0 && rustup toolchain install 1.100.0 --profile minimal -c rustfmt -c clippy`.
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
| 1 | Google logins | Live login for Gemini CLI and Antigravity: needs the user's OAuth client (step 3 under "Subscription logins"). Claude and ChatGPT passed live on 2026-11-12. | — |
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

2026-11-12, on the user's laptop: Claude and ChatGPT passed live (saved logins, a prompt each, and the two `--ignored` live tests). Google was not run: it needs the user's OAuth client. The live run found two bugs, both fixed: every ChatGPT request crashed in 0.2.41 (rustls had no crypto provider for the WebSocket), and Anthropic rejected Claude Fable 5.2 for Claude Code 2.1.280 (pin raised to 2.1.322). The user's ChatGPT account is on the Go plan, which allows only `gpt-6-luna`, `gpt-5.6-luna` and `gpt-5.6-terra`; run the ChatGPT live test with `HOOCODE_CODEX_LIVE_MODEL=gpt-6-luna`. `gpt-6.1-sol` for `openai-codex` is in the catalog but not checked live.

| Stage | Claude (`anthropic`) | ChatGPT (`openai-codex`) | Gemini (`google-gemini-cli`) |
|---|---|---|---|
| Authorize URL, code exchange, token refresh | Mock test | Mock test | Mock test |
| Refresh on expiry through `AuthStorage::get_api_key`, real provider id | Mock test | Mock test | Mock test |
| Request headers and body for a subscription token, on the wire | Mock server | Mock server | Mock server |
| 401 and 403 | Mock server: fails at once with status and body | Mock server: one SSE attempt, `ChatGPT API error (401): …` | Mock server: one request, `Cloud Code Assist API error (401): …` |
| Callback port taken | Fails with the bind error | Warns, then paste | Fails with the bind error |
| Live login, live model call | Passed 2026-11-12 | Passed 2026-11-12 (Go plan) | Not run |

To finish on a machine with a browser and the accounts:

1. Claude: `/login` → Anthropic. Send one prompt with a Claude model. Then `ANTHROPIC_OAUTH_TOKEN=<token> cargo test -p hoocode-ai-provider-anthropic --test it -- --ignored oauth_tool_names_round_trip`.
2. ChatGPT: `/login` → ChatGPT (port 1455 free). Send one prompt with a model your plan allows. Then `HOOCODE_CODEX_LIVE_MODEL=<that model> OPENAI_CODEX_OAUTH_TOKEN=<token> cargo test -p hoocode-ai-provider-openai-codex -- --ignored live_handles_sse`.
3. Gemini: set `HOOCODE_GEMINI_CLI_CLIENT_ID` and `HOOCODE_GEMINI_CLI_CLIENT_SECRET`, `/login` → Google, send one prompt with a `gemini-*` model. Paid tiers also need `GOOGLE_CLOUD_PROJECT`.
4. For each: restart hoocode and send a prompt again. After the token's expiry, send again.

Open auth notes: the pinned Claude Code version is `2.1.322`, the one published on 2026-11-11 (refresh rule in `agent.md`). Anthropic gates new models on it: Fable 5.2 needs 2.1.290 or newer. `docs/user/login.md` is not in the site's sidebar until the site repo adds it. The ChatGPT retry loop still retries other 4xx errors such as 400.

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

# Extras: triage of ledger 12.6 and the rest of 12.7

Status: **draft for review, 2026-10-07.** Design only. Each item gets a
recommendation, and the user gives each a go or no-go. Canvas is in
[canvas-and-mcp-apps.md](canvas-and-mcp-apps.md).
Index: [post-migration-roadmap.md](post-migration-roadmap.md).

## 0. Summary

| # | Item | TS size | Recommendation |
|---|---|---|---|
| 1 | Warm subagent pool (12.6) | 464 + 54 lines | **Already built** (10.9c, `code-subagents/src/warm.rs`). Close 12.6. |
| 2 | Thinking escalation | 82 lines | **Port now.** It's on by default in TS, so its absence changes model behaviour after tool errors. |
| 3 | Version check and update hint | `utils/version-check.ts` | **Port**, merged with distribution's "self-update" item |
| 4 | Completion chime | small | **Port** (the setting `chimeOnTurnComplete` is already parsed) |
| 5 | Prompt-reactive plugin nudges | 327 lines | **With plugins phase 3**, not before (it nudges toward plugin tools) |
| 6 | `/learn` | 3,116 + 712 lines | **Audit half later; mining half on demand** |
| 7 | Teams (`--team url\|auto`, hooteams client) | 712 lines | **Later**, as a client of hooteams' existing wire format. Don't adopt A2A yet. |
| 8 | Voice (voicetools) | panel + client | **Later** |
| 9 | HTML export, `/share` (gist) | 744 lines + template | **No** for now |
| 10 | Install telemetry | 13 lines (sends nothing) | **Drop.** Accept the setting key and ignore it. |
| 11 | `hoocode-user-agent` | — | Verify only: providers already send `user-agent: hoocode` |

## 1. Warm subagent pool (12.6): close

TS `warm-subagent-pool.ts` keeps long-lived RPC-mode workers per (agent type, model,
provider). It's opt-in (`--warm-subagents`) and falls back to the cold pool on any
failure. The Rust port landed as 10.9c ("warm pool + inbox"):
`cortexcode-code-subagents/src/warm.rs` (750 lines), with `get_warm_subagent_pool`
and `warm_subagents_enabled` used by the Task tool. [subagents.md](subagents.md) §6
records the further in-process runner. **Action:** mark 12.6 `moved` with a note
pointing at 10.9c. No new work.

## 2. Thinking escalation

After a tool error, raise the thinking level for the following turn(s), then restore
the baseline after a cooldown. Configured by `thinking_escalation` in
`hoo-config.json`, which Rust already merges (`code-modes/src/config.rs`) but nothing
acts on.

Design:

- an `ExtensionHooks` built-in in `code-modes` on `tool_execution_end`, `turn_end`
  and `agent_end`;
- `escalated_this_turn` so the failing turn doesn't consume a cooldown step;
- restore on `agent_end` or when the user changes the level by hand.

Tests: TS `thinking-escalation.test.ts` verbatim. About a day of work.

## 3. Version check and update hint

TS asks the npm registry for `@kolisachint/hoocode-agent/latest` (10s timeout,
skipped when offline), compares semver, and shows a startup notice with the update
command for the detected install method. Rust:

- **Source:** the GitHub releases API for `kolisachint/hoocode`, or npm
  `@kolisachint/hoocode` (both are published by `release.yml`). Recommend npm: no
  rate limit and the same check as TS.
- **Install method** (the distribution.md "self-update" item): a binary under
  `~/.hoocode/bin` means a curl install, so print `curl … | sh`; a path inside
  `node_modules` means npm or bun, so print the package-manager command; anything
  else (cargo or source) gets no command.
- **When:** at most once a day (timestamp in `~/.hoocode/rust/state.json`), never
  in print, json or rpc modes, and off with `HOOCODE_OFFLINE=1` or setting
  `checkForUpdates: false`.
- `cortex update`, a subcommand already reserved in `code-cli` `SUBCOMMANDS`,
  prints the right command. It doesn't replace the binary in place
  (signing and quarantine; see distribution.md).

Tests: TS `version-check.test.ts`, plus `config.test.ts` install-method cases
(ledger note on 12.7).

## 4. Completion chime

A single BEL byte when a turn that ran longer than 10s finishes (not when
aborted), or immediately when the agent opens the ask_options pane. There is one
5s debounce window. `chimeOnTurnComplete` is already a Rust setting. Port the
behaviour and TS `completion-chime.test.ts`. Small.

## 5. Prompt-reactive nudges

A curated cue → nudge table that, when tool output shows a repeated manual pattern,
suggests the plugin layer ("there's a plugin for this"). It only makes sense once
`SearchPlugins` and `InstallPlugin` exist ([plugins.md](plugins.md) phase 3). It
ships with that phase; TS `prompt-reactive-nudges.test.ts` is its reading list.

## 6. `/learn`

Two halves:

- **Audit** (deterministic, no model): find lines in context files (AGENTS.md,
  CLAUDE.md) whose backticked path referents no longer exist. It's cheap, safe and
  useful, so port it as `/learn audit` when wanted.
- **Mining** (model reads past session transcripts, clusters candidate rules,
  proposes rules and skills): it costs real tokens, states the price up front, and
  has a cache and clustering pipeline (`learn/` 3,116 lines). Port only if the user
  wants it; it is the largest single extra.

Reading list: `learn-audit`, `learn-extract`, `learn-cluster`, `learn-eval`,
`learn-session-replacement`, `learn-settings-pane`.

## 7. Teams

`--team <url|auto>` attaches the TUI's task panel to a **hooteams** server
(`kolisachint/hooteams`, MIT): it mirrors role state from its SSE `/events`
(`TeamEvent = AgentEvent & { role, agentId, ts }`), steers with `POST /steer`, and
answers approval gates (`task_paused` → options pane → `POST /tasks/:id/resume`,
first answer wins). `--team auto` finds `.agents/teams/default.json` or
`hooteams.config.json` up the tree and spawns `hooteams` from PATH.

Recommendation: later. When done, port the client unchanged (the wire format is
hooteams', and `hoocanvas` already consumes it). **A2A** (AAIF, v1.0) is the
standard for agent-to-agent tasks over HTTP, but it has no local or stdio story and
no consumer in this ecosystem yet. Revisit if hooteams or hoobot need to talk to
agents outside it.

## 8. Voice

The TUI voice panel drives the `voicetools` binary (`kolisachint/voicetools`, MIT)
over a line protocol (`STATUS`, `SEGMENT`, `DONE`, `ERROR`; daemon mode adds
`READY`, `LEVEL`, `PHASE`) and pastes segments into the editor. Keep it as an
external binary, not a library: it bundles models. Later, when wanted. Reading
list: `voice-transcribe`, `voice-panel`.

## 9. HTML export and `/share`

`/export` to HTML (template, ANSI-to-HTML, tool renderers, vendored assets) and
`/share` (secret gist via `gh`). The non-HTML `/export` already exists. Not
recommended now: the HTML template is a large UI surface to maintain twice, and
`/share` uploads session content to a third party. Reading list if revived:
`export-html-*` (XSS cases included).

## 10. Install telemetry

TS's `reportInstallTelemetry` is a no-op ("nothing leaves the machine") and
`enableInstallTelemetry` is inert. Don't port it. Keep accepting the settings key so
shared `settings.json` files don't warn.

## Standards notes

- **AGENTS.md:** already read (user scope `~/.agents/AGENTS.md` and
  `~/.hoocode/AGENTS.md`, plus cwd to root, additive). The spec says the nearest file
  wins; hoocode concatenates least-specific first, so the nearest file comes last and
  effectively wins. No change. The TS open item (project ancestor-walk
  `.agents/AGENTS.md`) is shared and small; do it in both or neither.
- **Agent Skills:** validation lives in [plugins.md](plugins.md) §5.

## Decisions for the user

| # | Item | Recommendation |
|---|---|---|
| X1 | Close 12.6 as built by 10.9c | Yes |
| X2 | Thinking escalation, version check, chime | Go |
| X3 | `/learn` | Audit later; mining only on request |
| X4 | Teams, voice | Later |
| X5 | HTML export, `/share`, telemetry | No |

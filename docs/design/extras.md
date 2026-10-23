# Extras

Status: **agreed 2026-10-07**. Items 1 and 2 under "What we build" are **done 2026-10-14**. Replaces ledger 12.6 and the rest of 12.7.

## Goal

Decide which of hoocode-ts's smaller features `hoocode` gets.

## Decisions

| Item | Decision | Note |
|---|---|---|
| Warm subagent pool (12.6) | **Already built** | Ported as 10.9c (`code-subagents/src/warm.rs`) |
| Version check | **Yes** | |
| Completion chime | **Yes** | |
| Thinking escalation | No (for now) | On by default in hoocode-ts; Rust reads its config but ignores it |
| `/learn` | No | |
| Teams (hooteams client) | No | The `--team` flag was removed with the teams lens (N9, decision 27). |
| Voice (voicetools) | No | |
| HTML export, `/share` | No | |
| Install telemetry | No | hoocode-ts's version sends nothing anyway; keep accepting the setting key |

## What we build

1. **Version check.**
   - At most once a day, and never in print, json or rpc modes.
   - Off with `HOOCODE_OFFLINE=1` or `checkForUpdates: false`.
   - Asks npm for the latest version of `@kolisachint/hoocode` (the scoped alias of `hoocode`, same version; the code's registry URL is unchanged).
   - If newer, shows a startup notice with the right update command for how
     `hoocode` was installed: under `~/.hoocode/bin` means curl; inside
     `node_modules` means npm or bun; anything else gets no command.
   - `hoocode update` prints the same command; it never replaces the binary itself.
2. **Completion chime.**
   - One terminal bell when a turn longer than 10 seconds finishes (not when you
     abort it), or right away when the agent asks you a question.
   - Repeat chimes within 5 seconds are dropped.
   - Uses the existing `chimeOnTurnComplete` setting.

Done when: both work, with hoocode-ts's `version-check` and `completion-chime`
tests ported.

Done 2026-10-14: both items are built, with their own unit tests (no TypeScript
reference was ported). Version check: `crates/hoocode-code-update` (state in
`~/.hoocode/version-check.json`; the latest version seen is kept, so a newer release
is announced on later starts without a request). Install commands: curl for
`~/.hoocode/bin` (`https://kolisachint.github.io/hoocode/install.sh`), npm or bun inside
`node_modules`. Chime: `crates/hoocode-code-tui-app/src/chime.rs`; it rings for AskUserQuestion only,
not for MCP elicitation forms. Review fixes 2026-10-14: pre-releases compare below their release,
symlinked npm and bun bins resolve to their package, and a turn chimes by its own submit time.

## Not doing

Everything marked No above.

## Open questions

- Thinking escalation is a model-quality feature hoocode-ts turns on by default.
  Revisit once there is a way to measure its effect.

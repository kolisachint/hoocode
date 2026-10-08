# Extras

Status: **agreed 2026-10-07**, design only. Replaces ledger 12.6 and the rest of 12.7.

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
| Teams (`--team`, hooteams client) | No | |
| Voice (voicetools) | No | |
| HTML export, `/share` | No | |
| Install telemetry | No | hoocode-ts's version sends nothing anyway; keep accepting the setting key |

## What we build

1. **Version check.**
   - At most once a day, and never in print, json or rpc modes.
   - Off with `HOOCODE_OFFLINE=1` or `checkForUpdates: false`.
   - Asks npm for `@kolisachint/hoocode`'s latest version.
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

## Not doing

Everything marked No above.

## Open questions

- Thinking escalation is a model-quality feature hoocode-ts turns on by default.
  Revisit once there is a way to measure its effect.

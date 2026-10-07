# Scheduler, `/loop` and the Cron tools

Status: **draft for review, 2026-10-07.** Design only. Replaces ledger task 12.5.
Index: [post-migration-roadmap.md](post-migration-roadmap.md).

hoocode-ts references: `docs/loop-and-plugin-system.md` §1, `core/scheduler.ts` (210
lines), `extensions/core/loop.ts` (354 lines).

## Problem

hoocode-ts can run prompts on a schedule and keep itself working without a human
re-prompting:

- **Scheduler.** `TaskScheduler` holds tasks
  `{ id, cron, prompt, recurring, createdAt, lastRunMinute? }`:
  - they are persisted to `<cwd>/.agents/scheduled_tasks.json` (legacy
    `<cwd>/.hoocode/scheduled_tasks.json` is read once and migrated forward);
  - 5-field cron in local time;
  - it ticks every 30s and fires **only when the session is idle**; a due task fires
    on a later tick within the same minute and is skipped if the session stays busy
    all minute;
  - one-shot tasks are deleted after firing;
  - firing means `sendUserMessage(prompt)`.
- **Tools:** `CronCreate({ cron, prompt, recurring? })`, `CronList()`,
  `CronDelete({ id })`. The model can schedule its own follow-ups.
- **`/loop`:**
  - `/loop "<cron>" <prompt>`, `/loop <5m|2h|1d> <prompt>`, `/loop once "<cron>" <prompt>`
  - `/loop list`, `/loop delete <id>`, `/loop stop`
  - `/loop auto [--max-turns N] <task>`: re-prompt on every `agent_end` until the
    reply contains `LOOP_DONE` or the budget (default 10) runs out. It yields while
    the user is steering.
- **`ask_options` interplay:** while `/loop auto` runs, `ask_options` doesn't block.
  It auto-picks `recommended` options, or emits `loop:halt` when a question has no
  recommended option.

In Rust:

- `/goal [--max-turns N]` is already ported (in `code-modes`). It is the same "keep
  going" idea, scoped to modes.
- `AskOptionsHost` has `set_auto_loop_active` and `halt_loop` waiting for a driver
  (10.2f note).
- Nothing schedules.

## Goals

1. TS parity for the tools, `/loop` and the `.agents/scheduled_tasks.json` format, so
   a schedule made in either hoocode is visible to the other.
2. No double firing: not across two `cortex` sessions in one directory, and not
   between `cortex` and hoocode-ts.
3. No silent loss. A due fire that couldn't run says so.

## Design

### Crate `cortexcode-code-scheduler`

- **Cron matcher:** a port of TS `matchesCron`/`parseField` (`*`, lists, ranges,
  steps, five fields, local time via `chrono`), **not** a cron library. Both tools
  read the same file, so they must agree on what an expression means.
  `croner` (named in the ledger) would add syntax hoocode-ts rejects (seconds,
  `L`, `#`, nicknames). That is Decision L4.
- **Store:** `<cwd>/.agents/scheduled_tasks.json`, the TS array shape, with unknown
  fields preserved. Every mutation and every fire follows the same sequence: lock
  the file (`fs4`, already a dependency), re-read it, check, write a temp file, and
  rename it over the original.
- **Fire claim (fixes double firing):** to fire a task, a process takes the lock,
  re-reads it, and fires only if `lastRunMinute` is still older than the current
  minute. It writes the new `lastRunMinute` before sending the prompt. Two sessions
  in one directory, or `cortex` next to hoocode-ts (which writes the same field),
  then fire once between them. hoocode-ts doesn't take the lock, so a race is still
  possible within a few milliseconds; this is documented, not engineered away.
- **Tick:** a tokio interval of 30s, created on session start and cancelled on
  shutdown. Idle comes from `AgentSession` (no run in flight and no queued
  steering).
- **Missed fires (Decision L1):** TS drops a fire if the session is busy for the
  whole minute. Proposal: a fire due while the session is busy is **coalesced and
  deferred** until the next idle moment, up to 10 minutes late. Past that it is
  skipped, and the status line says "skipped 1 scheduled prompt (busy)". A
  recurring task never queues more than one pending fire.

### Tools and commands

- `CronCreate`, `CronList` and `CronDelete` use the pin's names, schemas and
  descriptions. They are registered when the scheduler is enabled (default on, as in
  TS). Measure their combined schema cost with `--print-token-surface`; if it is
  significant, offer `enableLoopTools: false`.
- `/loop` follows the TS grammar exactly, including interval-to-cron conversion
  (`5m` becomes `*/5 * * * *`, `2h` becomes `0 */2 * * *`, `1d` becomes `0 0 */1 * *`; only `m`, `h` and `d` units, minimum 1).
- **`/loop auto`:**
  - an `agent_end` hook re-prompts with the TS continuation text;
  - `LOOP_DONE` detection and the `--max-turns` budget (default 10) match TS;
  - it yields to steering;
  - it drives `AskOptionsHost::set_auto_loop_active` and stops on `halt_loop`.
  `/goal` stays separate (mode-driven). Both share the same continuation helper.
- Firing a scheduled prompt goes through the normal input path, so modes, the
  permission gate and hooks all apply. A scheduled prompt that hits a permission
  prompt waits for a human like any other.

### Where schedules run (Decision L3)

TS fires schedules only while an interactive session is open in that directory.
The Rust [app-server](app-server.md) is a long-running daemon, so it *could* run
schedules headless. That would make `/loop` an actual cron for agents. It also makes
"gated tool with nobody watching" the normal case: the app-server would have to
deny gated tools on scheduled turns unless a client answers. Proposal: **not in v1.**
v1 runs schedules in the interactive TUI and in an app-server thread that is loaded
and subscribed. Headless daemon schedules need their own short doc once v1 is used.

## Standards note

There is no AAIF standard for scheduled agent prompts. goose has recipes (YAML task
definitions) that can be scheduled by an outside scheduler; Claude Code has
`CronCreate`/`CronList`/`CronDelete` tools with session-scoped semantics.
hoocode-ts's tool names follow Claude Code's, and we keep them. The
`.agents/scheduled_tasks.json` file is hoocode's own; it lives under `.agents/`
because it is user-meaningful state ([plugins.md](plugins.md) principle).

## Tests

- **Cron matcher:** TS `scheduler.test.ts` cases verbatim (MIT), plus DST edges in
  local time.
- **Store:** TS-written file round-trips; legacy migration; unknown fields kept.
- **Fire claim:** two schedulers on one file fire a due task once (threads and two
  processes).
- **Missed fire:** busy session, then idle within the window fires once; idle after
  the window skips with a notice.
- **`/loop auto`:** `LOOP_DONE` stops; budget stops; steering yields; ask_options
  auto-picks the recommended option and halts without one (TS
  `ask-options-loop.test.ts` "/loop auto flips ask_options into non-blocking mode",
  `loop-auto-start.test.ts`).

## Decisions for the user

| # | Question | Recommendation |
|---|---|---|
| L1 | Coalesce and defer busy fires up to 10 minutes, unlike TS's drop? | Yes |
| L2 | Lock-and-claim on the shared file? | Yes |
| L3 | Headless schedules in the app-server daemon? | Not in v1 |
| L4 | Port the TS cron matcher instead of `croner`? | Port, for identical semantics on the shared file |

# Scheduler and `/loop`

Status: **agreed 2026-10-07**. Replaces ledger 12.5.

Built so far (crate `hoocode-code-scheduler`; 2026-10-08, and 2026-10-14 for items 3, 4 and
the late-fire rule):

- Done 2026-10-08: the cron matcher (ported from `core/scheduler.ts`), the store at
  `<cwd>/.agents/scheduled_tasks.json` (same shape, legacy `.hoocode/` read until the new
  file exists, write-temp-and-rename, lock file so a fire is claimed once), the three
  tools with hoocode-ts's names, labels, descriptions and schemas, in the default bundle
  after AskUserQuestion (13 tools in the print-basic request, as in TS), and firing in
  the interactive loop: a 30-second tick, idle only, the due prompts sent as follow-ups.
- Done 2026-10-14, item 3: `/loop` (`mode/loop_command.rs` in `hoocode-code-tui-app`; the
  arguments are parsed in `hoocode-code-scheduler/src/loop_command.rs`). Forms:
  `/loop "<cron>" <prompt>`, `/loop <N>m|<N>h|<N>d <prompt>` (an interval becomes
  `*/N * * * *`, `0 */N * * *` or `0 0 */N * *`; minutes and hours must divide evenly),
  `/loop once <schedule> <prompt>`, `/loop list`, `/loop delete <id>`, `/loop stop`.
  `stop` ends `/loop auto` and removes the scheduled prompts this session's `/loop`
  created (kept in memory per session, with their store). Prompts from an earlier
  session, from the model's Cron tools, or from another session stay; `/loop delete <id>`
  removes any of them. The store has no origin field, so the session list is the record.
  A quoted cron is checked field by field (`validate`) and refused with a usage message
  when it cannot match.
- Done 2026-10-14, item 4: `/loop auto [--max-turns N] <task>` (default 10). The first
  prompt is turn 1; after each turn that did not end the loop, the continuation is sent.
  The loop ends when a reply contains `LOOP_DONE`, at the cap, or when a turn is aborted
  or fails. It yields while the agent works, while messages are queued, and while the
  editor has text: the continuation goes out on the first idle tick after that. While it
  runs, `AskUserQuestion` takes the recommended option of each question (`auto_loop.rs`),
  or halts the loop when a question has none. Not wired: `/grill` does not yet skip its
  questions during the loop, and `/goal` (`StartAutoLoop`) still has no listener.
- Done 2026-10-14, late fires: a task whose due minute is up to 10 minutes past (the
  busy turn is over, the next idle tick is within the window) fires once on that tick.
  A later tick skips it and shows a warning. A one-shot task is removed either way. The
  store records the due minute, so the same due minute never fires twice. Limits: a tick
  looks back a day for a due minute, so a schedule whose last due minute is older than a
  day (weekly, monthly) is not reported as skipped. A task is never due for a minute
  before its creation.
- Done 2026-10-14, review fixes: a fire claim fires nothing when it cannot take the lock
  file within 3 seconds (the next tick claims it, still inside the late window); the
  cron matcher reads `1-7` as Monday to Sunday (it used to become the empty `1-0`) and
  no longer overflows on huge steps; a request that returns an error ends `/loop auto`
  as an interruption. Known, not fixed: two processes can both take over one stale lock
  file at the same instant (a lock held by a process that died), which needs `flock`
  (`File::try_lock`, Rust 1.89; the workspace MSRV is 1.88); a scheduled prompt claimed on
  the same tick as an auto-loop continuation can collide with it, and the session then
  refuses the second prompt, which ends `/loop auto` with an error notice.
- Done 2026-10-15, T4.2: the jobs are rows in the task store (Schedule kind, `rows.rs`): fire, run, finish and next run. The Schedule tab's rendering is T4.1 ([tui-activity.md](tui-activity.md)).
- Differences from hoocode-ts: `CronCreate` returns a tool error when the store cannot be
  written (TS ignores the write error); the store is re-read on every operation, so a
  task another process created is not lost.
- Firing happens only in the interactive loop. Print, rpc and subagent processes do not
  tick. The tick uses the session's current cwd, the tools use the cwd the session
  started with, so `/cd` leaves the two pointing at different stores.

## Goal

Run prompts on a schedule (`/loop`, and Cron tools the model can call) and let the
agent keep working on its own (`/loop auto`), the way hoocode-ts does. A schedule
made in either hoocode is visible to the other.

## Decisions

| Decision | Why |
|---|---|
| Same file as hoocode-ts: `<cwd>/.agents/scheduled_tasks.json`, same shape | The two tools share schedules |
| Port hoocode-ts's cron matcher (5 fields, local time) instead of a cron library | Both tools must read the same expression the same way |
| Fires are claimed under a file lock, so a task fires once | Two sessions, or hoocode next to hoocode-ts, must not both fire it |
| A prompt due while the agent is busy **runs at the next idle moment, up to 10 minutes late**; after that it's skipped with a notice | hoocode-ts silently drops it |
| Schedules run only while a session is open (no headless daemon runs in v1) | Nobody would be there to approve tools |

## What we build

1. Crate `code-scheduler`:
   - cron matcher;
   - store (lock, re-read, write a temp file and rename);
   - tick every 30 seconds while the session is idle.
2. Tools `CronCreate`, `CronList`, `CronDelete` with hoocode-ts's names and schemas.
3. `/loop` with hoocode-ts's syntax:
   - `/loop "<cron>" <prompt>`, `/loop 5m|2h|1d <prompt>`, `/loop once …`
   - `/loop list`, `/loop delete <id>`, `/loop stop`
4. `/loop auto [--max-turns N] <task>`:
   - re-prompts after each turn until the reply contains `LOOP_DONE` or 10 turns
     pass;
   - yields while you type;
   - makes `AskUserQuestion` pick recommended answers, or stop the loop when there is
     none (the hooks already exist in Rust).
5. Scheduled prompts go through the normal input path, so modes and permissions
   apply.

Done when: hoocode-ts's scheduler tests pass, two processes fire a task once, and
busy-then-idle within 10 minutes fires once while longer is skipped with a notice.

## Not doing

- Headless schedules in the app-server daemon.
- Cron syntax beyond hoocode-ts's (seconds, `L`, `#`, nicknames).

## Open questions

- Do the three Cron tools cost enough tokens to need an off switch? Measure first.

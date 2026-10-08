# Scheduler and `/loop`

Status: **agreed 2026-10-07**, design only. Replaces ledger 12.5.

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

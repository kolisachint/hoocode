//! Schedule rows in the task store (`hoocode-code-task-store`), for the task
//! panel's Schedule tab: one row per scheduled job, under the job's id.
//!
//! A row's status follows its last run: `Pending` (waiting, or never run),
//! `InProgress` while the job is fired and not yet finished (note `queued`,
//! then `running`), `Done` or `Failed` after the run. `next_at` is the next
//! matching minute. A job that leaves the store leaves its row, once it is not
//! running. A one-shot job's row ends with its run.
//!
//! Every function here works on memory only. The tick passes the store's tasks
//! it has just read (see [`crate::Tick`]), so publishing does no I/O.

use std::collections::HashSet;

use chrono::{DateTime, Duration as ChronoDuration, Local, Timelike};
use hoocode_code_task_store::{
    CreateTaskOptions, Task, TaskPatch, TaskSource, TaskStatus, TaskStore,
};

use crate::cron;
use crate::store::ScheduledTask;

/// The key prefix of schedule rows: `schedule:<job id>`.
pub const KEY_PREFIX: &str = "schedule:";

/// How far ahead [`next_run_ms`] looks. A schedule whose next match is further
/// off shows no next run.
const NEXT_SEARCH_MINUTES: i64 = 8 * 24 * 60;

/// The stable row key of a job.
pub fn row_key(job_id: &str) -> String {
    format!("{KEY_PREFIX}{job_id}")
}

/// Creation options of a schedule row: source [`TaskSource::Schedule`] and no
/// agent, so the row is not a plan item.
fn options(key: &str) -> CreateTaskOptions {
    CreateTaskOptions {
        source: Some(TaskSource::Schedule),
        key: Some(key.to_owned()),
        ..Default::default()
    }
}

/// Epoch milliseconds of the first whole minute after `now` at which `expr`
/// matches, searched up to eight days ahead.
pub fn next_run_ms(expr: &str, now: &DateTime<Local>) -> Option<u64> {
    let minute_start = now.with_second(0)?.with_nanosecond(0)?;
    (1..=NEXT_SEARCH_MINUTES).find_map(|offset| {
        let at = minute_start + ChronoDuration::minutes(offset);
        if !cron::matches(expr, &at) {
            return None;
        }
        u64::try_from(at.timestamp_millis()).ok()
    })
}

/// The jobs the scheduler has fired and whose run has not finished. The
/// interactive loop keeps one and passes it to every tick.
#[derive(Debug, Default)]
pub struct Runs {
    /// Fired, and their prompt not yet started (the turn has not begun).
    waiting: Vec<ScheduledTask>,
    /// Started, and the turn not yet finished.
    running: Vec<ScheduledTask>,
}

impl Runs {
    /// The keys of the jobs fired and not finished.
    fn in_flight_keys(&self) -> HashSet<String> {
        self.waiting
            .iter()
            .chain(self.running.iter())
            .map(|job| row_key(&job.id))
            .collect()
    }

    /// One tick: a row for each live job, the fires of this tick (`fired`, status
    /// `InProgress`), and removal of the rows of jobs that have left the store.
    /// `jobs` is `None` when the tick read nothing; then no row is removed.
    pub fn publish_tick(
        &mut self,
        store: &TaskStore,
        jobs: Option<&[ScheduledTask]>,
        fired: &[ScheduledTask],
        now: &DateTime<Local>,
    ) {
        store.batch(|s| {
            if let Some(jobs) = jobs {
                for job in jobs {
                    upsert_job(s, job, now);
                }
            }
            for job in fired {
                self.waiting.push(job.clone());
                let key = row_key(&job.id);
                s.upsert_keyed(
                    &key,
                    &job.prompt,
                    options(&key),
                    TaskPatch {
                        status: Some(TaskStatus::InProgress),
                        note: Some(Some("queued".to_owned())),
                        // A one-shot job does not run again.
                        next_at: Some(job.recurring.then(|| next_run_ms(&job.cron, now)).flatten()),
                        ..Default::default()
                    },
                );
            }
            if let Some(jobs) = jobs {
                let live: HashSet<String> = jobs.iter().map(|j| row_key(&j.id)).collect();
                let in_flight = self.in_flight_keys();
                s.remove_keys_where(|key| {
                    key.starts_with(KEY_PREFIX) && !live.contains(key) && !in_flight.contains(key)
                });
            }
        });
    }

    /// A user message with `prompt` as its text has begun a turn: the first
    /// waiting job with that prompt is running, and its row shows `running`.
    /// Other waiting jobs stay `queued` until their own prompt begins. A prompt
    /// that matches no waiting job changes nothing.
    pub fn run_started(&mut self, store: &TaskStore, prompt: &str) {
        let Some(index) = self.waiting.iter().position(|job| job.prompt == prompt) else {
            return;
        };
        let job = self.waiting.remove(index);
        let key = row_key(&job.id);
        store.batch(|s| {
            s.upsert_keyed(
                &key,
                &job.prompt,
                options(&key),
                TaskPatch {
                    status: Some(TaskStatus::InProgress),
                    note: Some(Some("running".to_owned())),
                    ..Default::default()
                },
            );
        });
        self.running.push(job);
    }

    /// Queued prompts that will not run (the prompt queue was cleared, the
    /// session was switched, or the turn ended with them still waiting): their
    /// rows stop saying `queued`. A recurring job is pending for its next run;
    /// a one-shot job's row is removed.
    pub fn drop_waiting(&mut self, store: &TaskStore, now: &DateTime<Local>) {
        if self.waiting.is_empty() {
            return;
        }
        let dropped: Vec<ScheduledTask> = std::mem::take(&mut self.waiting);
        store.batch(|s| {
            for job in &dropped {
                let key = row_key(&job.id);
                if !job.recurring {
                    s.remove_keyed(&key);
                    continue;
                }
                s.upsert_keyed(
                    &key,
                    &job.prompt,
                    options(&key),
                    TaskPatch {
                        status: Some(TaskStatus::Pending),
                        note: Some(None),
                        next_at: Some(next_run_ms(&job.cron, now)),
                        ..Default::default()
                    },
                );
            }
        });
    }

    /// The turn ended: each started job's row shows `Done` or `Failed` and its
    /// next run. A one-shot job's row is removed. A job still waiting then
    /// never ran (see [`Runs::drop_waiting`]).
    pub fn run_finished(&mut self, store: &TaskStore, ok: bool, now: &DateTime<Local>) {
        self.drop_waiting(store, now);
        if self.running.is_empty() {
            return;
        }
        let finished: Vec<ScheduledTask> = std::mem::take(&mut self.running);
        store.batch(|s| {
            for job in &finished {
                let key = row_key(&job.id);
                if !job.recurring {
                    s.remove_keyed(&key);
                    continue;
                }
                s.upsert_keyed(
                    &key,
                    &job.prompt,
                    options(&key),
                    TaskPatch {
                        status: Some(if ok {
                            TaskStatus::Done
                        } else {
                            TaskStatus::Failed
                        }),
                        note: Some(if ok {
                            None
                        } else {
                            Some("run failed".to_owned())
                        }),
                        next_at: Some(next_run_ms(&job.cron, now)),
                        ..Default::default()
                    },
                );
            }
        });
    }
}

/// A live job's row: created as `Pending` when new, with its next run kept up
/// to date. The status of an existing row is left alone (it is the last run's).
/// A row whose next run is still ahead is not written, so a tick leaves it alone.
fn upsert_job(store: &TaskStore, job: &ScheduledTask, now: &DateTime<Local>) {
    let key = row_key(&job.id);
    let now_ms = u64::try_from(now.timestamp_millis()).unwrap_or(0);
    let stale = store
        .find_keyed(&key)
        .is_none_or(|row: Task| row.next_at.is_none_or(|at| at <= now_ms));
    if !stale {
        return;
    }
    store.upsert_keyed(
        &key,
        &job.prompt,
        options(&key),
        TaskPatch {
            next_at: stale.then(|| next_run_ms(&job.cron, now)),
            ..Default::default()
        },
    );
}

/// Publishes a job `/loop` or a Cron tool has just created, without waiting for a tick.
pub fn publish_created(store: &TaskStore, job: &ScheduledTask, now: &DateTime<Local>) {
    upsert_job(store, job, now);
}

/// Removes the row of a job the store no longer holds (`/loop delete`, `/loop stop`).
/// A job that is running now keeps its row until its run ends, as on a tick.
pub fn publish_removed(store: &TaskStore, job_id: &str, runs: &Runs) {
    let key = row_key(job_id);
    if runs.in_flight_keys().contains(&key) {
        return;
    }
    store.remove_keyed(&key);
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(hour: u32, minute: u32, second: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(2026, 10, 15, hour, minute, second)
            .single()
            .unwrap()
    }

    fn millis(when: &DateTime<Local>) -> u64 {
        u64::try_from(when.timestamp_millis()).unwrap()
    }

    fn job(id: &str, cron: &str, prompt: &str, recurring: bool) -> ScheduledTask {
        ScheduledTask {
            id: id.into(),
            cron: cron.into(),
            prompt: prompt.into(),
            recurring,
            created_at: 0,
            last_run_minute: None,
        }
    }

    fn row(store: &TaskStore, id: &str) -> Option<Task> {
        store.find_keyed(&row_key(id))
    }

    #[test]
    fn a_tick_leaves_a_row_whose_next_run_is_ahead_alone() {
        let store = TaskStore::new();
        let jobs = [job("keep", "30 9 * * *", "daily", true)];
        let mut runs = Runs::default();
        runs.publish_tick(&store, Some(&jobs), &[], &at(9, 0, 0));
        let first = row(&store, "keep").unwrap();
        hoocode_code_task_store::advance_clock_for_tests(60_000);
        runs.publish_tick(&store, Some(&jobs), &[], &at(9, 0, 30));
        let second = row(&store, "keep").unwrap();
        assert_eq!(second.next_at, first.next_at);
        assert_eq!(
            second.updated_at, first.updated_at,
            "no write on a quiet tick"
        );
    }

    #[test]
    fn next_run_is_the_first_matching_minute_after_now() {
        let next = next_run_ms("30 9 * * *", &at(9, 30, 10)).unwrap();
        assert_eq!(next, millis(&at(9, 30, 0)) + 86_400_000);
        assert!(next_run_ms("not a cron", &at(9, 30, 10)).is_none());
        // A weekly schedule is still found within the search window.
        assert!(next_run_ms("0 9 * * 1", &at(9, 30, 10)).is_some());
    }

    #[test]
    fn a_live_job_gets_a_pending_row_with_its_next_run_and_a_stable_id() {
        let store = TaskStore::new();
        let mut runs = Runs::default();
        let jobs = vec![job("task_a", "30 9 * * *", "check the build", true)];
        runs.publish_tick(&store, Some(&jobs), &[], &at(8, 0, 0));
        let first = row(&store, "task_a").unwrap();
        assert_eq!(first.status, TaskStatus::Pending);
        assert_eq!(first.title, "check the build");
        assert_eq!(first.kind(), hoocode_code_task_store::TaskKind::Schedule);
        assert_eq!(first.next_at, Some(millis(&at(9, 30, 0))));

        // Later ticks keep the row (same id) and its next run until that passes.
        runs.publish_tick(&store, Some(&jobs), &[], &at(8, 5, 0));
        let again = row(&store, "task_a").unwrap();
        assert_eq!(again.id, first.id);
        assert_eq!(again.next_at, first.next_at);
        assert_eq!(store.list().len(), 1);
    }

    #[test]
    fn fire_run_and_finish_move_the_row_through_its_states() {
        let store = TaskStore::new();
        let mut runs = Runs::default();
        let recurring = job("task_a", "30 9 * * *", "check", true);
        let jobs = vec![recurring.clone()];
        runs.publish_tick(&store, Some(&jobs), &[], &at(9, 29, 0));
        let id = row(&store, "task_a").unwrap().id;

        // Fire: queued, and the next run moves on to tomorrow.
        runs.publish_tick(
            &store,
            Some(&jobs),
            std::slice::from_ref(&recurring),
            &at(9, 30, 10),
        );
        let fired = row(&store, "task_a").unwrap();
        assert_eq!(fired.status, TaskStatus::InProgress);
        assert_eq!(fired.note.as_deref(), Some("queued"));
        assert_eq!(fired.id, id);
        let tomorrow = millis(&at(9, 30, 0)) + 86_400_000;
        assert_eq!(fired.next_at, Some(tomorrow));

        // Run: the turn that carries the prompt starts.
        runs.run_started(&store, "check");
        let running = row(&store, "task_a").unwrap();
        assert_eq!(running.note.as_deref(), Some("running"));
        assert_eq!(running.status, TaskStatus::InProgress);

        // Finish: done, with the next run kept.
        runs.run_finished(&store, true, &at(9, 31, 0));
        let done = row(&store, "task_a").unwrap();
        assert_eq!(done.status, TaskStatus::Done);
        assert_eq!(done.note, None);
        assert_eq!(done.id, id);
        assert_eq!(done.next_at, Some(tomorrow));

        // A failed run says so.
        runs.publish_tick(
            &store,
            Some(&jobs),
            std::slice::from_ref(&recurring),
            &at(9, 40, 0),
        );
        runs.run_started(&store, "check");
        runs.run_finished(&store, false, &at(9, 41, 0));
        let failed = row(&store, "task_a").unwrap();
        assert_eq!(failed.status, TaskStatus::Failed);
        assert_eq!(failed.note.as_deref(), Some("run failed"));
    }

    #[test]
    fn a_one_shot_row_ends_with_its_run() {
        let store = TaskStore::new();
        let mut runs = Runs::default();
        let once = job("task_once", "30 9 * * *", "once", false);
        // The store drops a one-shot when it fires: the live list no longer has it.
        runs.publish_tick(
            &store,
            Some(&[]),
            std::slice::from_ref(&once),
            &at(9, 30, 10),
        );
        let queued = row(&store, "task_once").unwrap();
        assert_eq!(queued.status, TaskStatus::InProgress);
        assert_eq!(queued.next_at, None, "a fired one-shot has no next run");
        runs.run_started(&store, "once");
        // Still there while it runs, although the store no longer lists it.
        runs.publish_tick(&store, Some(&[]), &[], &at(9, 31, 0));
        assert!(row(&store, "task_once").is_some());
        runs.run_finished(&store, true, &at(9, 32, 0));
        assert!(row(&store, "task_once").is_none());
    }

    #[test]
    fn removed_jobs_leave_the_store_but_a_running_one_stays_until_done() {
        let store = TaskStore::new();
        let mut runs = Runs::default();
        let a = job("task_a", "* * * * *", "a", true);
        let b = job("task_b", "* * * * *", "b", true);
        runs.publish_tick(&store, Some(&[a.clone(), b.clone()]), &[], &at(9, 0, 0));
        assert_eq!(store.list().len(), 2);

        // b is deleted from the file; a remains.
        runs.publish_tick(&store, Some(std::slice::from_ref(&a)), &[], &at(9, 1, 0));
        assert!(row(&store, "task_b").is_none());
        assert!(row(&store, "task_a").is_some());

        // A job deleted while it runs keeps its row until the run ends.
        runs.publish_tick(&store, Some(&[]), std::slice::from_ref(&a), &at(9, 2, 0));
        assert!(row(&store, "task_a").is_some());
        runs.run_started(&store, "a");
        runs.run_finished(&store, true, &at(9, 2, 30));
        // Its run is over and it is not listed: the next tick removes the row.
        runs.publish_tick(&store, Some(&[]), &[], &at(9, 3, 0));
        assert!(row(&store, "task_a").is_none());

        // A tick that read nothing removes nothing.
        runs.publish_tick(&store, Some(std::slice::from_ref(&a)), &[], &at(9, 4, 0));
        runs.publish_tick(&store, None, &[], &at(9, 5, 0));
        assert!(row(&store, "task_a").is_some());
    }

    #[test]
    fn a_queued_prompt_runs_only_when_its_own_turn_begins() {
        let store = TaskStore::new();
        let mut runs = Runs::default();
        let a = job("task_a", "* * * * *", "a", true);
        let b = job("task_b", "* * * * *", "b", true);
        let jobs = [a.clone(), b.clone()];
        runs.publish_tick(&store, Some(&jobs), &[a.clone(), b.clone()], &at(9, 0, 10));
        assert_eq!(
            row(&store, "task_b").unwrap().note.as_deref(),
            Some("queued")
        );

        // The first prompt's turn begins: b waits behind it, still queued.
        runs.run_started(&store, "a");
        assert_eq!(
            row(&store, "task_a").unwrap().note.as_deref(),
            Some("running")
        );
        assert_eq!(
            row(&store, "task_b").unwrap().note.as_deref(),
            Some("queued")
        );

        // b's own message begins its turn later.
        runs.run_started(&store, "b");
        assert_eq!(
            row(&store, "task_b").unwrap().note.as_deref(),
            Some("running")
        );
        runs.run_finished(&store, true, &at(9, 1, 0));
        assert_eq!(row(&store, "task_b").unwrap().status, TaskStatus::Done);
    }

    #[test]
    fn a_dropped_queued_prompt_is_pending_again_not_queued_forever() {
        let store = TaskStore::new();
        let mut runs = Runs::default();
        let a = job("task_a", "30 9 * * *", "a", true);
        let once = job("task_once", "30 9 * * *", "once", false);
        runs.publish_tick(
            &store,
            Some(std::slice::from_ref(&a)),
            &[a.clone(), once.clone()],
            &at(9, 30, 5),
        );
        runs.run_started(&store, "a");
        // The queue was cleared before "once" ran: its row stops saying queued.
        runs.drop_waiting(&store, &at(9, 30, 40));
        let pending = row(&store, "task_once");
        assert!(
            pending.is_none(),
            "a one-shot job that never ran has no row left"
        );
        let recurring = row(&store, "task_a").unwrap();
        assert_eq!(
            recurring.note.as_deref(),
            Some("running"),
            "the started one is not touched"
        );

        // A recurring job that never ran is pending for its next run.
        let b = job("task_b", "30 9 * * *", "b", true);
        runs.publish_tick(
            &store,
            Some(&[a.clone(), b.clone()]),
            std::slice::from_ref(&b),
            &at(9, 31, 0),
        );
        assert_eq!(
            row(&store, "task_b").unwrap().note.as_deref(),
            Some("queued")
        );
        runs.drop_waiting(&store, &at(9, 31, 5));
        let dropped = row(&store, "task_b").unwrap();
        assert_eq!(dropped.status, TaskStatus::Pending);
        assert_eq!(dropped.note, None);
        assert_eq!(dropped.next_at, Some(millis(&at(9, 30, 0)) + 86_400_000));
    }

    #[test]
    fn a_turn_that_ends_with_jobs_still_waiting_settles_them() {
        let store = TaskStore::new();
        let mut runs = Runs::default();
        let b = job("task_b", "30 9 * * *", "b", true);
        runs.publish_tick(
            &store,
            Some(std::slice::from_ref(&b)),
            std::slice::from_ref(&b),
            &at(9, 30, 5),
        );
        // No turn ever began with "b": the run ends and the row must not say queued.
        runs.run_finished(&store, true, &at(9, 31, 0));
        assert_eq!(row(&store, "task_b").unwrap().status, TaskStatus::Pending);
    }

    #[test]
    fn created_and_deleted_jobs_publish_without_a_tick() {
        let store = TaskStore::new();
        let runs = Runs::default();
        let made = job("task_c", "0 9 * * *", "made by /loop", true);
        publish_created(&store, &made, &at(8, 0, 0));
        assert_eq!(row(&store, "task_c").unwrap().title, "made by /loop");
        publish_removed(&store, "task_c", &runs);
        assert!(row(&store, "task_c").is_none());
        // Removing a job that has no row is harmless.
        publish_removed(&store, "task_none", &runs);
    }
}

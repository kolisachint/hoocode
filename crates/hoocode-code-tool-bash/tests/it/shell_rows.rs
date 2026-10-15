//! Background shell jobs as task-store rows (T5.3): a job's row while it runs,
//! its end (`Done`, `Failed` with the exit status, `Cancelled` when killed), the
//! row's Abort, and the log reads a view makes off the UI thread.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hoocode_code_task_store::{
    ActionNotifier, RowAction, Task, TaskKind, TaskSource, TaskStatus, TaskStore,
};
use hoocode_code_tool_bash::jobs::{read_log, JobState};
use hoocode_code_tool_bash::{get_shell_env, JobRegistry, LocalBashOperations};

const WAIT: Duration = Duration::from_secs(15);

/// A store that lives for the test, with the registry publishing into it.
fn setup(dir: &Path) -> (JobRegistry, &'static TaskStore) {
    let store: &'static TaskStore = Box::leak(Box::new(TaskStore::new()));
    let jobs = JobRegistry::new(dir.join("jobs"), LocalBashOperations::new(None));
    jobs.set_task_rows(store);
    (jobs, store)
}

fn wait_until(mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    done()
}

fn row(store: &TaskStore, key: &str) -> Option<Task> {
    store.find_keyed(key)
}

fn no_notice() -> ActionNotifier {
    Arc::new(|_| {})
}

#[test]
fn a_running_job_is_a_shell_row_with_its_log_and_an_abort() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (jobs, store) = setup(dir.path());
    let info = jobs
        .spawn("sleep 30", dir.path(), get_shell_env())
        .expect("spawn");
    let key = format!("shell:{}", info.id);
    let task = row(store, &key).expect("the job has a row");
    assert_eq!(task.kind(), TaskKind::Shell);
    assert_eq!(task.source, Some(TaskSource::Shell));
    assert_eq!(task.title, "sleep 30");
    assert_eq!(task.log_path.as_deref(), Some(info.log_path.as_path()));
    assert_eq!(task.status, TaskStatus::InProgress);
    assert_eq!(task.action, Some(RowAction::Abort));
    jobs.kill(&info.id).expect("kill");
}

#[test]
fn a_job_that_exits_zero_is_done_and_offers_no_action() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (jobs, store) = setup(dir.path());
    let info = jobs
        .spawn("echo hello", dir.path(), get_shell_env())
        .expect("spawn");
    let key = format!("shell:{}", info.id);
    assert!(wait_until(|| jobs
        .info(&info.id)
        .is_some_and(|i| !i.state.is_running())));
    assert!(wait_until(
        || row(store, &key).is_some_and(|t| t.status == TaskStatus::Done)
    ));
    let task = row(store, &key).expect("row");
    assert_eq!(task.note, None);
    assert_eq!(task.action, None, "a finished job offers nothing");
}

#[test]
fn a_job_that_exits_non_zero_is_failed_with_its_exit_status() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (jobs, store) = setup(dir.path());
    let info = jobs
        .spawn("exit 3", dir.path(), get_shell_env())
        .expect("spawn");
    let key = format!("shell:{}", info.id);
    assert!(wait_until(
        || row(store, &key).is_some_and(|t| t.status == TaskStatus::Failed)
    ));
    assert_eq!(
        row(store, &key).expect("row").note.as_deref(),
        Some("exit 3")
    );
}

#[test]
fn abort_on_the_row_kills_the_job_and_the_row_is_cancelled() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (jobs, store) = setup(dir.path());
    let info = jobs
        .spawn("sleep 30", dir.path(), get_shell_env())
        .expect("spawn");
    let key = format!("shell:{}", info.id);
    let id = row(store, &key).expect("row").id;

    store
        .perform(id, RowAction::Abort, no_notice())
        .expect("the running row offers Abort");
    assert!(
        wait_until(|| jobs
            .info(&info.id)
            .is_some_and(|i| i.state == JobState::Killed)),
        "the job was killed"
    );
    assert!(wait_until(
        || row(store, &key).is_some_and(|t| t.status == TaskStatus::Cancelled)
    ));
    assert_eq!(row(store, &key).expect("row").action, None);
    // Ended, the row no longer offers Abort; the store refuses before the handler runs.
    assert!(store.perform(id, RowAction::Abort, no_notice()).is_err());
}

#[test]
fn a_row_keeps_its_log_after_the_job_ends_and_a_read_follows_appends() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (jobs, store) = setup(dir.path());
    let info = jobs
        .spawn(
            "echo first; sleep 1; echo second",
            dir.path(),
            get_shell_env(),
        )
        .expect("spawn");
    let key = format!("shell:{}", info.id);

    // Read while it runs: the first line is there before the job ends.
    assert!(wait_until(|| jobs
        .read(&info.id, None)
        .is_ok_and(|r| r.text.contains("first"))));
    let early = read_log(&info.log_path, JobState::Running, None).expect("read");
    assert!(early.next <= early.size);

    assert!(wait_until(|| jobs
        .info(&info.id)
        .is_some_and(|i| !i.state.is_running())));
    // The rest of the log, from where the first read stopped.
    let rest = read_log(&info.log_path, JobState::Exited(Some(0)), Some(early.next))
        .expect("read the rest");
    let all = format!("{}{}", early.text, rest.text);
    assert!(all.contains("first") && all.contains("second"), "{all:?}");
    assert_eq!(
        rest.next, rest.size,
        "a finished job's log is read to its end"
    );

    let task = row(store, &key).expect("the finished row stays for its log");
    assert_eq!(task.log_path.as_deref(), Some(info.log_path.as_path()));
}

#[test]
fn a_store_without_rows_still_runs_jobs() {
    let dir = tempfile::tempdir().expect("temp dir");
    let jobs = JobRegistry::new(dir.path().join("jobs"), LocalBashOperations::new(None));
    let info = jobs
        .spawn("echo plain", dir.path(), get_shell_env())
        .expect("spawn");
    assert!(wait_until(|| jobs
        .info(&info.id)
        .is_some_and(|i| !i.state.is_running())));
    // Nothing was published anywhere: the global store has no row for it.
    assert!(hoocode_code_task_store::task_store()
        .find_keyed(&format!("shell:{}", info.id))
        .is_none());
}

#[test]
fn job_ids_are_unique_across_registries_so_their_rows_never_collide() {
    let dir = tempfile::tempdir().expect("temp dir");
    let first = JobRegistry::new(dir.path().join("a"), LocalBashOperations::new(None));
    let second = JobRegistry::new(dir.path().join("b"), LocalBashOperations::new(None));
    let a = first
        .spawn("true", dir.path(), get_shell_env())
        .expect("spawn");
    let b = second
        .spawn("true", dir.path(), get_shell_env())
        .expect("spawn");
    assert_ne!(a.id, b.id);
}

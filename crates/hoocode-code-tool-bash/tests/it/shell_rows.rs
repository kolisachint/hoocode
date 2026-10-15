//! Background shell jobs as task-store rows (T5.3): a job's row while it runs,
//! its end (`Done`, `Failed` with the exit status, `Cancelled` when killed), the
//! row's Abort, and the log reads a view makes off the UI thread.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hoocode_code_task_store::{
    ActionNotifier, RowAction, Task, TaskKind, TaskSource, TaskStatus, TaskStore,
};
use hoocode_code_tool_bash::jobs::{follow_log, output_open, read_log, JobState};
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

#[test]
fn a_job_writes_straight_to_its_log_and_finishes_after_the_registry_is_dropped() {
    let dir = tempfile::tempdir().expect("temp dir");
    let jobs = JobRegistry::new(dir.path().join("jobs"), LocalBashOperations::new(None));
    let info = jobs
        .spawn(
            "if [ -p /dev/stdout ]; then echo pipe; else echo file; fi; \
             for i in $(seq 1 20); do echo tick $i; sleep 0.05; done; echo done",
            dir.path(),
            get_shell_env(),
        )
        .expect("spawn");
    // Nothing of hoocode reads the output once the registry is gone. A job whose
    // output was a pipe would die of SIGPIPE here (the Leave-running quit case).
    drop(jobs);
    assert!(
        wait_until(|| std::fs::read_to_string(&info.log_path).is_ok_and(|t| t.contains("done"))),
        "the job finished writing"
    );
    let log = std::fs::read_to_string(&info.log_path).expect("log");
    assert!(log.starts_with("file\n"), "stdout is the log file: {log:?}");
    assert!(log.contains("tick 20"), "{log:?}");
}

#[test]
fn a_new_job_never_appends_to_an_older_log_of_the_same_number() {
    let dir = tempfile::tempdir().expect("temp dir");
    let folder = dir.path().join("jobs");
    std::fs::create_dir_all(&folder).expect("folder");
    // Logs of job numbers 1 to 300, as an earlier process would have left them.
    for n in 1..=300 {
        std::fs::write(folder.join(format!("job-{n}.log")), "old\n").expect("old log");
    }
    let jobs = JobRegistry::new(&folder, LocalBashOperations::new(None));
    let info = jobs
        .spawn("echo new", dir.path(), get_shell_env())
        .expect("spawn");
    let number: u64 = info
        .id
        .strip_prefix("job-")
        .and_then(|n| n.parse().ok())
        .expect("a job number");
    assert!(
        number > 300,
        "the id skips the logs that exist: {}",
        info.id
    );
    assert!(wait_until(|| jobs
        .info(&info.id)
        .is_some_and(|i| !i.state.is_running())));
    for n in 1..=300 {
        let old = std::fs::read_to_string(folder.join(format!("job-{n}.log"))).expect("old");
        assert_eq!(old, "old\n", "job-{n}.log was not appended to");
    }
    let new = std::fs::read_to_string(&info.log_path).expect("new log");
    assert_eq!(new, "new\n");
}

#[test]
fn a_follower_catches_up_within_one_call_and_skips_ahead_when_far_behind() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("job-9.log");
    // 60 KiB is more than one read returns, so one call reads it all.
    std::fs::write(&path, "x".repeat(60 * 1024)).expect("write");
    let read = follow_log(&path, JobState::Exited(Some(0)), Some(0)).expect("follow");
    assert_eq!(read.from, 0);
    assert_eq!(read.text.len(), 60 * 1024, "both chunks in one call");
    assert_eq!(read.next, read.size);

    // 1 MiB behind: the follower starts at the tail, and the text it skipped is not returned.
    std::fs::write(&path, "a".repeat(1024 * 1024)).expect("write");
    let read = follow_log(&path, JobState::Running, Some(0)).expect("follow");
    assert!(read.from > 0, "skipped to the tail: {}", read.from);
    assert_eq!(read.next, read.size, "caught up to the end");
}

/// T5.3 (review, low): the shell exits, but a background child still writes to
/// the job's output. The output stays open (and is followed) until that child
/// is gone, not until the shell exits.
#[test]
fn output_written_after_the_shell_exits_is_still_followed() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (jobs, _store) = setup(dir.path());
    let go = dir.path().join("go");
    // The shell prints one line and exits. Its child waits for `go`, then prints one more.
    let command = format!(
        "echo early; (while [ ! -f '{}' ]; do sleep 0.01; done; echo late) &",
        go.display()
    );
    let info = jobs
        .spawn(&command, dir.path(), get_shell_env())
        .expect("spawn");
    assert!(wait_until(|| jobs
        .info(&info.id)
        .is_some_and(|i| !i.state.is_running())));

    // The shell is gone, but the output is still open: the child holds the log.
    assert_eq!(output_open(&info.log_path), Some(true));
    let read = follow_log(&info.log_path, JobState::Exited(Some(0)), None).expect("follow");
    assert!(read.open, "a read after the shell exit still reports open");

    std::fs::write(&go, "").expect("release the child");
    // Follow until the output reports closed and is read to its end.
    let mut text = String::new();
    let mut since = None;
    let deadline = Instant::now() + WAIT;
    loop {
        let read = follow_log(&info.log_path, JobState::Exited(Some(0)), since).expect("follow");
        text.push_str(&read.text);
        since = Some(read.next);
        if !read.open && read.next >= read.size {
            break;
        }
        assert!(Instant::now() < deadline, "the output never closed");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(text, "early\nlate\n", "the late line was followed");
    assert_eq!(output_open(&info.log_path), Some(false));
}

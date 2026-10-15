//! Background shell jobs: the registry (spawn, log file, exit status, kill,
//! offset reads, completions) and the Shell, ShellOutput and ShellKill tools.

use std::path::Path;
use std::time::{Duration, Instant};

use hoocode_agent_types::AgentToolResult;
use hoocode_ai_types::Content;
use hoocode_code_tool_api::ToolDefinition;
use hoocode_code_tool_bash::{
    completion_notice, create_bash_tool_definition, create_shell_kill_tool_definition,
    create_shell_output_tool_definition, get_shell_env, BashToolOptions, JobRegistry, JobState,
    LocalBashOperations,
};
use serde_json::{json, Value};

const WAIT: Duration = Duration::from_secs(15);

fn registry(dir: &Path) -> JobRegistry {
    JobRegistry::new(dir.join("jobs"), LocalBashOperations::new(None))
}

/// Polls until `done` holds or the wait runs out; returns whether it held.
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

fn spawn(jobs: &JobRegistry, cwd: &Path, command: &str) -> String {
    jobs.spawn(command, cwd, get_shell_env())
        .expect("spawn a background job")
        .id
}

fn wait_for_exit(jobs: &JobRegistry, id: &str) -> JobState {
    assert!(
        wait_until(|| jobs.info(id).is_some_and(|i| !i.state.is_running())),
        "job {id} did not finish"
    );
    jobs.info(id).expect("job is recorded").state
}

fn text_of(result: &AgentToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|c| match c {
            Content::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect()
}

fn run(tool: &ToolDefinition, args: Value) -> Result<AgentToolResult, String> {
    (tool.execute)("call".into(), args, None, None, None).map_err(|e| e.to_string())
}

fn shell_with_jobs(cwd: &Path, jobs: &JobRegistry) -> ToolDefinition {
    create_bash_tool_definition(
        cwd,
        BashToolOptions {
            jobs: Some(jobs.clone()),
            ..Default::default()
        },
    )
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

#[test]
fn spawn_writes_stdout_and_stderr_to_the_log_and_exits_zero() {
    let tmp = tempfile::tempdir().unwrap();
    let jobs = registry(tmp.path());
    let id = spawn(&jobs, tmp.path(), "echo hello; echo oops >&2");

    assert_eq!(wait_for_exit(&jobs, &id), JobState::Exited(Some(0)));
    let info = jobs.info(&id).unwrap();
    assert_eq!(info.command, "echo hello; echo oops >&2");
    assert_eq!(
        info.log_path,
        tmp.path().join("jobs").join(format!("{id}.log"))
    );
    let log = std::fs::read_to_string(&info.log_path).unwrap();
    assert!(log.contains("hello"), "{log}");
    assert!(log.contains("oops"), "{log}");
}

#[test]
fn exit_status_is_recorded() {
    let tmp = tempfile::tempdir().unwrap();
    let jobs = registry(tmp.path());
    let id = spawn(&jobs, tmp.path(), "exit 3");
    assert_eq!(wait_for_exit(&jobs, &id), JobState::Exited(Some(3)));
}

#[test]
fn kill_stops_a_running_job_and_records_killed() {
    let tmp = tempfile::tempdir().unwrap();
    let jobs = registry(tmp.path());
    let id = spawn(&jobs, tmp.path(), "sleep 30");
    assert_eq!(jobs.running_jobs().len(), 1);

    let started = Instant::now();
    let info = jobs.kill(&id).expect("kill a running job");
    assert_eq!(info.state, JobState::Killed);
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(jobs.running_jobs().is_empty());
}

#[test]
fn kill_of_a_finished_job_leaves_its_state() {
    let tmp = tempfile::tempdir().unwrap();
    let jobs = registry(tmp.path());
    let id = spawn(&jobs, tmp.path(), "true");
    wait_for_exit(&jobs, &id);
    assert_eq!(jobs.kill(&id).unwrap().state, JobState::Exited(Some(0)));
}

#[test]
fn kill_all_stops_every_running_job() {
    let tmp = tempfile::tempdir().unwrap();
    let jobs = registry(tmp.path());
    let a = spawn(&jobs, tmp.path(), "sleep 30");
    let b = spawn(&jobs, tmp.path(), "sleep 30");
    assert_eq!(jobs.kill_all(), 2);
    assert_eq!(wait_for_exit(&jobs, &a), JobState::Killed);
    assert_eq!(wait_for_exit(&jobs, &b), JobState::Killed);
    assert_eq!(jobs.kill_all(), 0, "nothing is running any more");
}

#[test]
fn reads_resume_from_an_offset() {
    let tmp = tempfile::tempdir().unwrap();
    let jobs = registry(tmp.path());
    let id = spawn(&jobs, tmp.path(), "printf abc; sleep 0.5; printf def");

    assert!(wait_until(|| jobs
        .read(&id, Some(0))
        .is_ok_and(|r| r.text.contains("abc"))));
    let first = jobs.read(&id, Some(0)).unwrap();
    assert_eq!(first.from, 0);
    assert!(first.next >= 3 && first.next <= 6, "{first:?}");

    assert_eq!(wait_for_exit(&jobs, &id), JobState::Exited(Some(0)));
    let rest = jobs.read(&id, Some(3)).unwrap();
    assert_eq!(rest.text, "def");
    assert_eq!(rest.from, 3);
    assert_eq!(rest.next, 6);
    assert_eq!(rest.size, 6);
    assert_eq!(rest.state, JobState::Exited(Some(0)));

    // Past the end: nothing new, and the offset stays at the end.
    let none = jobs.read(&id, Some(99)).unwrap();
    assert_eq!(none.text, "");
    assert_eq!(none.from, 6);
    assert_eq!(none.next, 6);
}

#[test]
fn a_read_never_ends_inside_a_utf8_character() {
    let tmp = tempfile::tempdir().unwrap();
    let jobs = registry(tmp.path());
    // "é" is two bytes, C3 A9; the first byte arrives alone at first.
    let id = spawn(
        &jobs,
        tmp.path(),
        "printf '\\303'; sleep 0.5; printf '\\251'",
    );

    assert!(wait_until(|| jobs
        .read(&id, Some(0))
        .is_ok_and(|r| r.size >= 1)));
    let partial = jobs.read(&id, Some(0)).unwrap();
    assert_eq!(partial.text, "");
    assert_eq!(
        partial.next, 0,
        "the read must stop before the split character"
    );

    wait_for_exit(&jobs, &id);
    let whole = jobs.read(&id, Some(partial.next)).unwrap();
    assert_eq!(whole.text, "é");
    assert_eq!(whole.next, 2);
}

#[test]
fn a_read_without_an_offset_returns_the_tail() {
    let tmp = tempfile::tempdir().unwrap();
    let jobs = registry(tmp.path());
    let id = spawn(&jobs, tmp.path(), "printf head; printf tail");
    wait_for_exit(&jobs, &id);
    let read = jobs.read(&id, None).unwrap();
    assert!(read.text.ends_with("headtail"), "{read:?}");
    assert_eq!(read.next, read.size);
}

#[test]
fn completions_are_announced_once() {
    let tmp = tempfile::tempdir().unwrap();
    let jobs = registry(tmp.path());
    // One after the other, so the finish order is fixed.
    let a = spawn(&jobs, tmp.path(), "exit 0");
    wait_for_exit(&jobs, &a);
    let b = spawn(&jobs, tmp.path(), "exit 2");
    wait_for_exit(&jobs, &b);

    let first = jobs.take_completions();
    let ids: Vec<&str> = first.iter().map(|j| j.id.as_str()).collect();
    assert_eq!(ids, vec![a.as_str(), b.as_str()]);
    assert!(jobs.take_completions().is_empty());

    let notice = completion_notice(&first[1]);
    assert!(notice.contains(&b), "{notice}");
    assert!(notice.contains("finished (exit 2)"), "{notice}");
    assert!(notice.contains("ShellOutput"), "{notice}");
}

#[test]
fn a_killed_job_is_announced_as_killed() {
    let tmp = tempfile::tempdir().unwrap();
    let jobs = registry(tmp.path());
    let id = spawn(&jobs, tmp.path(), "sleep 30");
    jobs.kill(&id).unwrap();
    let completions = jobs.take_completions();
    assert_eq!(completions.len(), 1);
    assert!(completion_notice(&completions[0]).contains("was killed"));
}

#[test]
fn a_missing_working_directory_records_no_job() {
    let tmp = tempfile::tempdir().unwrap();
    let jobs = registry(tmp.path());
    let missing = tmp.path().join("nope");
    let err = jobs
        .spawn("true", &missing, get_shell_env())
        .expect_err("spawn in a missing directory");
    assert!(
        err.to_string().contains("Working directory does not exist"),
        "{err}"
    );
    assert!(jobs.running_jobs().is_empty());
    assert!(jobs.take_completions().is_empty());
}

#[test]
fn an_unknown_id_is_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let jobs = registry(tmp.path());
    assert!(jobs.read("job-404", None).is_err());
    assert!(jobs.kill("job-404").is_err());
}

// ---------------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------------

#[test]
fn shell_schema_has_run_in_background_only_with_jobs() {
    let tmp = tempfile::tempdir().unwrap();
    let plain = create_bash_tool_definition(tmp.path(), BashToolOptions::default());
    assert!(plain.parameters["properties"]
        .get("run_in_background")
        .is_none());
    assert!(!plain.description.contains("run_in_background"));

    let jobs = registry(tmp.path());
    let with_jobs = shell_with_jobs(tmp.path(), &jobs);
    assert_eq!(
        with_jobs.parameters["properties"]["run_in_background"]["type"],
        "boolean"
    );
    assert!(with_jobs.description.contains("run_in_background"));
    assert_eq!(with_jobs.parameters["required"], json!(["command"]));
}

#[test]
fn shell_without_jobs_ignores_run_in_background() {
    let tmp = tempfile::tempdir().unwrap();
    let tool = create_bash_tool_definition(tmp.path(), BashToolOptions::default());
    let result = run(
        &tool,
        json!({"command": "echo foreground", "run_in_background": true}),
    )
    .expect("runs in the foreground");
    assert!(
        text_of(&result).contains("foreground"),
        "{}",
        text_of(&result)
    );
}

#[test]
fn shell_run_in_background_returns_a_job_id_at_once() {
    let tmp = tempfile::tempdir().unwrap();
    let jobs = registry(tmp.path());
    let tool = shell_with_jobs(tmp.path(), &jobs);

    let started = Instant::now();
    let result = run(
        &tool,
        json!({"command": "sleep 30", "run_in_background": true, "timeout": 0.1}),
    )
    .expect("starts the job");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "must not wait for the job"
    );

    let text = text_of(&result);
    let info = jobs.running_jobs().pop().expect("the job is running");
    assert!(text.contains(&info.id), "{text}");
    assert!(
        text.contains("ShellOutput") && text.contains("ShellKill"),
        "{text}"
    );
    assert_eq!(result.details["jobId"], json!(info.id));
    jobs.kill(&info.id).unwrap();
}

#[test]
fn shell_output_and_shell_kill_drive_a_background_job() {
    let tmp = tempfile::tempdir().unwrap();
    let jobs = registry(tmp.path());
    let shell = shell_with_jobs(tmp.path(), &jobs);
    let output = create_shell_output_tool_definition(jobs.clone());
    let kill = create_shell_kill_tool_definition(jobs.clone());

    let started = run(
        &shell,
        json!({"command": "echo first; sleep 30", "run_in_background": true}),
    )
    .unwrap();
    let id = started.details["jobId"].as_str().unwrap().to_owned();

    // The first line shows up in the output while the job still runs.
    assert!(wait_until(
        || run(&output, json!({"id": id.clone()})).is_ok_and(|r| text_of(&r).contains("first"))
    ));
    let read = run(&output, json!({"id": id.clone()})).unwrap();
    assert!(
        text_of(&read).contains("[job-") && text_of(&read).contains("running"),
        "{}",
        text_of(&read)
    );

    let stopped = run(&kill, json!({"id": id.clone()})).unwrap();
    assert!(
        text_of(&stopped).contains(&format!("{id} is killed")),
        "{}",
        text_of(&stopped)
    );

    // A finished job reports its exit, and the completion waits for the model.
    let after = run(&output, json!({"id": id.clone()})).unwrap();
    assert!(text_of(&after).contains("killed"), "{}", text_of(&after));
    let completions = jobs.take_completions();
    assert_eq!(completions.len(), 1);
    assert_eq!(completions[0].id, id);
}

#[test]
fn shell_output_resumes_from_since() {
    let tmp = tempfile::tempdir().unwrap();
    let jobs = registry(tmp.path());
    let id = spawn(&jobs, tmp.path(), "printf abc; sleep 0.3; printf def");
    wait_for_exit(&jobs, &id);
    let output = create_shell_output_tool_definition(jobs.clone());

    let whole = run(&output, json!({"id": id.clone(), "since": 0})).unwrap();
    assert!(text_of(&whole).starts_with("abcdef"), "{}", text_of(&whole));
    assert!(
        text_of(&whole).contains("pass since 6"),
        "{}",
        text_of(&whole)
    );

    let rest = run(&output, json!({"id": id.clone(), "since": 3})).unwrap();
    assert!(text_of(&rest).starts_with("def"), "{}", text_of(&rest));
}

#[test]
fn shell_output_and_kill_reject_bad_arguments() {
    let tmp = tempfile::tempdir().unwrap();
    let jobs = registry(tmp.path());
    let output = create_shell_output_tool_definition(jobs.clone());
    let kill = create_shell_kill_tool_definition(jobs.clone());

    assert!(run(&output, json!({})).is_err());
    assert!(run(&output, json!({"id": ""})).is_err());
    assert!(run(&output, json!({"id": "job-1", "since": -1})).is_err());
    assert!(run(&output, json!({"id": "job-404"})).is_err());
    assert!(run(&kill, json!({})).is_err());
    assert!(run(&kill, json!({"id": "job-404"})).is_err());
}

#[test]
fn the_opt_in_tools_are_named_as_the_model_sees_them() {
    let tmp = tempfile::tempdir().unwrap();
    let jobs = registry(tmp.path());
    assert_eq!(
        create_shell_output_tool_definition(jobs.clone()).name,
        "ShellOutput"
    );
    assert_eq!(create_shell_kill_tool_definition(jobs).name, "ShellKill");
}

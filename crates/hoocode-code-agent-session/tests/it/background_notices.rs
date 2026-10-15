//! Background job completion notices (`JobNotices`): each finished job is handed
//! to one model call, and a notice whose call did not get a good reply is handed
//! to the next call again.

use std::path::Path;
use std::time::{Duration, Instant};

use hoocode_agent_types::AgentMessage;
use hoocode_ai_types::{AssistantMessage, Content, StopReason, UserMessage};
use hoocode_code_agent_session::services::JobNotices;
use hoocode_code_tool_bash::{get_shell_env, JobInfo, JobRegistry, LocalBashOperations};

fn registry(dir: &Path) -> JobRegistry {
    JobRegistry::new(dir.join("jobs"), LocalBashOperations::new(None))
}

/// Runs a job to its end and returns its id.
fn finished_job(jobs: &JobRegistry, dir: &Path) -> String {
    let id = jobs.spawn("true", dir, get_shell_env()).expect("spawn").id;
    let deadline = Instant::now() + Duration::from_secs(15);
    while jobs.info(&id).is_some_and(|i| i.state.is_running()) {
        assert!(Instant::now() < deadline, "job {id} did not finish");
        std::thread::sleep(Duration::from_millis(10));
    }
    id
}

fn ids(notices: &[JobInfo]) -> Vec<String> {
    notices.iter().map(|j| j.id.clone()).collect()
}

fn user(text: &str) -> AgentMessage {
    AgentMessage::User(UserMessage {
        content: vec![Content::text(text)].into(),
        timestamp: 0,
    })
}

fn reply(stop_reason: StopReason) -> AgentMessage {
    AgentMessage::Assistant(AssistantMessage {
        stop_reason,
        ..Default::default()
    })
}

#[test]
fn a_notice_is_handed_to_one_call_when_its_reply_is_ok() {
    let tmp = tempfile::tempdir().unwrap();
    let jobs = registry(tmp.path());
    let notices = JobNotices::new(jobs.clone());
    let a = finished_job(&jobs, tmp.path());

    let first = vec![user("go")];
    assert_eq!(ids(&notices.for_call(&first)), vec![a]);

    let second = vec![user("go"), reply(StopReason::Stop), user("again")];
    assert!(notices.for_call(&second).is_empty());
}

#[test]
fn a_notice_is_queued_again_after_an_error_reply_and_keeps_its_order() {
    let tmp = tempfile::tempdir().unwrap();
    let jobs = registry(tmp.path());
    let notices = JobNotices::new(jobs.clone());
    let a = finished_job(&jobs, tmp.path());
    assert_eq!(ids(&notices.for_call(&[user("go")])), vec![a.clone()]);

    // The request failed; a later job finishes before the user tries again.
    let b = finished_job(&jobs, tmp.path());
    let retry = vec![user("go"), reply(StopReason::Error), user("try again")];
    assert_eq!(ids(&notices.for_call(&retry)), vec![a.clone(), b.clone()]);

    // Once a good reply comes, nothing is queued any more.
    let done = vec![
        user("go"),
        reply(StopReason::Error),
        user("try again"),
        reply(StopReason::Stop),
    ];
    assert!(notices.for_call(&done).is_empty());
}

#[test]
fn a_notice_is_queued_again_after_an_abort_or_when_the_call_got_no_reply() {
    let tmp = tempfile::tempdir().unwrap();
    let jobs = registry(tmp.path());
    let notices = JobNotices::new(jobs.clone());
    let a = finished_job(&jobs, tmp.path());
    let first = vec![user("go")];
    assert_eq!(ids(&notices.for_call(&first)), vec![a.clone()]);

    // The stream failed before any reply was added: the transcript is unchanged.
    assert_eq!(ids(&notices.for_call(&first)), vec![a.clone()]);

    // The user aborted the reply.
    let aborted = vec![user("go"), reply(StopReason::Aborted), user("next")];
    assert_eq!(ids(&notices.for_call(&aborted)), vec![a]);
}

#[test]
fn a_compacted_transcript_counts_as_delivered() {
    let tmp = tempfile::tempdir().unwrap();
    let jobs = registry(tmp.path());
    let notices = JobNotices::new(jobs.clone());
    let a = finished_job(&jobs, tmp.path());
    let long = vec![user("1"), user("2"), user("3")];
    assert_eq!(ids(&notices.for_call(&long)), vec![a]);

    // Compaction shortened the transcript after the call went through.
    assert!(notices.for_call(&[user("summary")]).is_empty());
}

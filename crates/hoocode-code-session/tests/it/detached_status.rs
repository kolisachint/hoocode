//! The status of a detached session, read from hand-written session files (SA-12).

use std::path::{Path, PathBuf};

use hoocode_code_session::{detached_status, DetachedStatus, SessionLock};

const HEADER: &str =
    r#"{"type":"session","version":3,"id":"s1","timestamp":"2026-11-25T00:00:00Z","cwd":"/work"}"#;

fn temp_file(tag: &str, lines: &[&str]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "detached-status-{tag}-{}-{}",
        std::process::id(),
        hoocode_code_session::create_session_id()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("2026_s1.jsonl");
    let mut text = String::from(HEADER);
    text.push('\n');
    for line in lines {
        text.push_str(line);
        text.push('\n');
    }
    std::fs::write(&path, text).expect("session file");
    path
}

fn exit_entry(id: &str, data: &str) -> String {
    format!(
        r#"{{"type":"custom","id":"{id}","parentId":null,"timestamp":"2026-11-25T00:00:01Z","customType":"detached_exit","data":{data}}}"#
    )
}

fn parked_entry(id: &str, call: &str) -> String {
    format!(
        r#"{{"type":"custom","id":"{id}","parentId":null,"timestamp":"2026-11-25T00:00:02Z","customType":"parked_call","data":{{"toolCallId":"{call}"}}}}"#
    )
}

fn tool_result_entry(id: &str, call: &str) -> String {
    format!(
        r#"{{"type":"message","id":"{id}","parentId":null,"timestamp":"2026-11-25T00:00:03Z","message":{{"role":"toolResult","toolCallId":"{call}","toolName":"Shell","content":[{{"type":"text","text":"ok"}}],"isError":false,"timestamp":0}}}}"#
    )
}

fn status_of(tag: &str, lines: &[String]) -> DetachedStatus {
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    detached_status(&temp_file(tag, &refs))
}

#[test]
fn a_done_run_reads_done() {
    let status = status_of(
        "done",
        &[exit_entry("e1", r#"{"status":"done","reason":null}"#)],
    );
    assert_eq!(status, DetachedStatus::Done);
}

#[test]
fn a_failed_run_reads_its_reason() {
    let status = status_of(
        "failed",
        &[exit_entry(
            "e1",
            r#"{"status":"failed","reason":"rate limited"}"#,
        )],
    );
    assert_eq!(
        status,
        DetachedStatus::Failed {
            reason: "rate limited".into()
        }
    );
}

#[test]
fn a_failed_run_without_a_reason_says_failed() {
    let status = status_of(
        "noreason",
        &[exit_entry("e1", r#"{"status":"failed","reason":null}"#)],
    );
    assert_eq!(
        status,
        DetachedStatus::Failed {
            reason: "failed".into()
        }
    );
}

#[test]
fn the_last_exit_record_wins() {
    let status = status_of(
        "last",
        &[
            exit_entry("e1", r#"{"status":"failed","reason":"first try"}"#),
            exit_entry("e2", r#"{"status":"done","reason":null}"#),
        ],
    );
    assert_eq!(status, DetachedStatus::Done);
}

#[test]
fn a_file_with_no_exit_record_crashed() {
    let status = status_of("crashed", &[]);
    assert_eq!(
        status,
        DetachedStatus::Failed {
            reason: "exited without a result (crashed or killed)".into()
        }
    );
}

#[test]
fn an_open_parked_call_reads_parked() {
    let status = status_of(
        "parked",
        &[tool_result_entry("m1", "t0"), parked_entry("p1", "t9")],
    );
    assert_eq!(status, DetachedStatus::Parked);
}

#[test]
fn a_parked_call_with_a_later_result_is_not_open() {
    let status = status_of(
        "answered",
        &[
            parked_entry("p1", "t9"),
            tool_result_entry("m1", "t9"),
            exit_entry("e1", r#"{"status":"done","reason":null}"#),
        ],
    );
    assert_eq!(status, DetachedStatus::Done);
}

#[test]
fn a_result_before_the_park_does_not_answer_it() {
    let status = status_of(
        "earlier",
        &[tool_result_entry("m1", "t9"), parked_entry("p1", "t9")],
    );
    assert_eq!(status, DetachedStatus::Parked);
}

#[test]
fn a_held_lock_reads_running_even_with_an_exit_record() {
    let path = temp_file(
        "running",
        &[&exit_entry("e1", r#"{"status":"done","reason":null}"#)],
    );
    let lock = SessionLock::acquire(&path).expect("lock");
    assert_eq!(
        detached_status(&path),
        DetachedStatus::Running {
            pid: std::process::id()
        }
    );
    drop(lock);
    assert_eq!(detached_status(&path), DetachedStatus::Done);
}

#[test]
fn a_missing_file_crashed() {
    let path = Path::new("/nonexistent/hoocode-detached/none.jsonl");
    assert_eq!(
        detached_status(path),
        DetachedStatus::Failed {
            reason: "exited without a result (crashed or killed)".into()
        }
    );
}

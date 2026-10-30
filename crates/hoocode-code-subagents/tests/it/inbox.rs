#![allow(clippy::disallowed_methods)] // test code: #[tokio::test] expands to a runtime builder
//! subagent-inbox.test.ts. The inbox is process-wide, so the cases share one
//! test (serialized) like the TS file's afterEach(clear).

use std::time::Duration;

use hoocode_code_subagents::inbox::{subagent_inbox, CancelAsk, TaskLifecycle};
use hoocode_code_subagents::pool::{ResultStatus, SubagentResult, TaskResult};
use serde_json::json;

fn ok_result(task_id: &str, summary: &str) -> TaskResult {
    TaskResult {
        task_id: Some(task_id.into()),
        agent_type: Some("explore".into()),
        result: Some(SubagentResult {
            task_id: task_id.into(),
            ok: true,
            exit_code: Some(0),
            status: Some(ResultStatus::Complete),
            result_data: json!({"summary": summary, "files_changed": [], "confidence": 0.9, "status": "complete"})
                .as_object()
                .cloned(),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn fail_result(task_id: &str, status: ResultStatus, error: &str) -> TaskResult {
    TaskResult {
        task_id: Some(task_id.into()),
        agent_type: Some("explore".into()),
        result: Some(SubagentResult {
            task_id: task_id.into(),
            exit_code: Some(1),
            status: Some(status),
            error: Some(error.into()),
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[tokio::test]
async fn subagent_inbox_lifecycle() {
    // The inbox is process-wide: take the same lock the other modules take.
    let _serial = crate::SERIAL.lock().await;
    let inbox = subagent_inbox();

    // allocates monotonic per-agent labels
    inbox.clear();
    assert_eq!(inbox.next_label("explore"), "explore#1");
    assert_eq!(inbox.next_label("explore"), "explore#2");
    assert_eq!(inbox.next_label("plan"), "plan#1");

    // a background call's label is reserved at its placeholder, in call order,
    // and its execute takes that one back whatever order the executes start in
    inbox.clear();
    assert_eq!(
        inbox
            .reserve_label("call_0", "explore", "auth", None)
            .as_deref(),
        Some("explore#1")
    );
    assert_eq!(
        inbox
            .reserve_label("call_1", "explore", "db", None)
            .as_deref(),
        Some("explore#2")
    );
    assert_eq!(
        inbox.take_label("call_1", "explore", "db", None),
        "explore#2"
    );
    assert_eq!(
        inbox.take_label("call_0", "explore", "auth", None),
        "explore#1"
    );
    // taken once: a reused call id is a new run
    assert_eq!(
        inbox.take_label("call_0", "explore", "auth", None),
        "explore#3"
    );
    // an id repeated while still reserved (within one message) gets no handle
    assert_eq!(
        inbox
            .reserve_label("call_2", "explore", "a", None)
            .as_deref(),
        Some("explore#4")
    );
    assert_eq!(inbox.reserve_label("call_2", "explore", "b", None), None);
    // A reservation is always taken by its call id, even with other
    // arguments (the execute's validated ones): the model was told that
    // handle. The changed description replaces the reserved one.
    assert_eq!(
        inbox.take_label("call_2", "explore", "b", None),
        "explore#4"
    );
    assert_eq!(
        inbox.get("explore#4").unwrap().description.as_deref(),
        Some("b")
    );
    assert_eq!(
        inbox.take_label("call_2", "explore", "a", None),
        "explore#5"
    );
    // a call that ends without starting its reserved run leaves no ghost,
    // and the reason says whether the run had started
    inbox.reserve_label("call_3", "plan", "p", None);
    let rec = inbox.settle_abandoned(None, "plan#1").unwrap();
    assert_eq!(rec.lifecycle, TaskLifecycle::Failed);
    assert!(rec.error.as_deref().unwrap().starts_with("not started"));
    assert!(inbox.settle_abandoned(None, "plan#1").is_none());
    inbox.start("bound", "plan#2", "plan", "q");
    let rec = inbox.settle_abandoned(None, "plan#2").unwrap();
    assert!(
        rec.error
            .as_deref()
            .unwrap()
            .starts_with("stopped unexpectedly"),
        "{rec:?}"
    );

    // reservations are per session: the same call id in two sessions is two
    // calls, and neither takes the other's handle
    inbox.clear();
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.jsonl").to_string_lossy().into_owned();
    let b = dir.path().join("b.jsonl").to_string_lossy().into_owned();
    assert_eq!(
        inbox
            .reserve_label("call_0", "explore", "in a", Some(&a))
            .as_deref(),
        Some("explore#1")
    );
    assert_eq!(
        inbox
            .reserve_label("call_0", "explore", "in b", Some(&b))
            .as_deref(),
        Some("explore#1"),
        "not refused as a repeat of a's call"
    );
    assert_eq!(
        inbox.take_label("call_0", "explore", "in b", Some(&b)),
        "explore#1"
    );
    assert_eq!(
        inbox
            .get_in(Some(&b), "explore#1")
            .unwrap()
            .description
            .as_deref(),
        Some("in b")
    );
    assert_eq!(
        inbox
            .get_in(Some(&a), "explore#1")
            .unwrap()
            .description
            .as_deref(),
        Some("in a"),
        "a's reservation is untouched"
    );
    assert_eq!(
        inbox.take_label("call_0", "explore", "in a", Some(&a)),
        "explore#1"
    );
    assert_eq!(
        inbox.take_label("call_0", "explore", "again", Some(&a)),
        "explore#2"
    );

    // a run carries its description, shown next to its label
    inbox.clear();
    inbox.start("t1", "explore#1", "explore", "");
    assert_eq!(inbox.get("t1").unwrap().named(), "explore#1");
    inbox.start("t2", "explore#2", "explore", "  audit the db layer ");
    assert_eq!(
        inbox.get("explore#2").unwrap().named(),
        "explore#2 (\"audit the db layer\")"
    );

    // tracks a task from running to done and keeps the body until collected
    inbox.clear();
    inbox.start("t1", "explore#1", "explore", "");
    assert_eq!(inbox.get("t1").unwrap().lifecycle, TaskLifecycle::Running);
    assert_eq!(inbox.outstanding().len(), 1);
    inbox.finish(
        "t1",
        &ok_result("t1", "Found the bug in foo.ts:42\nmore detail"),
    );
    let done = inbox.get("t1").unwrap();
    assert_eq!(done.lifecycle, TaskLifecycle::Done);
    assert_eq!(
        done.summary_line.as_deref(),
        Some("Found the bug in foo.ts:42")
    );
    assert!(inbox.outstanding().is_empty());
    let (_, body) = inbox.collect("t1").unwrap();
    assert!(body.contains("more detail"));
    assert_eq!(inbox.get("t1").unwrap().lifecycle, TaskLifecycle::Collected);
    assert!(inbox.collect("t1").is_none());
    assert_eq!(
        inbox.get("t1").unwrap().summary_line.as_deref(),
        Some("Found the bug in foo.ts:42")
    );

    // resolves a handle by task id or by label
    inbox.clear();
    inbox.start("abc123", "plan#1", "plan", "");
    assert_eq!(inbox.get("abc123").unwrap().label, "plan#1");
    assert_eq!(inbox.get("plan#1").unwrap().task_id, "abc123");
    assert!(inbox.get("nope").is_none());

    // maps failure status onto stalled/timeout/failed
    inbox.clear();
    inbox.start("f1", "explore#1", "explore", "");
    inbox.finish(
        "f1",
        &fail_result("f1", ResultStatus::Stalled, "no heartbeat"),
    );
    let failed = inbox.get("f1").unwrap();
    assert_eq!(failed.lifecycle, TaskLifecycle::Stalled);
    assert_eq!(failed.error.as_deref(), Some("no heartbeat"));
    assert!(inbox.collect("f1").is_none());

    // settles a task with no TaskResult via fail()
    inbox.clear();
    inbox.start("e1", "explore#1", "explore", "");
    inbox.fail("e1", "dispatch threw", TaskLifecycle::Failed);
    let e1 = inbox.get("e1").unwrap();
    assert_eq!(e1.lifecycle, TaskLifecycle::Failed);
    assert_eq!(e1.summary_line.as_deref(), Some("dispatch threw"));

    // the wait helpers resolve on settle, or at the timeout
    inbox.clear();
    inbox.start("w1", "explore#1", "explore", "");
    let waiter = tokio::spawn(async {
        subagent_inbox()
            .wait_for("explore#1", Duration::from_secs(5))
            .await
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    inbox.finish("w1", &ok_result("w1", "done"));
    assert_eq!(
        waiter.await.unwrap().unwrap().lifecycle,
        TaskLifecycle::Done
    );
    inbox.start("w2", "explore#2", "explore", "");
    let started = std::time::Instant::now();
    inbox.wait_for_all(Duration::from_millis(50)).await;
    assert!(started.elapsed() >= Duration::from_millis(50));
    inbox.fail("w2", "gone", TaskLifecycle::Failed);
    inbox.wait_for_all(Duration::from_secs(5)).await;

    // keeps at most 50 settled records, never pruning running ones
    inbox.clear();
    inbox.start("live", "explore#1", "explore", "");
    for i in 0..60 {
        let id = format!("s{i}");
        inbox.start(&id, &format!("x#{i}"), "explore", "");
        inbox.fail(&id, "x", TaskLifecycle::Failed);
    }
    inbox.start("last", "explore#2", "explore", "");
    let listed = inbox.list();
    assert!(listed.iter().any(|r| r.task_id == "live"));
    assert_eq!(
        listed
            .iter()
            .filter(|r| r.lifecycle == TaskLifecycle::Failed)
            .count(),
        50
    );
    inbox.clear();
}

#[tokio::test]
async fn session_bindings_are_capped_like_records() {
    let _serial = crate::SERIAL.lock().await;
    let inbox = subagent_inbox();
    inbox.clear();
    // Binding the same label again does not count as a new entry.
    inbox.bind_label("explore#1", "t-first");
    inbox.bind_label("explore#1", "t-first-again");
    assert_eq!(
        inbox.session_id_for("explore#1").as_deref(),
        Some("t-first-again")
    );
    for i in 2..=60 {
        inbox.bind_label(&format!("explore#{i}"), &format!("t{i}"));
    }
    // The oldest bindings are gone; the newest 50 still resolve.
    assert_eq!(inbox.session_id_for("explore#1"), None);
    assert_eq!(inbox.session_id_for("explore#10"), None);
    assert_eq!(inbox.session_id_for("explore#11").as_deref(), Some("t11"));
    assert_eq!(inbox.session_id_for("explore#60").as_deref(), Some("t60"));
    inbox.clear();
}

/// The race the reviewer measured (2 of 13 eval runs): the model was told
/// `explore#2` at the placeholder, asked for the roster before the detached
/// execute had registered the run, and was told no subagent existed. The run
/// is now listed from the placeholder on, and its execute binds it in place.
#[tokio::test]
async fn a_reserved_run_is_listed_and_bound_in_place() {
    let _serial = crate::SERIAL.lock().await;
    let inbox = subagent_inbox();
    inbox.clear();
    inbox.reserve_label("call_0", "explore", "audit the auth module", None);
    inbox.reserve_label("call_1", "explore", "audit the db layer", None);
    let listed = inbox.list();
    assert_eq!(listed.len(), 2);
    assert!(listed
        .iter()
        .all(|r| r.starting && r.lifecycle == TaskLifecycle::Running));
    assert_eq!(inbox.outstanding().len(), 2);
    let rec = inbox
        .get("explore#2")
        .expect("listed before its execute starts");
    assert_eq!(rec.description.as_deref(), Some("audit the db layer"));

    // The second call's execute starts first; it binds its own record.
    let label = inbox.take_label("call_1", "explore", "audit the db layer", None);
    let bound = inbox.start_in(None, "run-db", &label, "explore", "audit the db layer");
    assert!(!bound.starting);
    assert_eq!(bound.task_id, "run-db");
    let order: Vec<String> = inbox.list().iter().map(|r| r.label.clone()).collect();
    assert_eq!(order, ["explore#1", "explore#2"], "bound in its place");
    assert_eq!(inbox.get("run-db").unwrap().label, "explore#2");
    inbox.clear();
}

#[tokio::test]
async fn a_run_cancelled_while_starting_never_starts() {
    let _serial = crate::SERIAL.lock().await;
    let inbox = subagent_inbox();
    inbox.clear();
    inbox.reserve_label("call_0", "explore", "audit", None);
    match inbox.request_cancel(None, "explore#1") {
        Some(CancelAsk::BeforeStart(rec)) => {
            assert_eq!(rec.lifecycle, TaskLifecycle::Cancelled)
        }
        other => panic!("{other:?}"),
    }
    let label = inbox.take_label("call_0", "explore", "audit", None);
    let rec = inbox.start_in(None, "run-1", &label, "explore", "audit");
    assert_eq!(
        rec.lifecycle,
        TaskLifecycle::Cancelled,
        "the execute must not dispatch it"
    );
    // A running run is flagged for its execute to stop through the pool.
    inbox.start("run-2", "explore#2", "explore", "");
    assert!(matches!(
        inbox.request_cancel(None, "explore#2"),
        Some(CancelAsk::Running(_))
    ));
    assert!(inbox.cancel_requested("run-2"));
    assert!(matches!(
        inbox.request_cancel(None, "explore#1"),
        Some(CancelAsk::Settled(_))
    ));
    assert_eq!(inbox.request_cancel(None, "explore#9"), None);
    inbox.clear();
}

/// A reserved run whose execute refused the call is settled with the reason.
#[tokio::test]
async fn a_reserved_run_that_never_started_fails_with_its_reason() {
    let _serial = crate::SERIAL.lock().await;
    let inbox = subagent_inbox();
    inbox.clear();
    inbox.reserve_label("call_0", "nope", "x", None);
    let rec = inbox
        .fail_reserved(None, "nope#1", "Unknown subagent_type")
        .unwrap();
    assert_eq!(rec.lifecycle, TaskLifecycle::Failed);
    assert_eq!(
        inbox.get("nope#1").unwrap().error.as_deref(),
        Some("Unknown subagent_type")
    );
    assert_eq!(inbox.outstanding().len(), 0);
    inbox.clear();
}

/// Runs, labels and bindings are per session: `/new` and app-server sessions
/// do not see each other's runs, and each numbers its own.
#[tokio::test]
async fn runs_and_labels_are_scoped_to_their_session() {
    let _serial = crate::SERIAL.lock().await;
    let inbox = subagent_inbox();
    inbox.clear();
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.jsonl").to_string_lossy().into_owned();
    let b = dir.path().join("b.jsonl").to_string_lossy().into_owned();
    assert_eq!(inbox.next_label_in(Some(&a), "explore"), "explore#1");
    assert_eq!(inbox.next_label_in(Some(&b), "explore"), "explore#1");
    inbox.start_in(Some(&a), "run-a", "explore#1", "explore", "in a");
    inbox.start_in(Some(&b), "run-b", "explore#1", "explore", "in b");
    assert_eq!(inbox.list_in(Some(&a)).len(), 1);
    assert_eq!(inbox.outstanding_in(Some(&b)).len(), 1);
    assert_eq!(
        inbox.get_in(Some(&b), "explore#1").unwrap().task_id,
        "run-b"
    );
    assert!(inbox.get_in(None, "explore#1").is_none());
    inbox.bind_label_in(Some(&a), "explore#1", "run-a");
    assert_eq!(inbox.session_id_for_in(Some(&b), "explore#1"), None);
    assert_eq!(
        inbox.session_id_for_in(Some(&a), "explore#1").as_deref(),
        Some("run-a")
    );
    inbox.clear();
}

/// A resumed session (a new process) continues its label numbers after the
/// handles already in its file, so `explore#1` never names two runs, and an
/// old handle is recognised as a run of an earlier process.
#[tokio::test]
async fn a_resumed_session_continues_its_handles() {
    let _serial = crate::SERIAL.lock().await;
    let inbox = subagent_inbox();
    inbox.clear();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("s.jsonl");
    std::fs::write(
        &file,
        concat!(
            "{\"content\":\"explore#1 (\\\"audit\\\") started in the background\"}\n",
            "{\"content\":\"AgentOutput(\\\"explore#3\\\")  general-purpose#2 finished\"}\n",
        ),
    )
    .unwrap();
    let session = file.to_string_lossy().into_owned();
    assert_eq!(
        inbox
            .reserve_label("c", "explore", "next", Some(&session))
            .as_deref(),
        Some("explore#4")
    );
    assert_eq!(
        inbox.next_label_in(Some(&session), "general-purpose"),
        "general-purpose#3"
    );
    assert_eq!(inbox.next_label_in(Some(&session), "plan"), "plan#1");
    assert!(inbox.is_earlier_run(Some(&session), "explore#1"));
    assert!(inbox.is_earlier_run(Some(&session), "explore#3"));
    assert!(
        !inbox.is_earlier_run(Some(&session), "explore#4"),
        "live in this process"
    );
    assert!(!inbox.is_earlier_run(Some(&session), "explore#9"));
    assert!(!inbox.is_earlier_run(None, "explore#1"));
    inbox.clear();
}

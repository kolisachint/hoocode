#![allow(clippy::disallowed_methods)] // test code: threads to play the parent, #[tokio::test]
//! The clearance relay between a subagent child and its parent: the child's
//! end (`ChildRelay`) and the pool's end, over a scripted runner.

use std::io::{BufReader, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, Instant};

use hoocode_code_permissions::{ClearanceRelay, RelayRequest};
use hoocode_code_subagents::clearance::{
    parse_response, request_line, response_line, ChildRelay, NO_UI_REASON,
};
use hoocode_code_subagents::pool::{SubagentPool, SubagentPoolOptions, SubagentPoolTask};
use hoocode_code_subagents::runner::{ScriptedRunner, ScriptedStep};
use serde_json::{json, Value};

// ---- the child's end -------------------------------------------------------

/// A writer the test can read back.
#[derive(Clone, Default)]
struct Wire(Arc<Mutex<Vec<u8>>>);

impl Wire {
    fn lines(&self) -> Vec<String> {
        String::from_utf8_lossy(&self.0.lock().unwrap())
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// The request lines written so far, parsed.
    fn requests(&self) -> Vec<Value> {
        self.lines()
            .iter()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|v| v["type"] == "clearance_request")
            .collect()
    }
}

impl Write for Wire {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A stdin the test feeds line by line; dropping the sender is EOF.
struct Feed(Receiver<Vec<u8>>, Vec<u8>);

impl Read for Feed {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.1.is_empty() {
            match self.0.recv() {
                Ok(bytes) => self.1 = bytes,
                Err(_) => return Ok(0),
            }
        }
        let n = buf.len().min(self.1.len());
        buf[..n].copy_from_slice(&self.1[..n]);
        self.1.drain(..n);
        Ok(n)
    }
}

fn feed() -> (Sender<Vec<u8>>, BufReader<Feed>) {
    let (tx, rx) = channel();
    (tx, BufReader::new(Feed(rx, Vec::new())))
}

fn wait_for<T>(what: &str, mut probe: impl FnMut() -> Option<T>) -> T {
    let started = Instant::now();
    loop {
        if let Some(found) = probe() {
            return found;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "timed out: {what}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn ask_on_thread(
    relay: &Arc<ChildRelay>,
    request: RelayRequest,
) -> std::thread::JoinHandle<Option<String>> {
    let relay = relay.clone();
    std::thread::spawn(move || relay.ask(&request))
}

fn shell(command: &str) -> RelayRequest {
    RelayRequest {
        tool: "Shell".into(),
        input: json!({"command": command}),
        ..Default::default()
    }
}

#[test]
fn the_wire_format_round_trips() {
    let line = request_line(7, "Shell", &json!({"command": "ls"}), Some("explore t1"));
    let v: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(
        v,
        json!({"type": "clearance_request", "id": 7, "tool": "Shell",
               "input": {"command": "ls"}, "via": "explore t1"})
    );
    let no_via: Value = serde_json::from_str(&request_line(1, "Edit", &json!({}), None)).unwrap();
    assert!(no_via.get("via").is_none());

    assert_eq!(parse_response(&response_line(7, &None)), Some((7, None)));
    assert_eq!(
        parse_response(&response_line(8, &Some("no".into()))),
        Some((8, Some("no".into())))
    );
    // Anything but an explicit allow is a denial; other lines are not answers.
    assert_eq!(
        parse_response(r#"{"type":"clearance_response","id":9}"#),
        Some((9, Some("Denied by permission gate".into())))
    );
    assert_eq!(parse_response(r#"{"ping":true}"#), None);
    assert_eq!(parse_response("not json"), None);
}

#[test]
fn the_child_waits_for_the_parents_answer() {
    let wire = Wire::default();
    let relay = ChildRelay::new(Box::new(wire.clone()), Duration::from_secs(30));
    let (stdin, reader) = feed();
    relay.serve(reader);

    let allowed = ask_on_thread(&relay, shell("ls"));
    let first = wait_for("the first request", || wire.requests().first().cloned());
    assert_eq!(first["tool"], "Shell");
    assert_eq!(first["input"]["command"], "ls");
    assert!(first.get("via").is_none());
    let id = first["id"].as_u64().unwrap();
    stdin
        .send(format!("{}\n", response_line(id, &None)).into_bytes())
        .unwrap();
    assert_eq!(allowed.join().unwrap(), None);

    // A second, concurrent request is answered by its own id.
    let denied = ask_on_thread(
        &relay,
        RelayRequest {
            via: Some("explore t2".into()),
            ..shell("rm -rf /")
        },
    );
    let second = wait_for("the second request", || wire.requests().get(1).cloned());
    assert_eq!(second["via"], "explore t2");
    assert_ne!(second["id"], first["id"]);
    stdin
        .send(
            format!(
                "{}\n",
                response_line(second["id"].as_u64().unwrap(), &Some("not that".into()))
            )
            .into_bytes(),
        )
        .unwrap();
    assert_eq!(denied.join().unwrap().as_deref(), Some("not that"));
}

#[test]
fn the_child_denies_when_its_stdin_ends() {
    let wire = Wire::default();
    let relay = ChildRelay::new(Box::new(wire.clone()), Duration::from_secs(30));
    let (stdin, reader) = feed();
    relay.serve(reader);

    // A request waiting when the parent goes away is denied...
    let waiting = ask_on_thread(&relay, shell("ls"));
    wait_for("the request", || wire.requests().first().cloned());
    drop(stdin);
    let reason = waiting.join().unwrap().expect("denied");
    assert!(reason.contains("no longer reachable"), "{reason}");

    // ...and so is every later one, without being written anywhere.
    let before = wire.requests().len();
    assert!(relay.ask(&shell("ls")).is_some());
    assert_eq!(wire.requests().len(), before);
}

#[test]
fn the_child_denies_at_once_when_its_stdin_is_empty() {
    let relay = ChildRelay::new(Box::new(Wire::default()), Duration::from_secs(30));
    relay.serve(BufReader::new(std::io::empty()));
    assert!(wait_for("denial", || relay.ask(&shell("ls"))).contains("reachable"));
}

#[test]
fn a_waiting_child_keeps_pinging_and_stops_when_cancelled() {
    let wire = Wire::default();
    let relay = ChildRelay::new(Box::new(wire.clone()), Duration::from_millis(20));
    let (_stdin, reader) = feed();
    relay.serve(reader);

    let cancelled = Arc::new(AtomicBool::new(false));
    let waiting = ask_on_thread(
        &relay,
        RelayRequest {
            cancelled: cancelled.clone(),
            ..shell("ls")
        },
    );
    // The run loop may be blocked on this very call, so the relay itself
    // writes the heartbeat that keeps the parent from reaping the child.
    wait_for("a heartbeat", || {
        wire.lines()
            .iter()
            .any(|line| line.contains(r#""ping":true"#))
            .then_some(())
    });
    cancelled.store(true, Ordering::SeqCst);
    let reason = waiting.join().unwrap().expect("denied");
    assert!(reason.contains("ended"), "{reason}");
}

// ---- the pool's end --------------------------------------------------------

fn isolate_agent_dir() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let dir =
            std::env::temp_dir().join(format!("hoocode-clearance-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("HOOCODE_CODING_AGENT_DIR", &dir);
    });
}

/// A relay that records the question and answers with a fixed verdict, or
/// waits until the run ends (`wait_for_cancel`).
struct Parent {
    verdict: Option<String>,
    wait_for_cancel: bool,
    asked: Mutex<Vec<(Option<String>, String, Value)>>,
}

impl Parent {
    fn new(verdict: Option<&str>, wait_for_cancel: bool) -> Arc<Self> {
        Arc::new(Self {
            verdict: verdict.map(str::to_string),
            wait_for_cancel,
            asked: Mutex::new(Vec::new()),
        })
    }
}

impl ClearanceRelay for Parent {
    fn ask(&self, request: &RelayRequest) -> Option<String> {
        self.asked.lock().unwrap().push((
            request.via.clone(),
            request.tool.clone(),
            request.input.clone(),
        ));
        if self.wait_for_cancel {
            wait_for("the run to end", || {
                request.cancelled.load(Ordering::SeqCst).then_some(())
            });
            return Some("run ended".into());
        }
        self.verdict.clone()
    }
}

fn pool(runner: ScriptedRunner, cwd: &Path, relay: Option<Arc<Parent>>) -> SubagentPool {
    isolate_agent_dir();
    SubagentPool::new(SubagentPoolOptions {
        runner: Some(Arc::new(runner)),
        cwd: Some(cwd.to_path_buf()),
        clearance: relay.map(|r| r as Arc<dyn ClearanceRelay>),
        ..Default::default()
    })
}

fn child_asking(id: u64) -> ScriptedRunner {
    let line = request_line(id, "Shell", &json!({"command": "rm -rf x"}), None);
    ScriptedRunner::new(vec![ScriptedStep {
        stdout: vec![format!("{line}\n")],
        hang: true,
        ..Default::default()
    }])
}

fn task() -> SubagentPoolTask {
    SubagentPoolTask {
        task_id: "t1".into(),
        agent_type: "explore".into(),
        task: "work".into(),
        ..Default::default()
    }
}

fn answers(runner: &ScriptedRunner) -> Vec<(u64, Option<String>)> {
    wait_for("an answer on the child's stdin", || {
        let lines = runner.stdin_lines();
        (!lines.is_empty()).then(|| lines.iter().filter_map(|l| parse_response(l)).collect())
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_pool_relays_a_request_and_writes_the_verdict_to_the_childs_stdin() {
    let dir = tempfile::tempdir().unwrap();
    let runner = child_asking(7);
    let parent = Parent::new(None, false);
    let p = pool(runner.clone(), dir.path(), Some(parent.clone()));

    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    p.on(move |event| {
        if event.name == "task_clearance" {
            sink.lock().unwrap().push(event.data.clone());
        }
    });
    p.spawn(task()).unwrap();

    assert_eq!(answers(&runner), [(7, None)]);
    // The relay is asked as the subagent, about the call as the child sent it.
    assert_eq!(
        parent.asked.lock().unwrap().clone(),
        [(
            Some("explore".to_string()),
            "Shell".to_string(),
            json!({"command": "rm -rf x"})
        )]
    );
    // The parent's Agent block hears that the run is held, then released.
    let events = wait_for("both clearance events", || {
        let seen = events.lock().unwrap().clone();
        (seen.len() == 2).then_some(seen)
    });
    assert_eq!(events[0]["held"], true);
    assert_eq!(events[0]["task_id"], "t1");
    assert_eq!(events[0]["tool"], "Shell");
    assert_eq!(events[0]["subject"], "rm -rf x");
    assert_eq!(events[1]["held"], false);
    p.dispose();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_pool_relays_a_denial_with_its_reason() {
    let dir = tempfile::tempdir().unwrap();
    let runner = child_asking(3);
    let p = pool(
        runner.clone(),
        dir.path(),
        Some(Parent::new(Some("nope"), false)),
    );
    p.spawn(task()).unwrap();
    assert_eq!(answers(&runner), [(3, Some("nope".to_string()))]);
    p.dispose();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pool_with_no_relay_denies_without_asking_anyone() {
    let dir = tempfile::tempdir().unwrap();
    let runner = child_asking(5);
    let p = pool(runner.clone(), dir.path(), None);
    p.spawn(task()).unwrap();
    assert_eq!(answers(&runner), [(5, Some(NO_UI_REASON.to_string()))]);
    p.dispose();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_request_stops_waiting_when_its_run_ends() {
    let dir = tempfile::tempdir().unwrap();
    // The scripted child exits at once, so the question is withdrawn.
    let runner = child_asking(9);
    let p = pool(runner.clone(), dir.path(), Some(Parent::new(None, true)));
    p.spawn(task()).unwrap();
    assert_eq!(answers(&runner), [(9, Some("run ended".to_string()))]);
    p.dispose();
}

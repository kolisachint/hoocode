#![allow(clippy::disallowed_methods)] // test code: threads to play the parent, #[tokio::test]
//! The clearance relay between a subagent child and its parent: the child's
//! end (`ChildRelay`) and the pool's end, over a scripted runner.

use std::io::{BufReader, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, Instant};

use hoocode_code_permissions::{ClearanceRelay, EnsureHelperAnswer, RelayRequest};
use hoocode_code_subagents::clearance::{
    ensure_request_line, ensure_response_line, ensure_response_line_full, parse_ensure_response,
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
        via: None,
        tool: "Shell".into(),
        origin: hoocode_agent_types::ToolOrigin::BuiltIn,
        input: json!({"command": command}),
        cancelled: Arc::new(AtomicBool::new(false)),
    }
}

#[test]
fn the_wire_format_round_trips() {
    let builtin = hoocode_agent_types::ToolOrigin::BuiltIn;
    let line = request_line(
        7,
        "Shell",
        &builtin,
        &json!({"command": "ls"}),
        Some("explore t1"),
    );
    let v: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(
        v,
        json!({"type": "clearance_request", "id": 7, "tool": "Shell",
               "origin": {"kind": "builtIn"},
               "input": {"command": "ls"}, "via": "explore t1"})
    );
    // An MCP tool's origin goes on the wire whole: the parent gates on it, not on the name.
    let mcp = hoocode_agent_types::ToolOrigin::Mcp {
        server: "fs".into(),
        tool: "read".into(),
        plugin: None,
    };
    let wire: Value =
        serde_json::from_str(&request_line(2, "mcp__fs__read", &mcp, &json!({}), None)).unwrap();
    assert_eq!(
        wire["origin"],
        json!({"kind": "mcp", "server": "fs", "tool": "read", "plugin": null})
    );
    let no_via: Value =
        serde_json::from_str(&request_line(1, "Edit", &builtin, &json!({}), None)).unwrap();
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
    let line = request_line(
        id,
        "Shell",
        &hoocode_agent_types::ToolOrigin::BuiltIn,
        &json!({"command": "rm -rf x"}),
        None,
    );
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
async fn a_request_without_a_readable_origin_is_denied_and_not_relayed() {
    // The parent gates on the origin, so a line without one cannot be judged: it is denied.
    let dir = tempfile::tempdir().unwrap();
    let runner = ScriptedRunner::new(vec![ScriptedStep {
        stdout: vec![
            "{\"type\":\"clearance_request\",\"id\":5,\"tool\":\"Shell\",\"input\":{\"command\":\"ls\"}}\n"
                .to_string(),
        ],
        hang: true,
        ..Default::default()
    }]);
    let parent = Parent::new(None, false);
    let p = pool(runner.clone(), dir.path(), Some(parent.clone()));
    p.spawn(task()).unwrap();

    let answered = answers(&runner);
    assert_eq!(answered.len(), 1, "{answered:?}");
    assert_eq!(answered[0].0, 5);
    assert!(answered[0].1.is_some(), "{answered:?}");
    assert!(parent.asked.lock().unwrap().is_empty());
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

// ---- EnsureHelper (tier C, spec D5) ----------------------------------------

/// The EnsureHelper wire format round-trips; neither side reads the other's
/// clearance lines.
#[test]
fn the_ensure_wire_format_round_trips() {
    let request: Value = serde_json::from_str(&ensure_request_line(3, "echo")).unwrap();
    assert_eq!(
        request,
        json!({"type": "native_ensure_helper", "id": 3, "plugin": "echo"})
    );
    assert_eq!(
        parse_ensure_response(&ensure_response_line(3, &Ok("/run/echo.sock".into()))),
        Some((3, Ok(("/run/echo.sock".to_string(), None))))
    );
    assert_eq!(
        parse_ensure_response(&ensure_response_line(3, &Err("down".into()))),
        Some((3, Err("down".to_string())))
    );
    // The executable rides along as an optional field: a new parent's answer
    // parses, and an old parent's answer (no `exe`) still does.
    let full =
        ensure_response_line_full(3, &Ok(("/run/echo.sock".into(), Some("/bin/echo".into()))));
    assert_eq!(
        parse_ensure_response(&full),
        Some((
            3,
            Ok(("/run/echo.sock".to_string(), Some("/bin/echo".to_string())))
        ))
    );
    assert_eq!(parse_ensure_response(r#"{"ping":true}"#), None);
    assert_eq!(parse_ensure_response("not json"), None);
    // Each side ignores the other's answers.
    assert_eq!(
        parse_response(&ensure_response_line(3, &Ok("s".into()))),
        None
    );
    assert_eq!(parse_ensure_response(&response_line(3, &None)), None);
}

/// The child asks for its helper's socket and gets it back, by request id.
#[test]
fn the_child_receives_the_helper_socket() {
    let wire = Wire::default();
    let relay = ChildRelay::new(Box::new(wire.clone()), Duration::from_secs(30));
    let (stdin, reader) = feed();
    relay.serve(reader);

    let asked = std::thread::spawn({
        let relay = relay.clone();
        move || relay.ask_ensure("echo")
    });
    let line = wait_for("the ensure request", || {
        wire.lines()
            .into_iter()
            .find(|line| line.contains("native_ensure_helper"))
    });
    let id: u64 = serde_json::from_str::<Value>(&line).unwrap()["id"]
        .as_u64()
        .unwrap();
    stdin
        .send(
            format!(
                "{}\n",
                ensure_response_line(id, &Ok("/run/echo.sock".into()))
            )
            .into_bytes(),
        )
        .unwrap();
    assert_eq!(
        asked.join().unwrap(),
        Ok(("/run/echo.sock".to_string(), None))
    );
}

/// A parent denial (or a closed stdin) is an error, not a socket.
#[test]
fn a_denied_ensure_is_an_error() {
    let wire = Wire::default();
    let relay = ChildRelay::new(Box::new(wire.clone()), Duration::from_secs(30));
    let (stdin, reader) = feed();
    relay.serve(reader);

    let asked = std::thread::spawn({
        let relay = relay.clone();
        move || relay.ask_ensure("echo")
    });
    let line = wait_for("the ensure request", || {
        wire.lines()
            .into_iter()
            .find(|line| line.contains("native_ensure_helper"))
    });
    let id: u64 = serde_json::from_str::<Value>(&line).unwrap()["id"]
        .as_u64()
        .unwrap();
    stdin
        .send(format!("{}\n", ensure_response_line(id, &Err("no helper".into()))).into_bytes())
        .unwrap();
    assert_eq!(asked.join().unwrap(), Err("no helper".to_string()));

    // A closed stdin (the parent is gone) denies every later request at once.
    drop(stdin);
    assert!(relay.ask_ensure("echo").is_err());
}

// ---- the pool's end ----------------------------------------------------------

use std::path::PathBuf;

use hoocode_code_subagents::pool::{HostFuture, NativeChildHost};

/// A scripted child asking for its tier C helper's socket.
fn child_ensuring(id: u64) -> ScriptedRunner {
    let line = ensure_request_line(id, "t3c");
    ScriptedRunner::new(vec![ScriptedStep {
        stdout: vec![format!("{line}\n")],
        hang: true,
        ..Default::default()
    }])
}

/// A [`NativeChildHost`] with a fixed socket and canned env, recording the
/// children it deregisters.
struct FakeHost {
    vars: Vec<(String, String)>,
    ended: Mutex<Vec<String>>,
}

impl FakeHost {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            vars: vec![("HOOCODE_AGENT_ID".into(), "root/t1".into())],
            ended: Mutex::new(Vec::new()),
        })
    }
}

impl NativeChildHost for FakeHost {
    fn child_env(&self, task_id: &str) -> Option<Vec<(String, String)>> {
        assert_eq!(task_id, "t1");
        Some(self.vars.clone())
    }

    fn ensure_helper<'a>(
        &'a self,
        task_id: &'a str,
        plugin: &'a str,
    ) -> HostFuture<'a, Result<(PathBuf, Option<PathBuf>), String>> {
        assert_eq!((task_id, plugin), ("t1", "t3c"));
        Box::pin(async move {
            Ok((
                PathBuf::from("/run/t3c.sock"),
                Some(PathBuf::from("/bin/t3c")),
            ))
        })
    }

    fn child_ended<'a>(&'a self, task_id: &'a str) -> HostFuture<'a, ()> {
        self.ended.lock().unwrap().push(task_id.to_string());
        Box::pin(async move {})
    }
}

fn pool_with_host(runner: ScriptedRunner, cwd: &Path, host: Arc<FakeHost>) -> SubagentPool {
    isolate_agent_dir();
    SubagentPool::new(SubagentPoolOptions {
        runner: Some(Arc::new(runner)),
        cwd: Some(cwd.to_path_buf()),
        native_host: Some(host as Arc<dyn NativeChildHost>),
        ..Default::default()
    })
}

fn ensure_answers(runner: &ScriptedRunner) -> Vec<(u64, EnsureHelperAnswer)> {
    wait_for("an ensure answer on the child's stdin", || {
        let parsed: Vec<_> = runner
            .stdin_lines()
            .iter()
            .filter_map(|line| parse_ensure_response(line))
            .collect();
        (!parsed.is_empty()).then_some(parsed)
    })
}

/// The pool answers EnsureHelper from its host, registering the child first.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_pool_answers_ensure_helper_from_its_host() {
    let dir = tempfile::tempdir().unwrap();
    let runner = child_ensuring(7);
    let p = pool_with_host(runner.clone(), dir.path(), FakeHost::new());
    p.spawn(task()).unwrap();
    assert_eq!(
        ensure_answers(&runner),
        [(
            7,
            Ok(("/run/t3c.sock".to_string(), Some("/bin/t3c".to_string())))
        )]
    );
    p.dispose();
}

/// With no host the session has no shared helper: the request is denied.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ensure_helper_without_a_host_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let runner = child_ensuring(5);
    let p = pool(runner.clone(), dir.path(), None);
    p.spawn(task()).unwrap();
    let answers = ensure_answers(&runner);
    assert_eq!(answers.len(), 1);
    assert_eq!(answers[0].0, 5);
    assert!(answers[0].1.is_err(), "{answers:?}");
    p.dispose();
}

/// A child that ended is deregistered, even when its run failed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_ended_child_is_deregistered() {
    let dir = tempfile::tempdir().unwrap();
    // The scripted child exits at once with no result: the run fails, and the
    // settle still deregisters the child first.
    let line = ensure_request_line(7, "t3c");
    let runner = ScriptedRunner::new(vec![ScriptedStep {
        stdout: vec![format!("{line}\n")],
        ..Default::default()
    }]);
    let host = FakeHost::new();
    let p = pool_with_host(runner.clone(), dir.path(), host.clone());
    p.spawn(task()).unwrap();
    let _ = p.wait_for("t1").await;
    assert_eq!(host.ended.lock().unwrap().clone(), ["t1"]);
    p.dispose();
}

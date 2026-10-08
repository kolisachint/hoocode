#![allow(clippy::disallowed_methods)] // test code: #[tokio::test] expands to a runtime builder
//! MCP login state and elicitation in a session (`docs/design/mcp.md`, steps 3 and 4): a server
//! that answers 401 is `AuthNeeded` and can be logged in to; a question in print mode is declined.

use std::io::BufRead;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hoocode_code_agent_session::mcp::{
    DeclineElicitation, ElicitationAnswer, ElicitationHandler, ElicitationRequest, McpHub,
    McpServerInfo, McpStatus,
};
use hoocode_code_mcp::ConfigSources;

/// A Streamable HTTP endpoint that answers every request with 401 and a bearer challenge, so the
/// server needs a login. Prints its port on the first line.
const UNAUTHORIZED_SERVER: &str = r##"
import http.server
class Handler(http.server.BaseHTTPRequestHandler):
    def _deny(self):
        self.send_response(401)
        self.send_header("WWW-Authenticate", 'Bearer resource_metadata="http://127.0.0.1:9/none"')
        self.send_header("Content-Length", "0")
        self.end_headers()
    do_GET = _deny
    do_POST = _deny
    do_DELETE = _deny
    def log_message(self, *args):
        pass
server = http.server.HTTPServer(("127.0.0.1", 0), Handler)
print(server.server_port, flush=True)
server.serve_forever()
"##;

fn python3_available() -> bool {
    Command::new("python3")
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success())
}

/// Polls the hub until `done` holds, or fails after 20 seconds.
async fn wait_until(hub: &McpHub, done: impl Fn(&[McpServerInfo]) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let servers = hub.servers();
        if done(&servers) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "servers did not reach the expected state: {servers:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn write_json(path: &Path, value: serde_json::Value) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, value.to_string()).unwrap();
}

/// Starts the 401 server; returns its child process and URL.
fn unauthorized_server(dir: &Path) -> (Child, String) {
    let script = dir.join("unauthorized.py");
    std::fs::write(&script, UNAUTHORIZED_SERVER).unwrap();
    let mut child = Command::new("python3")
        .arg(&script)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut port = String::new();
    std::io::BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut port)
        .unwrap();
    (child, format!("http://127.0.0.1:{}/mcp", port.trim()))
}

/// A server that answers 401 is `AuthNeeded`, not `Failed`, so the hub can offer a login. The
/// login refuses unknown servers, and a login that fails reports why and leaves the server
/// `AuthNeeded`.
#[tokio::test(flavor = "multi_thread")]
async fn a_401_server_needs_a_login_and_a_failed_login_reports_why() {
    if !python3_available() {
        eprintln!("skipped: python3 is not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (mut child, url) = unauthorized_server(dir.path());
    let user = dir.path().join("home/.agents/mcp.json");
    write_json(
        &user,
        serde_json::json!({"mcpServers": {"docs": {"url": url}}}),
    );
    let sources = ConfigSources {
        user,
        project_folder: dir.path().join("repo"),
        plugins: Vec::new(),
    };
    let hub = McpHub::load(&sources, dir.path().join("trust/mcp-trust.json"));
    hub.start();
    wait_until(&hub, |s| {
        s.iter()
            .any(|x| x.name == "docs" && x.status == McpStatus::AuthNeeded)
    })
    .await;

    let refused = hub.login("nope", |_| {}).expect_err("no such server");
    assert!(refused.contains("No MCP server named nope"), "{refused}");

    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = events.clone();
    hub.login("docs", move |m| sink.lock().unwrap().push(m))
        .expect("the login starts");
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let done = events
            .lock()
            .unwrap()
            .iter()
            .any(|m| m.starts_with("Login to docs failed"));
        if done {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "no outcome: {:?}",
            events.lock().unwrap()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // The failed login leaves the server where it was, ready for another try.
    assert!(hub
        .servers()
        .iter()
        .any(|x| x.name == "docs" && x.status == McpStatus::AuthNeeded));
    hub.shutdown();
    let _ = child.kill();
    let _ = child.wait();
}

/// Print and rpc modes: a server's question is declined, and the decline is reported.
#[tokio::test]
async fn a_question_in_print_mode_is_declined_and_reported() {
    let reports = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = reports.clone();
    let handler = DeclineElicitation::new(move |m| sink.lock().unwrap().push(m));
    let answer = handler
        .elicit(
            "docs",
            ElicitationRequest::Form {
                message: "Pick".to_owned(),
                schema: serde_json::json!({"type": "object"}),
            },
        )
        .await;
    assert_eq!(answer, ElicitationAnswer::Decline);
    let reports = reports.lock().unwrap();
    assert_eq!(reports.len(), 1);
    assert!(
        reports[0].contains("MCP server docs asked for input"),
        "{reports:?}"
    );
}

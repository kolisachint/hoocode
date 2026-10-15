#![allow(clippy::disallowed_methods)] // test code: #[tokio::test] expands to a runtime builder
//! MCP rows in the task store (`docs/design/tui-activity.md`, T4.3): a server that needs a login
//! has a row, its `Login` action runs the code path of `/mcp login`, and the rows go when the
//! session ends.

use std::io::BufRead;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hoocode_code_agent_session::mcp::{McpHub, McpServerInfo, McpStatus};
use hoocode_code_agent_session::mcp_rows::server_key;
use hoocode_code_mcp::ConfigSources;
use hoocode_code_task_store::{RowAction, TaskStatus, TaskStore};

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

/// Polls `check` until it yields a value, or fails after 20 seconds.
async fn wait_for<T>(what: &str, mut check: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(value) = check() {
            return value;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The server row's id, once the server is in `status`.
fn row_in(store: &TaskStore, name: &str, status: TaskStatus) -> Option<u64> {
    store
        .find_keyed(&server_key(name))
        .filter(|row| row.status == status)
        .map(|row| row.id)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_needs_a_login_has_a_row_whose_login_is_the_slash_command_path() {
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
    // The session's store is process-wide in the TUI; a leaked one is the same shape for a test.
    let store: &'static TaskStore = Box::leak(Box::new(TaskStore::new()));
    hub.set_task_rows(store);
    hub.start();

    let id = wait_for("the login row", || {
        row_in(store, "docs", TaskStatus::Pending)
    })
    .await;
    let row = store.find_keyed(&server_key("docs")).unwrap();
    assert_eq!(row.id, id);
    assert_eq!(row.action, Some(RowAction::Login));
    assert_eq!(row.note.as_deref(), Some("needs a login"));

    // A server row offers no abort.
    let refused = store
        .perform(id, RowAction::Abort, Arc::new(|_| {}))
        .unwrap_err();
    assert!(refused.contains("no Abort action"), "{refused}");

    // Login from the row runs the same path as `/mcp login docs`: the hub reports the outcome
    // to the notifier it was given.
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = events.clone();
    store
        .perform(
            id,
            RowAction::Login,
            Arc::new(move |message| sink.lock().unwrap().push(message)),
        )
        .expect("the login starts");
    wait_for("the login outcome", || {
        events
            .lock()
            .unwrap()
            .iter()
            .any(|m| m.starts_with("Login to docs failed"))
            .then_some(())
    })
    .await;

    // The failed login leaves the server where it was, and its row too.
    let still: McpServerInfo = hub
        .servers()
        .into_iter()
        .find(|s| s.name == "docs")
        .unwrap();
    assert_eq!(still.status, McpStatus::AuthNeeded);
    assert!(row_in(store, "docs", TaskStatus::Pending).is_some());

    // The session ends: its rows go.
    hub.shutdown();
    assert!(
        store.list().is_empty(),
        "rows left after shutdown: {:?}",
        store.list()
    );
    let _ = child.kill();
    let _ = child.wait();
}

//! The echo tier C helper fixture used by the hoocode-code-native tests. Not a
//! product binary.
//!
//! Test-only: it is an example of `hoocode-agent-mcp`, so it builds with the
//! crate's dev-dependencies and is never shipped. The `hoocode-code-native`
//! supervisor tests (and later end-to-end tests in `hoocode-code-main`) build
//! it in test setup with
//! `cargo build -p hoocode-agent-mcp --example echo_helper`
//! and run it at `target/debug/examples/echo_helper` (or `$CARGO_TARGET_DIR`
//! when set). See `docs/design/plugins.md`, Native tier contract.
//!
//! The helper side of the contract:
//!
//! - Reads `HOOCODE_HELPER_SOCKET` (Unix socket path to bind), `HOOCODE_HELPER_TOKEN`
//!   (the root bearer token, pre-registered as the root agent) and `HOOCODE_AGENT_ID`
//!   (default `root`). `HOOCODE_ECHO_RECEIPT_DIR`, when set, receives a
//!   `session-ending.received` marker file on `hoocode/sessionEnding` (lets tests
//!   observe the notification without racing the runtime-dir removal).
//! - Serves rmcp Streamable HTTP on the socket, like the
//!   `tests/it/unix_socket.rs` server. Every request needs
//!   `Authorization: Bearer <known token>`; anything else gets HTTP 401.
//! - `token -> {agentId, parentId, depth}` map. `hoocode/agentStarted`
//!   `{agentId, parentId, depth, token}` registers only when the sender is known
//!   and `parentId` equals the sender's own agentId; `hoocode/agentEnded`
//!   removes by agentId; `hoocode/sessionEnding` exits 0.
//! - Tools: `echo {text}` returns `{text, agentId, parentId, depth, metaAgentId}`
//!   (`metaAgentId` is `_meta["dev.hoocode/agent"].agentId`); `crash` exits the
//!   process without answering (restart tests); `sleep {ms}` waits, then answers.
//! - Exits when `--parent-pid <pid>` is gone (polled every 500 ms).

use std::collections::HashMap;
use std::convert::Infallible;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use hyper_util::service::TowerToHyperService;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, CustomNotification,
    ErrorData, ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::{NotificationContext, RequestContext};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{RoleServer, ServerHandler};
use serde_json::{Map, Value};

/// One registered agent: what the supervisor sent in `hoocode/agentStarted`.
#[derive(Clone, Debug)]
struct AgentRecord {
    agent_id: String,
    parent_id: Option<String>,
    depth: u64,
}

/// Test receipts: the dir from `HOOCODE_ECHO_RECEIPT_DIR`, when set. Lets
/// end-to-end tests observe registrations, calls and shutdowns without
/// racing the runtime-dir removal.
fn receipt_dir() -> Option<PathBuf> {
    std::env::var("HOOCODE_ECHO_RECEIPT_DIR").ok().map(PathBuf::from)
}

/// File-safe agent ids (`root/task-1` becomes `root__task-1`).
fn sanitize(agent_id: &str) -> String {
    agent_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

fn write_receipt(name: &str, body: &str) {
    if let Some(dir) = receipt_dir() {
        let _ = std::fs::write(dir.join(name), body);
    }
}

/// Per-call counter, so concurrent calls leave one receipt each.
static CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Helper boot time: call receipts carry start/end offsets from it, so tests
/// can prove calls overlapped.
static BOOT: std::sync::LazyLock<std::time::Instant> =
    std::sync::LazyLock::new(std::time::Instant::now);

/// Milliseconds since the helper booted.
fn now_ms() -> u64 {
    BOOT.elapsed().as_millis().min(u64::MAX as u128) as u64
}

type Tokens = Arc<Mutex<HashMap<String, AgentRecord>>>;

/// Bearer token of a request, from its `Authorization` header.
fn bearer_of(parts: &http::request::Parts) -> Option<String> {
    let value = parts.headers.get(http::header::AUTHORIZATION)?;
    let value = value.to_str().ok()?;
    value.strip_prefix("Bearer ").map(str::to_owned)
}

/// The token map lookup for an MCP handler context.
fn sender_of(tokens: &Tokens, parts: &http::request::Parts) -> Option<(String, AgentRecord)> {
    let token = bearer_of(parts)?;
    let map = tokens.lock().expect("tokens");
    map.get(&token).cloned().map(|record| (token, record))
}

/// True when the request carries a known bearer token.
fn is_authorized<T>(request: &Request<T>, tokens: &Tokens) -> bool {
    let Some(value) = request.headers().get(http::header::AUTHORIZATION) else {
        return false;
    };
    let Some(token) = value.to_str().ok().and_then(|v| v.strip_prefix("Bearer ")) else {
        return false;
    };
    tokens.lock().expect("tokens").contains_key(token)
}

/// HTTP 401 for an unknown or missing token.
fn unauthorized_response() -> Response<BoxBody<Bytes, Infallible>> {
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .body(
            Full::new(Bytes::from_static(b"unknown token"))
                .map_err(|never| match never {})
                .boxed(),
        )
        .expect("401 response")
}

/// The rmcp service behind an HTTP-401 bearer check. rmcp has no auth layer of
/// its own, so unknown tokens are rejected here, before rmcp sees them.
#[derive(Clone)]
struct AuthService<S> {
    tokens: Tokens,
    inner: S,
}

impl<S> hyper::service::Service<Request<Incoming>> for AuthService<S>
where
    S: hyper::service::Service<
            Request<Incoming>,
            Response = Response<BoxBody<Bytes, Infallible>>,
            Error = Infallible,
        > + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
{
    type Response = Response<BoxBody<Bytes, Infallible>>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Infallible>> + Send>>;

    fn call(&self, request: Request<Incoming>) -> Self::Future {
        let inner = self.inner.clone();
        let tokens = self.tokens.clone();
        Box::pin(async move {
            if !is_authorized(&request, &tokens) {
                return Ok(unauthorized_response());
            }
            inner.call(request).await
        })
    }
}

/// The echo MCP server: identity comes from the token map, `metaAgentId` from
/// `_meta["dev.hoocode/agent"]`.
#[derive(Clone)]
struct EchoServer {
    tokens: Tokens,
    receipt_dir: Option<PathBuf>,
}

impl EchoServer {
    fn agent_response(
        &self,
        tool: &str,
        start_ms: u64,
        parts: &http::request::Parts,
        meta: &Value,
        mut extra: Map<String, Value>,
    ) -> Result<CallToolResponse, ErrorData> {
        let Some((_token, record)) = sender_of(&self.tokens, parts) else {
            return Err(ErrorData::new(
                rmcp::model::ErrorCode::INVALID_REQUEST,
                "unknown token".to_string(),
                None,
            ));
        };
        let meta_agent = meta
            .get("dev.hoocode/agent")
            .and_then(|agent| agent.get("agentId"))
            .cloned()
            .unwrap_or(Value::Null);
        // One receipt per call, so tests can observe concurrent calls with
        // their resolved identities.
        let n = CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        write_receipt(
            &format!("call-{n}.json"),
            &serde_json::json!({
                "tool": tool,
                "agentId": record.agent_id,
                "parentId": record.parent_id,
                "depth": record.depth,
                "metaAgentId": meta_agent,
                "startMs": start_ms,
                "endMs": now_ms(),
            })
            .to_string(),
        );
        extra.insert("agentId".to_owned(), Value::String(record.agent_id));
        extra.insert(
            "parentId".to_owned(),
            record.parent_id.map(Value::String).unwrap_or(Value::Null),
        );
        extra.insert("depth".to_owned(), Value::Number(record.depth.into()));
        extra.insert("metaAgentId".to_owned(), meta_agent);
        Ok(
            CallToolResult::success(vec![ContentBlock::text(Value::Object(extra).to_string())])
                .into(),
        )
    }
}

fn object_schema(
    properties: &[(&str, &str)],
    required: &[&str],
) -> std::sync::Arc<Map<String, Value>> {
    let mut props = Map::new();
    for (name, kind) in properties {
        props.insert(
            (*name).to_owned(),
            Value::Object(
                [("type".to_owned(), Value::String((*kind).to_owned()))]
                    .into_iter()
                    .collect(),
            ),
        );
    }
    std::sync::Arc::new(
        [
            ("type".to_owned(), Value::String("object".to_owned())),
            ("properties".to_owned(), Value::Object(props)),
            (
                "required".to_owned(),
                Value::Array(
                    required
                        .iter()
                        .map(|name| Value::String((*name).to_owned()))
                        .collect(),
                ),
            ),
        ]
        .into_iter()
        .collect(),
    )
}

impl ServerHandler for EchoServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(vec![
            Tool::new(
                "echo",
                "Returns its text argument with the caller identity.",
                object_schema(&[("text", "string")], &["text"]),
            ),
            Tool::new(
                "crash",
                "Exits the helper process without answering.",
                object_schema(&[], &[]),
            ),
            Tool::new(
                "sleep",
                "Waits ms milliseconds, then answers.",
                object_schema(&[("ms", "integer")], &["ms"]),
            ),
        ]))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let parts = context.extensions.get::<http::request::Parts>().cloned();
        let Some(parts) = parts else {
            return Err(ErrorData::new(
                rmcp::model::ErrorCode::INTERNAL_ERROR,
                "no request parts".to_string(),
                None,
            ));
        };
        let meta = serde_json::to_value(&context.meta).unwrap_or(Value::Null);
        match request.name.as_ref() {
            "echo" => {
                let start_ms = now_ms();
                let text = request
                    .arguments
                    .as_ref()
                    .and_then(|args| args.get("text"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let mut extra = Map::new();
                extra.insert("text".to_owned(), Value::String(text));
                self.agent_response("echo", start_ms, &parts, &meta, extra)
            }
            "crash" => {
                // Exit without answering, so the in-flight call fails and the
                // supervisor restarts the helper.
                std::process::exit(1);
            }
            "sleep" => {
                let start_ms = now_ms();
                let ms = request
                    .arguments
                    .as_ref()
                    .and_then(|args| args.get("ms"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
                let mut extra = Map::new();
                extra.insert("sleptMs".to_owned(), Value::Number(ms.into()));
                self.agent_response("sleep", start_ms, &parts, &meta, extra)
            }
            name => Err(ErrorData::new(
                rmcp::model::ErrorCode::METHOD_NOT_FOUND,
                format!("unknown tool {name}"),
                None,
            )),
        }
    }

    async fn on_custom_notification(
        &self,
        notification: CustomNotification,
        context: NotificationContext<RoleServer>,
    ) {
        let parts = context.extensions.get::<http::request::Parts>().cloned();
        let Some(parts) = parts else { return };
        match notification.method.as_str() {
            "hoocode/agentStarted" => {
                let Some((_token, sender)) = sender_of(&self.tokens, &parts) else {
                    return;
                };
                let params = notification.params.unwrap_or(Value::Null);
                let agent_id = params
                    .get("agentId")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let parent_id = params
                    .get("parentId")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let depth = params.get("depth").and_then(Value::as_u64).unwrap_or(0);
                let token = params
                    .get("token")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                if agent_id.is_empty() || token.is_empty() {
                    return;
                }
                // Helper-side rule (spec D5): the sender may only register its
                // own children.
                if parent_id.as_deref() != Some(sender.agent_id.as_str()) {
                    return;
                }
                self.tokens.lock().expect("tokens").insert(
                    token,
                    AgentRecord {
                        agent_id: agent_id.clone(),
                        parent_id: parent_id.clone(),
                        depth,
                    },
                );
                write_receipt(
                    &format!("agent-started-{}.received", sanitize(&agent_id)),
                    &serde_json::json!({
                        "agentId": agent_id,
                        "parentId": parent_id,
                        "depth": depth,
                    })
                    .to_string(),
                );
            }
            "hoocode/agentEnded" => {
                if sender_of(&self.tokens, &parts).is_none() {
                    return;
                }
                let params = notification.params.unwrap_or(Value::Null);
                if let Some(agent_id) = params.get("agentId").and_then(Value::as_str) {
                    let mut map = self.tokens.lock().expect("tokens");
                    map.retain(|_, record| record.agent_id != agent_id);
                    write_receipt(
                        &format!("agent-ended-{}.received", sanitize(agent_id)),
                        agent_id,
                    );
                }
            }
            "hoocode/sessionEnding" => {
                if let Some(dir) = &self.receipt_dir {
                    let _ = std::fs::write(dir.join("session-ending.received"), "sessionEnding");
                }
                std::process::exit(0);
            }
            _ => {}
        }
    }
}

fn env_required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| {
        eprintln!("echo_helper: {name} is required");
        std::process::exit(2);
    })
}

/// True while the parent process still exists.
fn parent_alive(pid: u32) -> bool {
    // SAFETY: kill with signal 0 only checks, it sends nothing.
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if result == 0 {
        return true;
    }
    let error = std::io::Error::last_os_error();
    // EPERM: a process exists that we may not signal. Anything else (ESRCH,
    // EINVAL) means it is gone.
    error.raw_os_error() == Some(libc::EPERM)
}

// Test helper process only: it starts its own runtime, like any helper server.
#[cfg(unix)]
#[allow(clippy::disallowed_methods)]
#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let mut parent_pid: Option<u32> = None;
    while let Some(arg) = args.next() {
        if arg == "--parent-pid" {
            let pid: u32 = args
                .next()
                .unwrap_or_else(|| {
                    eprintln!("echo_helper: --parent-pid needs a value");
                    std::process::exit(2);
                })
                .parse()
                .unwrap_or_else(|_| {
                    eprintln!("echo_helper: --parent-pid needs a numeric value");
                    std::process::exit(2);
                });
            parent_pid = Some(pid);
        } else {
            eprintln!("echo_helper: unknown argument {arg}");
            std::process::exit(2);
        }
    }
    let Some(parent_pid) = parent_pid else {
        eprintln!("echo_helper: --parent-pid <pid> is required");
        std::process::exit(2);
    };

    let socket = PathBuf::from(env_required("HOOCODE_HELPER_SOCKET"));
    let root_token = env_required("HOOCODE_HELPER_TOKEN");
    let root_agent = std::env::var("HOOCODE_AGENT_ID").unwrap_or_else(|_| "root".to_owned());
    let receipt_dir = std::env::var("HOOCODE_ECHO_RECEIPT_DIR")
        .ok()
        .map(PathBuf::from);

    let tokens: Tokens = Arc::new(Mutex::new(HashMap::from([(
        root_token,
        AgentRecord {
            agent_id: root_agent,
            parent_id: None,
            depth: 0,
        },
    )])));

    // One boot receipt per helper process, so tests can count starts.
    write_receipt(
        &format!("helper-boot-{}.received", std::process::id()),
        "boot",
    );

    // The helper dies with its parent (spec D4): the supervisor additionally
    // relies on kill_on_drop.
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            if !parent_alive(parent_pid) {
                std::process::exit(0);
            }
        }
    });

    let _ = std::fs::remove_file(&socket);
    let listener = tokio::net::UnixListener::bind(&socket).unwrap_or_else(|error| {
        eprintln!("echo_helper: cannot bind {}: {error}", socket.display());
        std::process::exit(1);
    });
    // Goes to the plugin log: proves the process started and bound its socket.
    eprintln!("echo_helper: listening on {}", socket.display());

    let server = EchoServer {
        tokens: tokens.clone(),
        receipt_dir,
    };
    let service = StreamableHttpService::new(
        move || Ok(server.clone()),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );
    let base = TowerToHyperService::new(service);

    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        let service = AuthService {
            tokens: tokens.clone(),
            inner: base.clone(),
        };
        tokio::spawn(async move {
            let _ = http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .await;
        });
    }
}

/// Unix sockets and process groups are Unix-only: without them the helper
/// cannot serve. This stub keeps the example building on every platform.
#[cfg(not(unix))]
fn main() {
    eprintln!("echo_helper: tier C helpers need a Unix socket; not supported on this platform");
    std::process::exit(2);
}

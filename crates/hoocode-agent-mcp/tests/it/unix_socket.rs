#![cfg(unix)]

//! Tier C transport tests: Streamable HTTP over a Unix socket. The client
//! dials the socket with reqwest's `unix_socket` connector; the URI host is
//! irrelevant (`http://localhost/mcp`). The server is the same rmcp
//! `StreamableHttpService` the TCP tests use, served per connection.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hyper::server::conn::http1;
use hyper_util::rt::TokioIo;
use hyper_util::service::TowerToHyperService;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, CustomNotification,
    ErrorData, ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::{NotificationContext, RequestContext};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::RoleServer;
use rmcp::ServerHandler;
use serde_json::{json, Map, Value};
use tokio_util::sync::CancellationToken;

use hoocode_agent_mcp::{CallOptions, ClientOptions, McpClient, McpServerConfig, AGENT_META_KEY};

const WAIT: Duration = Duration::from_secs(10);

/// What the recording server observed: request headers, request `_meta`, and
/// custom client->server notifications.
#[derive(Debug, Default)]
struct Seen {
    authorization: Vec<Option<String>>,
    metas: Vec<Value>,
    notifications: Vec<(String, Option<Value>)>,
}

/// An MCP server that records what it sees instead of doing anything useful.
#[derive(Clone)]
struct RecordingServer {
    seen: Arc<Mutex<Seen>>,
}

fn echo_tool() -> Tool {
    Tool::new("echo", "Returns its text argument.", Arc::new(Map::new()))
}

impl ServerHandler for RecordingServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(vec![echo_tool()]))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let authorization = context
            .extensions
            .get::<http::request::Parts>()
            .and_then(|parts| parts.headers.get(http::header::AUTHORIZATION))
            .and_then(|value| value.to_str().ok().map(str::to_owned));
        let meta = serde_json::to_value(&context.meta).unwrap_or(Value::Null);
        self.seen
            .lock()
            .expect("seen")
            .authorization
            .push(authorization);
        self.seen.lock().expect("seen").metas.push(meta);
        let text = request
            .arguments
            .as_ref()
            .and_then(|args| args.get("text"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        Ok(CallToolResult::success(vec![ContentBlock::text(text)]).into())
    }

    async fn on_custom_notification(
        &self,
        notification: CustomNotification,
        _context: NotificationContext<RoleServer>,
    ) {
        self.seen
            .lock()
            .expect("seen")
            .notifications
            .push((notification.method.clone(), notification.params.clone()));
    }
}

/// A `StreamableHttpService` on a Unix socket in a fresh tempdir. Stops and
/// removes the directory when dropped.
struct UnixServer {
    path: PathBuf,
    dir: PathBuf,
    shutdown: CancellationToken,
}

impl UnixServer {
    async fn start(seen: Arc<Mutex<Seen>>) -> UnixServer {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "hoocode-mcp-unix-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).expect("tempdir");
        let path = dir.join("mcp.sock");
        let _ = std::fs::remove_file(&path);
        let listener = tokio::net::UnixListener::bind(&path).expect("bind unix socket");
        let shutdown = CancellationToken::new();
        let service = StreamableHttpService::new(
            move || Ok(RecordingServer { seen: seen.clone() }),
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default().with_cancellation_token(shutdown.child_token()),
        );
        let stop = shutdown.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = stop.cancelled() => break,
                    accepted = listener.accept() => {
                        let Ok((stream, _)) = accepted else { continue };
                        let service = TowerToHyperService::new(service.clone());
                        tokio::spawn(async move {
                            let _ = http1::Builder::new()
                                .serve_connection(TokioIo::new(stream), service)
                                .await;
                        });
                    }
                }
            }
        });
        UnixServer {
            path,
            dir,
            shutdown,
        }
    }

    fn config(&self) -> McpServerConfig {
        McpServerConfig::Unix {
            path: self.path.clone(),
            headers: BTreeMap::new(),
        }
    }

    fn config_with_headers(&self, headers: &[(&str, &str)]) -> McpServerConfig {
        McpServerConfig::Unix {
            path: self.path.clone(),
            headers: headers
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
        }
    }
}

impl Drop for UnixServer {
    fn drop(&mut self) {
        self.shutdown.cancel();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

async fn wait_for(seen: &Mutex<Seen>, mut ready: impl FnMut(&Seen) -> bool) {
    tokio::time::timeout(WAIT, async {
        loop {
            if ready(&seen.lock().expect("seen")) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the server observes the request");
}

#[tokio::test]
async fn unix_lists_and_calls() {
    let seen = Arc::new(Mutex::new(Seen::default()));
    let server = UnixServer::start(seen).await;
    let client = McpClient::connect("unix", server.config(), ClientOptions::default())
        .await
        .expect("connect over the socket");
    let tools = client.list_tools().await.expect("list tools");
    assert!(
        tools.iter().any(|t| t.name == "echo"),
        "echo is listed: {tools:?}"
    );
    let output = client
        .call_tool("echo", json!({"text": "hi"}), CallOptions::default())
        .await
        .expect("call echo");
    assert!(!output.is_error);
}

#[tokio::test]
async fn unix_sends_the_authorization_header() {
    let seen = Arc::new(Mutex::new(Seen::default()));
    let server = UnixServer::start(seen.clone()).await;
    let client = McpClient::connect(
        "unix",
        server.config_with_headers(&[("Authorization", "Bearer test-token")]),
        ClientOptions::default(),
    )
    .await
    .expect("connect over the socket");
    client
        .call_tool("echo", json!({"text": "hi"}), CallOptions::default())
        .await
        .expect("call echo");
    wait_for(&seen, |seen| !seen.authorization.is_empty()).await;
    assert_eq!(
        seen.lock().expect("seen").authorization,
        vec![Some("Bearer test-token".to_owned())]
    );
}

#[tokio::test]
async fn unix_call_meta_reaches_the_tool() {
    let seen = Arc::new(Mutex::new(Seen::default()));
    let server = UnixServer::start(seen.clone()).await;
    let client = McpClient::connect("unix", server.config(), ClientOptions::default())
        .await
        .expect("connect over the socket");
    let mut meta = Map::new();
    meta.insert(
        AGENT_META_KEY.to_owned(),
        json!({"agentId": "root", "parentId": null, "depth": 0}),
    );
    client.set_call_meta(meta);
    client
        .call_tool("echo", json!({"text": "hi"}), CallOptions::default())
        .await
        .expect("call echo");
    wait_for(&seen, |seen| !seen.metas.is_empty()).await;
    let agent = seen.lock().expect("seen").metas[0]
        .get(AGENT_META_KEY)
        .cloned();
    assert_eq!(
        agent,
        Some(json!({"agentId": "root", "parentId": null, "depth": 0})),
        "the tool handler sees the agent identity"
    );
}

#[tokio::test]
async fn unix_custom_notification_reaches_the_server() {
    let seen = Arc::new(Mutex::new(Seen::default()));
    let server = UnixServer::start(seen.clone()).await;
    let client = McpClient::connect("unix", server.config(), ClientOptions::default())
        .await
        .expect("connect over the socket");
    client
        .notify(
            "hoocode/agentStarted",
            json!({"agentId": "root", "parentId": null, "depth": 0, "token": "secret"}),
        )
        .await
        .expect("notify");
    wait_for(&seen, |seen| !seen.notifications.is_empty()).await;
    assert_eq!(
        seen.lock().expect("seen").notifications,
        vec![(
            "hoocode/agentStarted".to_owned(),
            Some(json!({"agentId": "root", "parentId": null, "depth": 0, "token": "secret"}))
        )]
    );
}

#[tokio::test]
async fn unix_invalid_header_is_a_config_error() {
    let seen = Arc::new(Mutex::new(Seen::default()));
    let server = UnixServer::start(seen).await;
    let result = McpClient::connect(
        "unix",
        server.config_with_headers(&[("bad header", "x")]),
        ClientOptions::default(),
    )
    .await;
    assert!(
        matches!(result, Err(hoocode_agent_mcp::McpError::Config(_))),
        "{result:?}"
    );
}

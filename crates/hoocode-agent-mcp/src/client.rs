//! One MCP server connection: caps, deadlines, cancellation, progress and a
//! single reconnect after a drop.
//!
//! Every wait has a deadline: the permit wait, connecting, sending and the
//! response. A call that times out or is cancelled sends `notifications/cancelled`
//! to the server, so the server can stop the work (`docs/design/concurrency.md`
//! section 4).

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use http::{HeaderName, HeaderValue};
use rmcp::model::{
    CallToolRequest, CallToolRequestParams, CancelledNotificationParam, ClientConfig,
    ClientRequest, ElicitRequestParams, ElicitResult, ElicitationAction, ElicitationCapability,
    FormElicitationCapability, ListToolsResult, PaginatedRequestParams, ProgressNotificationParam,
    ProgressToken, RequestId, ServerResult, UrlElicitationCapability,
};
use rmcp::service::{
    ClientInitializeError, NotificationContext, Peer, PeerRequestOptions, RequestContext,
    RunningService, ServiceError,
};
use rmcp::transport::auth::AuthClient;
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransportConfig, StreamableHttpError,
};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::{ClientHandler, ErrorData, RoleClient, ServiceExt};
use serde_json::Value;
use tokio::sync::{watch, OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::config::McpServerConfig;
use crate::elicitation::{DeclineAll, ElicitationAnswer, ElicitationHandler, ElicitationRequest};
use crate::error::McpError;
use crate::oauth;
use crate::result::{from_call_result, ToolOutput};
use crate::stdio;

/// Most requests in flight to one server; the rest wait for a permit.
pub const MAX_IN_FLIGHT_REQUESTS: usize = 8;
/// Largest response accepted from a server. A bigger tool result becomes an
/// error result; a bigger tool list is an error.
pub const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;
/// Default deadline for one request, from the start of the wait for a permit.
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// Largest server-sent event accepted on a Streamable HTTP connection, in bytes
/// (`docs/design/concurrency.md` section 3). A bigger event fails the request.
pub const MAX_SSE_EVENT_BYTES: usize = 16 * 1024 * 1024;
/// Most pages of a tool list read before giving up.
const MAX_LIST_PAGES: usize = 1000;
/// How long a cancel notification may take to send.
const CANCEL_NOTIFY_TIMEOUT: Duration = Duration::from_secs(1);
/// How long a shutdown may take before the child process is killed.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);

/// Limits for one client. The defaults are the caps in `docs/design/concurrency.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientOptions {
    /// Deadline for one request, including the wait for a permit and connecting.
    pub request_timeout: Duration,
    /// Requests in flight at once (at least 1).
    pub max_in_flight: usize,
    /// Largest response accepted, in bytes.
    pub max_response_bytes: usize,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            max_in_flight: MAX_IN_FLIGHT_REQUESTS,
            max_response_bytes: MAX_RESPONSE_BYTES,
        }
    }
}

/// A progress report from a running tool, as the server sent it.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolProgress {
    pub progress: f64,
    pub total: Option<f64>,
    pub message: Option<String>,
}

/// Options for one `tools/call`.
#[derive(Debug)]
pub struct CallOptions {
    /// Cancelling it aborts the call: the client returns [`McpError::Cancelled`]
    /// and tells the server.
    pub cancel: CancellationToken,
    /// Receives the latest progress report. Only the newest value is kept; older
    /// ones are overwritten. The channel closes when the call ends. A report
    /// that arrives before the request is sent to the server's route is not
    /// delivered; that window is a few microseconds, and progress is best effort.
    pub progress: Option<watch::Sender<Option<ToolProgress>>>,
}

impl Default for CallOptions {
    fn default() -> Self {
        Self {
            cancel: CancellationToken::new(),
            progress: None,
        }
    }
}

/// A tool as the server lists it. The name is the server's own name; use
/// [`crate::tool_name`] for the model-facing name.
#[derive(Debug, Clone, PartialEq)]
pub struct McpTool {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Value,
}

/// Client handler: routes progress to the calls that wait for it and records
/// tool-list changes.
#[derive(Clone)]
struct Handler {
    progress: Arc<ProgressRoutes>,
    tools_changed: Arc<AtomicBool>,
    /// Answers the server's `elicitation/create` requests, with the server's name.
    elicitation: Elicitation,
}

#[derive(Clone)]
struct Elicitation {
    server: Arc<str>,
    handler: Arc<dyn ElicitationHandler>,
}

impl Handler {
    fn new(server: &str, elicitation: Arc<dyn ElicitationHandler>) -> Self {
        Self {
            progress: Arc::default(),
            tools_changed: Arc::default(),
            elicitation: Elicitation {
                server: Arc::from(server),
                handler: elicitation,
            },
        }
    }
}

impl ClientHandler for Handler {
    async fn on_progress(
        &self,
        params: ProgressNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        self.progress.deliver(params);
    }

    async fn on_tool_list_changed(&self, _context: NotificationContext<RoleClient>) {
        self.tools_changed.store(true, Ordering::SeqCst);
    }

    /// Declares form and URL elicitation, so servers may ask.
    fn get_info(&self) -> ClientConfig {
        let mut info = ClientConfig::default();
        info.capabilities.elicitation = Some(
            ElicitationCapability::new()
                .with_form(FormElicitationCapability::default())
                .with_url(UrlElicitationCapability::default()),
        );
        info
    }

    async fn create_elicitation(
        &self,
        request: ElicitRequestParams,
        _context: RequestContext<RoleClient>,
    ) -> Result<ElicitResult, ErrorData> {
        let question = match request {
            ElicitRequestParams::FormElicitationParams {
                message,
                requested_schema,
                ..
            } => ElicitationRequest::Form {
                message,
                schema: serde_json::to_value(&requested_schema).unwrap_or(Value::Null),
            },
            ElicitRequestParams::UrlElicitationParams {
                message,
                url,
                elicitation_id,
                ..
            } => ElicitationRequest::Url {
                message,
                url,
                elicitation_id,
            },
            // A mode this client does not know: decline.
            _ => return Ok(ElicitResult::new(ElicitationAction::Decline)),
        };
        let answer = self
            .elicitation
            .handler
            .elicit(&self.elicitation.server, question)
            .await;
        Ok(match answer {
            ElicitationAnswer::Accept(content) => {
                let mut result = ElicitResult::new(ElicitationAction::Accept);
                result.content = content;
                result
            }
            ElicitationAnswer::Decline => ElicitResult::new(ElicitationAction::Decline),
            ElicitationAnswer::Cancel => ElicitResult::new(ElicitationAction::Cancel),
        })
    }
}

/// Progress senders of the calls in flight, by progress token.
#[derive(Default)]
struct ProgressRoutes {
    routes: Mutex<HashMap<ProgressToken, watch::Sender<Option<ToolProgress>>>>,
}

impl ProgressRoutes {
    fn deliver(&self, params: ProgressNotificationParam) {
        let routes = lock(&self.routes);
        if let Some(sender) = routes.get(&params.progress_token) {
            sender.send_replace(Some(ToolProgress {
                progress: params.progress,
                total: params.total,
                message: params.message,
            }));
        }
    }
}

/// Removes a progress route when its call ends.
struct RouteGuard {
    routes: Arc<ProgressRoutes>,
    token: ProgressToken,
}

impl Drop for RouteGuard {
    fn drop(&mut self) {
        lock(&self.routes.routes).remove(&self.token);
    }
}

/// The live connection, tagged so that only the request that found it dead
/// can replace it.
struct Session {
    service: RunningService<RoleClient, Handler>,
    generation: u64,
}

struct Inner {
    server: String,
    config: McpServerConfig,
    /// Set for OAuth connections: the hoocode data directory holding tokens.
    oauth_dir: Option<PathBuf>,
    options: ClientOptions,
    permits: Arc<Semaphore>,
    handler: Handler,
    session: tokio::sync::Mutex<Option<Session>>,
    next_generation: AtomicU64,
    /// Set by `shutdown`: calls fail instead of starting the server again.
    closed: AtomicBool,
}

/// A connection to one MCP server. Clones share the connection.
#[derive(Clone)]
pub struct McpClient {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for McpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpClient")
            .field("server", &self.inner.server)
            .finish_non_exhaustive()
    }
}

impl McpClient {
    /// Starts the server and initializes the session, within
    /// `options.request_timeout`. `server` is the name used in tool names.
    ///
    /// A Streamable HTTP server that answers 401 with no stored token gives
    /// [`McpError::AuthRequired`]; see [`connect_oauth`](Self::connect_oauth).
    pub async fn connect(
        server: impl Into<String>,
        config: McpServerConfig,
        options: ClientOptions,
    ) -> Result<Self, McpError> {
        Self::connect_with(server.into(), config, None, options, Arc::new(DeclineAll)).await
    }

    /// [`connect`](Self::connect) with a handler for the server's elicitation requests. The
    /// handler is kept for reconnects.
    pub async fn connect_with_elicitation(
        server: impl Into<String>,
        config: McpServerConfig,
        options: ClientOptions,
        elicitation: Arc<dyn ElicitationHandler>,
    ) -> Result<Self, McpError> {
        Self::connect_with(server.into(), config, None, options, elicitation).await
    }

    /// Connects to a Streamable HTTP server that uses OAuth, with tokens kept
    /// under `data_dir` (see [`oauth::token_store_dir`]).
    ///
    /// A stored token for the server's authorization server is used, and
    /// refreshed when it is near expiry or rejected. With no usable token the
    /// server's 401 gives [`McpError::AuthRequired`]: run
    /// [`oauth::begin_login`], finish the login, then connect again.
    pub async fn connect_oauth(
        server: impl Into<String>,
        url: impl Into<String>,
        headers: BTreeMap<String, String>,
        data_dir: &Path,
        options: ClientOptions,
    ) -> Result<Self, McpError> {
        let config = McpServerConfig::Http {
            url: url.into(),
            headers,
        };
        Self::connect_with(
            server.into(),
            config,
            Some(data_dir.to_path_buf()),
            options,
            Arc::new(DeclineAll),
        )
        .await
    }

    /// [`connect_oauth`](Self::connect_oauth) with a handler for elicitation requests.
    pub async fn connect_oauth_with_elicitation(
        server: impl Into<String>,
        url: impl Into<String>,
        headers: BTreeMap<String, String>,
        data_dir: &Path,
        options: ClientOptions,
        elicitation: Arc<dyn ElicitationHandler>,
    ) -> Result<Self, McpError> {
        let config = McpServerConfig::Http {
            url: url.into(),
            headers,
        };
        Self::connect_with(
            server.into(),
            config,
            Some(data_dir.to_path_buf()),
            options,
            elicitation,
        )
        .await
    }

    async fn connect_with(
        server: String,
        config: McpServerConfig,
        oauth_dir: Option<PathBuf>,
        options: ClientOptions,
        elicitation: Arc<dyn ElicitationHandler>,
    ) -> Result<Self, McpError> {
        let options = ClientOptions {
            max_in_flight: options.max_in_flight.max(1),
            ..options
        };
        let handler = Handler::new(&server, elicitation);
        let service = bounded_by(options.request_timeout, async {
            start(&server, &config, handler.clone(), oauth_dir.as_deref()).await
        })
        .await?;
        let inner = Inner {
            server,
            config,
            oauth_dir,
            permits: Arc::new(Semaphore::new(options.max_in_flight)),
            options,
            handler,
            session: tokio::sync::Mutex::new(Some(Session {
                service,
                generation: 0,
            })),
            next_generation: AtomicU64::new(1),
            closed: AtomicBool::new(false),
        };
        Ok(Self {
            inner: Arc::new(inner),
        })
    }

    /// The server name given to [`connect`](Self::connect).
    pub fn server_name(&self) -> &str {
        &self.inner.server
    }

    /// True once the server has sent `notifications/tools/list_changed`, until
    /// [`take_tools_changed`](Self::take_tools_changed) clears it.
    pub fn tools_changed(&self) -> bool {
        self.inner.handler.tools_changed.load(Ordering::SeqCst)
    }

    /// Returns the tool-list-changed flag and clears it.
    pub fn take_tools_changed(&self) -> bool {
        self.inner
            .handler
            .tools_changed
            .swap(false, Ordering::SeqCst)
    }

    /// Lists every tool, reading all pages. Deadline and in-flight cap apply to each page.
    pub async fn list_tools(&self) -> Result<Vec<McpTool>, McpError> {
        let cancel = CancellationToken::new();
        let deadline = self.deadline();
        let mut tools = Vec::new();
        let mut bytes = 0usize;
        let mut cursor = None;
        for _ in 0..MAX_LIST_PAGES {
            let page = self.list_page(&cancel, deadline, cursor.take()).await?;
            bytes += serde_json::to_vec(&page.tools)
                .map_err(|e| McpError::Protocol(e.to_string()))?
                .len();
            if bytes > self.inner.options.max_response_bytes {
                return Err(McpError::TooLarge {
                    limit: self.inner.options.max_response_bytes,
                });
            }
            tools.extend(page.tools.into_iter().map(|tool| McpTool {
                name: tool.name.to_string(),
                description: tool.description.map(|d| d.to_string()),
                input_schema: Value::Object((*tool.input_schema).clone()),
            }));
            match page.next_cursor {
                Some(next) if !next.is_empty() => cursor = Some(next),
                _ => return Ok(tools),
            }
        }
        Err(McpError::Protocol(format!(
            "the server listed more than {MAX_LIST_PAGES} pages of tools"
        )))
    }

    /// Calls `tool` (the server's own name) with `arguments`, which must be a
    /// JSON object or null.
    ///
    /// A tool that fails and says so comes back as `Ok` with `is_error` set.
    /// `Err` means the request itself failed: deadline, cancel, dropped server.
    /// A response over the size cap comes back as an error result.
    pub async fn call_tool(
        &self,
        tool: &str,
        arguments: Value,
        options: CallOptions,
    ) -> Result<ToolOutput, McpError> {
        let arguments = match arguments {
            Value::Object(map) => Some(map),
            Value::Null => None,
            _ => {
                return Err(McpError::Config(
                    "tool arguments must be a JSON object".to_owned(),
                ))
            }
        };
        let CallOptions { cancel, progress } = options;
        let deadline = self.deadline();
        let _permit = self.acquire_permit(&cancel, deadline).await?;

        let mut params = CallToolRequestParams::new(tool.to_owned());
        params.arguments = arguments;
        let request = ClientRequest::CallToolRequest(CallToolRequest::new(params));

        // The request is re-sent at most once, and only if it never left the
        // client: the server cannot have run it.
        let mut progress = progress;
        let mut retried = false;
        loop {
            let (peer, generation) = self.bounded(&cancel, deadline, self.live_peer()).await?;
            let sent = self
                .bounded(&cancel, deadline, async {
                    Ok(peer
                        .send_cancellable_request(request.clone(), PeerRequestOptions::no_options())
                        .await)
                })
                .await?;
            let handle = match sent {
                Ok(handle) => handle,
                Err(ServiceError::TransportClosed) if !retried => {
                    retried = true;
                    self.reset_session(generation).await;
                    continue;
                }
                Err(error) => {
                    if matches!(error, ServiceError::TransportClosed) {
                        self.reset_session(generation).await;
                    }
                    return Err(map_service(&self.inner.server, error));
                }
            };

            // Progress from the server reaches the sender through this route. The
            // route is removed when the call ends.
            let _route = progress.take().map(|sender| {
                let token = handle.progress_token.clone();
                lock(&self.inner.handler.progress.routes).insert(token.clone(), sender);
                RouteGuard {
                    routes: Arc::clone(&self.inner.handler.progress),
                    token,
                }
            });

            let id = handle.id.clone();
            let response = tokio::select! {
                biased;
                _ = cancel.cancelled() => None,
                r = tokio::time::timeout_at(deadline, handle.await_response()) => Some(r),
            };
            let result = match response {
                None => {
                    notify_cancelled(&peer, id, "cancelled by the client").await;
                    return Err(McpError::Cancelled);
                }
                Some(Err(_elapsed)) => {
                    notify_cancelled(&peer, id, "request timeout").await;
                    return Err(McpError::Timeout(self.inner.options.request_timeout));
                }
                Some(Ok(Err(error))) => {
                    if matches!(error, ServiceError::TransportClosed) {
                        self.reset_session(generation).await;
                    }
                    return Err(map_service(&self.inner.server, error));
                }
                Some(Ok(Ok(result))) => result,
            };
            return match result {
                ServerResult::CallToolResult(result) => {
                    let output = from_call_result(result);
                    if output.payload_bytes() > self.inner.options.max_response_bytes {
                        Ok(ToolOutput::error_text(format!(
                            "MCP response exceeds {} bytes",
                            self.inner.options.max_response_bytes
                        )))
                    } else {
                        Ok(output)
                    }
                }
                ServerResult::InputRequiredResult(_) => Err(McpError::Protocol(
                    "the server asked for input during a tool call; this client does not support that"
                        .to_owned(),
                )),
                _ => Err(McpError::Protocol(
                    "unexpected response to tools/call".to_owned(),
                )),
            };
        }
    }

    /// Closes the session and stops the server process (stdio) or drops the
    /// HTTP session. Waits at most a few seconds. Later calls fail with
    /// [`McpError::Disconnected`]; they do not start the server again.
    pub async fn shutdown(&self) {
        self.inner.closed.store(true, Ordering::SeqCst);
        let taken = self.inner.session.lock().await.take();
        if let Some(session) = taken {
            let _ = tokio::time::timeout(SHUTDOWN_TIMEOUT, session.service.cancel()).await;
        }
    }

    fn deadline(&self) -> Instant {
        Instant::now() + self.inner.options.request_timeout
    }

    /// Runs `fut` until `deadline`, or until `cancel` fires.
    async fn bounded<T>(
        &self,
        cancel: &CancellationToken,
        deadline: Instant,
        fut: impl Future<Output = Result<T, McpError>>,
    ) -> Result<T, McpError> {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(McpError::Cancelled),
            r = tokio::time::timeout_at(deadline, fut) => match r {
                Ok(result) => result,
                Err(_elapsed) => Err(McpError::Timeout(self.inner.options.request_timeout)),
            },
        }
    }

    async fn acquire_permit(
        &self,
        cancel: &CancellationToken,
        deadline: Instant,
    ) -> Result<OwnedSemaphorePermit, McpError> {
        let permits = Arc::clone(&self.inner.permits);
        self.bounded(cancel, deadline, async move {
            permits
                .acquire_owned()
                .await
                .map_err(|_| McpError::Protocol("the request limiter closed".to_owned()))
        })
        .await
    }

    async fn list_page(
        &self,
        cancel: &CancellationToken,
        deadline: Instant,
        cursor: Option<String>,
    ) -> Result<ListToolsResult, McpError> {
        let _permit = self.acquire_permit(cancel, deadline).await?;
        let mut params = PaginatedRequestParams::default();
        params.cursor = cursor;
        let mut retried = false;
        loop {
            let (peer, generation) = self.bounded(cancel, deadline, self.live_peer()).await?;
            let result = self
                .bounded(cancel, deadline, {
                    let params = params.clone();
                    let server = self.inner.server.as_str();
                    async move {
                        peer.list_tools(Some(params))
                            .await
                            .map_err(|e| map_service(server, e))
                    }
                })
                .await;
            match result {
                Ok(page) => return Ok(page),
                // Listing is safe to repeat: reconnect and try once more.
                Err(McpError::Disconnected(_)) if !retried => {
                    retried = true;
                    self.reset_session(generation).await;
                }
                Err(error) => {
                    if matches!(error, McpError::Disconnected(_)) {
                        self.reset_session(generation).await;
                    }
                    return Err(error);
                }
            }
        }
    }

    /// The live session's peer. If the server dropped, starts it again once.
    async fn live_peer(&self) -> Result<(Peer<RoleClient>, u64), McpError> {
        let mut slot = self.inner.session.lock().await;
        if self.inner.closed.load(Ordering::SeqCst) {
            return Err(McpError::Disconnected(
                "the client was shut down".to_owned(),
            ));
        }
        if let Some(session) = slot.as_ref() {
            if !session.service.is_closed() {
                return Ok((session.service.peer().clone(), session.generation));
            }
        }
        // Dropping the old session stops its process.
        *slot = None;
        let service = start(
            &self.inner.server,
            &self.inner.config,
            self.inner.handler.clone(),
            self.inner.oauth_dir.as_deref(),
        )
        .await?;
        let generation = self.inner.next_generation.fetch_add(1, Ordering::SeqCst);
        let peer = service.peer().clone();
        *slot = Some(Session {
            service,
            generation,
        });
        Ok((peer, generation))
    }

    /// Drops the session if it is still the one that failed.
    async fn reset_session(&self, generation: u64) {
        let Ok(mut slot) = tokio::time::timeout(
            self.inner.options.request_timeout,
            self.inner.session.lock(),
        )
        .await
        else {
            return;
        };
        if slot
            .as_ref()
            .is_some_and(|session| session.generation == generation)
        {
            *slot = None;
        }
    }
}

/// Starts the server for `config` and initializes the MCP session. With
/// `oauth_dir`, a Streamable HTTP server is reached with its stored OAuth token.
async fn start(
    server: &str,
    config: &McpServerConfig,
    handler: Handler,
    oauth_dir: Option<&Path>,
) -> Result<RunningService<RoleClient, Handler>, McpError> {
    match config {
        McpServerConfig::Stdio {
            command,
            args,
            env,
            cwd,
        } => {
            let transport = stdio::spawn(command, args, env, cwd.as_deref())
                .map_err(|e| McpError::Connect(format!("cannot start {command}: {e}")))?;
            handler
                .serve(transport)
                .await
                .map_err(|e| connect_error(server, e))
        }
        McpServerConfig::Http { url, headers } => {
            let mut custom = HashMap::new();
            for (name, value) in headers {
                let name = HeaderName::from_bytes(name.as_bytes())
                    .map_err(|e| McpError::Config(format!("header name {name:?}: {e}")))?;
                let value = HeaderValue::from_str(value)
                    .map_err(|e| McpError::Config(format!("value of header {name}: {e}")))?;
                custom.insert(name, value);
            }
            let mut config =
                StreamableHttpClientTransportConfig::with_uri(url.as_str()).custom_headers(custom);
            config.max_sse_event_size = MAX_SSE_EVENT_BYTES;
            let result = match oauth_dir {
                None => {
                    handler
                        .serve(StreamableHttpClientTransport::from_config(config))
                        .await
                }
                Some(dir) => {
                    let manager = oauth::authorization_manager(url, dir).await?;
                    let client = AuthClient::new(reqwest::Client::new(), manager);
                    handler
                        .serve(StreamableHttpClientTransport::with_client(client, config))
                        .await
                }
            };
            result.map_err(|e| connect_error(server, e))
        }
    }
}

/// Maps a failed initialize. A 401 becomes [`McpError::AuthRequired`].
fn connect_error(server: &str, error: ClientInitializeError) -> McpError {
    let challenge = match &error {
        ClientInitializeError::TransportError { error, .. } => auth_challenge(&*error.error),
        _ => None,
    };
    match challenge {
        Some(www_authenticate) => McpError::AuthRequired {
            server: server.to_owned(),
            www_authenticate,
        },
        None => McpError::Connect(error.to_string()),
    }
}

/// The `WWW-Authenticate` challenge in a 401 from the Streamable HTTP
/// transport, if `error` is one.
fn auth_challenge(error: &(dyn std::error::Error + Send + Sync + 'static)) -> Option<String> {
    match error.downcast_ref::<StreamableHttpError<reqwest::Error>>()? {
        StreamableHttpError::AuthRequired(required) => {
            Some(required.www_authenticate_header.clone())
        }
        _ => None,
    }
}

/// Runs `fut` with a deadline of `timeout`.
async fn bounded_by<T>(
    timeout: Duration,
    fut: impl Future<Output = Result<T, McpError>>,
) -> Result<T, McpError> {
    tokio::time::timeout(timeout, fut)
        .await
        .unwrap_or(Err(McpError::Timeout(timeout)))
}

/// Tells the server to stop request `id`. Best effort, with its own short deadline.
async fn notify_cancelled(peer: &Peer<RoleClient>, id: RequestId, reason: &str) {
    let params = CancelledNotificationParam::new(Some(id), Some(reason.to_owned()));
    let _ = tokio::time::timeout(CANCEL_NOTIFY_TIMEOUT, peer.notify_cancelled(params)).await;
}

/// Maps an rmcp error. A dropped connection is `Disconnected` so callers can
/// tell it apart; a 401 is `AuthRequired`.
fn map_service(server: &str, error: ServiceError) -> McpError {
    match error {
        ServiceError::TransportClosed => McpError::Disconnected("the connection closed".to_owned()),
        ServiceError::TransportSend(error) => match auth_challenge(&*error.error) {
            Some(www_authenticate) => McpError::AuthRequired {
                server: server.to_owned(),
                www_authenticate,
            },
            None => McpError::Protocol(format!("transport send error: {}", error.error)),
        },
        ServiceError::Timeout { timeout } => McpError::Timeout(timeout),
        ServiceError::McpError(error) => McpError::Rpc(error.message.to_string()),
        other => McpError::Protocol(other.to_string()),
    }
}

/// Locks a std mutex, ignoring poisoning: the maps hold no invariants a panic
/// could break.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

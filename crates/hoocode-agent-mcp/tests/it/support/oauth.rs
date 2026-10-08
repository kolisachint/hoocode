//! In-process servers for the OAuth tests, all on 127.0.0.1 at ephemeral ports:
//!
//! - a fake authorization server: metadata, dynamic client registration, an
//!   authorize endpoint that redirects straight back with a code, and a token
//!   endpoint for the code and refresh-token grants (PKCE S256 is checked);
//! - a Streamable HTTP MCP server that answers 401 unless the request carries
//!   a bearer token the authorization server issued;
//! - a server whose SSE event is larger than the client accepts.

use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::future::Future;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use bytes::Bytes;
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{AUTHORIZATION, CONTENT_TYPE, LOCATION, WWW_AUTHENTICATE};
use hyper::server::conn::http1;
use hyper::service::{service_fn, Service};
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use hyper_util::service::TowerToHyperService;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use url::Url;

use super::server::TestServer;

type Body = BoxBody<Bytes, Infallible>;

/// Stops its server when dropped.
pub struct Stop(CancellationToken);

impl Drop for Stop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

/// What the fake authorization server has seen and issued.
#[derive(Default)]
pub struct AuthState {
    /// The issuer (the server's base URL).
    pub issuer: String,
    /// Advertise Client ID Metadata Document support.
    pub cimd: bool,
    /// `expires_in` of issued access tokens, in seconds.
    pub expires_in: u64,
    /// Overrides the `issuer` in the metadata document.
    pub metadata_issuer: Option<String>,
    /// Overrides the `iss` parameter sent on the redirect (RFC 9207).
    pub redirect_iss: Option<String>,
    /// Requests to the authorize endpoint.
    pub authorize_calls: usize,
    /// Dynamic client registrations.
    pub registrations: usize,
    /// Grant types of the token requests, in order.
    pub grants: Vec<String>,
    /// Access tokens the server accepts. The MCP server reads this set.
    pub valid_tokens: HashSet<String>,
    codes: HashMap<String, Grant>,
    refresh_tokens: HashSet<String>,
    counter: u64,
}

struct Grant {
    challenge: String,
    redirect_uri: String,
}

/// A running fake authorization server.
pub struct FakeAuth {
    pub base: String,
    pub state: Arc<Mutex<AuthState>>,
    _stop: Stop,
}

/// Starts the fake authorization server. `cimd` sets whether it advertises
/// Client ID Metadata Documents; without it the client registers dynamically.
pub async fn start_auth(cimd: bool, expires_in: u64) -> FakeAuth {
    let listener = bind().await;
    let base = format!("http://{}", listener.local_addr().expect("address"));
    let state = Arc::new(Mutex::new(AuthState {
        issuer: base.clone(),
        cimd,
        expires_in,
        ..Default::default()
    }));
    let shared = Arc::clone(&state);
    let stop = serve(listener, move |req| {
        let shared = Arc::clone(&shared);
        async move { handle_auth(&shared, req).await }
    });
    FakeAuth {
        base,
        state,
        _stop: Stop(stop),
    }
}

/// A Streamable HTTP MCP server that needs a bearer token from `auth`.
pub struct ProtectedMcp {
    /// The MCP endpoint, `http://127.0.0.1:<port>/mcp`.
    pub url: String,
    _stop: Stop,
}

/// Starts a bearer-protected MCP server. Its protected-resource metadata names
/// the fake authorization server as the way to get a token.
pub async fn start_protected_mcp(auth: &FakeAuth) -> ProtectedMcp {
    let listener = bind().await;
    let base = format!("http://{}", listener.local_addr().expect("address"));
    let mcp = StreamableHttpService::new(
        || Ok(TestServer { allow_exit: false }),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );
    let tokens = Arc::clone(&auth.state);
    let auth_base = auth.base.clone();
    let base_url = base.clone();
    let stop = serve(listener, move |req| {
        let tokens = Arc::clone(&tokens);
        let auth_base = auth_base.clone();
        let base = base.clone();
        let mcp = mcp.clone();
        async move { gate(req, &tokens, &base, &auth_base, mcp).await }
    });
    ProtectedMcp {
        url: format!("{}/mcp", base_url),
        _stop: Stop(stop),
    }
}

/// A server that answers `initialize` with a valid JSON-RPC result whose
/// `instructions` text is `padding` bytes long, sent as one SSE event. Other
/// POSTs (notifications) get 202. Returns the endpoint URL.
pub async fn start_padded_sse(padding: usize) -> (String, Stop) {
    let listener = bind().await;
    let url = format!("http://{}/mcp", listener.local_addr().expect("address"));
    let stop = serve(listener, move |req| async move {
        let version = req.headers().get("mcp-protocol-version").cloned();
        let body = req
            .into_body()
            .collect()
            .await
            .map(|collected| collected.to_bytes())
            .unwrap_or_default();
        let request: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        let Some(id) = request.get("id").cloned() else {
            return reply(StatusCode::ACCEPTED, "text/plain", Bytes::new());
        };
        let protocol = request["params"]["protocolVersion"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| version.and_then(|v| v.to_str().ok().map(str::to_owned)))
            .unwrap_or_default();
        let result = json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": protocol,
                "capabilities": {},
                "serverInfo": {"name": "padded", "version": "1"},
                "instructions": "x".repeat(padding),
            },
        });
        reply(
            StatusCode::OK,
            "text/event-stream",
            format!("data: {result}\n\n"),
        )
    });
    (url, Stop(stop))
}

async fn bind() -> TcpListener {
    TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind 127.0.0.1:0")
}

fn reply(status: StatusCode, content_type: &str, body: impl Into<Bytes>) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, content_type)
        .body(Full::new(body.into()).boxed())
        .expect("valid response")
}

fn json_reply(status: StatusCode, value: Value) -> Response<Body> {
    reply(status, "application/json", value.to_string())
}

/// Serves `handler` on `listener` until the returned token is cancelled.
fn serve<F, Fut>(listener: TcpListener, handler: F) -> CancellationToken
where
    F: Fn(Request<Incoming>) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Response<Body>> + Send + 'static,
{
    let stop = CancellationToken::new();
    let token = stop.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = stop.cancelled() => break,
                accepted = listener.accept() => {
                    let Ok((stream, _)) = accepted else { continue };
                    let handler = handler.clone();
                    tokio::spawn(async move {
                        let service = service_fn(move |req| {
                            let handler = handler.clone();
                            async move { Ok::<_, Infallible>(handler(req).await) }
                        });
                        let _ = http1::Builder::new()
                            .serve_connection(TokioIo::new(stream), service)
                            .await;
                    });
                }
            }
        }
    });
    token
}

/// Routes an MCP-server request: protected-resource metadata is public, the
/// MCP endpoint needs a token the authorization server issued.
async fn gate(
    req: Request<Incoming>,
    tokens: &Mutex<AuthState>,
    base: &str,
    auth_base: &str,
    mcp: StreamableHttpService<TestServer, LocalSessionManager>,
) -> Response<Body> {
    let path = req.uri().path().to_owned();
    if path.starts_with("/.well-known/oauth-protected-resource") {
        return json_reply(
            StatusCode::OK,
            json!({"resource": format!("{base}/mcp"), "authorization_servers": [auth_base]}),
        );
    }
    let bearer = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::to_owned);
    let accepted = bearer.is_some_and(|token| {
        tokens
            .lock()
            .expect("auth state")
            .valid_tokens
            .contains(&token)
    });
    if !accepted {
        return Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .header(
                WWW_AUTHENTICATE,
                format!("Bearer resource_metadata=\"{base}/.well-known/oauth-protected-resource\""),
            )
            .body(Full::new(Bytes::new()).boxed())
            .expect("valid response");
    }
    let service = TowerToHyperService::new(mcp);
    let Ok(response) = service.call(req).await;
    response
}

async fn handle_auth(state: &Mutex<AuthState>, req: Request<Incoming>) -> Response<Body> {
    let method = req.method().as_str().to_owned();
    let path = req.uri().path().to_owned();
    let query = req.uri().query().unwrap_or_default().to_owned();
    let body = req
        .into_body()
        .collect()
        .await
        .map(|collected| collected.to_bytes())
        .unwrap_or_default();
    respond(state, &method, &path, &query, &body)
}

fn respond(
    state: &Mutex<AuthState>,
    method: &str,
    path: &str,
    query: &str,
    body: &[u8],
) -> Response<Body> {
    let mut s = state.lock().expect("auth state");
    match (method, path) {
        ("GET", "/.well-known/oauth-authorization-server") => {
            let issuer = s
                .metadata_issuer
                .clone()
                .unwrap_or_else(|| s.issuer.clone());
            json_reply(
                StatusCode::OK,
                json!({
                    "issuer": issuer,
                    "authorization_endpoint": format!("{}/authorize", s.issuer),
                    "token_endpoint": format!("{}/token", s.issuer),
                    "registration_endpoint": format!("{}/register", s.issuer),
                    "response_types_supported": ["code"],
                    "grant_types_supported": ["authorization_code", "refresh_token"],
                    "code_challenge_methods_supported": ["S256"],
                    "token_endpoint_auth_methods_supported": ["none"],
                    "client_id_metadata_document_supported": s.cimd,
                }),
            )
        }
        ("POST", "/register") => {
            s.registrations += 1;
            let request: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
            json_reply(
                StatusCode::CREATED,
                json!({
                    "client_id": format!("dcr-client-{}", s.registrations),
                    "redirect_uris": request.get("redirect_uris").cloned().unwrap_or(json!([])),
                    "token_endpoint_auth_method": "none",
                }),
            )
        }
        ("GET", "/authorize") => {
            let params: HashMap<String, String> = url::form_urlencoded::parse(query.as_bytes())
                .into_owned()
                .collect();
            s.authorize_calls += 1;
            s.counter += 1;
            let code = format!("code-{}", s.counter);
            let redirect_uri = params.get("redirect_uri").cloned().unwrap_or_default();
            s.codes.insert(
                code.clone(),
                Grant {
                    challenge: params.get("code_challenge").cloned().unwrap_or_default(),
                    redirect_uri: redirect_uri.clone(),
                },
            );
            let iss = s.redirect_iss.clone().unwrap_or_else(|| s.issuer.clone());
            let mut location = Url::parse(&redirect_uri).expect("redirect URI");
            location
                .query_pairs_mut()
                .append_pair("code", &code)
                .append_pair("state", params.get("state").map_or("", String::as_str))
                .append_pair("iss", &iss);
            Response::builder()
                .status(StatusCode::FOUND)
                .header(LOCATION, location.as_str())
                .body(Full::new(Bytes::new()).boxed())
                .expect("valid response")
        }
        ("POST", "/token") => {
            let form: HashMap<String, String> =
                url::form_urlencoded::parse(body).into_owned().collect();
            let grant = form.get("grant_type").cloned().unwrap_or_default();
            s.grants.push(grant.clone());
            match grant.as_str() {
                "authorization_code" => {
                    let code = form.get("code").cloned().unwrap_or_default();
                    let Some(stored) = s.codes.remove(&code) else {
                        return invalid_grant();
                    };
                    let verifier = form.get("code_verifier").map_or("", String::as_str);
                    let computed = base64::engine::general_purpose::URL_SAFE_NO_PAD
                        .encode(Sha256::digest(verifier.as_bytes()));
                    let redirect_ok = form.get("redirect_uri") == Some(&stored.redirect_uri);
                    if computed != stored.challenge || !redirect_ok {
                        return invalid_grant();
                    }
                    json_reply(StatusCode::OK, issue(&mut s, true))
                }
                "refresh_token" => {
                    let token = form.get("refresh_token").cloned().unwrap_or_default();
                    if !s.refresh_tokens.contains(&token) {
                        return invalid_grant();
                    }
                    json_reply(StatusCode::OK, issue(&mut s, true))
                }
                _ => invalid_grant(),
            }
        }
        _ => reply(StatusCode::NOT_FOUND, "text/plain", "not found"),
    }
}

fn invalid_grant() -> Response<Body> {
    json_reply(StatusCode::BAD_REQUEST, json!({"error": "invalid_grant"}))
}

/// Issues a new access token (and a refresh token) and records both.
fn issue(s: &mut AuthState, with_refresh: bool) -> Value {
    s.counter += 1;
    let access = format!("at-{}", s.counter);
    s.valid_tokens.insert(access.clone());
    let mut response = json!({
        "access_token": access,
        "token_type": "Bearer",
        "expires_in": s.expires_in,
    });
    if with_refresh {
        let refresh = format!("rt-{}", s.counter);
        s.refresh_tokens.insert(refresh.clone());
        response["refresh_token"] = json!(refresh);
    }
    response
}

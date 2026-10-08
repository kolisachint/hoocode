//! OAuth for Streamable HTTP servers, against the in-process fake authorization
//! server and the bearer-protected MCP server in `support/oauth.rs`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use hoocode_agent_mcp::oauth::{token_store_dir, token_store_path};
use hoocode_agent_mcp::{begin_login, ClientOptions, McpClient, McpError, McpServerConfig};

use super::support::oauth::{start_auth, start_padded_sse, start_protected_mcp};

fn options() -> ClientOptions {
    ClientOptions {
        request_timeout: Duration::from_secs(10),
        ..Default::default()
    }
}

/// A fresh data directory for one test under cargo's per-test temp dir.
fn data_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("oauth-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Plays the browser: requests the authorization URL, following the redirect
/// to the loopback listener. Returns the status of the last response.
async fn browse(url: &str) -> reqwest::StatusCode {
    reqwest::Client::new()
        .get(url)
        .send()
        .await
        .expect("browser request")
        .status()
}

/// Runs a full login against `mcp_url` and returns once the tokens are stored.
async fn log_in(mcp_url: &str, dir: &Path) {
    let (authorize_url, handle) = begin_login(mcp_url, dir).await.expect("begin login");
    let finish = tokio::spawn(handle.finish(Duration::from_secs(20)));
    assert_eq!(browse(&authorize_url).await, reqwest::StatusCode::OK);
    finish.await.expect("finish task").expect("login completes");
}

/// The token files under the store directory, sorted.
fn token_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(token_store_dir(dir)) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .map(|entry| entry.expect("directory entry").path())
        .collect();
    files.sort();
    files
}

/// The access token stored for the single issuer under `dir`.
fn stored_access_token(dir: &Path) -> String {
    let files = token_files(dir);
    assert_eq!(files.len(), 1, "one token file per issuer");
    let stored: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&files[0]).expect("token file")).expect("json");
    stored["token_response"]["access_token"]
        .as_str()
        .expect("access token")
        .to_owned()
}

#[tokio::test]
async fn unauthenticated_connect_reports_auth_required() {
    let auth = start_auth(false, 3600).await;
    let mcp = start_protected_mcp(&auth).await;
    let dir = data_dir("auth-required");

    let err = McpClient::connect_oauth("docs", &mcp.url, BTreeMap::new(), &dir, options())
        .await
        .expect_err("no token is stored");
    match err {
        McpError::AuthRequired {
            server,
            www_authenticate,
        } => {
            assert_eq!(server, "docs");
            assert!(www_authenticate.contains("resource_metadata"));
        }
        other => panic!("expected AuthRequired, got {other:?}"),
    }
    assert_eq!(auth.state.lock().unwrap().authorize_calls, 0);
}

#[tokio::test]
async fn login_stores_the_token_owner_only() {
    let auth = start_auth(false, 3600).await;
    let mcp = start_protected_mcp(&auth).await;
    let dir = data_dir("login");

    log_in(&mcp.url, &dir).await;

    let files = token_files(&dir);
    assert_eq!(files, vec![token_store_path(&dir, &auth.base)]);
    assert!(stored_access_token(&dir).starts_with("at-"));
    assert_eq!(
        auth.state.lock().unwrap().registrations,
        1,
        "registered once"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let dir_mode = std::fs::metadata(token_store_dir(&dir))
            .unwrap()
            .permissions()
            .mode();
        let file_mode = std::fs::metadata(&files[0]).unwrap().permissions().mode();
        assert_eq!(dir_mode & 0o777, 0o700);
        assert_eq!(file_mode & 0o777, 0o600);
    }
}

#[tokio::test]
async fn reconnect_uses_the_stored_token() {
    let auth = start_auth(false, 3600).await;
    let mcp = start_protected_mcp(&auth).await;
    let dir = data_dir("reconnect");
    log_in(&mcp.url, &dir).await;

    let client = McpClient::connect_oauth("docs", &mcp.url, BTreeMap::new(), &dir, options())
        .await
        .expect("connect with the stored token");
    assert!(!client.list_tools().await.expect("list tools").is_empty());
    client.shutdown().await;

    let again = McpClient::connect_oauth("docs", &mcp.url, BTreeMap::new(), &dir, options())
        .await
        .expect("second connect");
    assert!(!again.list_tools().await.expect("list tools").is_empty());

    let state = auth.state.lock().unwrap();
    assert_eq!(state.authorize_calls, 1, "no second login");
    assert_eq!(
        state.grants,
        vec!["authorization_code"],
        "only the login's code exchange; reconnecting does not refresh"
    );
}

#[tokio::test]
async fn an_expired_token_is_refreshed_on_connect() {
    // Tokens expire in one second, inside the refresh margin, so the client
    // renews before it uses them.
    let auth = start_auth(false, 1).await;
    let mcp = start_protected_mcp(&auth).await;
    let dir = data_dir("refresh");
    log_in(&mcp.url, &dir).await;
    let first = stored_access_token(&dir);

    let client = McpClient::connect_oauth("docs", &mcp.url, BTreeMap::new(), &dir, options())
        .await
        .expect("connect after expiry");
    assert!(!client.list_tools().await.expect("list tools").is_empty());

    let state = auth.state.lock().unwrap();
    assert!(
        state.grants.iter().any(|grant| grant == "refresh_token"),
        "grants: {:?}",
        state.grants
    );
    assert_eq!(state.authorize_calls, 1, "refresh needs no new login");
    drop(state);
    assert_ne!(
        stored_access_token(&dir),
        first,
        "the stored token was renewed"
    );
}

#[tokio::test]
async fn callback_with_a_different_issuer_is_rejected() {
    let auth = start_auth(false, 3600).await;
    auth.state.lock().unwrap().redirect_iss = Some("https://evil.example".to_owned());
    let mcp = start_protected_mcp(&auth).await;
    let dir = data_dir("callback-iss");

    let (authorize_url, handle) = begin_login(&mcp.url, &dir).await.expect("begin login");
    let finish = tokio::spawn(handle.finish(Duration::from_secs(20)));
    browse(&authorize_url).await;
    let err = finish
        .await
        .expect("finish task")
        .expect_err("the iss does not match");
    assert!(matches!(err, McpError::Auth(_)), "{err:?}");
    assert!(token_files(&dir).is_empty(), "nothing stored");
}

#[tokio::test]
async fn metadata_with_a_different_issuer_is_rejected() {
    let auth = start_auth(false, 3600).await;
    auth.state.lock().unwrap().metadata_issuer = Some("https://elsewhere.example".to_owned());
    let mcp = start_protected_mcp(&auth).await;
    let dir = data_dir("metadata-issuer");

    let err = begin_login(&mcp.url, &dir)
        .await
        .err()
        .expect("the metadata issuer does not match the server");
    assert!(matches!(err, McpError::Auth(_)), "{err:?}");
    assert!(token_files(&dir).is_empty(), "nothing stored");
}

#[tokio::test]
async fn a_pasted_redirect_completes_the_login() {
    let auth = start_auth(false, 3600).await;
    let mcp = start_protected_mcp(&auth).await;
    let dir = data_dir("pasted");

    let (authorize_url, handle) = begin_login(&mcp.url, &dir).await.expect("begin login");
    // Read the redirect without following it, as a user would copy it.
    let redirect = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
        .get(&authorize_url)
        .send()
        .await
        .expect("authorize")
        .headers()
        .get("location")
        .expect("redirect location")
        .to_str()
        .unwrap()
        .to_owned();
    handle
        .finish_redirect(&redirect)
        .await
        .expect("login completes");
    assert!(stored_access_token(&dir).starts_with("at-"));
}

/// Connects to a server whose initialize result is `padding` bytes of text.
async fn connect_padded(padding: usize) -> Result<McpClient, McpError> {
    let (url, _stop) = start_padded_sse(padding).await;
    McpClient::connect(
        "padded",
        McpServerConfig::Http {
            url,
            headers: BTreeMap::new(),
        },
        options(),
    )
    .await
}

#[tokio::test]
async fn sse_event_under_the_cap_connects() {
    // 15 MiB: the same response as the next test, just under 16 MiB.
    connect_padded(15 * 1024 * 1024 - 4096)
        .await
        .expect("an event under the cap is accepted");
}

#[tokio::test]
async fn oversized_sse_event_fails_the_request() {
    // 17 MiB: over the 16 MiB cap, so the request fails.
    let err = connect_padded(17 * 1024 * 1024)
        .await
        .expect_err("a 17 MiB event is over the 16 MiB cap");
    assert!(matches!(err, McpError::Connect(_)), "{err:?}");
}

fn own_header(value: &str) -> BTreeMap<String, String> {
    BTreeMap::from([("Authorization".to_owned(), value.to_owned())])
}

#[tokio::test]
async fn own_authorization_header_connects_without_oauth() {
    let auth = start_auth(false, 3600).await;
    auth.state
        .lock()
        .unwrap()
        .valid_tokens
        .insert("static-token".to_owned());
    let mcp = start_protected_mcp(&auth).await;
    let dir = data_dir("own-header");

    // The header is the credential: the server accepts it and no login runs.
    let client = McpClient::connect_oauth(
        "docs",
        &mcp.url,
        own_header("Bearer static-token"),
        &dir,
        options(),
    )
    .await
    .expect("the header is accepted");
    assert!(!client.list_tools().await.expect("list").is_empty());
    assert_eq!(auth.state.lock().unwrap().authorize_calls, 0);
    assert!(token_files(&dir).is_empty(), "no token file is written");

    // The name is case-insensitive.
    let lower = BTreeMap::from([("authorization".to_owned(), "Bearer static-token".to_owned())]);
    McpClient::connect_oauth("docs", &mcp.url, lower, &dir, options())
        .await
        .expect("a lower-case header name works too");
}

#[tokio::test]
async fn wrong_own_authorization_header_is_unauthorized_not_auth_required() {
    let auth = start_auth(false, 3600).await;
    let mcp = start_protected_mcp(&auth).await;
    let dir = data_dir("own-header-wrong");

    let err =
        McpClient::connect_oauth("docs", &mcp.url, own_header("Bearer nope"), &dir, options())
            .await
            .expect_err("the server rejects the header");
    assert!(
        matches!(err, McpError::Unauthorized(_)),
        "expected Unauthorized, got {err:?}"
    );
    assert_eq!(
        err.to_string(),
        "unauthorized: check the Authorization header"
    );
    assert_eq!(auth.state.lock().unwrap().authorize_calls, 0);
}

#[tokio::test]
async fn own_authorization_ignores_a_stored_login_token() {
    // A login stored a working token for this server. With the entry's own header, the
    // stored token is never read, so a wrong header still fails as Unauthorized.
    let auth = start_auth(false, 3600).await;
    let mcp = start_protected_mcp(&auth).await;
    let dir = data_dir("own-header-stored");
    log_in(&mcp.url, &dir).await;
    assert_eq!(token_files(&dir).len(), 1);

    let err =
        McpClient::connect_oauth("docs", &mcp.url, own_header("Bearer nope"), &dir, options())
            .await
            .expect_err("the stored token is not used");
    assert!(matches!(err, McpError::Unauthorized(_)), "{err:?}");
}

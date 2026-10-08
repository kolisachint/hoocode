//! OAuth for Streamable HTTP servers, on rmcp's authorization support.
//!
//! rmcp does the protocol work: discovery of the authorization server, PKCE
//! (S256 only), the RFC 9207 `iss` check on the redirect, the issuer check on
//! the metadata, client identity (a Client ID Metadata Document when the server
//! supports it, dynamic client registration otherwise) and refresh-token
//! renewal. This module adds what a hoocode client needs on top:
//!
//! - [`token_store_dir`] / [`token_store_path`]: one JSON file per issuer under
//!   `<data_dir>/mcp-auth/`. Files are written owner-only (0600 in a 0700
//!   directory on Unix) and atomically (temporary file, then rename). A file is
//!   used only for the issuer in its name and in its contents.
//! - [`begin_login`] and [`LoginHandle`]: the authorization-code flow with a
//!   redirect to a loopback listener on `127.0.0.1` at an ephemeral port. The
//!   caller opens the returned URL (see [`open_browser`]), then calls
//!   [`LoginHandle::finish`]. Tokens are stored when the callback is verified.
//!
//! Only a Streamable HTTP server takes these credentials; stdio servers do not
//! use OAuth. The client never builds a runtime or a thread: the futures here
//! run on the caller's runtime.

use std::io;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use rmcp::transport::auth::{
    AuthError, AuthorizationManager, AuthorizationRequest, AuthorizationSession, CredentialStore,
    StoredCredentials,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::error::McpError;

/// The Client ID Metadata Document (SEP-991) this client identifies itself
/// with. Used when the server advertises support for it.
pub const CLIENT_METADATA_URL: &str = "https://kolisachint.github.io/hoocode/oauth-client.json";
/// The name sent when the client registers dynamically.
pub const CLIENT_NAME: &str = "hoocode";
/// Path of the loopback redirect. The port is chosen at login.
pub const CALLBACK_PATH: &str = "/callback";

/// Directory holding the token files, under the hoocode data directory.
pub fn token_store_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("mcp-auth")
}

/// The token file for `issuer`. The name has the host (for people reading the
/// directory) and a 64-bit hash of the whole issuer, so two issuers never share
/// a file. The file also records its issuer, and that is checked on load.
pub fn token_store_path(data_dir: &Path, issuer: &str) -> PathBuf {
    let host: String = issuer
        .split_once("://")
        .map_or(issuer, |(_, rest)| rest)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' {
                c
            } else {
                '_'
            }
        })
        .take(60)
        .collect();
    token_store_dir(data_dir).join(format!("{host}-{:016x}.json", fnv1a64(issuer.as_bytes())))
}

/// FNV-1a, 64 bit. Only used to name files, so a stable hash is all it needs.
fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Token store for one issuer. rmcp calls it when it loads, saves and clears
/// credentials.
pub(crate) struct FileCredentialStore {
    dir: PathBuf,
    path: PathBuf,
    issuer: String,
}

impl FileCredentialStore {
    pub(crate) fn new(data_dir: &Path, issuer: &str) -> Self {
        Self {
            dir: token_store_dir(data_dir),
            path: token_store_path(data_dir, issuer),
            issuer: issuer.to_owned(),
        }
    }
}

#[async_trait]
impl CredentialStore for FileCredentialStore {
    async fn load(&self) -> Result<Option<StoredCredentials>, AuthError> {
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(store_error(&self.path, &e)),
        };
        let stored: StoredCredentials = serde_json::from_slice(&bytes).map_err(|e| {
            AuthError::CredentialStoreError(format!(
                "{}: not a token file: {e}",
                self.path.display()
            ))
        })?;
        if stored.issuer.as_deref() != Some(self.issuer.as_str()) {
            // A file for another issuer is never used for this one.
            return Ok(None);
        }
        Ok(Some(stored))
    }

    async fn save(&self, credentials: StoredCredentials) -> Result<(), AuthError> {
        let bytes = serde_json::to_vec_pretty(&credentials)
            .map_err(|e| AuthError::CredentialStoreError(e.to_string()))?;
        write_private(&self.dir, &self.path, &bytes).map_err(|e| store_error(&self.path, &e))
    }

    async fn clear(&self) -> Result<(), AuthError> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(store_error(&self.path, &e)),
        }
    }
}

fn store_error(path: &Path, error: &io::Error) -> AuthError {
    AuthError::CredentialStoreError(format!("{}: {error}", path.display()))
}

/// Writes `bytes` to `path` through a temporary file in `dir`, so a reader
/// never sees a partial file. The file is owner-only from its creation.
fn write_private(dir: &Path, path: &Path, bytes: &[u8]) -> io::Result<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    create_private_dir(dir)?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp = dir.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        io::Write::write_all(&mut file, bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result?;
    sync_dir(dir);
    Ok(())
}

/// Creates the token directory, owner-only, and tightens it if it already exists.
fn create_private_dir(dir: &Path) -> io::Result<()> {
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Makes the rename durable where the platform allows it. Best effort.
fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    {
        if let Ok(handle) = std::fs::File::open(dir) {
            let _ = handle.sync_all();
        }
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// Discovers the server's authorization server and returns a manager that
/// holds the stored credentials for it, if any.
///
/// A server that does not advertise OAuth gets a manager with no metadata and
/// no stored credentials, so its requests go out without a token.
pub(crate) async fn authorization_manager(
    server_url: &str,
    data_dir: &Path,
) -> Result<AuthorizationManager, McpError> {
    let mut manager = AuthorizationManager::new(server_url)
        .await
        .map_err(|e| McpError::Config(format!("server URL {server_url}: {e}")))?;
    let Ok(resolved) = manager.resolve_metadata().await else {
        return Ok(manager);
    };
    let Some(issuer) = resolved.metadata.issuer.clone() else {
        return Ok(manager);
    };
    manager.set_metadata(resolved.metadata);
    manager.set_credential_store(FileCredentialStore::new(data_dir, &issuer));
    manager
        .initialize_from_store()
        .await
        .map_err(|e| McpError::Auth(e.to_string()))?;
    Ok(manager)
}

/// A login in progress: the loopback listener and the authorization session.
/// Dropping it cancels the login.
pub struct LoginHandle {
    listener: TcpListener,
    session: AuthorizationSession,
    redirect_uri: String,
}

/// Starts a login with the server at `server_url`. Returns the URL to open in
/// the user's browser and the handle that completes the login.
///
/// The redirect goes to a listener on `127.0.0.1` at an ephemeral port that is
/// bound now. Calls to [`LoginHandle::finish`] accept that redirect.
pub async fn begin_login(
    server_url: &str,
    data_dir: &Path,
) -> Result<(String, LoginHandle), McpError> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|e| McpError::Connect(format!("cannot listen on 127.0.0.1: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| McpError::Connect(e.to_string()))?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}{CALLBACK_PATH}");
    let manager = authorization_manager(server_url, data_dir).await?;
    let request = AuthorizationRequest::new(redirect_uri.clone())
        .with_client_name(CLIENT_NAME)
        .with_client_metadata_url(CLIENT_METADATA_URL);
    let session = AuthorizationSession::new(manager, request)
        .await
        .map_err(|(_, e)| McpError::Auth(e.to_string()))?;
    let url = session.get_authorization_url().to_owned();
    Ok((
        url,
        LoginHandle {
            listener,
            session,
            redirect_uri,
        },
    ))
}

/// How long one request on the loopback listener may take to arrive.
const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(10);
/// Largest request head read from the browser.
const MAX_REQUEST_HEAD: usize = 16 * 1024;

impl LoginHandle {
    /// The redirect URI sent to the server, `http://127.0.0.1:<port>/callback`.
    pub fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }

    /// Waits up to `timeout` for the browser to reach the redirect URI, then
    /// verifies the callback and stores the tokens. Requests for other paths
    /// (a favicon, say) get a 404 and the wait goes on.
    pub async fn finish(self, timeout: Duration) -> Result<(), McpError> {
        let received = tokio::time::timeout(timeout, self.receive_callback())
            .await
            .map_err(|_| McpError::Auth("the login was not completed in time".to_owned()))??;
        let (mut stream, url) = received;
        let result = self.session.handle_callback_url(&url).await;
        respond_page(&mut stream, result.is_ok()).await;
        result
            .map(|_| ())
            .map_err(|e| McpError::Auth(e.to_string()))
    }

    /// Completes the login from a redirect URL the user pasted, for when the
    /// browser cannot reach the loopback listener. The state and `iss` in the
    /// URL are checked as they are for the loopback redirect.
    pub async fn finish_redirect(self, redirect_url: &str) -> Result<(), McpError> {
        self.session
            .handle_callback_url(redirect_url)
            .await
            .map(|_| ())
            .map_err(|e| McpError::Auth(e.to_string()))
    }

    /// Accepts connections until one is a request for the callback path.
    async fn receive_callback(&self) -> Result<(TcpStream, String), McpError> {
        let port = self
            .listener
            .local_addr()
            .map_err(|e| McpError::Connect(e.to_string()))?
            .port();
        loop {
            let (mut stream, _) = self
                .listener
                .accept()
                .await
                .map_err(|e| McpError::Connect(e.to_string()))?;
            let Ok(Ok(head)) =
                tokio::time::timeout(REQUEST_READ_TIMEOUT, read_head(&mut stream)).await
            else {
                continue;
            };
            let Some(target) = request_target(&head) else {
                continue;
            };
            let path = target.split('?').next().unwrap_or_default();
            if path != CALLBACK_PATH {
                respond(&mut stream, "404 Not Found", "text/plain", "not found").await;
                continue;
            }
            return Ok((stream, format!("http://127.0.0.1:{port}{target}")));
        }
    }
}

/// Reads a request head: everything up to the blank line, bounded.
async fn read_head(stream: &mut TcpStream) -> io::Result<String> {
    let mut head = Vec::new();
    let mut chunk = [0u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        if head.len() >= MAX_REQUEST_HEAD {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "request head too large",
            ));
        }
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        head.extend_from_slice(&chunk[..n]);
    }
    Ok(String::from_utf8_lossy(&head).into_owned())
}

/// The request target of a `GET` request line, for example `/callback?code=..`.
fn request_target(head: &str) -> Option<String> {
    let line = head.lines().next()?;
    let mut parts = line.split(' ');
    (parts.next()? == "GET").then_some(())?;
    parts.next().map(str::to_owned)
}

async fn respond_page(stream: &mut TcpStream, ok: bool) {
    let body = if ok {
        "<!doctype html><meta charset=utf-8><title>hoocode</title>\
         <p>Login complete. You can close this window and return to hoocode.</p>"
    } else {
        "<!doctype html><meta charset=utf-8><title>hoocode</title>\
         <p>Login failed. Return to hoocode for details.</p>"
    };
    respond(stream, "200 OK", "text/html; charset=utf-8", body).await;
}

async fn respond(stream: &mut TcpStream, status: &str, content_type: &str, body: &str) {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(body.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// Opens `url` in the user's browser, using `open` on macOS, `rundll32` on
/// Windows and `xdg-open` elsewhere. Returns false if no launcher started. The
/// launcher is not waited for.
pub fn open_browser(url: &str) -> bool {
    let (program, args): (&str, &[&str]) = if cfg!(target_os = "macos") {
        ("open", &[])
    } else if cfg!(windows) {
        ("rundll32", &["url.dll,FileProtocolHandler"])
    } else {
        ("xdg-open", &[])
    };
    Command::new(program)
        .args(args)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
}

//! The send path for an allowed plugin request: connect to the addresses the
//! SSRF check returned, never resolve the host again, then speak HTTP/1.1.
//!
//! The host name is used only for two things: the `Host` header (unless the
//! guest set one) and the TLS server name (SNI and certificate check). The TCP
//! connection goes to one of the checked [`SocketAddr`]s, tried in order, so a
//! DNS answer that changes between the check and the connect cannot move the
//! request to an address the check refused.
//!
//! HTTP/1.1 only. TLS uses rustls with the bundled webpki-roots and the
//! aws-lc-rs provider, chosen explicitly because the workspace enables both
//! rustls providers somewhere and the process default would be ambiguous.

use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use http::header::HOST;
use http::uri::PathAndQuery;
use http::{HeaderValue, Request, Response, Uri, Version};
use http_body_util::BodyExt;
use hyper_util::rt::TokioIo;
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, RootCertStore};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use wasmtime_wasi_http::{Error, RequestOptions, WasiBody};

/// The connect timeout when the guest sets none. It applies to each address
/// attempt, and separately to the TLS handshake.
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// The wait for the first response byte when the guest sets none.
const DEFAULT_FIRST_BYTE_TIMEOUT: Duration = Duration::from_secs(600);

/// The driver of an in-flight request. It reports a connection error that
/// happened after the response head arrived, such as a broken body.
pub(crate) type Outgoing = Box<dyn Future<Output = Result<(), Error>> + Send>;

/// Send `request` to `addrs`, which must be the addresses the SSRF check
/// returned for the request's host. Returns the response head and body, and the
/// driver future for the connection.
pub(crate) async fn send_checked(
    request: Request<WasiBody>,
    addrs: &[SocketAddr],
    options: Option<RequestOptions>,
) -> Result<(Response<WasiBody>, Outgoing), Error> {
    let connect_timeout = options
        .and_then(|o| o.connect_timeout)
        .unwrap_or(DEFAULT_CONNECT_TIMEOUT);
    let first_byte_timeout = options
        .and_then(|o| o.first_byte_timeout)
        .unwrap_or(DEFAULT_FIRST_BYTE_TIMEOUT);

    let tcp = connect_checked(addrs, connect_timeout).await?;
    if request.uri().scheme_str() == Some("https") {
        let name = server_name(request.uri())?;
        let tls = tls_handshake(tcp, name, connect_timeout).await?;
        exchange(tls, request, first_byte_timeout).await
    } else {
        exchange(tcp, request, first_byte_timeout).await
    }
}

/// Connect to the first of `addrs` that accepts, trying them in order. Returns
/// the last failure when none does.
async fn connect_checked(addrs: &[SocketAddr], timeout: Duration) -> Result<TcpStream, Error> {
    let mut last = Error::DestinationNotFound;
    for addr in addrs {
        match tokio::time::timeout(timeout, TcpStream::connect(addr)).await {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(err)) => last = map_connect_error(err),
            Err(_) => last = Error::ConnectionTimeout,
        }
    }
    Err(last)
}

fn map_connect_error(err: io::Error) -> Error {
    match err.kind() {
        io::ErrorKind::ConnectionRefused => Error::ConnectionRefused,
        io::ErrorKind::TimedOut => Error::ConnectionTimeout,
        _ => Error::Connect(err),
    }
}

/// The host of `uri` without IPv6 brackets.
fn uri_host(uri: &Uri) -> Result<&str, Error> {
    uri.host()
        .map(|h| h.trim_start_matches('[').trim_end_matches(']'))
        .filter(|h| !h.is_empty())
        .ok_or(Error::HttpRequestUriInvalid)
}

/// The TLS server name: an IP literal, or a DNS name for SNI and verification.
fn server_name(uri: &Uri) -> Result<ServerName<'static>, Error> {
    let host = uri_host(uri)?;
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(ServerName::from(ip));
    }
    ServerName::try_from(host.to_owned()).map_err(|_| Error::HttpRequestUriInvalid)
}

async fn tls_handshake(
    tcp: TcpStream,
    name: ServerName<'static>,
    timeout: Duration,
) -> Result<tokio_rustls::client::TlsStream<TcpStream>, Error> {
    let connector = TlsConnector::from(tls_config()?);
    match tokio::time::timeout(timeout, connector.connect(name, tcp)).await {
        Ok(Ok(stream)) => Ok(stream),
        Ok(Err(err)) => Err(map_tls_error(&err)),
        Err(_) => Err(Error::ConnectionTimeout),
    }
}

fn map_tls_error(err: &io::Error) -> Error {
    let rustls_err = err
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>());
    match rustls_err {
        Some(rustls::Error::InvalidCertificate(_)) => Error::TlsCertificateError,
        _ => Error::TlsProtocolError,
    }
}

/// The client TLS config, built once. Only a successful build is cached.
fn tls_config() -> Result<Arc<ClientConfig>, Error> {
    static CONFIG: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    if let Some(config) = CONFIG.get() {
        return Ok(config.clone());
    }
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|_| Error::TlsProtocolError)?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(CONFIG.get_or_init(|| Arc::new(config)).clone())
}

/// Run one HTTP/1.1 exchange over `io`. The request goes out in origin form,
/// with the authority kept as the `Host` header unless the guest set one.
async fn exchange<T>(
    io: T,
    request: Request<WasiBody>,
    first_byte_timeout: Duration,
) -> Result<(Response<WasiBody>, Outgoing), Error>
where
    T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let request = origin_form(request)?;
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(io)).await?;
    let driver = tokio::spawn(conn);

    let response =
        match tokio::time::timeout(first_byte_timeout, sender.send_request(request)).await {
            Ok(Ok(response)) => response,
            Ok(Err(err)) => {
                driver.abort();
                return Err(err.into());
            }
            Err(_) => {
                driver.abort();
                return Err(Error::HttpResponseTimeout);
            }
        };
    let response = response.map(|body| body.map_err(Error::from).boxed_unsync());
    let outgoing: Outgoing = Box::new(async move {
        match driver.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(Error::from(err)),
            Err(join) => Err(Error::InternalError(Some(format!(
                "connection task: {join}"
            )))),
        }
    });
    Ok((response, outgoing))
}

/// Rewrite the request to origin form for the wire, set HTTP/1.1, and keep the
/// original authority as `Host` when the guest did not set one.
fn origin_form(request: Request<WasiBody>) -> Result<Request<WasiBody>, Error> {
    let (mut parts, body) = request.into_parts();
    let path = parts
        .uri
        .path_and_query()
        .cloned()
        .unwrap_or_else(|| PathAndQuery::from_static("/"));
    if !parts.headers.contains_key(HOST) {
        let authority = parts
            .uri
            .authority()
            .ok_or(Error::HttpRequestUriInvalid)?
            .as_str();
        let value = HeaderValue::from_str(authority).map_err(|_| Error::HttpRequestUriInvalid)?;
        parts.headers.insert(HOST, value);
    }
    parts.uri = Uri::from(path);
    parts.version = Version::HTTP_11;
    Ok(Request::from_parts(parts, body))
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    use hoocode_runtime::{block_on_entry, spawn_thread};
    use http_body_util::Empty;

    use super::*;

    fn empty_body() -> WasiBody {
        Empty::new().map_err(Error::from).boxed_unsync()
    }

    /// Accept one connection, send the request head to the returned channel,
    /// and answer `200 ok`.
    fn serve_once() -> (SocketAddr, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let (tx, rx) = mpsc::channel();
        let _server = spawn_thread("connect-test-server", move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut head = Vec::new();
            let mut buf = [0u8; 1024];
            while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                match stream.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => head.extend_from_slice(&buf[..n]),
                }
            }
            let _ = tx.send(String::from_utf8_lossy(&head).into_owned());
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
        });
        (addr, rx)
    }

    /// A loopback port with nothing listening on it.
    fn closed_addr() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        listener.local_addr().expect("addr")
    }

    #[test]
    fn connects_to_checked_address_without_resolving_the_host() {
        block_on_entry(async {
            // `example.invalid` is reserved and never resolves, so reaching the
            // server proves the checked address was used, not a fresh lookup.
            let (addr, heads) = serve_once();
            let request = Request::builder()
                .uri("http://example.invalid/path?q=1")
                .body(empty_body())
                .expect("request");
            let (response, outgoing) = send_checked(request, &[addr], None).await.expect("send");
            assert_eq!(response.status(), 200);
            let body = response
                .into_body()
                .collect()
                .await
                .expect("body")
                .to_bytes();
            assert_eq!(&body[..], b"ok");
            Box::into_pin(outgoing).await.expect("connection");

            let head = heads.recv().expect("head");
            assert!(head.starts_with("GET /path?q=1 HTTP/1.1\r\n"), "{head}");
            assert!(
                head.to_ascii_lowercase()
                    .contains("\r\nhost: example.invalid\r\n"),
                "{head}"
            );
        });
    }

    #[test]
    fn falls_through_to_the_next_checked_address() {
        block_on_entry(async {
            let (addr, heads) = serve_once();
            let request = Request::builder()
                .uri("http://example.invalid/")
                .body(empty_body())
                .expect("request");
            let (response, _outgoing) = send_checked(request, &[closed_addr(), addr], None)
                .await
                .expect("send");
            assert_eq!(response.status(), 200);
            assert!(heads.recv().is_ok());
        });
    }

    #[test]
    fn refused_connection_maps_to_connection_refused() {
        block_on_entry(async {
            let request = Request::builder()
                .uri("http://example.invalid/")
                .body(empty_body())
                .expect("request");
            let err = send_checked(request, &[closed_addr()], None)
                .await
                .map(|_| ())
                .unwrap_err();
            assert!(matches!(err, Error::ConnectionRefused), "{err:?}");
        });
    }
}

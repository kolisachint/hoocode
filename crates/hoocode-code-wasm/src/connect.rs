//! The send path for an allowed plugin request: connect to the addresses the
//! SSRF check returned, never resolve the host again, then speak HTTP/1.1.
//!
//! The host name is used only for two things: the `Host` header (unless the
//! guest set one) and the TLS server name (SNI and certificate check). The TCP
//! connection goes to one of the checked [`SocketAddr`]s, tried in order, so a
//! DNS answer that changes between the check and the connect cannot move the
//! request to an address the check refused.
//!
//! HTTP/1.1 only. TLS uses rustls with the bundled webpki-roots plus the host's
//! [`TrustedCas`] (the same trust set the providers use: `--ca-cert`,
//! `HOOCODE_CA_CERT`, `NODE_EXTRA_CA_CERTS`, `--use-system-ca`), on the aws-lc-rs
//! provider, chosen explicitly because the workspace enables both rustls
//! providers somewhere and the process default would be ambiguous. Verification
//! is always on.

use std::collections::HashSet;
use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock};
use std::task::{Context, Poll};
use std::time::Duration;

use hoocode_ai_util::tls::{ExtraCa, TrustedCas};
use http::header::HOST;
use http::uri::PathAndQuery;
use http::{HeaderValue, Request, Response, Uri, Version};
use http_body_util::BodyExt;
use hyper::body::{Body, Bytes, Frame, SizeHint};
use hyper_util::rt::TokioIo;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName};
use rustls::{ClientConfig, RootCertStore};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::time::{Instant, Sleep};
use tokio_rustls::TlsConnector;
use wasmtime_wasi_http::{Error, RequestOptions, WasiBody};

use crate::write_host_log;

/// The connect timeout when the guest sets none. It applies to each address
/// attempt, and separately to the TLS handshake.
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// The wait for the first response byte when the guest sets none.
const DEFAULT_FIRST_BYTE_TIMEOUT: Duration = Duration::from_secs(600);

/// The longest gap between two response body frames when the guest sets none.
const DEFAULT_BETWEEN_BYTES_TIMEOUT: Duration = Duration::from_secs(600);

/// The driver of an in-flight request. It reports a connection error that
/// happened after the response head arrived, such as a broken body.
pub(crate) type Outgoing = Box<dyn Future<Output = Result<(), Error>> + Send>;

/// Send `request` to `addrs`, which must be the addresses the SSRF check
/// returned for the request's host. `tls` verifies https servers and is only
/// used for an https request. Returns the response head and body, and the
/// driver future for the connection.
pub(crate) async fn send_checked(
    request: Request<WasiBody>,
    addrs: &[SocketAddr],
    options: Option<RequestOptions>,
    tls: Arc<ClientConfig>,
) -> Result<(Response<WasiBody>, Outgoing), Error> {
    let connect_timeout = options
        .and_then(|o| o.connect_timeout)
        .unwrap_or(DEFAULT_CONNECT_TIMEOUT);
    let first_byte_timeout = options
        .and_then(|o| o.first_byte_timeout)
        .unwrap_or(DEFAULT_FIRST_BYTE_TIMEOUT);
    let between_bytes_timeout = options
        .and_then(|o| o.between_bytes_timeout)
        .unwrap_or(DEFAULT_BETWEEN_BYTES_TIMEOUT);

    let tcp = connect_checked(addrs, connect_timeout).await?;
    let timeouts = Timeouts {
        first_byte: first_byte_timeout,
        between_bytes: between_bytes_timeout,
    };
    if request.uri().scheme_str() == Some("https") {
        let name = server_name(request.uri())?;
        let stream = tls_handshake(tcp, name, tls, connect_timeout).await?;
        exchange(stream, request, timeouts).await
    } else {
        exchange(tcp, request, timeouts).await
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
    config: Arc<ClientConfig>,
    timeout: Duration,
) -> Result<tokio_rustls::client::TlsStream<TcpStream>, Error> {
    let connector = TlsConnector::from(config);
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

/// Writes a host log line the first time `text` is seen in this process. The TLS
/// config is rebuilt for every build, refresh and child, so a skipped CA would
/// otherwise be reported again each time.
fn log_once(text: &str) {
    static SEEN: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let seen = SEEN.get_or_init(Mutex::default);
    let first = seen
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(text.to_owned());
    if first {
        write_host_log(text);
    }
}

/// The client TLS config for the send path: the bundled webpki roots plus the
/// certificates of `trust`. Verification is never disabled.
pub(crate) fn tls_config(trust: &TrustedCas) -> Result<Arc<ClientConfig>, rustls::Error> {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    for cert in extra_certificates(trust) {
        // A certificate the store refuses is skipped: trust only ever grows
        // by what verifies, so this cannot widen it.
        if let Err(err) = roots.add(cert) {
            log_once(&format!("plugin TLS: skipping a CA certificate ({err})\n"));
        }
    }
    Ok(Arc::new(client_config(roots)?))
}

/// The extra trust set as DER certificates. A PEM bundle that does not parse is
/// logged and skipped, as the providers do.
fn extra_certificates(trust: &TrustedCas) -> Vec<CertificateDer<'static>> {
    let mut out = Vec::new();
    for ca in &trust.extra {
        match ca {
            ExtraCa::Pem(pem) => {
                match CertificateDer::pem_slice_iter(pem.as_bytes()).collect::<Result<Vec<_>, _>>()
                {
                    Ok(certs) => out.extend(certs),
                    Err(err) => log_once(&format!(
                        "plugin TLS: CA certificate bundle could not be parsed, skipping ({err})\n"
                    )),
                }
            }
            ExtraCa::SystemDer(der) => out.push(CertificateDer::from(der.clone())),
        }
    }
    out
}

/// A client config that verifies server certificates against `roots`, on the
/// aws-lc-rs provider with the safe default protocol versions.
fn client_config(roots: RootCertStore) -> Result<ClientConfig, rustls::Error> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    Ok(ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_root_certificates(roots)
        .with_no_client_auth())
}

/// The wait limits of one exchange.
#[derive(Clone, Copy)]
struct Timeouts {
    /// From sending the request to the response head.
    first_byte: Duration,
    /// Between two body frames, and from the head to the first frame.
    between_bytes: Duration,
}

/// A response body that fails when no frame arrives within `timeout` of the
/// previous one. The timer is re-armed on every frame.
struct BetweenBytes<B> {
    body: B,
    timeout: Duration,
    timer: Pin<Box<Sleep>>,
}

impl<B> BetweenBytes<B> {
    fn new(body: B, timeout: Duration) -> Self {
        Self {
            body,
            timeout,
            timer: Box::pin(tokio::time::sleep(timeout)),
        }
    }
}

impl<B> Body for BetweenBytes<B>
where
    B: Body<Data = Bytes, Error = Error> + Unpin,
{
    type Data = Bytes;
    type Error = Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Error>>> {
        let this = self.get_mut();
        match Pin::new(&mut this.body).poll_frame(cx) {
            Poll::Ready(item) => {
                this.timer.as_mut().reset(Instant::now() + this.timeout);
                Poll::Ready(item)
            }
            Poll::Pending => match this.timer.as_mut().poll(cx) {
                Poll::Ready(()) => Poll::Ready(Some(Err(Error::ConnectionReadTimeout))),
                Poll::Pending => Poll::Pending,
            },
        }
    }

    fn is_end_stream(&self) -> bool {
        self.body.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.body.size_hint()
    }
}

/// Run one HTTP/1.1 exchange over `io`. The request goes out in origin form,
/// with the authority kept as the `Host` header unless the guest set one.
async fn exchange<T>(
    io: T,
    request: Request<WasiBody>,
    timeouts: Timeouts,
) -> Result<(Response<WasiBody>, Outgoing), Error>
where
    T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let request = origin_form(request)?;
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(io)).await?;
    let driver = tokio::spawn(conn);

    let response =
        match tokio::time::timeout(timeouts.first_byte, sender.send_request(request)).await {
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
    let between = timeouts.between_bytes;
    let response = response.map(|body| {
        let body = body.map_err(Error::from).boxed_unsync();
        BetweenBytes::new(body, between).boxed_unsync()
    });
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
    use rcgen::{BasicConstraints, CertificateParams, CertifiedIssuer, IsCa, KeyPair};
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    use rustls::server::Acceptor;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_rustls::LazyConfigAcceptor;

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
            let (response, outgoing) = send_checked(request, &[addr], None, plain_tls())
                .await
                .expect("send");
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
            let (response, _outgoing) =
                send_checked(request, &[closed_addr(), addr], None, plain_tls())
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
            let err = send_checked(request, &[closed_addr()], None, plain_tls())
                .await
                .map(|_| ())
                .unwrap_err();
            assert!(matches!(err, Error::ConnectionRefused), "{err:?}");
        });
    }

    /// A throwaway CA for the https tests, so no real root is needed.
    struct TestPki {
        ca: CertifiedIssuer<'static, KeyPair>,
    }

    /// A leaf certificate and its key, as the test server presents them.
    struct ServerCert {
        chain: Vec<CertificateDer<'static>>,
        key: PrivateKeyDer<'static>,
    }

    impl TestPki {
        fn new() -> Self {
            let mut params = CertificateParams::new(Vec::<String>::new()).expect("ca params");
            params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
            let key = KeyPair::generate().expect("ca key");
            let ca = CertifiedIssuer::self_signed(params, key).expect("ca");
            Self { ca }
        }

        /// A certificate for `name`, issued by this CA.
        fn leaf(&self, name: &str) -> ServerCert {
            let key = KeyPair::generate().expect("leaf key");
            let params = CertificateParams::new(vec![name.to_owned()]).expect("leaf params");
            let cert = params.signed_by(&key, &*self.ca).expect("leaf");
            ServerCert {
                chain: vec![cert.der().clone()],
                key: PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
            }
        }
    }

    /// The config that trusts only the bundled roots.
    fn plain_tls() -> Arc<ClientConfig> {
        tls_config(&TrustedCas::default()).expect("tls config")
    }

    /// A trust set holding the test CA as an explicit PEM bundle, as
    /// `--ca-cert` supplies it.
    fn trust_with(pki: &TestPki) -> TrustedCas {
        TrustedCas {
            extra: vec![ExtraCa::Pem(pki.ca.pem())],
        }
    }

    /// What the TLS test server saw of one connection.
    struct Seen {
        sni: Option<String>,
        head: String,
    }

    /// Serve one TLS connection on 127.0.0.1 with `cert`, read the request head,
    /// report what the client sent, and answer `200 ok`. A failed handshake
    /// reports nothing.
    async fn serve_tls(cert: ServerCert) -> (SocketAddr, mpsc::Receiver<Seen>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let config = Arc::new(
            rustls::ServerConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .expect("versions")
                .with_no_client_auth()
                .with_single_cert(cert.chain, cert.key)
                .expect("server cert"),
        );
        let (tx, rx) = mpsc::channel();
        tokio::spawn(async move {
            let Ok((tcp, _)) = listener.accept().await else {
                return;
            };
            let Ok(start) = LazyConfigAcceptor::new(Acceptor::default(), tcp).await else {
                return;
            };
            let sni = start.client_hello().server_name().map(str::to_owned);
            let Ok(mut tls) = start.into_stream(config).await else {
                return;
            };
            let mut head = Vec::new();
            let mut buf = [0u8; 1024];
            while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                match tls.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => head.extend_from_slice(&buf[..n]),
                }
            }
            let _ = tx.send(Seen {
                sni,
                head: String::from_utf8_lossy(&head).into_owned(),
            });
            let _ = tls
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await;
            let _ = tls.shutdown().await;
        });
        (addr, rx)
    }

    fn https_request(port: u16) -> Request<WasiBody> {
        Request::builder()
            .uri(format!("https://localhost.test:{port}/"))
            .body(empty_body())
            .expect("request")
    }

    /// The error of an https send to `addr` whose client trusts `trust`.
    async fn tls_error(addr: SocketAddr, trust: &TrustedCas) -> Error {
        let request = https_request(addr.port());
        let config = tls_config(trust).expect("tls config");
        match send_checked(request, &[addr], None, config).await {
            Ok(_) => panic!("the TLS handshake should have failed"),
            Err(err) => err,
        }
    }

    #[test]
    fn https_send_trusts_an_extra_ca_from_the_trust_set() {
        block_on_entry(async {
            let pki = TestPki::new();
            let (addr, seen) = serve_tls(pki.leaf("localhost.test")).await;
            let config = tls_config(&trust_with(&pki)).expect("tls config");
            let request = https_request(addr.port());
            let (response, outgoing) = send_checked(request, &[addr], None, config)
                .await
                .expect("send through the extra CA");
            assert_eq!(response.status(), 200);
            Box::into_pin(outgoing).await.expect("connection");
            assert!(seen.recv().is_ok(), "the server saw the request");
        });
    }

    #[test]
    fn https_send_verifies_against_the_root_and_sends_sni_and_host() {
        block_on_entry(async {
            let pki = TestPki::new();
            let (addr, seen) = serve_tls(pki.leaf("localhost.test")).await;
            let config = tls_config(&trust_with(&pki)).expect("tls config");
            let request = https_request(addr.port());
            let (response, outgoing) = send_checked(request, &[addr], None, config)
                .await
                .expect("send");
            assert_eq!(response.status(), 200);
            let body = response
                .into_body()
                .collect()
                .await
                .expect("body")
                .to_bytes();
            assert_eq!(&body[..], b"ok");
            Box::into_pin(outgoing).await.expect("connection");

            let seen = seen.recv().expect("server saw the request");
            assert_eq!(seen.sni.as_deref(), Some("localhost.test"));
            assert!(seen.head.starts_with("GET / HTTP/1.1\r\n"), "{}", seen.head);
            let host = format!("host: localhost.test:{}\r\n", addr.port());
            assert!(
                seen.head.to_ascii_lowercase().contains(&host),
                "{}",
                seen.head
            );
        });
    }

    #[test]
    fn https_send_rejects_a_certificate_for_another_name() {
        block_on_entry(async {
            let pki = TestPki::new();
            let (addr, seen) = serve_tls(pki.leaf("other.test")).await;
            let err = tls_error(addr, &trust_with(&pki)).await;
            assert!(matches!(err, Error::TlsCertificateError), "{err:?}");
            assert!(seen.recv().is_err(), "no request may reach the server");
        });
    }

    #[test]
    fn https_send_rejects_an_untrusted_certificate() {
        block_on_entry(async {
            let pki = TestPki::new();
            let (addr, seen) = serve_tls(pki.leaf("localhost.test")).await;
            // No extra CA: only the bundled roots, which do not include the test CA.
            let err = tls_error(addr, &TrustedCas::default()).await;
            assert!(matches!(err, Error::TlsCertificateError), "{err:?}");
            assert!(seen.recv().is_err(), "no request may reach the server");
        });
    }
}

//! The gated `wasi:http` path: the SSRF block, the gate and the send, through
//! the fetch tool of the echo fixture.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hoocode_ai_types::AbortSignal;
use hoocode_code_wasm::{CallEnv, Extension, ExtensionSpec, NetGate, WasmError, WasmHost};
use hoocode_runtime::{block_on_entry, spawn_thread};
use serde_json::json;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/echo-ext.wasm")
}

/// A gate that records every URL it is asked about and allows or refuses all.
struct CountingGate {
    allow: bool,
    urls: Mutex<Vec<String>>,
}

impl CountingGate {
    fn new(allow: bool) -> Arc<Self> {
        Arc::new(Self {
            allow,
            urls: Mutex::new(Vec::new()),
        })
    }

    fn asked(&self) -> Vec<String> {
        self.urls.lock().unwrap().clone()
    }
}

impl NetGate for CountingGate {
    fn check(&self, url: &str) -> Result<(), String> {
        self.urls.lock().unwrap().push(url.to_owned());
        if self.allow {
            Ok(())
        } else {
            Err("refused by the test gate".to_owned())
        }
    }
}

fn env(gate: &Arc<CountingGate>) -> CallEnv {
    env_abort(gate, None)
}

fn env_abort(gate: &Arc<CountingGate>, abort: Option<AbortSignal>) -> CallEnv {
    let net: Arc<dyn NetGate> = gate.clone();
    CallEnv {
        config: Arc::new(|_| None),
        net,
        abort,
    }
}

/// A gate that takes `wait` before it allows the request, as a user prompt would.
struct SlowGate(Duration);

impl NetGate for SlowGate {
    fn check(&self, _url: &str) -> Result<(), String> {
        std::thread::sleep(self.0);
        Ok(())
    }
}

/// A local server that answers every connection with `200 hello`. `hits`
/// counts the connections it accepted.
struct Server {
    port: u16,
    hits: Arc<AtomicUsize>,
}

fn serve_hello() -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    let _server = spawn_thread("http-test-server", move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            counter.fetch_add(1, Ordering::SeqCst);
            let mut head = Vec::new();
            let mut buf = [0u8; 1024];
            while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                match stream.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => head.extend_from_slice(&buf[..n]),
                }
            }
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello",
            );
        }
    });
    Server { port, hits }
}

fn load(dir: &Path, host: WasmHost) -> Extension {
    host.load(ExtensionSpec {
        plugin: "echo".to_owned(),
        wasm_path: fixture(),
        data_dir: dir.join("data"),
    })
    .expect("load fixture")
}

fn host(dir: &Path) -> WasmHost {
    WasmHost::new(dir.join("cache")).expect("host")
}

#[test]
fn loopback_is_refused_before_the_gate_by_default() {
    block_on_entry(async {
        let dir = tempfile::tempdir().unwrap();
        let ext = load(dir.path(), host(dir.path()));
        let server = serve_hello();
        let gate = CountingGate::new(true);
        let url = format!("http://127.0.0.1:{}/", server.port);
        let err = ext
            .call("fetch", json!({ "url": url }), env(&gate))
            .await
            .unwrap_err();
        assert!(matches!(err, WasmError::Failed(_)), "{err:?}");
        assert!(
            gate.asked().is_empty(),
            "gate was asked: {:?}",
            gate.asked()
        );
        assert_eq!(server.hits.load(Ordering::SeqCst), 0);
    });
}

#[test]
fn loopback_allowed_and_gate_allowing_fetches() {
    block_on_entry(async {
        let dir = tempfile::tempdir().unwrap();
        let ext = load(dir.path(), host(dir.path()).with_loopback_allowed());
        let server = serve_hello();
        let gate = CountingGate::new(true);
        let url = format!("http://127.0.0.1:{}/", server.port);
        let out = ext
            .call("fetch", json!({ "url": url }), env(&gate))
            .await
            .unwrap();
        assert_eq!(out, json!({ "status": 200, "body": "hello" }));
        assert_eq!(gate.asked(), vec![url]);
        assert_eq!(server.hits.load(Ordering::SeqCst), 1);
    });
}

#[test]
fn loopback_allowed_but_gate_refusing_sends_nothing() {
    block_on_entry(async {
        let dir = tempfile::tempdir().unwrap();
        let ext = load(dir.path(), host(dir.path()).with_loopback_allowed());
        let server = serve_hello();
        let gate = CountingGate::new(false);
        let url = format!("http://127.0.0.1:{}/", server.port);
        let err = ext
            .call("fetch", json!({ "url": url }), env(&gate))
            .await
            .unwrap_err();
        assert!(matches!(err, WasmError::Failed(_)), "{err:?}");
        assert_eq!(gate.asked(), vec![url]);
        assert_eq!(server.hits.load(Ordering::SeqCst), 0);
    });
}

#[test]
fn private_and_mapped_addresses_are_refused_without_asking_the_gate() {
    block_on_entry(async {
        let dir = tempfile::tempdir().unwrap();
        let ext = load(dir.path(), host(dir.path()));
        let server = serve_hello();
        let gate = CountingGate::new(true);
        for url in [
            "http://10.0.0.1/".to_owned(),
            format!("http://[::ffff:127.0.0.1]:{}/", server.port),
        ] {
            let err = ext
                .call("fetch", json!({ "url": url }), env(&gate))
                .await
                .unwrap_err();
            assert!(matches!(err, WasmError::Failed(_)), "{url}: {err:?}");
        }
        assert!(
            gate.asked().is_empty(),
            "gate was asked: {:?}",
            gate.asked()
        );
        assert_eq!(server.hits.load(Ordering::SeqCst), 0);
    });
}

#[test]
fn non_http_schemes_are_refused_without_asking_the_gate() {
    block_on_entry(async {
        let dir = tempfile::tempdir().unwrap();
        let ext = load(dir.path(), host(dir.path()).with_loopback_allowed());
        let gate = CountingGate::new(true);
        for url in ["gopher://127.0.0.1:70/", "file://localhost/etc/passwd"] {
            let err = ext
                .call("fetch", json!({ "url": url }), env(&gate))
                .await
                .unwrap_err();
            assert!(matches!(err, WasmError::Failed(_)), "{url}: {err:?}");
        }
        assert!(
            gate.asked().is_empty(),
            "gate was asked: {:?}",
            gate.asked()
        );
    });
}

/// A local server that sends a head promising 100 body bytes, sends two of
/// them, and then holds the connection open without sending more.
fn serve_stalled() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let _server = spawn_thread("http-stall-server", move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut head = Vec::new();
            let mut buf = [0u8; 1024];
            while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                match stream.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => head.extend_from_slice(&buf[..n]),
                }
            }
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\nhe",
            );
            let _ = stream.flush();
            std::thread::sleep(Duration::from_secs(30));
        }
    });
    port
}

#[test]
fn stalled_body_hits_the_call_timeout() {
    block_on_entry(async {
        let dir = tempfile::tempdir().unwrap();
        let host = host(dir.path())
            .with_loopback_allowed()
            .with_call_timeout(Duration::from_secs(1));
        let ext = load(dir.path(), host);
        let port = serve_stalled();
        let gate = CountingGate::new(true);
        let url = format!("http://127.0.0.1:{port}/");
        let started = Instant::now();
        let err = ext
            .call("fetch", json!({ "url": url }), env(&gate))
            .await
            .unwrap_err();
        // The guest is blocked in a host read, so only the wall-clock timer can stop it.
        assert!(matches!(err, WasmError::Timeout), "{err:?}");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
    });
}

#[test]
fn abort_during_a_stalled_body_cancels_quickly() {
    block_on_entry(async {
        let dir = tempfile::tempdir().unwrap();
        let ext = load(dir.path(), host(dir.path()).with_loopback_allowed());
        let port = serve_stalled();
        let gate = CountingGate::new(true);
        let signal = AbortSignal::new();
        let trigger = signal.clone();
        spawn_thread("stall-abort-trigger", move || {
            std::thread::sleep(Duration::from_millis(300));
            trigger.abort();
        });
        let url = format!("http://127.0.0.1:{port}/");
        let started = Instant::now();
        let err = ext
            .call(
                "fetch",
                json!({ "url": url }),
                env_abort(&gate, Some(signal)),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, WasmError::Cancelled), "{err:?}");
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "{:?}",
            started.elapsed()
        );
    });
}

#[test]
fn gate_wait_does_not_count_toward_the_call_timeout() {
    block_on_entry(async {
        let dir = tempfile::tempdir().unwrap();
        let host = host(dir.path())
            .with_loopback_allowed()
            .with_call_timeout(Duration::from_secs(1));
        let ext = load(dir.path(), host);
        let server = serve_hello();
        let net: Arc<dyn NetGate> = Arc::new(SlowGate(Duration::from_millis(1500)));
        let env = CallEnv {
            config: Arc::new(|_| None),
            net,
            abort: None,
        };
        let url = format!("http://127.0.0.1:{}/", server.port);
        let out = ext
            .call("fetch", json!({ "url": url }), env)
            .await
            .expect("the 1.5 s gate wait is not call time");
        assert_eq!(out, json!({ "status": 200, "body": "hello" }));
        assert_eq!(server.hits.load(Ordering::SeqCst), 1);
    });
}

/// The gate comes before DNS, so an unresolvable name is asked about like any
/// other host. Once allowed, the lookup fails and the guest gets the same
/// `HttpRequestDenied` a block gets, so it cannot tell the two apart. The
/// lookup itself is counted in the unit tests of `http.rs`.
#[test]
fn unresolvable_name_is_asked_then_denied_with_the_block_error() {
    block_on_entry(async {
        let dir = tempfile::tempdir().unwrap();
        let ext = load(dir.path(), host(dir.path()));
        let gate = CountingGate::new(true);
        let url = "http://nonexistent.invalid/".to_owned();
        let err = ext
            .call("fetch", json!({ "url": url }), env(&gate))
            .await
            .unwrap_err();
        match &err {
            WasmError::Failed(msg) => {
                assert!(msg.contains("HttpRequestDenied"), "{msg}");
                assert!(!msg.contains("DnsError"), "{msg}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
        assert_eq!(gate.asked(), vec![url]);
    });
}

#[test]
fn gate_refused_unresolvable_name_gets_the_same_error() {
    block_on_entry(async {
        let dir = tempfile::tempdir().unwrap();
        let ext = load(dir.path(), host(dir.path()));
        let gate = CountingGate::new(false);
        let err = ext
            .call(
                "fetch",
                json!({ "url": "https://secret.nonexistent.invalid/" }),
                env(&gate),
            )
            .await
            .unwrap_err();
        match &err {
            WasmError::Failed(msg) => {
                assert!(msg.contains("HttpRequestDenied"), "{msg}");
                assert!(!msg.contains("DnsError"), "{msg}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
        assert_eq!(gate.asked().len(), 1);
    });
}

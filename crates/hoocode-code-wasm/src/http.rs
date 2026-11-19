//! The gated `wasi:http` outgoing handler.
//!
//! Every request a plugin makes is checked in this order, and a refused request
//! fails with the WASI `http-request-denied` code:
//!
//! 1. Before the gate, only the scheme and a literal IP host are checked
//!    ([`crate::ssrf::check_literal`]). No DNS lookup happens here, so a plugin
//!    cannot leak data through the names it looks up, or probe internal names,
//!    for a request the user refuses.
//! 2. The host's [`NetGate`](crate::NetGate), on a blocking thread because the
//!    gate may prompt the user. A call that has already ended is not asked.
//! 3. Only after the gate allows the request, the host name is resolved and
//!    every address checked ([`crate::ssrf::resolve_and_check`]). A DNS failure
//!    and a blocked address give the same error to the guest; the reason goes to
//!    the host log.
//!
//! An allowed request is sent by [`crate::connect`], which connects to the
//! addresses checked in step 3 and never resolves the host again.
//!
//! In wasmtime 48 `send_request` is a method of `WasiHttpHooks`, reached through
//! `WasiHttpView::http()`, not of `WasiHttpView` itself.

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use http::{Request, Response};
use wasmtime_wasi_http::{Error, RequestOptions, WasiBody, WasiHttpHooks};

use crate::connect::{self, Outgoing};
use crate::ssrf::{self, CheckError, Resolve};
use crate::{AbortSignal, NetGate};

/// The most time one call may spend paused on the network gate. Past it, a pause
/// no longer extends the call's deadlines, so a prompt left open cannot keep a
/// guest alive forever.
pub(crate) const MAX_PAUSE: Duration = Duration::from_secs(300);

/// The result of one send: the response head and body, and the connection driver.
type SendOutcome = Result<(Response<WasiBody>, Outgoing), Error>;

/// The future a send returns to wasmtime.
type SendFuture = Box<dyn Future<Output = SendOutcome> + Send>;

/// The per-call hooks: the SSRF block and the gate in front of the default send path.
pub(crate) struct GatedHooks {
    pub(crate) net: Arc<dyn NetGate>,
    /// Test-only: let loopback addresses through the SSRF block.
    pub(crate) allow_loopback: bool,
    /// The TLS config for https sends: bundled roots plus the host's trust set.
    pub(crate) tls: Arc<rustls::ClientConfig>,
    /// Time spent waiting on the gate, which the call timeout does not count.
    pub(crate) clock: Arc<PauseClock>,
    /// The call's abort signal. An aborted call is refused before the gate is asked.
    pub(crate) abort: Option<AbortSignal>,
}

/// The time one call spent waiting on the network gate (which may prompt the
/// user). The call's timeouts subtract it, so a slow answer from the user does
/// not use up the 30 s. Waits that overlap count once. The total is capped at
/// [`MAX_PAUSE`].
pub(crate) struct PauseClock {
    state: Mutex<PauseState>,
    cap: Duration,
}

#[derive(Default)]
struct PauseState {
    /// Gate waits that have ended.
    done: Duration,
    /// Gate waits in progress, and when the first of them began.
    open: usize,
    since: Option<Instant>,
}

impl Default for PauseClock {
    fn default() -> Self {
        Self {
            state: Mutex::default(),
            cap: MAX_PAUSE,
        }
    }
}

impl PauseClock {
    #[cfg(test)]
    fn with_cap(cap: Duration) -> Self {
        Self {
            state: Mutex::default(),
            cap,
        }
    }

    fn lock(&self) -> MutexGuard<'_, PauseState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Total time spent paused so far, including a wait still in progress,
    /// capped at the clock's cap.
    pub(crate) fn paused(&self) -> Duration {
        let state = self.lock();
        let total = state.done + state.since.map_or(Duration::ZERO, |t| t.elapsed());
        total.min(self.cap)
    }

    /// Start a pause. It ends when the guard is dropped.
    pub(crate) fn pause(&self) -> PauseGuard<'_> {
        let mut state = self.lock();
        if state.open == 0 {
            state.since = Some(Instant::now());
        }
        state.open += 1;
        PauseGuard(self)
    }
}

/// Ends a pause on drop, so a cancelled gate wait still stops the clock.
pub(crate) struct PauseGuard<'a>(&'a PauseClock);

impl Drop for PauseGuard<'_> {
    fn drop(&mut self) {
        let mut state = self.0.lock();
        state.open -= 1;
        if state.open == 0 {
            if let Some(since) = state.since.take() {
                state.done += since.elapsed();
            }
        }
    }
}

/// Marks one send as live. Its flag is cleared when the send future is dropped,
/// which happens when the call times out, is aborted or ends. A gate request
/// still queued on the blocking pool then does not prompt for a dead call.
struct Liveness(Arc<AtomicBool>);

impl Liveness {
    fn new() -> Self {
        Self(Arc::new(AtomicBool::new(true)))
    }

    fn flag(&self) -> Arc<AtomicBool> {
        self.0.clone()
    }
}

impl Drop for Liveness {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Ask the gate about `url`, unless the call that asked has already ended.
fn ask_gate(alive: &AtomicBool, net: &dyn NetGate, url: &str) -> Result<(), String> {
    if !alive.load(Ordering::SeqCst) {
        return Err(format!("{url}: the call ended before the gate was asked"));
    }
    net.check(url)
}

/// Log a refusal, tell the net gate its reason, and fail the request with
/// `http-request-denied`. The guest gets the same code for every refusal, so it
/// cannot tell a policy block from a lookup failure.
fn refuse(net: &dyn NetGate, detail: &str) -> Result<(Response<WasiBody>, Outgoing), Error> {
    crate::write_host_log(&format!("wasm http request refused: {detail}\n"));
    net.refused(detail);
    Err(Error::HttpRequestDenied)
}

impl GatedHooks {
    /// The gated send, with the name resolver supplied by the caller (the
    /// system resolver in production, a counting one in tests).
    fn send_with(
        &self,
        request: Request<WasiBody>,
        options: Option<RequestOptions>,
        resolve: Resolve,
    ) -> SendFuture {
        let net = self.net.clone();
        let allow_loopback = self.allow_loopback;
        let tls = self.tls.clone();
        let clock = self.clock.clone();
        let abort = self.abort.clone();
        let liveness = Liveness::new();
        let alive = liveness.flag();
        Box::new(async move {
            let _liveness = liveness;
            let url = request.uri().to_string();

            // 1. Before the gate: scheme, host, and a literal IP host. No DNS.
            let target = match ssrf::parse_target(request.uri()) {
                Ok(target) => target,
                Err(reason) => return refuse(net.as_ref(), &format!("{url}: {reason}")),
            };
            if let Err(reason) = ssrf::check_literal(&target, allow_loopback) {
                return refuse(net.as_ref(), &format!("{url}: {reason}"));
            }

            // 2. The gate. The wait may be a user prompt; it does not count
            // against the call timeout (see PauseClock). An aborted call is
            // refused first, so the user is not asked for it.
            if abort.as_ref().is_some_and(AbortSignal::aborted) {
                return refuse(
                    net.as_ref(),
                    &format!("{url}: the call was aborted before the gate was asked"),
                );
            }
            let pause = clock.pause();
            let gate_url = url.clone();
            let gate_net = net.clone();
            let verdict = hoocode_runtime::run_blocking(move || {
                ask_gate(&alive, gate_net.as_ref(), &gate_url)
            })
            .await;
            drop(pause);
            match verdict {
                Ok(Ok(())) => {}
                Ok(Err(reason)) => return refuse(net.as_ref(), &reason),
                Err(e) => return Err(Error::InternalError(Some(format!("net gate: {e}")))),
            }

            // 3. Only after Allow: resolve the host and check every address. A
            // DNS failure and a blocked address are refused the same way.
            let checked = hoocode_runtime::run_blocking(move || {
                ssrf::resolve_and_check(&target, allow_loopback, resolve.as_ref())
            })
            .await;
            let addrs = match checked {
                Ok(Ok(addrs)) => addrs,
                Ok(Err(CheckError::Dns(reason) | CheckError::Blocked(reason))) => {
                    return refuse(net.as_ref(), &format!("{url}: {reason}"));
                }
                Err(e) => return Err(Error::InternalError(Some(format!("ssrf check: {e}")))),
            };

            // The send connects to `addrs`, the addresses checked above, and does
            // not resolve the host again, so a changed DNS answer cannot move the
            // request to a refused address.
            connect::send_checked(request, &addrs, options, tls).await
        })
    }
}

impl WasiHttpHooks for GatedHooks {
    fn send_request(
        &mut self,
        request: Request<WasiBody>,
        options: Option<RequestOptions>,
        _fut: Box<dyn Future<Output = Result<(), Error>> + Send>,
    ) -> SendFuture {
        self.send_with(request, options, ssrf::system_resolver())
    }
}

#[cfg(test)]
mod tests {
    use hoocode_ai_util::tls::TrustedCas;
    use std::io;
    use std::net::SocketAddr;
    use std::sync::atomic::AtomicUsize;

    use hoocode_runtime::block_on_entry;
    use http_body_util::{BodyExt, Empty};

    use super::*;

    /// A gate that counts the URLs it is asked about and allows or refuses all.
    struct Gate {
        allow: bool,
        asked: AtomicUsize,
    }

    impl NetGate for Gate {
        fn check(&self, _url: &str) -> Result<(), String> {
            self.asked.fetch_add(1, Ordering::SeqCst);
            if self.allow {
                Ok(())
            } else {
                Err("refused by the test gate".to_owned())
            }
        }
    }

    /// A resolver that counts its lookups and answers `answer`. An empty
    /// answer is a lookup failure.
    fn counting_resolver(lookups: Arc<AtomicUsize>, answer: Vec<SocketAddr>) -> Resolve {
        Arc::new(move |_host: &str, _port: u16| {
            lookups.fetch_add(1, Ordering::SeqCst);
            if answer.is_empty() {
                Err(io::Error::new(io::ErrorKind::NotFound, "no such host"))
            } else {
                Ok(answer.clone())
            }
        })
    }

    /// Send a GET to `url` through the gated hooks with `resolve`, and return
    /// the outcome as a unit result.
    fn send(gate: &Arc<Gate>, resolve: Resolve, url: &str) -> Result<(), Error> {
        let net: Arc<dyn NetGate> = gate.clone();
        let hooks = GatedHooks {
            net,
            allow_loopback: false,
            tls: connect::tls_config(&TrustedCas::default()).expect("tls config"),
            clock: Arc::new(PauseClock::default()),
            abort: None,
        };
        let request = Request::builder()
            .uri(url)
            .body(Empty::new().map_err(Error::from).boxed_unsync())
            .expect("request");
        let fut = hooks.send_with(request, None, resolve);
        block_on_entry(Box::into_pin(fut)).map(|_| ())
    }

    fn gate(allow: bool) -> Arc<Gate> {
        Arc::new(Gate {
            allow,
            asked: AtomicUsize::new(0),
        })
    }

    fn public() -> Vec<SocketAddr> {
        vec!["93.184.216.34:443".parse().expect("addr")]
    }

    #[test]
    fn denied_request_to_an_unresolvable_name_never_resolves() {
        let gate = gate(false);
        let lookups = Arc::new(AtomicUsize::new(0));
        let resolve = counting_resolver(lookups.clone(), Vec::new());
        let err = send(&gate, resolve, "https://secret-42.attacker.invalid/").unwrap_err();
        assert!(matches!(err, Error::HttpRequestDenied), "{err:?}");
        assert_eq!(gate.asked.load(Ordering::SeqCst), 1);
        assert_eq!(lookups.load(Ordering::SeqCst), 0, "DNS ran before Allow");
    }

    #[test]
    fn unresolvable_name_after_allow_is_the_same_error_as_a_block() {
        let gate = gate(true);
        let lookups = Arc::new(AtomicUsize::new(0));
        let resolve = counting_resolver(lookups.clone(), Vec::new());
        let err = send(&gate, resolve, "https://nonexistent.invalid/").unwrap_err();
        assert!(matches!(err, Error::HttpRequestDenied), "{err:?}");
        assert_eq!(lookups.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn name_resolving_to_a_blocked_address_is_denied_after_allow() {
        let gate = gate(true);
        let lookups = Arc::new(AtomicUsize::new(0));
        let private = vec!["10.0.0.5:443".parse().expect("addr")];
        let resolve = counting_resolver(lookups.clone(), private);
        let err = send(&gate, resolve, "https://internal.example/").unwrap_err();
        assert!(matches!(err, Error::HttpRequestDenied), "{err:?}");
        assert_eq!(gate.asked.load(Ordering::SeqCst), 1);
        assert_eq!(lookups.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn literal_private_host_is_refused_before_the_gate_and_before_dns() {
        let gate = gate(true);
        let lookups = Arc::new(AtomicUsize::new(0));
        let resolve = counting_resolver(lookups.clone(), public());
        let err = send(&gate, resolve, "http://10.1.2.3/").unwrap_err();
        assert!(matches!(err, Error::HttpRequestDenied), "{err:?}");
        assert_eq!(gate.asked.load(Ordering::SeqCst), 0);
        assert_eq!(lookups.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn call_that_already_ended_is_not_asked() {
        let gate = gate(true);
        let liveness = Liveness::new();
        let alive = liveness.flag();
        // The send future is dropped: the call timed out, was aborted or ended.
        drop(liveness);
        let err = ask_gate(&alive, gate.as_ref(), "https://example.com/").unwrap_err();
        assert!(err.contains("ended"), "{err}");
        assert_eq!(gate.asked.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn live_call_is_asked() {
        let gate = gate(true);
        let liveness = Liveness::new();
        let alive = liveness.flag();
        assert!(ask_gate(&alive, gate.as_ref(), "https://example.com/").is_ok());
        assert_eq!(gate.asked.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn pause_is_capped_so_a_long_prompt_stops_extending_the_deadline() {
        let clock = PauseClock::with_cap(Duration::from_millis(200));
        let guard = clock.pause();
        std::thread::sleep(Duration::from_millis(400));
        assert_eq!(clock.paused(), Duration::from_millis(200));
        drop(guard);
        assert_eq!(clock.paused(), Duration::from_millis(200));
    }

    #[test]
    fn pause_below_the_cap_is_counted_in_full() {
        let clock = PauseClock::with_cap(Duration::from_secs(60));
        {
            let _guard = clock.pause();
            std::thread::sleep(Duration::from_millis(50));
        }
        let paused = clock.paused();
        assert!(paused >= Duration::from_millis(50), "{paused:?}");
        assert!(paused < Duration::from_secs(60), "{paused:?}");
    }
}

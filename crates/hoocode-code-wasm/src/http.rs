//! The gated `wasi:http` outgoing handler.
//!
//! Every request a plugin makes is checked in this order, and a refused request
//! fails with the WASI `http-request-denied` code:
//!
//! 1. The SSRF block ([`crate::ssrf`]): only http and https, and no address
//!    of the target host may be private, loopback, link-local and so on.
//!    This runs before the gate, so a request that would be refused anyway
//!    never prompts the user.
//! 2. The host's [`NetGate`](crate::NetGate), on a blocking thread because the
//!    gate may prompt the user.
//!
//! An allowed request goes through wasmtime-wasi-http's own send path.
//!
//! In wasmtime 48 `send_request` is a method of `WasiHttpHooks`, reached through
//! `WasiHttpView::http()`, not of `WasiHttpView` itself.

use std::future::Future;
use std::sync::Arc;

use http::{Request, Response};
use wasmtime_wasi_http::{Error, RequestOptions, WasiBody, WasiHttpHooks};

use crate::{ssrf, NetGate};

/// The per-call hooks: the SSRF block and the gate in front of the default send path.
pub(crate) struct GatedHooks {
    pub(crate) net: Arc<dyn NetGate>,
    /// Test-only: let loopback addresses through the SSRF block.
    pub(crate) allow_loopback: bool,
}

/// The stock send path, reached by calling the trait's default `send_request`.
struct DefaultHooks;

impl WasiHttpHooks for DefaultHooks {}

/// Log a refusal and fail the request with `http-request-denied`.
fn refuse(detail: &str) -> Result<(Response<WasiBody>, Outgoing), Error> {
    crate::write_host_log(&format!("wasm http request refused: {detail}\n"));
    Err(Error::HttpRequestDenied)
}

type Outgoing = Box<dyn Future<Output = Result<(), Error>> + Send>;

impl WasiHttpHooks for GatedHooks {
    fn send_request(
        &mut self,
        request: Request<WasiBody>,
        options: Option<RequestOptions>,
        fut: Box<dyn Future<Output = Result<(), Error>> + Send>,
    ) -> Box<dyn Future<Output = Result<(Response<WasiBody>, Outgoing), Error>> + Send> {
        let net = self.net.clone();
        let allow_loopback = self.allow_loopback;
        Box::new(async move {
            let url = request.uri().to_string();

            // PL-9b: SSRF block. Resolve the host and refuse any request whose
            // target maps to a private, loopback, link-local or other special
            // address. Runs on every hop (redirects included), before the gate.
            let target = match ssrf::parse_target(request.uri()) {
                Ok(target) => target,
                Err(reason) => return refuse(&format!("{url}: {reason}")),
            };
            let checked = hoocode_runtime::run_blocking(move || {
                ssrf::resolve_and_check(&target, allow_loopback)
            })
            .await;
            match checked {
                Ok(Ok(_addrs)) => {}
                Ok(Err(reason)) => return refuse(&format!("{url}: {reason}")),
                Err(e) => return Err(Error::InternalError(Some(format!("ssrf check: {e}")))),
            }

            let verdict = hoocode_runtime::run_blocking(move || net.check(&url)).await;
            match verdict {
                Ok(Ok(())) => {}
                Ok(Err(reason)) => {
                    crate::write_host_log(&format!("wasm http request refused: {reason}\n"));
                    return Err(Error::HttpRequestDenied);
                }
                Err(e) => return Err(Error::InternalError(Some(format!("net gate: {e}")))),
            }

            // PL-9b: rebinding window. wasmtime-wasi-http 48 exposes no way to send
            // over a given SocketAddr: `default_send_request` takes only the request
            // and connects to its authority itself. So the host is resolved again
            // here, and a name whose DNS answer changes between the check above and
            // this connect can reach an address the check refused. The residual
            // risk is accepted for now; closing it needs a send path that connects
            // to the checked address and keeps the host for TLS SNI and Host.
            Box::into_pin(DefaultHooks.send_request(request, options, fut)).await
        })
    }
}

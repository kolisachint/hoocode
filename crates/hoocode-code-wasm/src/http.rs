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
//! An allowed request is sent by [`crate::connect`], which connects to the
//! addresses the SSRF check returned and never resolves the host again.
//!
//! In wasmtime 48 `send_request` is a method of `WasiHttpHooks`, reached through
//! `WasiHttpView::http()`, not of `WasiHttpView` itself.

use std::future::Future;
use std::sync::Arc;

use http::{Request, Response};
use wasmtime_wasi_http::{Error, RequestOptions, WasiBody, WasiHttpHooks};

use crate::connect::{self, Outgoing};
use crate::{ssrf, NetGate};

/// The per-call hooks: the SSRF block and the gate in front of the default send path.
pub(crate) struct GatedHooks {
    pub(crate) net: Arc<dyn NetGate>,
    /// Test-only: let loopback addresses through the SSRF block.
    pub(crate) allow_loopback: bool,
}

/// Log a refusal and fail the request with `http-request-denied`.
fn refuse(detail: &str) -> Result<(Response<WasiBody>, Outgoing), Error> {
    crate::write_host_log(&format!("wasm http request refused: {detail}\n"));
    Err(Error::HttpRequestDenied)
}

impl WasiHttpHooks for GatedHooks {
    fn send_request(
        &mut self,
        request: Request<WasiBody>,
        options: Option<RequestOptions>,
        _fut: Box<dyn Future<Output = Result<(), Error>> + Send>,
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
            let addrs = match checked {
                Ok(Ok(addrs)) => addrs,
                Ok(Err(reason)) => return refuse(&format!("{url}: {reason}")),
                Err(e) => return Err(Error::InternalError(Some(format!("ssrf check: {e}")))),
            };

            let verdict = hoocode_runtime::run_blocking(move || net.check(&url)).await;
            match verdict {
                Ok(Ok(())) => {}
                Ok(Err(reason)) => {
                    crate::write_host_log(&format!("wasm http request refused: {reason}\n"));
                    return Err(Error::HttpRequestDenied);
                }
                Err(e) => return Err(Error::InternalError(Some(format!("net gate: {e}")))),
            }

            // PL-9b: the send connects to `addrs`, the addresses checked above,
            // and does not resolve the host again, so a changed DNS answer
            // cannot move the request to a refused address.
            connect::send_checked(request, &addrs, options).await
        })
    }
}

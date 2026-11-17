//! The gated `wasi:http` outgoing handler.
//!
//! Every request a plugin makes asks the host's [`NetGate`](crate::NetGate)
//! first, on a blocking thread because the gate may prompt the user. A refused
//! request fails with the WASI `http-request-denied` code. An allowed request
//! goes through wasmtime-wasi-http's own send path.
//!
//! In wasmtime 48 `send_request` is a method of `WasiHttpHooks`, reached through
//! `WasiHttpView::http()`, not of `WasiHttpView` itself.

use std::future::Future;
use std::sync::Arc;

use http::{Request, Response};
use wasmtime_wasi_http::{Error, RequestOptions, WasiBody, WasiHttpHooks};

use crate::NetGate;

/// The per-call hooks: the gate in front of the default send path.
pub(crate) struct GatedHooks {
    pub(crate) net: Arc<dyn NetGate>,
}

/// The stock send path, reached by calling the trait's default `send_request`.
struct DefaultHooks;

impl WasiHttpHooks for DefaultHooks {}

type Outgoing = Box<dyn Future<Output = Result<(), Error>> + Send>;

impl WasiHttpHooks for GatedHooks {
    fn send_request(
        &mut self,
        request: Request<WasiBody>,
        options: Option<RequestOptions>,
        fut: Box<dyn Future<Output = Result<(), Error>> + Send>,
    ) -> Box<dyn Future<Output = Result<(Response<WasiBody>, Outgoing), Error>> + Send> {
        let net = self.net.clone();
        Box::new(async move {
            let url = request.uri().to_string();
            // PL-9b: SSRF block. Resolve the host and refuse private, loopback and
            // link-local addresses on every hop, before the request is sent.
            let verdict = hoocode_runtime::run_blocking(move || net.check(&url)).await;
            match verdict {
                Ok(Ok(())) => {}
                Ok(Err(reason)) => {
                    crate::write_host_log(&format!("wasm http request refused: {reason}\n"));
                    return Err(Error::HttpRequestDenied);
                }
                Err(e) => return Err(Error::InternalError(Some(format!("net gate: {e}")))),
            }
            Box::into_pin(DefaultHooks.send_request(request, options, fut)).await
        })
    }
}

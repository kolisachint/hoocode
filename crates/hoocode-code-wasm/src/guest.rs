//! The guest side: bindings generated from `wit/` for the `extension` world,
//! the state of one call's store, and the host functions the plugin imports.

use std::path::Path;
use std::sync::atomic::AtomicU8;
use std::sync::Arc;
use std::time::Duration;

use wasmtime::component::{HasSelf, Linker, ResourceTable};
use wasmtime::{Engine, ResourceLimiter, Store, StoreLimits, StoreLimitsBuilder, UpdateDeadline};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpCtxView, WasiHttpView};

use crate::error::WasmError;
use crate::http::GatedHooks;
use crate::{AbortSignal, CallEnv};

wasmtime::component::bindgen!({
    path: "../../wit",
    world: "extension",
    // Tool calls are async (the store yields on epoch ticks). Host imports stay sync.
    exports: { default: async },
});

pub(crate) use hoocode::extension::types::{ToolError, ToolInfo as GuestToolInfo};

/// Memory a single call may grow to.
pub(crate) const MEMORY_LIMIT: usize = 64 << 20;

/// What ended a call early. Written by the epoch callback, read after a trap.
pub(crate) const REASON_NONE: u8 = 0;
pub(crate) const REASON_TIMEOUT: u8 = 1;
pub(crate) const REASON_CANCELLED: u8 = 2;

/// Store data for one call. A fresh store is built for every call.
pub(crate) struct State {
    pub(crate) plugin: String,
    pub(crate) env: CallEnv,
    pub(crate) wasi: WasiCtx,
    pub(crate) table: ResourceTable,
    pub(crate) http: WasiHttpCtx,
    pub(crate) hooks: GatedHooks,
    pub(crate) limits: StoreLimits,
}

impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl WasiHttpView for State {
    fn http(&mut self) -> WasiHttpCtxView<'_> {
        WasiHttpCtxView {
            hooks: &mut self.hooks,
            table: &mut self.table,
            ctx: &mut self.http,
        }
    }
}

impl hoocode::extension::host::Host for State {
    fn config_get(&mut self, key: String) -> Option<String> {
        (self.env.config)(&key)
    }

    fn log(&mut self, level: hoocode::extension::host::LogLevel, message: String) {
        use hoocode::extension::host::LogLevel;
        let level = match level {
            LogLevel::Trace => "trace",
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        };
        crate::write_host_log(&format!("[wasm {}] {level}: {message}\n", self.plugin));
    }
}

/// Link the plugin-facing `hoocode:extension/host` functions.
pub(crate) fn link_host(linker: &mut Linker<State>) -> wasmtime::Result<()> {
    hoocode::extension::host::add_to_linker::<State, HasSelf<State>>(linker, |s| s)
}

/// A fresh store for one call, with the memory limit and the call's env.
pub(crate) fn new_store(
    engine: &Engine,
    plugin: &str,
    data_dir: &Path,
    env: CallEnv,
) -> Result<Store<State>, WasmError> {
    let state = State {
        plugin: plugin.to_owned(),
        wasi: crate::wasi::build_wasi(plugin, data_dir)?,
        table: ResourceTable::new(),
        http: WasiHttpCtx::new(),
        hooks: GatedHooks {
            net: env.net.clone(),
        },
        limits: StoreLimitsBuilder::new().memory_size(MEMORY_LIMIT).build(),
        env,
    };
    let mut store = Store::new(engine, state);
    store.limiter(|s: &mut State| -> &mut dyn ResourceLimiter { &mut s.limits });
    Ok(store)
}

/// Trap at the first epoch deadline after `timeout` or once `abort` is set.
/// The epoch thread ticks every 100 ms, so a check runs about that often.
pub(crate) fn set_deadline(
    store: &mut Store<State>,
    timeout: Duration,
    abort: Option<AbortSignal>,
    reason: Arc<AtomicU8>,
) {
    let started = std::time::Instant::now();
    store.set_epoch_deadline(1);
    store.epoch_deadline_callback(move |_| {
        if abort.as_ref().is_some_and(AbortSignal::aborted) {
            reason.store(REASON_CANCELLED, std::sync::atomic::Ordering::Relaxed);
            return Ok(UpdateDeadline::Interrupt);
        }
        if started.elapsed() > timeout {
            reason.store(REASON_TIMEOUT, std::sync::atomic::Ordering::Relaxed);
            return Ok(UpdateDeadline::Interrupt);
        }
        Ok(UpdateDeadline::Continue(1))
    });
}

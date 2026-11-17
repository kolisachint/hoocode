//! WASM extension host for the hoocode coding agent.
//!
//! Runs internal-tier plugins: WASI 0.2 components that implement the
//! `hoocode:extension@0.1.0` world (`wit/`). See `docs/design/plugins.md`
//! ("Contract" and "Host runtime").
//!
//! Each call gets a fresh store and instance, a 64 MiB memory limit, a 30 s
//! timeout (`CALL_TIMEOUT`) and the abort signal of the caller. The WASI context
//! has no environment or arguments, stdout and stderr go to the host log, and
//! the plugin's data folder is the only preopen (at `/data`). Sockets are denied.
//! `wasi:http` requests to private or special addresses are refused (the SSRF
//! block, `ssrf.rs`), and the rest go through the caller's [`NetGate`].

mod cache;
mod connect;
mod error;
mod extension;
mod guest;
mod http;
mod ssrf;
mod wasi;

use std::future::Future;
use std::path::PathBuf;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::time::Duration;

use hoocode_ai_types::AbortSignal;
use wasmtime::component::Linker;
use wasmtime::{Config, Engine};

pub use error::WasmError;
pub use extension::{Extension, ToolInfo};

/// The default timeout of one tool call.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// How often the epoch counter advances. Calls are checked at this rate.
const EPOCH_TICK: Duration = Duration::from_millis(100);

/// Decides whether a plugin may reach a URL. It may block (for example to
/// prompt the user), so the host calls it on a blocking thread.
pub trait NetGate: Send + Sync {
    /// `Ok` to allow the request, or `Err` with the reason it was refused.
    fn check(&self, url: &str) -> Result<(), String>;
}

/// Reads one of a plugin's own settings. `None` when the key is not set.
pub type ConfigLookup = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// What the host provides to one call: the config lookup, the network gate and
/// the caller's abort signal.
#[derive(Clone)]
pub struct CallEnv {
    /// Reads one of this plugin's own settings. `None` when unset.
    pub config: ConfigLookup,
    /// Gate for every outgoing `wasi:http` request.
    pub net: Arc<dyn NetGate>,
    /// Ends the call early when aborted.
    pub abort: Option<AbortSignal>,
}

impl CallEnv {
    /// The env used while listing tools at load: no config, no network.
    fn for_discovery() -> Self {
        struct DenyAll;
        impl NetGate for DenyAll {
            fn check(&self, _url: &str) -> Result<(), String> {
                Err("no network while the extension loads".to_owned())
            }
        }
        Self {
            config: Arc::new(|_| None),
            net: Arc::new(DenyAll),
            abort: None,
        }
    }
}

/// Where one extension's files are.
#[derive(Debug, Clone)]
pub struct ExtensionSpec {
    /// Plugin name, used in the host log.
    pub plugin: String,
    /// The component file (`.wasm`).
    pub wasm_path: PathBuf,
    /// The plugin's data folder, mounted at `/data`. Created if missing.
    pub data_dir: PathBuf,
}

/// The engine and cache shared by every extension of one host.
pub(crate) struct Inner {
    pub(crate) engine: Engine,
    pub(crate) linker: Linker<guest::State>,
    pub(crate) cache_dir: PathBuf,
}

/// The WASM host. Cheap to clone; clones share one engine.
#[derive(Clone)]
pub struct WasmHost {
    inner: Arc<Inner>,
    call_timeout: Duration,
    allow_loopback: bool,
}

impl WasmHost {
    /// Create a host whose precompiled components live in `cache_dir`.
    pub fn new(cache_dir: PathBuf) -> Result<Self, WasmError> {
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.epoch_interruption(true);
        let engine = Engine::new(&config).map_err(|e| WasmError::Engine(e.to_string()))?;

        let mut linker = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)
            .map_err(|e| WasmError::Engine(format!("wasi: {e}")))?;
        wasmtime_wasi_http::p2::add_only_http_to_linker_async(&mut linker)
            .map_err(|e| WasmError::Engine(format!("wasi:http: {e}")))?;
        guest::link_host(&mut linker).map_err(|e| WasmError::Engine(format!("host: {e}")))?;

        cache::create_cache_dir(&cache_dir)?;
        let inner = Arc::new(Inner {
            engine,
            linker,
            cache_dir,
        });
        spawn_epoch_ticker(&inner)?;
        Ok(Self {
            inner,
            call_timeout: CALL_TIMEOUT,
            allow_loopback: false,
        })
    }

    /// Use `timeout` instead of [`CALL_TIMEOUT`] for calls made from this host.
    pub fn with_call_timeout(mut self, timeout: Duration) -> Self {
        self.call_timeout = timeout;
        self
    }

    /// test-only: lets loopback addresses through the SSRF block; never set from plugin config.
    pub fn with_loopback_allowed(mut self) -> Self {
        self.allow_loopback = true;
        self
    }

    /// Load one extension: compile (or reuse the cached compile), link, and
    /// list its tools. Blocks for the tool listing, which takes milliseconds.
    pub fn load(&self, spec: ExtensionSpec) -> Result<Extension, WasmError> {
        std::fs::create_dir_all(&spec.data_dir).map_err(|source| WasmError::Io {
            path: spec.data_dir.clone(),
            source,
        })?;
        let component =
            cache::load_component(&self.inner.engine, &self.inner.cache_dir, &spec.wasm_path)?;
        let instance_pre = self
            .inner
            .linker
            .instantiate_pre(&component)
            .map_err(|e| WasmError::Compile(e.to_string()))?;
        let pre = guest::ExtensionPre::new(instance_pre)
            .map_err(|e| WasmError::Compile(e.to_string()))?;
        let mut extension = Extension {
            host: self.inner.clone(),
            pre,
            plugin: spec.plugin,
            data_dir: spec.data_dir,
            call_timeout: self.call_timeout,
            allow_loopback: self.allow_loopback,
            tools: Vec::new(),
        };
        extension.tools = block_on(extension.discover_tools())?;
        Ok(extension)
    }
}

/// Advance the engine epoch every `EPOCH_TICK` until the last host is dropped.
/// The thread holds only a weak reference, so it does not keep the engine alive.
fn spawn_epoch_ticker(inner: &Arc<Inner>) -> Result<(), WasmError> {
    let weak = Arc::downgrade(inner);
    hoocode_runtime::spawn_named_thread("hoocode-wasm-epoch", move || loop {
        std::thread::sleep(EPOCH_TICK);
        match weak.upgrade() {
            Some(inner) => inner.engine.increment_epoch(),
            None => break,
        }
    })
    .map(|_| ())
    .map_err(|e| WasmError::Engine(format!("epoch thread: {e}")))
}

/// Run a future to completion on this thread, parking while it waits.
fn block_on<F: Future>(future: F) -> F::Output {
    struct Unpark(std::thread::Thread);
    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut future = pin!(future);
    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::thread::park(),
        }
    }
}

/// Host diagnostics (plugin logs, cache notes). Goes to the debug log in the
/// TUI and to stderr otherwise.
pub(crate) fn write_host_log(text: &str) {
    hoocode_runtime::diagnostics::write_stderr(text);
}

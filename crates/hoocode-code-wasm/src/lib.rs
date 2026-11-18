//! WASM extension host for the hoocode coding agent.
//!
//! Runs internal-tier plugins: WASI 0.2 components that implement the
//! `hoocode:extension@0.1.0` world (`wit/`). See `docs/design/plugins.md`
//! ("Contract" and "Host runtime").
//!
//! Each call gets a fresh store and instance, 64 MiB of memory in total, a 30 s
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

use std::path::PathBuf;
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use hoocode_ai_types::AbortSignal;
use hoocode_ai_util::tls::TrustedCas;
use hoocode_runtime::block_on_current_thread;
use rustls::ClientConfig;
use wasmtime::component::Linker;
use wasmtime::{Config, Engine};

pub use error::WasmError;
pub use extension::{Extension, ToolInfo};

/// The default timeout of one tool call.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// The wall-clock bound on loading one extension (compile or cache read, link,
/// list-tools). Independent of the per-host call timeout: a cold compile is not
/// a call, and a short call timeout must not make loads fail.
const LOAD_TIMEOUT: Duration = Duration::from_secs(60);

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

/// The engine and cache shared by every extension of one host. `cache` is
/// `None` when the cache folder cannot be used safely (see `cache.rs`).
pub(crate) struct Inner {
    pub(crate) engine: Engine,
    pub(crate) linker: Linker<guest::State>,
    pub(crate) cache: Option<cache::Cache>,
}

/// The WASM host. Cheap to clone; clones share one engine.
#[derive(Clone)]
pub struct WasmHost {
    inner: Arc<Inner>,
    call_timeout: Duration,
    allow_loopback: bool,
    /// The client TLS config for plugin https requests (bundled roots plus
    /// `trust`). Built once, on first use, so a host never builds a config for a
    /// trust set it does not use.
    tls: OnceLock<Arc<ClientConfig>>,
    trust: TrustedCas,
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

        let cache = match cache::Cache::open(&cache_dir) {
            Ok(cache) => Some(cache),
            Err(reason) => {
                write_host_log(&format!(
                    "wasm cache disabled, components compile in memory: {reason}\n"
                ));
                None
            }
        };
        let inner = Arc::new(Inner {
            engine,
            linker,
            cache,
        });
        spawn_epoch_ticker(&inner)?;
        Ok(Self {
            inner,
            call_timeout: CALL_TIMEOUT,
            allow_loopback: false,
            tls: OnceLock::new(),
            trust: TrustedCas::default(),
        })
    }

    /// Trust `trust` (hoocode's CA set: `--ca-cert`, `HOOCODE_CA_CERT`,
    /// `NODE_EXTRA_CA_CERTS`, `--use-system-ca`) for https plugin requests, on top
    /// of the bundled roots. Verification stays on.
    pub fn with_tls_trust(mut self, trust: &TrustedCas) -> Result<Self, WasmError> {
        let tls = connect::tls_config(trust)
            .map_err(|e| WasmError::Engine(format!("plugin TLS config: {e}")))?;
        self.tls = OnceLock::from(tls);
        self.trust = trust.clone();
        Ok(self)
    }

    /// The client TLS config for this host's trust set, built on first use.
    fn tls(&self) -> Result<Arc<ClientConfig>, WasmError> {
        if let Some(tls) = self.tls.get() {
            return Ok(tls.clone());
        }
        let built = connect::tls_config(&self.trust)
            .map_err(|e| WasmError::Engine(format!("plugin TLS config: {e}")))?;
        Ok(self.tls.get_or_init(|| built).clone())
    }

    /// The CA set beyond the bundled roots that plugin https requests trust.
    pub fn tls_trust(&self) -> &TrustedCas {
        &self.trust
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
    /// list its tools. The work runs on a dedicated thread and the call waits at
    /// most `LOAD_TIMEOUT` for it; a load that overruns is an error.
    pub fn load(&self, spec: ExtensionSpec) -> Result<Extension, WasmError> {
        std::fs::create_dir_all(&spec.data_dir).map_err(|source| WasmError::Io {
            path: spec.data_dir.clone(),
            source,
        })?;
        let inner = self.inner.clone();
        let call_timeout = self.call_timeout;
        let allow_loopback = self.allow_loopback;
        let tls = self.tls()?;
        let label = format!("plugin {}", spec.plugin);
        bounded(LOAD_TIMEOUT, label, move || {
            let component =
                cache::load_component(&inner.engine, inner.cache.as_ref(), &spec.wasm_path)?;
            let instance_pre = inner
                .linker
                .instantiate_pre(&component)
                .map_err(|e| WasmError::Compile(e.to_string()))?;
            let pre = guest::ExtensionPre::new(instance_pre)
                .map_err(|e| WasmError::Compile(e.to_string()))?;
            let mut extension = Extension {
                host: inner,
                pre,
                plugin: spec.plugin,
                data_dir: spec.data_dir,
                call_timeout,
                allow_loopback,
                tls,
                tools: Vec::new(),
            };
            extension.tools = block_on_current_thread(extension.discover_tools())?;
            Ok(extension)
        })
    }
}

/// Runs `work` on a dedicated thread and waits at most `bound` for its result.
/// Compiling and listing a component are not covered by the epoch deadline, so
/// this wall-clock bound is what ends a hung load. A thread that overruns keeps
/// running until its own epoch deadline stops the guest; its result is dropped.
fn bounded<T, F>(bound: Duration, label: String, work: F) -> Result<T, WasmError>
where
    F: FnOnce() -> Result<T, WasmError> + Send + 'static,
    T: Send + 'static,
{
    let (tx, rx) = hoocode_runtime::sync_bounded_channel(1);
    hoocode_runtime::spawn_named_thread("hoocode-wasm-load", move || {
        let _ = tx.send(work());
    })
    .map_err(|e| WasmError::Engine(format!("{label}: cannot start the loader: {e}")))?;
    match rx.recv_timeout(bound) {
        Ok(result) => result,
        Err(RecvTimeoutError::Timeout) => Err(WasmError::Compile(format!(
            "{label}: did not finish loading within {} s",
            bound.as_secs()
        ))),
        Err(RecvTimeoutError::Disconnected) => Err(WasmError::Engine(format!(
            "{label}: the loader thread stopped"
        ))),
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

/// Host diagnostics (plugin logs, cache notes). Goes to the debug log in the
/// TUI and to stderr otherwise.
pub(crate) fn write_host_log(text: &str) {
    hoocode_runtime::diagnostics::write_stderr(text);
}

/// Whether debug output is on: `DEBUG_AGENT_LOG=1`, the switch that also
/// enables the agent log's debug log.
pub(crate) fn debug_enabled() -> bool {
    hoocode_code_paths::env_override("DEBUG_AGENT_LOG").as_deref() == Some("1")
}

/// Debug output (plugin trace and debug logs, guest stdout and stderr). It
/// never reaches stderr: it is appended to the debug log. Callers check
/// `debug_enabled` first.
pub(crate) fn write_debug_log(text: &str) {
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(hoocode_code_paths::debug_log_path())
    {
        let _ = file.write_all(text.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_result_within_the_bound_is_returned() {
        let value = bounded(Duration::from_secs(5), "fast".to_owned(), || Ok(7)).unwrap();
        assert_eq!(value, 7);
    }

    #[test]
    fn an_error_within_the_bound_is_returned() {
        let err = bounded::<(), _>(Duration::from_secs(5), "bad".to_owned(), || {
            Err(WasmError::Compile("broken".to_owned()))
        })
        .unwrap_err();
        assert!(
            matches!(err, WasmError::Compile(ref m) if m == "broken"),
            "{err:?}"
        );
    }

    #[test]
    fn a_slow_load_is_a_load_error_at_the_bound() {
        let started = std::time::Instant::now();
        let err = bounded(Duration::from_millis(50), "slow".to_owned(), || {
            std::thread::sleep(Duration::from_millis(1500));
            Ok(())
        })
        .unwrap_err();
        assert!(
            matches!(err, WasmError::Compile(ref m) if m.contains("did not finish loading")),
            "{err:?}"
        );
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}

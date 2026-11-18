//! The guest side: bindings generated from `wit/` for the `extension` world,
//! the state of one call's store, and the host functions the plugin imports.

use std::path::Path;
use std::sync::atomic::AtomicU8;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use wasmtime::component::{HasSelf, Linker, ResourceTable};
use wasmtime::{Engine, ResourceLimiter, Store, UpdateDeadline};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpCtxView, WasiHttpView};

use crate::error::WasmError;
use crate::http::{GatedHooks, PauseClock};
use crate::{AbortSignal, CallEnv};

wasmtime::component::bindgen!({
    path: "../../wit",
    world: "extension",
    // Tool calls are async (the store yields on epoch ticks). Host imports stay sync.
    exports: { default: async },
});

pub(crate) use hoocode::extension::types::{ToolError, ToolInfo as GuestToolInfo};

/// Memory all of one call's linear memories may reach together.
pub(crate) const MEMORY_LIMIT: usize = 64 << 20;

/// Table elements all of one call's tables may hold together (8 bytes each on
/// 64-bit hosts, so about 8 MB at most).
pub(crate) const TABLE_ELEMENT_LIMIT: usize = 1_000_000;

/// Most linear memories, tables and instances one call may create. A Rust
/// component needs one memory and a handful of instances (the WASI adapter).
const MAX_MEMORIES: usize = 16;
const MAX_TABLES: usize = 32;
const MAX_INSTANCES: usize = 64;

/// Most log lines one call may write (`log` and the WASI stdout/stderr together).
pub(crate) const MAX_LOG_MESSAGES: u32 = 200;

/// Longest log message kept, in bytes, the suffix included.
const LOG_MESSAGE_CAP: usize = 4096;
const LOG_TRUNCATED_SUFFIX: &str = " ...[truncated]";

/// What ended a call early. Written by the epoch callback, read after a trap.
pub(crate) const REASON_NONE: u8 = 0;
pub(crate) const REASON_TIMEOUT: u8 = 1;
pub(crate) const REASON_CANCELLED: u8 = 2;

/// The resource limits of one call, summed over all its memories and tables.
#[derive(Debug, Default)]
pub(crate) struct CallLimits {
    memory_bytes: usize,
    table_elements: usize,
}

impl ResourceLimiter for CallLimits {
    // `current` is the size of the one memory growing. The memories of the call
    // add up to `memory_bytes`, so the new total replaces that memory's share.
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        let total = self
            .memory_bytes
            .saturating_sub(current)
            .saturating_add(desired);
        if total > MEMORY_LIMIT {
            return Ok(false);
        }
        self.memory_bytes = total;
        Ok(true)
    }

    fn table_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        let total = self
            .table_elements
            .saturating_sub(current)
            .saturating_add(desired);
        if total > TABLE_ELEMENT_LIMIT {
            return Ok(false);
        }
        self.table_elements = total;
        Ok(true)
    }

    fn instances(&self) -> usize {
        MAX_INSTANCES
    }

    fn tables(&self) -> usize {
        MAX_TABLES
    }

    fn memories(&self) -> usize {
        MAX_MEMORIES
    }
}

/// The log lines one call has written so far. Shared by the `log` import and
/// the WASI stdout and stderr streams of that call, so the cap is per call.
#[derive(Default)]
pub(crate) struct LogBudget {
    written: Mutex<u32>,
}

impl LogBudget {
    /// The host log line for one message, or `None` once the call is over its
    /// budget. The first message past the budget gets a "log truncated" line.
    pub(crate) fn line(&self, prefix: &str, message: &str) -> Option<String> {
        let n = {
            let mut written = self.written.lock().unwrap_or_else(|e| e.into_inner());
            let n = *written;
            *written = n.saturating_add(1);
            n
        };
        if n < MAX_LOG_MESSAGES {
            Some(format!("{prefix}{}\n", clean_message(message)))
        } else if n == MAX_LOG_MESSAGES {
            Some(format!(
                "{prefix}log truncated: further messages from this call are dropped\n"
            ))
        } else {
            None
        }
    }

    /// Write one message to the host log, within the budget.
    pub(crate) fn write(&self, prefix: &str, message: &str) {
        if let Some(line) = self.line(prefix, message) {
            crate::write_host_log(&line);
        }
    }
}

/// Drops control characters except `\n` and `\t` (so no ESC or OSC sequences
/// reach the terminal), then cuts the text to `LOG_MESSAGE_CAP` bytes.
pub(crate) fn clean_message(message: &str) -> String {
    let budget = LOG_MESSAGE_CAP - LOG_TRUNCATED_SUFFIX.len();
    let mut out = String::new();
    let mut cut = false;
    for ch in message.chars() {
        if ch.is_control() && ch != '\n' && ch != '\t' {
            continue;
        }
        if out.len() + ch.len_utf8() > budget {
            cut = true;
            break;
        }
        out.push(ch);
    }
    if cut {
        out.push_str(LOG_TRUNCATED_SUFFIX);
    }
    out
}

/// Store data for one call. A fresh store is built for every call.
pub(crate) struct State {
    pub(crate) plugin: String,
    pub(crate) env: CallEnv,
    pub(crate) wasi: WasiCtx,
    pub(crate) table: ResourceTable,
    pub(crate) http: WasiHttpCtx,
    pub(crate) hooks: GatedHooks,
    pub(crate) limits: CallLimits,
    pub(crate) logs: Arc<LogBudget>,
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
        self.logs
            .write(&format!("[wasm {}] {level}: ", self.plugin), &message);
    }
}

/// Link the plugin-facing `hoocode:extension/host` functions.
pub(crate) fn link_host(linker: &mut Linker<State>) -> wasmtime::Result<()> {
    hoocode::extension::host::add_to_linker::<State, HasSelf<State>>(linker, |s| s)
}

/// A fresh store for one call, with the memory limit and the call's env.
/// `clock` counts the time the call spends waiting on the network gate.
pub(crate) fn new_store(
    engine: &Engine,
    plugin: &str,
    data_dir: &Path,
    env: CallEnv,
    allow_loopback: bool,
    clock: Arc<PauseClock>,
) -> Result<Store<State>, WasmError> {
    let logs = Arc::new(LogBudget::default());
    let state = State {
        plugin: plugin.to_owned(),
        wasi: crate::wasi::build_wasi(plugin, data_dir, logs.clone())?,
        table: ResourceTable::new(),
        http: WasiHttpCtx::new(),
        hooks: GatedHooks {
            net: env.net.clone(),
            allow_loopback,
            clock,
            abort: env.abort.clone(),
        },
        limits: CallLimits::default(),
        logs,
        env,
    };
    let mut store = Store::new(engine, state);
    store.limiter(|s: &mut State| -> &mut dyn ResourceLimiter { &mut s.limits });
    Ok(store)
}

/// Trap at the first epoch deadline after `timeout` of call time, or once
/// `abort` is set. Time spent waiting on the network gate is not call time.
/// The epoch thread ticks every 100 ms, so a check runs about that often.
/// This only fires while guest code runs; a host await is bounded by the
/// wall-clock timer in `Extension::call`.
pub(crate) fn set_deadline(
    store: &mut Store<State>,
    timeout: Duration,
    abort: Option<AbortSignal>,
    reason: Arc<AtomicU8>,
    clock: Arc<PauseClock>,
) {
    let started = std::time::Instant::now();
    store.set_epoch_deadline(1);
    store.epoch_deadline_callback(move |_| {
        if abort.as_ref().is_some_and(AbortSignal::aborted) {
            reason.store(REASON_CANCELLED, std::sync::atomic::Ordering::Relaxed);
            return Ok(UpdateDeadline::Interrupt);
        }
        if started.elapsed().saturating_sub(clock.paused()) > timeout {
            reason.store(REASON_TIMEOUT, std::sync::atomic::Ordering::Relaxed);
            return Ok(UpdateDeadline::Interrupt);
        }
        Ok(UpdateDeadline::Continue(1))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: usize = 1 << 20;

    #[test]
    fn memories_share_one_total() {
        let mut limits = CallLimits::default();
        assert!(limits.memory_growing(0, 40 * MIB, None).unwrap());
        // A second memory of 40 MiB would make 80 MiB in total.
        assert!(!limits.memory_growing(0, 40 * MIB, None).unwrap());
        // The first memory grows to 64 MiB, which is the whole budget.
        assert!(!limits.memory_growing(40 * MIB, 65 * MIB, None).unwrap());
        assert!(limits.memory_growing(40 * MIB, 64 * MIB, None).unwrap());
        // Nothing is left for another memory.
        assert!(!limits.memory_growing(0, 64 * 1024, None).unwrap());
    }

    #[test]
    fn a_refused_growth_is_not_counted() {
        let mut limits = CallLimits::default();
        assert!(!limits.memory_growing(0, MEMORY_LIMIT + 1, None).unwrap());
        assert!(limits.memory_growing(0, MEMORY_LIMIT, None).unwrap());
    }

    #[test]
    fn tables_share_one_element_total() {
        let mut limits = CallLimits::default();
        assert!(limits.table_growing(0, TABLE_ELEMENT_LIMIT, None).unwrap());
        assert!(!limits.table_growing(0, 1, None).unwrap());
        assert!(!limits
            .table_growing(TABLE_ELEMENT_LIMIT, TABLE_ELEMENT_LIMIT + 10, None)
            .unwrap());
    }

    #[test]
    fn counts_are_capped() {
        let limits = CallLimits::default();
        assert_eq!(limits.memories(), MAX_MEMORIES);
        assert_eq!(limits.tables(), MAX_TABLES);
        assert_eq!(limits.instances(), MAX_INSTANCES);
    }

    #[test]
    fn control_characters_are_stripped_but_newlines_and_tabs_stay() {
        let cleaned = clean_message("a\x1b]0;title\x07b\tc\nd\u{9b}e\r");
        assert_eq!(cleaned, "a]0;titleb\tc\nde");
        assert!(!cleaned.contains('\x1b'));
    }

    #[test]
    fn long_messages_are_cut_to_the_cap() {
        let long = "x".repeat(10_000);
        let cleaned = clean_message(&long);
        assert!(cleaned.len() <= LOG_MESSAGE_CAP, "{}", cleaned.len());
        assert!(cleaned.ends_with(LOG_TRUNCATED_SUFFIX));
        // Cut on a character boundary.
        let wide = "é".repeat(5_000);
        let cleaned = clean_message(&wide);
        assert!(cleaned.len() <= LOG_MESSAGE_CAP);
    }

    #[test]
    fn a_call_writes_at_most_the_budget_of_lines() {
        let budget = LogBudget::default();
        let mut lines = Vec::new();
        for i in 0..(MAX_LOG_MESSAGES + 5) {
            if let Some(line) = budget.line("[wasm p] info: ", &format!("m{i}")) {
                lines.push(line);
            }
        }
        assert_eq!(lines.len(), MAX_LOG_MESSAGES as usize + 1);
        assert!(lines[0].ends_with("m0\n"));
        assert!(lines.last().unwrap().contains("log truncated"));
    }
}

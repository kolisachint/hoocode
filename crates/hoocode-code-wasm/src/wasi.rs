//! The WASI 0.2 context of one call: no environment, no arguments, no stdin,
//! stdout and stderr to the host log, and one preopen (the plugin's data folder).
//! Sockets stay denied: the context never enables TCP, UDP or name lookup.

use std::io;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use tokio::io::AsyncWrite;
use wasmtime_wasi::cli::{IsTerminal, StdoutStream};
use wasmtime_wasi::{FsPerms, WasiCtx, WasiCtxBuilder};

use crate::error::WasmError;
use crate::guest::{LogBudget, LogRoute};

/// Build the WASI context for one call, with `data_dir` mounted at `/data`.
/// Stdout and stderr write into `logs`, the call's shared log budget.
pub(crate) fn build_wasi(
    plugin: &str,
    data_dir: &Path,
    logs: Arc<LogBudget>,
) -> Result<WasiCtx, WasmError> {
    let plugin: Arc<str> = Arc::from(plugin);
    let mut builder = WasiCtxBuilder::new();
    // Guest stdout is debug output. Guest stderr goes to the host log: a Rust
    // guest writes its panic message there, and nothing else would show it.
    builder
        .stdout(LogSink {
            plugin: plugin.clone(),
            stream: "stdout",
            route: LogRoute::Debug,
            logs: logs.clone(),
        })
        .stderr(LogSink {
            plugin,
            stream: "stderr",
            route: LogRoute::Host,
            logs,
        });
    builder
        .preopened_dir(data_dir, "/data", FsPerms::ReadWrite)
        .map_err(|e| WasmError::Engine(format!("preopen {}: {e}", data_dir.display())))?;
    Ok(builder.build())
}

/// Sends guest stdout or stderr to its route, one write at a time: stdout to
/// the debug output (shown only while debugging is on), stderr to the host log.
#[derive(Clone)]
struct LogSink {
    plugin: Arc<str>,
    stream: &'static str,
    route: LogRoute,
    logs: Arc<LogBudget>,
}

impl IsTerminal for LogSink {
    fn is_terminal(&self) -> bool {
        false
    }
}

impl StdoutStream for LogSink {
    fn async_stream(&self) -> Box<dyn AsyncWrite + Send + Sync> {
        Box::new(self.clone())
    }
}

impl AsyncWrite for LogSink {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let text = String::from_utf8_lossy(buf);
        let text = text.trim_end_matches('\n');
        if !text.is_empty() {
            self.logs.write(
                self.route,
                &format!("[wasm {} {}] ", self.plugin, self.stream),
                text,
            );
        }
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

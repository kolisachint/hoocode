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

/// Build the WASI context for one call, with `data_dir` mounted at `/data`.
pub(crate) fn build_wasi(plugin: &str, data_dir: &Path) -> Result<WasiCtx, WasmError> {
    let plugin: Arc<str> = Arc::from(plugin);
    let mut builder = WasiCtxBuilder::new();
    builder
        .stdout(LogSink {
            plugin: plugin.clone(),
            stream: "stdout",
        })
        .stderr(LogSink {
            plugin,
            stream: "stderr",
        });
    builder
        .preopened_dir(data_dir, "/data", FsPerms::ReadWrite)
        .map_err(|e| WasmError::Engine(format!("preopen {}: {e}", data_dir.display())))?;
    Ok(builder.build())
}

/// Sends guest stdout or stderr to the host log, one write at a time.
#[derive(Clone)]
struct LogSink {
    plugin: Arc<str>,
    stream: &'static str,
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
            crate::write_host_log(&format!("[wasm {} {}] {text}\n", self.plugin, self.stream));
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

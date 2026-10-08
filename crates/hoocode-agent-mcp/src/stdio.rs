//! The stdio transport: a child process that speaks MCP on its stdin and stdout.
//!
//! rmcp's own child-process transport reads lines with no length limit, so one
//! line from a broken or hostile server could be buffered whole. This module
//! builds the transport from rmcp's async read/write transport and puts a
//! [`LineLimit`] reader under it. The reader fails as soon as a line passes
//! [`MAX_STDIO_LINE_BYTES`], before that line is held in memory. A failed read
//! ends the connection; the client then starts the server again on the next
//! request.

use std::collections::BTreeMap;
use std::future::Future;
use std::io;
use std::path::Path;
use std::pin::Pin;
use std::process::Stdio;
use std::task::{Context, Poll};
use std::time::Duration;

use rmcp::service::{RxJsonRpcMessage, TxJsonRpcMessage};
use rmcp::transport::async_rw::AsyncRwTransport;
use rmcp::transport::Transport;
use rmcp::RoleClient;
use tokio::io::{AsyncRead, ReadBuf};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

/// Largest stdio message (one line of JSON), in bytes, without its newline.
pub const MAX_STDIO_LINE_BYTES: usize = 32 * 1024 * 1024;
/// How long a server may take to exit after its stdin closes, before it is killed.
const EXIT_GRACE: Duration = Duration::from_secs(3);

/// An `AsyncRead` that fails once one line (the bytes since the last `\n`)
/// is longer than `limit`. The check runs on each chunk as it is read, so the
/// reader never hands on more than one chunk past the limit.
pub struct LineLimit<R> {
    inner: R,
    limit: usize,
    /// Bytes read since the last newline.
    run: usize,
}

impl<R> LineLimit<R> {
    pub fn new(inner: R, limit: usize) -> Self {
        Self {
            inner,
            limit,
            run: 0,
        }
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for LineLimit<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        match Pin::new(&mut self.inner).poll_read(cx, buf) {
            Poll::Ready(Ok(())) => {
                let fresh = &buf.filled()[before..];
                match fresh.iter().rposition(|&byte| byte == b'\n') {
                    Some(at) => self.run = fresh.len() - at - 1,
                    None => self.run += fresh.len(),
                }
                if self.run > self.limit {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("MCP stdio message exceeds {} bytes", self.limit),
                    )));
                }
                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}

/// The client side of a child process's stdio. Dropping it kills the child
/// (`kill_on_drop`), so a client that goes away never leaves a server behind.
pub struct StdioTransport {
    inner: AsyncRwTransport<RoleClient, LineLimit<ChildStdout>, ChildStdin>,
    child: Option<Child>,
}

/// Starts `command` with `args`, `env` and `cwd`. Its stdin and stdout carry
/// MCP; its stderr is discarded so it cannot write into the terminal.
pub fn spawn(
    command: &str,
    args: &[String],
    env: &BTreeMap<String, String>,
    cwd: Option<&Path>,
) -> io::Result<StdioTransport> {
    let mut process = Command::new(command);
    process
        .args(args)
        .envs(env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    if let Some(dir) = cwd {
        process.current_dir(dir);
    }
    let mut child = process.spawn()?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("the child's stdin was not piped"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("the child's stdout was not piped"))?;
    Ok(StdioTransport {
        inner: AsyncRwTransport::new_client(LineLimit::new(stdout, MAX_STDIO_LINE_BYTES), stdin),
        child: Some(child),
    })
}

impl Transport<RoleClient> for StdioTransport {
    type Error = io::Error;

    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleClient>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        self.inner.send(item)
    }

    fn receive(&mut self) -> impl Future<Output = Option<RxJsonRpcMessage<RoleClient>>> + Send {
        self.inner.receive()
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        // Closing stdin tells the server to exit. Kill it if it does not.
        let closed = self.inner.close().await;
        if let Some(mut child) = self.child.take() {
            if tokio::time::timeout(EXIT_GRACE, child.wait())
                .await
                .is_err()
            {
                let _ = child.kill().await;
            }
        }
        closed
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // test code: #[tokio::test] builds a runtime
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    #[tokio::test]
    async fn lines_under_the_limit_pass_and_long_runs_fail() {
        let limit = 64;
        // Two short lines, then one line over the limit with no newline.
        let mut data = b"short\nalso short\n".to_vec();
        data.extend(std::iter::repeat_n(b'x', limit + 1));
        let mut reader = LineLimit::new(&data[..], limit);
        let mut out = Vec::new();
        let mut chunk = [0u8; 7];
        let err = loop {
            match reader.read(&mut chunk).await {
                Ok(0) => panic!("the reader reached EOF before the limit"),
                Ok(n) => out.extend_from_slice(&chunk[..n]),
                Err(e) => break e,
            }
        };
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(out.starts_with(b"short\nalso short\n"));
        // The failing read stopped the stream before the whole run was read.
        assert!(out.len() < data.len());
    }

    #[tokio::test]
    async fn a_line_exactly_at_the_limit_passes() {
        let limit = 16;
        let mut data = vec![b'a'; limit];
        data.push(b'\n');
        let mut reader = LineLimit::new(&data[..], limit);
        let mut out = Vec::new();
        reader.read_to_end(&mut out).await.unwrap();
        assert_eq!(out, data);
    }
}

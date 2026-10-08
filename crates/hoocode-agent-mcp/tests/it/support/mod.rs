//! Test helpers: an in-process Streamable HTTP server and the stdio server's config.

pub mod oauth;
pub mod server;

use std::path::Path;
use std::sync::Arc;

use hyper::server::conn::http1;
use hyper_util::rt::TokioIo;
use hyper_util::service::TowerToHyperService;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use tokio_util::sync::CancellationToken;

use hoocode_agent_mcp::McpServerConfig;

/// A Streamable HTTP MCP server on 127.0.0.1 (an ephemeral port), running in this process.
/// It stops when dropped.
pub struct HttpServer {
    pub url: String,
    shutdown: CancellationToken,
}

impl HttpServer {
    pub fn config(&self) -> McpServerConfig {
        McpServerConfig::Http {
            url: self.url.clone(),
            headers: Default::default(),
        }
    }
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

/// Starts the test server on 127.0.0.1:0.
pub async fn start_http_server() -> HttpServer {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind 127.0.0.1:0");
    let listener_addr = listener.local_addr().expect("local address");
    let shutdown = CancellationToken::new();
    let service = StreamableHttpService::new(
        || Ok(server::TestServer { allow_exit: false }),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default().with_cancellation_token(shutdown.child_token()),
    );
    let stop = shutdown.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = stop.cancelled() => break,
                accepted = listener.accept() => {
                    let Ok((stream, _)) = accepted else { continue };
                    let service = TowerToHyperService::new(service.clone());
                    tokio::spawn(async move {
                        let _ = http1::Builder::new()
                            .serve_connection(TokioIo::new(stream), service)
                            .await;
                    });
                }
            }
        }
    });
    HttpServer {
        url: format!("http://{}/mcp", listener_addr),
        shutdown,
    }
}

/// The stdio test server: the `mcp_test_server` example, which `cargo test`
/// builds next to this test binary (`target/<profile>/examples/`).
pub fn stdio_config() -> McpServerConfig {
    let exe = std::env::current_exe().expect("test binary path");
    let profile_dir = exe
        .parent()
        .and_then(Path::parent)
        .expect("target/<profile>/deps/<test>");
    let server = profile_dir
        .join("examples")
        .join(format!("mcp_test_server{}", std::env::consts::EXE_SUFFIX));
    McpServerConfig::Stdio {
        command: server.to_string_lossy().into_owned(),
        args: Vec::new(),
        env: Default::default(),
        cwd: None,
    }
}

//! The stdio MCP server used by the hoocode-agent-mcp tests. Not a product binary.
//!
//! The server code lives in `tests/it/support/server.rs`, shared with the
//! in-process HTTP server.

#[path = "../tests/it/support/server.rs"]
#[allow(dead_code)]
mod server;

use rmcp::transport::stdio;
use rmcp::ServiceExt;

// Test helper process only: it starts its own runtime, like any stdio server.
#[allow(clippy::disallowed_methods)]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let service = server::TestServer { allow_exit: true }
        .serve(stdio())
        .await?;
    service.waiting().await?;
    Ok(())
}

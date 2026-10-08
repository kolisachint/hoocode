//! MCP client for hoocode agents, built on the `rmcp` crate.
//!
//! Only this crate depends on `rmcp` (`migration/dep-firewall.json`). It speaks
//! stdio and Streamable HTTP, not the legacy HTTP+SSE transport.
//!
//! - [`McpServerConfig`]: how to reach a server.
//! - [`McpClient`]: connect, [`list_tools`](McpClient::list_tools),
//!   [`call_tool`](McpClient::call_tool) with cancellation and progress.
//! - [`tool_name`]: the model-facing name `mcp_<server>_<tool>`.
//! - [`ToolOutput`]: the result as plain text and image content.
//!
//! Caps (`docs/design/concurrency.md` section 3): 8 requests in flight per
//! server, a 32 MiB response limit and a 60 s default deadline, all
//! configurable through [`ClientOptions`]. The client builds no runtime and no
//! thread; it runs on the runtime its caller is on (`hoocode-io` in the binary).

mod client;
mod config;
mod error;
mod naming;
mod result;

pub use client::{
    CallOptions, ClientOptions, McpClient, McpTool, ToolProgress, DEFAULT_REQUEST_TIMEOUT,
    MAX_IN_FLIGHT_REQUESTS, MAX_RESPONSE_BYTES,
};
pub use config::McpServerConfig;
pub use error::McpError;
pub use naming::{tool_name, MAX_TOOL_NAME_LEN};
pub use result::{ToolContent, ToolOutput};

//! MCP client for hoocode agents, built on the `rmcp` crate.
//!
//! Only this crate depends on `rmcp` (`migration/dep-firewall.json`). It speaks
//! stdio and Streamable HTTP, not the legacy HTTP+SSE transport.
//!
//! - [`McpServerConfig`]: how to reach a server.
//! - [`McpClient`]: connect (or [`connect_oauth`](McpClient::connect_oauth) for
//!   a Streamable HTTP server that needs a login), [`list_tools`](McpClient::list_tools),
//!   [`call_tool`](McpClient::call_tool) with cancellation and progress.
//! - [`oauth`]: the login flow and the per-issuer token store.
//! - [`tool_name`]: the model-facing name `mcp_<server>_<tool>`.
//! - [`ToolOutput`]: the result as plain text and image content.
//!
//! Caps (`docs/design/concurrency.md` section 3): 8 requests in flight per
//! server, a 32 MiB response limit and a 60 s default deadline, all
//! configurable through [`ClientOptions`]. Two transport limits are fixed: a
//! stdio message (one line) over 32 MiB ends the connection, and a server-sent
//! event over 16 MiB fails the request ([`MAX_STDIO_LINE_BYTES`] and
//! [`MAX_SSE_EVENT_BYTES`]).
//!
//! Threads and runtimes: the client builds no tokio runtime and spawns no OS
//! thread of its own. Its futures run on the runtime of the caller (`hoocode-io`
//! in the binary). The rmcp worker tasks and the child process reaper are
//! tokio tasks on that same runtime. The crate has no `std::thread` use, and the
//! only process it starts is the stdio server the config names.
//!
//! OAuth: rmcp does discovery, PKCE, `iss` checks and refresh; this crate adds
//! the owner-only per-issuer token files and the loopback login. Tokens are
//! never sent to a stdio server.

mod client;
mod config;
mod error;
mod naming;
pub mod oauth;
mod result;
mod stdio;

pub use client::{
    CallOptions, ClientOptions, McpClient, McpTool, ToolProgress, DEFAULT_REQUEST_TIMEOUT,
    MAX_IN_FLIGHT_REQUESTS, MAX_RESPONSE_BYTES, MAX_SSE_EVENT_BYTES,
};
pub use config::McpServerConfig;
pub use error::McpError;
pub use naming::{tool_name, MAX_TOOL_NAME_LEN};
pub use oauth::{begin_login, LoginHandle};
pub use result::{ToolContent, ToolOutput};
pub use stdio::MAX_STDIO_LINE_BYTES;

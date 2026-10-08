#![allow(clippy::disallowed_methods)] // test code: #[tokio::test] expands to a runtime builder
//! Integration tests for hoocode-agent-mcp. Each behaviour runs over the stdio
//! test server and over the in-process Streamable HTTP server, unless it needs
//! a process exit (stdio only).

mod client;
mod oauth;
mod support;

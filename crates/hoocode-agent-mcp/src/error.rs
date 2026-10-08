//! Errors from talking to an MCP server. A tool that ran and failed is not an
//! error here: it is a [`ToolOutput`](crate::ToolOutput) with `is_error` set.

use std::fmt;
use std::time::Duration;

/// Why an MCP request did not produce a result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpError {
    /// The configuration cannot be used (bad header name, bad arguments, ...).
    Config(String),
    /// The server could not be started or initialized.
    Connect(String),
    /// The request did not finish before its deadline.
    Timeout(Duration),
    /// The caller cancelled the request.
    Cancelled,
    /// The connection dropped, and the one reconnect did not fix it.
    Disconnected(String),
    /// The server answered with a JSON-RPC error.
    Rpc(String),
    /// The server sent something the client cannot use.
    Protocol(String),
    /// A list response is over the size cap.
    TooLarge { limit: usize },
    /// The server answered 401 and no usable token is stored. Log in with
    /// [`oauth::begin_login`](crate::oauth::begin_login), then reconnect.
    /// `www_authenticate` is the server's challenge header, as sent.
    AuthRequired {
        server: String,
        www_authenticate: String,
    },
    /// Login or token handling failed (discovery, the callback, the token
    /// exchange, or the token store).
    Auth(String),
}

impl fmt::Display for McpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(m) => write!(f, "MCP configuration error: {m}"),
            Self::Connect(m) => write!(f, "MCP connect failed: {m}"),
            Self::Timeout(d) => write!(f, "MCP request timed out after {}s", d.as_secs_f64()),
            Self::Cancelled => write!(f, "MCP request cancelled"),
            Self::Disconnected(m) => write!(f, "MCP server disconnected: {m}"),
            Self::Rpc(m) => write!(f, "MCP server error: {m}"),
            Self::Protocol(m) => write!(f, "MCP protocol error: {m}"),
            Self::TooLarge { limit } => write!(f, "MCP response exceeds {limit} bytes"),
            Self::AuthRequired {
                server,
                www_authenticate,
            } => write!(
                f,
                "MCP server {server} requires login ({www_authenticate}); run the OAuth login first"
            ),
            Self::Auth(m) => write!(f, "MCP OAuth error: {m}"),
        }
    }
}

impl std::error::Error for McpError {}

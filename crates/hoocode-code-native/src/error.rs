//! Errors for tier C native helpers: resolution, download, start, watch and calls.
//!
//! A tool that ran and reported a failure is not an error here: it is a
//! [`ToolOutput`](hoocode_agent_mcp::ToolOutput) with `is_error` set.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use hoocode_agent_mcp::McpError;

/// Why a native helper could not be resolved, started or called.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeError {
    /// Tier C helpers need a Unix socket and process groups: not supported here.
    Unsupported(String),
    /// No helper with this plugin name was given to the supervisor.
    UnknownPlugin(String),
    /// No agent with this id is known (root or registered) for minting a child.
    UnknownAgent(String),
    /// The helper process could not be spawned.
    Spawn(String),
    /// The socket never appeared, or `initialize` never succeeded, in time.
    Timeout { plugin: String, after: Duration },
    /// The helper used up its restart budget. It stays down until the session ends.
    Down { plugin: String, reason: String },
    /// The binary could not be downloaded.
    Download(String),
    /// The downloaded bytes do not match the manifest `sha256`: not installed.
    Checksum {
        plugin: String,
        expected: String,
        actual: String,
    },
    /// The manifest names an archive (`.tar.gz`, `.zip`): plain binaries only.
    Archive(String),
    /// The configured `binaryPath` cannot be used.
    InvalidBinary(String),
    /// A plugin name or manifest version is not a safe path component (it
    /// would escape its folder): never used as one.
    InvalidName(String),
    /// Talking to the running helper failed.
    Mcp(McpError),
    /// A filesystem or OS call failed.
    Io(String),
}

impl fmt::Display for NativeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(message) => write!(f, "native helpers are not supported here: {message}"),
            Self::UnknownPlugin(plugin) => write!(f, "no native helper for plugin {plugin:?}"),
            Self::UnknownAgent(agent) => write!(f, "unknown agent {agent:?}"),
            Self::Spawn(message) => write!(f, "cannot start the helper: {message}"),
            Self::Timeout { plugin, after } => write!(
                f,
                "the {plugin} helper was not ready within {}s",
                after.as_secs_f64()
            ),
            Self::Down { plugin, reason } => write!(f, "{plugin} helper is down: {reason}"),
            Self::Download(message) => write!(f, "cannot download the helper binary: {message}"),
            Self::Checksum {
                plugin,
                expected,
                actual,
            } => write!(
                f,
                "the {plugin} helper download does not match its sha256 (expected {expected}, got {actual}): not installed"
            ),
            Self::Archive(message) => write!(f, "{message}"),
            Self::InvalidBinary(message) => write!(f, "unusable binaryPath: {message}"),
            Self::InvalidName(message) => write!(f, "invalid plugin name or version: {message}"),
            Self::Mcp(error) => write!(f, "{error}"),
            Self::Io(message) => write!(f, "native helper I/O failed: {message}"),
        }
    }
}

impl std::error::Error for NativeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Mcp(error) => Some(error),
            _ => None,
        }
    }
}

impl From<McpError> for NativeError {
    fn from(error: McpError) -> Self {
        Self::Mcp(error)
    }
}

impl From<std::io::Error> for NativeError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

/// The supervisor shut down: every call fails with this.
pub fn shut_down(plugin: &str) -> NativeError {
    NativeError::Down {
        plugin: plugin.to_owned(),
        reason: "the supervisor shut down".to_owned(),
    }
}

/// Where a helper binary install lives: `~/.hoocode/plugin-bin/<plugin>/<version>/`.
pub fn plugin_bin_dir(home: &Path, plugin: &str, version: &str) -> PathBuf {
    home.join("plugin-bin").join(plugin).join(version)
}

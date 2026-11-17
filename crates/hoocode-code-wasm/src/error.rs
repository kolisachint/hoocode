//! Errors from loading a WASM extension and calling its tools.

use std::fmt;
use std::io;
use std::path::PathBuf;

/// Why an extension could not be loaded, or why a tool call gave no result.
#[derive(Debug)]
pub enum WasmError {
    /// The wasmtime engine could not be configured.
    Engine(String),
    /// A file or folder the host reads or creates failed.
    Io { path: PathBuf, source: io::Error },
    /// The component did not compile, or did not link against the host.
    Compile(String),
    /// The component's tool list is invalid: a bad name, a schema that is not
    /// a JSON object, or a duplicate name.
    InvalidTools(String),
    /// The plugin does not offer a tool with this name.
    UnknownTool(String),
    /// The arguments did not match the tool's input schema.
    InvalidArgs(String),
    /// The tool ran and failed. The message is for the model.
    Failed(String),
    /// The call ran longer than the call timeout.
    Timeout,
    /// The caller aborted the call.
    Cancelled,
    /// The guest trapped (a memory limit, a panic) or wasmtime failed.
    Trap(String),
    /// The tool's result text is not JSON.
    BadResult(String),
}

impl fmt::Display for WasmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Engine(msg) => write!(f, "wasm engine: {msg}"),
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::Compile(msg) => write!(f, "wasm component: {msg}"),
            Self::InvalidTools(msg) => write!(f, "invalid tool list: {msg}"),
            Self::UnknownTool(name) => write!(f, "unknown tool: {name}"),
            Self::InvalidArgs(msg) => write!(f, "invalid arguments: {msg}"),
            Self::Failed(msg) => write!(f, "tool failed: {msg}"),
            Self::Timeout => write!(f, "tool call timed out"),
            Self::Cancelled => write!(f, "tool call cancelled"),
            Self::Trap(msg) => write!(f, "tool trapped: {msg}"),
            Self::BadResult(msg) => write!(f, "tool result is not JSON: {msg}"),
        }
    }
}

impl std::error::Error for WasmError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

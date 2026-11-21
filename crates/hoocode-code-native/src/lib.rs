//! Tier C native helpers: the per-session supervisor that downloads, starts,
//! watches and restarts one helper process per plugin (`docs/design/plugins.md`,
//! Native tier contract; spec D2 to D5).
//!
//! The root process holds a [`Supervisor`]: `ensure` starts a helper on first
//! use, `call_tool` calls a tool for the root agent, `register_child` and
//! `end_agent` track subagent identities (re-sent after a restart).
//! [`NativeClient`] is the non-root side: a subagent process connects to the
//! relayed socket with its own token. Slice 4 builds the "ask parent
//! EnsureHelper" relay on top; this crate only mints identities and connects.

mod agents;
mod download;
mod error;
mod resolve;
mod runtime_dir;
mod supervisor;

pub use agents::{
    mint_agent, new_token, AgentIdentity, AGENT_DEPTH_ENV, AGENT_ID_ENV, AGENT_META_KEY,
    AGENT_PARENT_ENV, AGENT_TOKEN_ENV, ROOT_AGENT_ID,
};
pub use download::{
    ensure_plugin_binary, fetch_bytes, file_name_from_url, sha256_hex, verify_sha256,
    MAX_DOWNLOAD_BYTES,
};
pub use error::{plugin_bin_dir, NativeError};
pub use resolve::{
    binary_path_override, binary_path_overrides, default_home_dir, resolve_executable,
};
pub use runtime_dir::{
    create_runtime_dir, internal_runtime_dir, random_suffix, socket_path_for,
    validate_path_component, INTERNAL_RUNTIME_DIR_ENV, MAX_SOCKET_PATH_BYTES,
};
pub use supervisor::{
    helper_bin_env_name, helper_socket_env_name, NativeCallOptions, NativeClient, Supervisor,
    SupervisorConfig,
};
pub use supervisor::{HELPER_SOCKET_ENV, HELPER_TOKEN_ENV};

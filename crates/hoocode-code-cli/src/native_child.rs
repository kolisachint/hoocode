//! Tier C native helper tools in subagent children (PL-18 slice 4b, spec D5/D6).
//!
//! A subagent gets tier C tools only from plugins whose manifest says
//! `shared: true`, with the same names, origins, hints and collision rules as
//! the root. Plain plugin MCP tools and non-shared tier C tools stay absent.
//! The child has no supervisor: its first call of a plugin asks the parent
//! for the helper's socket over the existing parent channel (EnsureHelper),
//! connects with its own identity from the child env, and caches the
//! connection. On an auth or connection failure the child ensures once more
//! and retries once (the helper may have restarted and dropped its token);
//! anything else surfaces at once. A child with no parent channel reports a
//! clear error instead of hanging.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use hoocode_agent_mcp::{McpError, ToolOutput};
use hoocode_agent_types::ToolOrigin;
use hoocode_code_native::{
    helper_socket_env_name, mint_agent, AgentIdentity, NativeCallOptions, NativeClient,
    NativeError, Supervisor, AGENT_ID_ENV, AGENT_TOKEN_ENV,
};
use hoocode_code_plugins::PluginSet;
use hoocode_code_subagents::pool::{HostFuture, NativeChildHost};
use hoocode_code_tool_api::ToolDefinition;
use hoocode_code_tool_bash::{BashSpawnContext, BashSpawnHook};
use serde_json::Value;

use crate::native_tools::{self, NativeExecutor};
use crate::runtime::Diagnostics;

/// Per-session tier C state of a process that may run as a subagent child: its
/// own identity (from the child env; `None` in a root process) and its cached
/// helper connections, by plugin. Owned by the session (see
/// `runtime::assemble_session`): the plugin source, the Shell hook and the
/// pool host share it, and the next session replaces it.
pub(crate) struct ChildNativeState {
    identity: Option<AgentIdentity>,
    clients: Mutex<HashMap<String, ChildConn>>,
}

struct ChildConn {
    socket: PathBuf,
    client: NativeClient,
}

impl ChildNativeState {
    pub(crate) fn new() -> Self {
        Self {
            identity: AgentIdentity::from_env(),
            clients: Mutex::new(HashMap::new()),
        }
    }

    /// This process's tier C identity, when spawned as a subagent with one.
    pub(crate) fn identity(&self) -> Option<AgentIdentity> {
        self.identity.clone()
    }

    fn cached(&self, plugin: &str) -> Option<NativeClient> {
        self.clients
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(plugin)
            .map(|conn| conn.client.clone())
    }

    fn store(&self, plugin: &str, socket: PathBuf, client: NativeClient) {
        self.clients
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(plugin.to_owned(), ChildConn { socket, client });
    }

    fn forget(&self, plugin: &str) {
        self.clients
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(plugin);
    }

    /// The known helper sockets, for the Shell env.
    fn sockets(&self) -> Vec<(String, PathBuf)> {
        let mut out: Vec<(String, PathBuf)> = self
            .clients
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(plugin, conn)| (plugin.clone(), conn.socket.clone()))
            .collect();
        out.sort();
        out
    }
}

/// The pool's tier C host for one session (see
/// `hoocode_code_subagents::pool::NativeChildHost`): mints the D5 identity of
/// every spawned child, ensures helper sockets (registering the child), and
/// deregisters ended children. A root process serves from its supervisor; a
/// subagent child serves from its own connection, asking its own parent first
/// (recursively), so a grandchild's EnsureHelper reaches the helper through
/// every level.
pub(crate) struct CliNativeHost {
    /// Whether the session has a shared tier C helper at all: without one no
    /// identity is minted and every EnsureHelper is denied.
    has_shared: bool,
    shared: Arc<ChildNativeState>,
    mode: HostMode,
}

enum HostMode {
    Root {
        supervisor: Arc<Supervisor>,
        children: Mutex<HashMap<String, AgentIdentity>>,
        /// (task id, plugin) pairs already registered: each child is
        /// announced to a helper generation at most once (the supervisor
        /// itself re-announces every live agent after a restart).
        registered: Mutex<HashSet<(String, String)>>,
    },
    Child {
        children: Mutex<HashMap<String, ChildReg>>,
    },
}

struct ChildReg {
    identity: AgentIdentity,
    plugins: HashSet<String>,
}

impl CliNativeHost {
    pub(crate) fn root(
        supervisor: Arc<Supervisor>,
        shared: Arc<ChildNativeState>,
        has_shared: bool,
    ) -> Self {
        Self {
            has_shared,
            shared,
            mode: HostMode::Root {
                supervisor,
                children: Mutex::new(HashMap::new()),
                registered: Mutex::new(HashSet::new()),
            },
        }
    }

    pub(crate) fn child(shared: Arc<ChildNativeState>, has_shared: bool) -> Self {
        Self {
            has_shared,
            shared,
            mode: HostMode::Child {
                children: Mutex::new(HashMap::new()),
            },
        }
    }
}

impl NativeChildHost for CliNativeHost {
    fn child_env(&self, task_id: &str) -> Option<Vec<(String, String)>> {
        if !self.has_shared {
            return None;
        }
        match &self.mode {
            HostMode::Root {
                supervisor,
                children,
                ..
            } => {
                let identity = supervisor.mint_agent("root", task_id).ok()?;
                let vars = identity.child_env().into_iter().collect::<Vec<_>>();
                children
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(task_id.to_owned(), identity);
                Some(vars)
            }
            HostMode::Child { children } => {
                let own = self.shared.identity()?;
                let identity = mint_agent(&own, task_id);
                let vars = identity.child_env().into_iter().collect::<Vec<_>>();
                children.lock().unwrap_or_else(|e| e.into_inner()).insert(
                    task_id.to_owned(),
                    ChildReg {
                        identity,
                        plugins: HashSet::new(),
                    },
                );
                Some(vars)
            }
        }
    }

    fn ensure_helper<'a>(
        &'a self,
        task_id: &'a str,
        plugin: &'a str,
    ) -> HostFuture<'a, Result<PathBuf, String>> {
        Box::pin(async move {
            match &self.mode {
                HostMode::Root {
                    supervisor,
                    children,
                    registered,
                } => {
                    let identity = children
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .get(task_id)
                        .cloned()
                        .ok_or_else(|| {
                            format!("Unknown subagent {task_id}: no tier C identity.")
                        })?;
                    let socket = supervisor
                        .ensure(plugin)
                        .await
                        .map_err(|error| error.to_string())?;
                    let fresh = registered
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert((task_id.to_owned(), plugin.to_owned()));
                    if fresh {
                        supervisor
                            .register_child(plugin, &identity)
                            .await
                            .map_err(|error| error.to_string())?;
                    }
                    Ok(socket)
                }
                HostMode::Child { children } => {
                    let identity = children
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .get(task_id)
                        .map(|reg| reg.identity.clone())
                        .ok_or_else(|| {
                            format!("Unknown subagent {task_id}: no tier C identity.")
                        })?;
                    // Our own connection first (asking our own parent,
                    // recursively), then the child on top of it. The
                    // connection is rebuilt every time: after a helper
                    // restart the cached one holds a dropped token.
                    let relay = hoocode_code_subagents::instance::clearance_relay().ok_or_else(|| {
                        format!(
                            "The {plugin} helper needs the parent session: no parent channel in this process."
                        )
                    })?;
                    let socket: PathBuf =
                        relay
                            .ask_ensure(plugin)
                            .map(PathBuf::from)
                            .map_err(|error| {
                                format!("The {plugin} helper is not available: {error}")
                            })?;
                    let own = self.shared.identity().ok_or_else(|| {
                        "This subagent was started without a tier C identity.".to_string()
                    })?;
                    let client = NativeClient::connect(&socket, plugin, &own)
                        .await
                        .map_err(|error| error.to_string())?;
                    client
                        .register_child(&identity)
                        .await
                        .map_err(|error| error.to_string())?;
                    self.shared.store(plugin, socket.clone(), client);
                    children
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .get_mut(task_id)
                        .map(|reg| reg.plugins.insert(plugin.to_owned()));
                    Ok(socket)
                }
            }
        })
    }

    fn child_ended<'a>(&'a self, task_id: &'a str) -> HostFuture<'a, ()> {
        Box::pin(async move {
            match &self.mode {
                HostMode::Root {
                    supervisor,
                    children,
                    ..
                } => {
                    let identity = children
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(task_id);
                    if let Some(identity) = identity {
                        let _ = supervisor.end_agent(&identity.agent_id).await;
                    }
                }
                HostMode::Child { children } => {
                    let reg = children
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(task_id);
                    if let Some(reg) = reg {
                        for plugin in reg.plugins {
                            if let Some(client) = self.shared.cached(&plugin) {
                                let _ = client.end_agent(&reg.identity.agent_id).await;
                            }
                        }
                    }
                }
            }
        })
    }
}

/// The shared tier C tools of one subagent session (spec D6): the same names,
/// origins, hints and collision rules as the root, but only from plugins with
/// `shared: true`, and running through the child's cached connection. Plain
/// plugin MCP tools and non-shared tier C tools stay absent (the caller keeps
/// them out by never building them here).
pub(crate) fn build_shared(
    plugins: &PluginSet,
    shared: &Arc<ChildNativeState>,
    taken: &[(String, ToolOrigin)],
    diagnostics: &mut Diagnostics,
) -> Vec<ToolDefinition> {
    let trusted: std::collections::HashSet<&str> = plugins
        .plugins
        .iter()
        .filter(|p| p.marketplace_trusted)
        .map(|p| p.name.as_str())
        .collect();
    let entries: Vec<hoocode_code_plugins::NativeHelperEntry> = plugins
        .native_helpers()
        .into_iter()
        .filter(|entry| trusted.contains(entry.plugin.as_str()) && entry.helper.spec.shared)
        .collect();
    // The D2 hints apply in the child too: its gate uses this map, and gated
    // calls go up the existing clearance relay as today.
    native_tools::publish_hints(&entries);
    native_tools::build_from_entries(
        entries,
        taken,
        diagnostics,
        NativeExecutor::Child(shared.clone()),
    )
}

/// Run one shared tool in a subagent child: EnsureHelper for the socket (the
/// error names the plugin when there is no parent channel), connect with this
/// process's identity, call, and — on an auth or connection failure only —
/// ensure once more and retry once (the helper may have restarted and dropped
/// the token). Anything else surfaces at once. Calls are bounded by the
/// client's own timeout.
pub(crate) fn call_tool(
    state: &Arc<ChildNativeState>,
    handle: &tokio::runtime::Handle,
    plugin: &str,
    tool: &str,
    params: Value,
) -> Result<ToolOutput, NativeError> {
    let identity = state.identity().ok_or_else(|| {
        NativeError::Io(format!(
            "Cannot run the shared plugin tool {tool}: this subagent was started without a tier C identity."
            ))
    })?;
    let relay = hoocode_code_subagents::instance::clearance_relay().ok_or_else(|| {
        NativeError::Io(format!(
            "Cannot run the shared plugin tool {tool}: no parent channel in this process."
        ))
    })?;
    let attempt = || -> Result<ToolOutput, NativeError> {
        let socket: PathBuf = relay
            .ask_ensure(plugin)
            .map(PathBuf::from)
            .map_err(NativeError::Io)?;
        let client = match state.cached(plugin) {
            Some(client) => client,
            None => {
                let client = handle.block_on(NativeClient::connect(&socket, plugin, &identity))?;
                state.store(plugin, socket, client.clone());
                client
            }
        };
        handle.block_on(client.call_tool(tool, params.clone(), NativeCallOptions::default()))
    };
    match attempt() {
        Ok(output) => Ok(output),
        Err(error) if is_recoverable(&error) => {
            // The helper may have restarted since the last call: drop the
            // cached connection, register again, and retry once.
            state.forget(plugin);
            attempt()
        }
        Err(error) => Err(error),
    }
}

/// An error a second EnsureHelper round-trip may fix: the helper restarted
/// (dropped tokens, dropped connections) since the cached client connected.
fn is_recoverable(error: &NativeError) -> bool {
    matches!(
        error,
        NativeError::Mcp(
            McpError::Unauthorized(_)
                | McpError::AuthRequired { .. }
                | McpError::Disconnected(_)
                | McpError::Connect(_)
        )
    )
}

/// Shell commands in a subagent get the child's own identity and the started
/// helpers' sockets (spec D5), like the root's Shell hook but for this
/// process. A session with no helper contributes no extra variable, exactly
/// like the root's hook.
pub(crate) fn shell_spawn_hook(state: Arc<ChildNativeState>) -> BashSpawnHook {
    Arc::new(move |mut ctx: BashSpawnContext| {
        if let Some(identity) = state.identity() {
            ctx.env
                .insert(AGENT_ID_ENV.into(), identity.agent_id.into());
            ctx.env
                .insert(AGENT_TOKEN_ENV.into(), identity.token.into());
            for (plugin, socket) in state.sockets() {
                if let Some(path) = socket.to_str() {
                    ctx.env
                        .insert(helper_socket_env_name(&plugin).into(), path.into());
                }
            }
        }
        ctx
    })
}

/// Whether the loaded plugins offer subagents any tier C tool: at least one
/// native helper with `shared: true` (spec D6). Trust is not consulted here;
/// an untrusted plugin simply fails its EnsureHelper with a clear error.
pub(crate) fn has_shared_helpers(plugins: &PluginSet) -> bool {
    plugins
        .native_helpers()
        .iter()
        .any(|entry| entry.helper.spec.shared)
}

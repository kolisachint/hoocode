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
use hoocode_ai_types::AbortSignal;
use hoocode_code_native::{
    helper_socket_env_name, mint_agent, AgentIdentity, NativeCallOptions, NativeClient,
    NativeError, AGENT_ID_ENV, AGENT_TOKEN_ENV,
};
use hoocode_code_plugins::PluginSet;
use hoocode_code_subagents::pool::{HostFuture, NativeChildHost};
use hoocode_code_tool_api::ToolDefinition;
use hoocode_code_tool_bash::{BashSpawnContext, BashSpawnHook};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::native_tools::{self, NativeExecutor, NativeHolder};
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
    shared: Arc<ChildNativeState>,
    mode: HostMode,
}

enum HostMode {
    Root {
        holder: Arc<NativeHolder>,
        children: Mutex<HashMap<String, AgentIdentity>>,
        /// `(task id, plugin, token)` already registered: keyed by token so
        /// a retried task (same task id, fresh token) registers again
        /// instead of reusing the dead token. Entries leave in `child_ended`;
        /// a failed register never marks.
        registered: Mutex<HashSet<(String, String, String)>>,
    },
    Child {
        /// The shared plugin names this session offers: `EnsureHelper` for
        /// anything else is refused without a relay round trip.
        shared_plugins: HashSet<String>,
        children: Mutex<HashMap<String, ChildReg>>,
    },
}

struct ChildReg {
    identity: AgentIdentity,
    plugins: HashSet<String>,
}

impl CliNativeHost {
    /// The tier C host of a root session: serves from the session holder's
    /// current supervisor (resolved on every call, so a mid-session plugin
    /// reload is picked up).
    pub(crate) fn root(holder: Arc<NativeHolder>, shared: Arc<ChildNativeState>) -> Self {
        Self {
            shared,
            mode: HostMode::Root {
                holder,
                children: Mutex::new(HashMap::new()),
                registered: Mutex::new(HashSet::new()),
            },
        }
    }

    /// The tier C host of a subagent child: serves from its own connection,
    /// asking its own parent first. `shared_plugins` are the shared helper
    /// names this session offers.
    pub(crate) fn child(shared: Arc<ChildNativeState>, shared_plugins: HashSet<String>) -> Self {
        Self {
            shared,
            mode: HostMode::Child {
                shared_plugins,
                children: Mutex::new(HashMap::new()),
            },
        }
    }
}

impl NativeChildHost for CliNativeHost {
    fn child_env(&self, task_id: &str) -> Option<Vec<(String, String)>> {
        match &self.mode {
            HostMode::Root {
                holder, children, ..
            } => {
                // The root serves from its supervisor directly: mint while
                // the session has a shared helper.
                let supervisor = holder.supervisor().filter(|s| s.has_shared_helpers())?;
                let identity = supervisor.mint_agent("root", task_id).ok()?;
                let vars = identity.child_env().into_iter().collect::<Vec<_>>();
                children
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(task_id.to_owned(), identity);
                Some(vars)
            }
            HostMode::Child {
                shared_plugins,
                children,
            } => {
                // Without a shared helper there is nothing to hand out, and
                // without a parent channel there is no EnsureHelper relay: a
                // minted identity then would offer tools whose every call
                // fails.
                if shared_plugins.is_empty() {
                    return None;
                }
                // A relay that cannot answer EnsureHelper is no parent
                // channel (a headless fail-closed child, warm workers): mint
                // nothing, as when no relay is set at all.
                let relay = hoocode_code_subagents::instance::clearance_relay()?;
                if !relay.can_ensure() {
                    return None;
                }
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
                    holder,
                    children,
                    registered,
                } => {
                    let supervisor = holder
                        .supervisor()
                        .ok_or_else(|| "This session has no tier C helper.".to_string())?;
                    // A child could otherwise get a root-only helper: refuse
                    // before anything starts or registers.
                    if !supervisor.is_shared(plugin) {
                        return Err(format!(
                            "Plugin {plugin} does not share its helper with subagents (shared: false): no socket to hand out."
                        ));
                    }
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
                    // Keyed by token: a retried task (same task id, fresh
                    // token) registers again instead of reusing the dead
                    // token, and a failed register never marks.
                    let key = (
                        task_id.to_owned(),
                        plugin.to_owned(),
                        identity.token.clone(),
                    );
                    let fresh = registered
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(key.clone());
                    if fresh {
                        if let Err(error) = supervisor.register_child(plugin, &identity).await {
                            registered
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .remove(&key);
                            return Err(error.to_string());
                        }
                    }
                    Ok(socket)
                }
                HostMode::Child {
                    shared_plugins,
                    children,
                } => {
                    // Refuse root-only helpers before relaying up: the
                    // parent would refuse them anyway.
                    if !shared_plugins.contains(plugin) {
                        return Err(format!(
                            "Plugin {plugin} does not share its helper with subagents (shared: false): no socket to hand out."
                        ));
                    }
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
                    let relay = hoocode_code_subagents::instance::clearance_relay()
                        .filter(|relay| relay.can_ensure())
                        .ok_or_else(|| {
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
                    holder,
                    children,
                    registered,
                } => {
                    let identity = children
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(task_id);
                    // The token the retry (or the failed register) left
                    // behind must not block the next registration.
                    registered
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .retain(|(task, _, _)| task != task_id);
                    if let Some(identity) = identity {
                        if let Some(supervisor) = holder.supervisor() {
                            let _ = supervisor.end_agent(&identity.agent_id).await;
                        }
                    }
                }
                HostMode::Child { children, .. } => {
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
    // The D2 hints are published once for the whole set by the caller (the
    // plugin source, before its per-plugin loop): publishing here would
    // replace the process-wide map once per plugin and leave only the last
    // plugin's hints. The child's gate reads that map, and gated calls go
    // up the existing clearance relay as today.
    native_tools::build_from_entries(
        entries,
        taken,
        diagnostics,
        NativeExecutor::Child(shared.clone()),
    )
}

/// Run one shared tool in a subagent child: one EnsureHelper round trip per
/// plugin per process, then the cached socket and connection; re-asked only
/// after a recoverable error (the helper may have restarted and dropped the
/// token). The turn's abort signal fires `cancel` and ends the wait at once
/// (see `native_tools::abortable`). Calls are bounded by the client's own timeout.
pub(crate) fn call_tool(
    state: &Arc<ChildNativeState>,
    handle: &tokio::runtime::Handle,
    plugin: &str,
    tool: &str,
    params: Value,
    signal: Option<&AbortSignal>,
    cancel: CancellationToken,
) -> Result<ToolOutput, NativeError> {
    let cancel2 = cancel.clone();
    super::native_tools::abortable(handle, signal, cancel, async move {
        let identity = state.identity().ok_or_else(|| {
            NativeError::Io(format!(
            "Cannot run the shared plugin tool {tool}: this subagent was started without a tier C identity."
            ))
        })?;
        let relay = hoocode_code_subagents::instance::clearance_relay()
            .filter(|relay| relay.can_ensure())
            .ok_or_else(|| {
                NativeError::Io(format!(
                    "Cannot run the shared plugin tool {tool}: no parent channel in this process."
                ))
            })?;
        // One EnsureHelper round trip per plugin per process: the socket and
        // the parent's connection are cached; re-asked only after a
        // recoverable error. (The relay answer itself blocks the tool
        // thread; the abort token still cancels the call that follows.)
        let client = match state.cached(plugin) {
            Some(client) => client,
            None => connect_cached(state, &relay, plugin, &identity).await?,
        };
        let options = NativeCallOptions {
            cancel: cancel2.clone(),
        };
        match client.call_tool(tool, params.clone(), options).await {
            Ok(output) => Ok(output),
            Err(error) if is_recoverable(&error) => {
                // The request never reached the helper (a connect failure or
                // an auth rejection before the send): drop the cached
                // connection, register again, and retry once. Anything else
                // — including a mid-call disconnect — may have run and is
                // returned.
                state.forget(plugin);
                let client = connect_cached(state, &relay, plugin, &identity).await?;
                let options = NativeCallOptions { cancel: cancel2 };
                client.call_tool(tool, params, options).await
            }
            Err(error) => Err(error),
        }
    })
}

/// Ask the parent for `plugin`'s socket and connect with this process's
/// identity, caching both for the next call.
async fn connect_cached(
    state: &Arc<ChildNativeState>,
    relay: &Arc<dyn hoocode_code_permissions::ClearanceRelay>,
    plugin: &str,
    identity: &AgentIdentity,
) -> Result<NativeClient, NativeError> {
    let socket: PathBuf = relay
        .ask_ensure(plugin)
        .map(PathBuf::from)
        .map_err(NativeError::Io)?;
    let client = NativeClient::connect(&socket, plugin, identity).await?;
    state.store(plugin, socket, client.clone());
    Ok(client)
}

/// An error a second EnsureHelper round-trip may fix: the request never
/// reached the helper (a connect failure, or an auth rejection before the
/// send), so a replay is safe. A mid-call `Disconnected` may have run
/// server-side and must not be replayed.
fn is_recoverable(error: &NativeError) -> bool {
    matches!(
        error,
        NativeError::Mcp(
            McpError::Connect(_) | McpError::Unauthorized(_) | McpError::AuthRequired { .. }
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

#[cfg(all(test, unix))]
#[allow(clippy::disallowed_methods)] // tests drive the fixture helper on their own runtime
mod tests {
    use super::*;
    use hoocode_code_mcp::TrustStore;
    use hoocode_code_plugins::{load_plugins, LoadOptions};
    use hoocode_code_settings::SettingsManager;
    use serde_json::json;
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    /// The built echo helper fixture (same recipe as the native integration
    /// tests): builds once, reuses the binary afterwards.
    fn fixture_bin() -> PathBuf {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let workspace = manifest
            .parent()
            .and_then(|dir| dir.parent())
            .expect("crate lives in <workspace>/crates/<name>")
            .to_path_buf();
        let bin = workspace
            .join("target")
            .join("debug")
            .join("examples")
            .join(format!("echo_helper{}", std::env::consts::EXE_SUFFIX));
        if !bin.is_file() {
            let status = std::process::Command::new("cargo")
                .arg("build")
                .arg("-p")
                .arg("hoocode-agent-mcp")
                .arg("--example")
                .arg("echo_helper")
                .current_dir(&workspace)
                .status()
                .expect("cargo builds the echo fixture");
            assert!(status.success(), "cargo build --example echo_helper");
        }
        assert!(bin.is_file(), "the echo fixture was built");
        bin
    }

    fn write_plugin(dir: &Path, name: &str, shared: bool, receipts: &Path) {
        let root = dir.join(name);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("plugin.json"),
            json!({
                "$schema": "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json",
                "name": name,
                "version": "1.0.0",
                "description": "Tier C fixture",
                "extensions": {"dev.hoocode": {"native": {
                    "server": "echo",
                    "shared": shared,
                    "tools": [
                        {"name": "echo", "description": "Echo.",
                         "inputSchema": {"type": "object"}, "permission": "read"},
                    ],
                }}},
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            root.join("mcp.json"),
            json!({
                "$schema": "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json",
                "mcpServers": {"echo": {
                    "type": "stdio",
                    "command": "never-started-without-binaryPath",
                    "env": {"HOOCODE_ECHO_RECEIPT_DIR": receipts},
                }},
            })
            .to_string(),
        )
        .unwrap();
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        holder: Arc<NativeHolder>,
        receipts: PathBuf,
    }

    impl Fixture {
        fn start() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let plugins = dir.path().join("plugins");
            let receipts = dir.path().join("receipts");
            std::fs::create_dir_all(&receipts).unwrap();
            write_plugin(&plugins, "demo", true, &receipts);
            write_plugin(&plugins, "rt", false, &receipts);
            std::fs::create_dir_all(dir.path().join("repo")).unwrap();
            let trust = TrustStore::empty(dir.path().join("trust.json"));
            let set = load_plugins(&LoadOptions {
                project_folder: dir.path().join("repo"),
                user_plugins_dir: plugins,
                data_root: dir.path().join("plugin-data"),
                trust: &trust,
                untrusted_installs: BTreeSet::new(),
                installs_unreadable: None,
            });
            assert!(
                set.native_helpers().iter().any(|e| e.plugin == "demo"),
                "the shared fixture parses"
            );
            assert!(
                set.native_helpers().iter().any(|e| e.plugin == "rt"),
                "the root-only fixture parses"
            );
            let settings = SettingsManager::in_memory(
                json!({"nativeHelpers": {"demo": {"binaryPath": fixture_bin()}}})
                    .as_object()
                    .unwrap()
                    .clone(),
            );
            let holder = NativeHolder::new();
            let warnings = holder.sync(&set, &settings);
            assert!(warnings.is_empty(), "{warnings:?}");
            Self {
                _dir: dir,
                holder,
                receipts,
            }
        }

        fn agent_started_receipts(&self) -> Vec<PathBuf> {
            let mut out: Vec<PathBuf> = std::fs::read_dir(&self.receipts)
                .unwrap()
                .filter_map(|entry| {
                    let path = entry.ok()?.path();
                    let name = path.file_name()?.to_str()?;
                    (name.starts_with("agent-started-") && name.ends_with(".received"))
                        .then(|| path.clone())
                })
                .collect();
            out.sort();
            out
        }
    }

    /// Only pre-send failures are recoverable: a connect failure or an auth
    /// rejection means the helper never saw the request, so one replay is
    /// safe. A mid-call disconnect may have run server-side and surfaces.
    #[test]
    fn only_pre_send_failures_are_recoverable() {
        for error in [
            NativeError::Mcp(McpError::Connect("down".to_owned())),
            NativeError::Mcp(McpError::Unauthorized("bad token".to_owned())),
            NativeError::Mcp(McpError::AuthRequired {
                server: "helper".to_owned(),
                www_authenticate: "Bearer".to_owned(),
            }),
        ] {
            assert!(is_recoverable(&error), "{error}");
        }
        for error in [
            NativeError::Mcp(McpError::Disconnected("mid-call".to_owned())),
            NativeError::Mcp(McpError::Rpc("boom".to_owned())),
            NativeError::Mcp(McpError::Timeout(std::time::Duration::from_secs(1))),
            NativeError::Mcp(McpError::Cancelled),
            NativeError::UnknownPlugin("ghost".to_owned()),
        ] {
            assert!(!is_recoverable(&error), "{error}");
        }
    }

    /// Even with an identity, a child process with no parent channel mints
    /// nothing: without an EnsureHelper relay every shared-tool call would
    /// fail, so no identity (and no shared tool) is offered.
    #[test]
    fn child_host_mints_nothing_without_a_relay() {
        let _relay_lock = crate::RELAY_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        assert!(hoocode_code_subagents::instance::clearance_relay().is_none());
        let state = Arc::new(ChildNativeState {
            identity: Some(AgentIdentity::root("token".to_owned())),
            clients: Mutex::new(HashMap::new()),
        });
        let host = CliNativeHost::child(state, ["demo".to_owned()].into_iter().collect());
        assert!(host.child_env("t1").is_none());
    }

    /// The root host resolves the current supervisor on every call: after a
    /// mid-session plugin change the old identities stop minting and the old
    /// plugins stop serving.
    #[tokio::test]
    async fn root_host_follows_mid_session_plugin_changes() {
        let fixture = Fixture::start();
        let host = CliNativeHost::root(fixture.holder.clone(), Arc::new(ChildNativeState::new()));
        assert!(host.child_env("t1").is_some());
        host.ensure_helper("t1", "demo")
            .await
            .expect("ensure before the change");
        // The plugin set changes mid-session: no helpers at all now.
        let settings = SettingsManager::in_memory(serde_json::Map::new());
        let warnings = fixture
            .holder
            .sync(&hoocode_code_plugins::PluginSet::default(), &settings);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(fixture.holder.supervisor().is_none());
        assert!(
            host.child_env("t2").is_none(),
            "nothing to mint without a shared helper"
        );
        let error = host
            .ensure_helper("t2", "demo")
            .await
            .expect_err("the old supervisor is gone");
        assert!(error.contains("no tier C helper"), "{error}");
    }

    /// Cancelling a running helper call returns promptly: the abort fires
    /// the call's token instead of waiting out the 30 s sleep.
    #[tokio::test]
    async fn cancelled_sleep_call_returns_promptly() {
        let fixture = Fixture::start();
        let supervisor = fixture.holder.supervisor().expect("supervisor");
        supervisor.ensure("demo").await.expect("the helper starts");
        let cancel = CancellationToken::new();
        let options = NativeCallOptions {
            cancel: cancel.clone(),
        };
        let call = supervisor.call_tool("demo", "sleep", json!({"ms": 30_000}), options);
        let started = std::time::Instant::now();
        let canceller = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            cancel.cancel();
        });
        let result = call.await;
        let elapsed = started.elapsed();
        let _ = canceller.await;
        assert!(
            matches!(result, Err(NativeError::Mcp(McpError::Cancelled))),
            "the sleep is cancelled, not run to completion: {result:?}"
        );
        assert!(
            elapsed < std::time::Duration::from_secs(10),
            "the cancelled call returns promptly: {elapsed:?}"
        );
        let _ = supervisor.shutdown().await;
    }

    /// `EnsureHelper` for a `shared: false` plugin is refused on both sides
    /// before anything starts or registers.
    #[tokio::test]
    async fn unshared_ensure_helper_is_refused_before_start() {
        let fixture = Fixture::start();
        let host = CliNativeHost::root(fixture.holder.clone(), Arc::new(ChildNativeState::new()));
        assert!(host.child_env("t1").is_some(), "a shared helper mints");
        let error = host
            .ensure_helper("t1", "rt")
            .await
            .expect_err("root-only helpers hand out no socket");
        assert!(error.contains("shared"), "{error}");
        let supervisor = fixture.holder.supervisor().expect("supervisor");
        assert_eq!(supervisor.spawn_count("rt"), 0);
        assert!(!supervisor.live_sockets().contains_key("rt"));
        // The child side refuses before relaying up: no parent channel needed.
        let child = CliNativeHost::child(
            Arc::new(ChildNativeState::new()),
            ["demo".to_owned()].into_iter().collect(),
        );
        let error = child
            .ensure_helper("t9", "rt")
            .await
            .expect_err("root-only helpers are refused in the child");
        assert!(error.contains("shared"), "{error}");
    }

    /// A retried task (same task id, fresh token) registers again after
    /// `child_ended`: the helper re-announces the new token instead of
    /// skipping a stale registration.
    #[tokio::test]
    async fn retried_task_registers_again_after_child_ended() {
        let fixture = Fixture::start();
        let host = CliNativeHost::root(fixture.holder.clone(), Arc::new(ChildNativeState::new()));
        assert!(host.child_env("t1").is_some());
        host.ensure_helper("t1", "demo")
            .await
            .expect("first ensure registers");
        let supervisor = fixture.holder.supervisor().expect("supervisor");
        assert_eq!(supervisor.spawn_count("demo"), 1);
        assert_eq!(fixture.agent_started_receipts().len(), 1);
        // The retry mints a fresh token under the same task id.
        host.child_ended("t1").await;
        // Delete the announcement: the retry must announce the fresh token
        // again (the same file name would hide a skipped registration).
        for receipt in fixture.agent_started_receipts() {
            std::fs::remove_file(receipt).unwrap();
        }
        assert!(host.child_env("t1").is_some());
        host.ensure_helper("t1", "demo")
            .await
            .expect("the retry registers again");
        assert_eq!(supervisor.spawn_count("demo"), 1, "no restart");
        assert_eq!(
            fixture.agent_started_receipts().len(),
            1,
            "the fresh token is announced again"
        );
        let _ = supervisor.shutdown().await;
    }
}

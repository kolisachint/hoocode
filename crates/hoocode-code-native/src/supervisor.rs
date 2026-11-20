//! The tier C supervisor (root process only): one helper process per plugin.
//!
//! Started on first use, never at session start (spec D4). `ensure` starts the
//! helper for a plugin and waits until its socket exists and an MCP
//! `initialize` succeeds; `call_tool` calls a tool for the root agent,
//! restarting once when the connection broke. At most `max_restarts` restarts
//! in any `restart_window`, then the helper is down until the session ends.
//! After a restart every live registered agent is re-sent. `shutdown` ends
//! the session gracefully and removes the runtime dir.
//!
//! [`NativeClient`] is the non-root side: subagent processes connect with
//! their own token. Slice 4 builds the "ask parent EnsureHelper" relay on top;
//! this crate only connects, given the socket.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use hoocode_agent_mcp::{
    CallOptions, ClientOptions, McpClient, McpError, McpServerConfig, ToolOutput,
};
use hoocode_code_plugins::NativeHelperEntry;
use serde_json::Value;

use crate::agents::{mint_agent, AgentIdentity, ROOT_AGENT_ID};
use crate::error::{shut_down, NativeError};
use crate::resolve::resolve_executable;
use crate::runtime_dir::{create_runtime_dir, socket_path_for};

/// Passed to the helper in its environment (spec D4).
pub const HELPER_SOCKET_ENV: &str = "HOOCODE_HELPER_SOCKET";
/// The root bearer token, in the helper's environment.
pub const HELPER_TOKEN_ENV: &str = "HOOCODE_HELPER_TOKEN";
use crate::agents::AGENT_ID_ENV;

/// The Shell environment variable carrying one started helper's socket (spec D5):
/// `HOOCODE_HELPER_SOCKET_<PLUGIN>`, with the plugin name uppercased and every
/// non-alphanumeric character replaced by `_`.
pub fn helper_socket_env_name(plugin: &str) -> String {
    let mut name = String::from("HOOCODE_HELPER_SOCKET_");
    for c in plugin.chars() {
        if c.is_ascii_alphanumeric() {
            name.push(c.to_ascii_uppercase());
        } else {
            name.push('_');
        }
    }
    name
}

/// How long a fire-and-forget lifecycle notification (`agentStarted`,
/// `agentEnded`) may take before it stops blocking the caller. Helpers
/// answer these in milliseconds; waiting longer means the connection is
/// wedged, and the supervisor restarts the helper instead of hanging.
const LIFECYCLE_NOTIFY_TIMEOUT: Duration = Duration::from_secs(5);
/// How long `shutdown` waits for the `sessionEnding` notification to go out.
/// Shutdown must stay fast even when a helper is wedged: afterwards the
/// child is killed anyway.
const SHUTDOWN_NOTIFY_TIMEOUT: Duration = Duration::from_millis(500);

/// How long helpers get to become ready, and how their restarts are bounded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SupervisorConfig {
    /// Ready = socket exists and `initialize` succeeds. Default 10 s.
    pub ready_timeout: Duration,
    /// Restarts allowed in any `restart_window` before the helper is down.
    pub max_restarts: u32,
    /// The rolling window for `max_restarts`. Default 3 minutes.
    pub restart_window: Duration,
    /// How long `shutdown` waits for a helper to exit after `sessionEnding`.
    pub shutdown_timeout: Duration,
}

impl Default for SupervisorConfig {
    fn default() -> Self {
        Self {
            ready_timeout: Duration::from_secs(10),
            max_restarts: 5,
            restart_window: Duration::from_secs(3 * 60),
            shutdown_timeout: Duration::from_secs(2),
        }
    }
}

impl SupervisorConfig {
    /// Short timeouts and a small budget, for tests.
    pub fn for_tests() -> Self {
        Self {
            ready_timeout: Duration::from_millis(500),
            max_restarts: 5,
            restart_window: Duration::from_secs(60),
            shutdown_timeout: Duration::from_secs(2),
        }
    }
}

/// How often the ready wait polls for the socket.
const READY_POLL: Duration = Duration::from_millis(50);

struct RegisteredAgent {
    identity: AgentIdentity,
    plugins: HashSet<String>,
}

enum HelperState {
    Idle,
    Live {
        socket: PathBuf,
        client: McpClient,
        child: tokio::process::Child,
    },
    Down {
        reason: String,
    },
}

struct HelperSlot {
    /// Serializes starts and restarts of one helper.
    start: tokio::sync::Mutex<()>,
    state: tokio::sync::Mutex<HelperState>,
    /// Bumped on every spawn: lets a call tell a fresh helper from a stale one.
    generation: AtomicU64,
    /// Spawn attempts so far (initial start plus restarts).
    spawn_count: AtomicU64,
    /// When restarts happened, for the rolling budget.
    restarts: Mutex<VecDeque<Instant>>,
}

impl HelperSlot {
    fn new() -> Self {
        Self {
            start: tokio::sync::Mutex::new(()),
            state: tokio::sync::Mutex::new(HelperState::Idle),
            generation: AtomicU64::new(0),
            spawn_count: AtomicU64::new(0),
            restarts: Mutex::new(VecDeque::new()),
        }
    }
}

struct SupervisorInner {
    config: SupervisorConfig,
    home: PathBuf,
    entries: HashMap<String, NativeHelperEntry>,
    overrides: HashMap<String, PathBuf>,
    root_identity: AgentIdentity,
    runtime_dir: PathBuf,
    slots: HashMap<String, HelperSlot>,
    agents: Mutex<HashMap<String, RegisteredAgent>>,
    shutdown: AtomicBool,
}

impl Drop for SupervisorInner {
    fn drop(&mut self) {
        // Best effort: shutdown removes the dir properly; the children die
        // with the state (kill_on_drop) and the helper exits on --parent-pid.
        let _ = std::fs::remove_dir_all(&self.runtime_dir);
    }
}

/// The supervisor: cheap to clone, safe to share. Root process only.
pub struct Supervisor {
    inner: std::sync::Arc<SupervisorInner>,
}

impl Clone for Supervisor {
    fn clone(&self) -> Self {
        Self {
            inner: std::sync::Arc::clone(&self.inner),
        }
    }
}

impl Supervisor {
    /// Build the supervisor: makes the runtime dir and mints the root token.
    /// `home` is the hoocode home dir (`~/.hoocode/plugin-bin` lives under it;
    /// see [`crate::resolve::default_home_dir`]), injectable for tests.
    pub fn new(
        home: impl Into<PathBuf>,
        helpers: Vec<NativeHelperEntry>,
        overrides: HashMap<String, PathBuf>,
        config: SupervisorConfig,
    ) -> Result<Self, NativeError> {
        let runtime_dir =
            crate::runtime_dir::internal_runtime_dir().unwrap_or(create_runtime_dir()?);
        let root_identity = AgentIdentity::root(crate::agents::new_token());
        let slots = helpers
            .iter()
            .map(|entry| (entry.plugin.clone(), HelperSlot::new()))
            .collect();
        let entries = helpers
            .into_iter()
            .map(|entry| (entry.plugin.clone(), entry))
            .collect();
        Ok(Self {
            inner: std::sync::Arc::new(SupervisorInner {
                config,
                home: home.into(),
                entries,
                overrides,
                root_identity,
                runtime_dir,
                slots,
                agents: Mutex::new(HashMap::new()),
                shutdown: AtomicBool::new(false),
            }),
        })
    }

    /// The session runtime dir holding the sockets and logs.
    pub fn runtime_dir(&self) -> &Path {
        &self.inner.runtime_dir
    }

    /// The root agent identity (id `root`, depth 0, the minted token).
    pub fn root_identity(&self) -> &AgentIdentity {
        &self.inner.root_identity
    }

    /// How many times the helper was spawned (initial start plus restarts).
    pub fn spawn_count(&self, plugin: &str) -> u64 {
        self.inner
            .slots
            .get(plugin)
            .map(|slot| slot.spawn_count.load(Ordering::SeqCst))
            .unwrap_or(0)
    }

    /// The known identity of one agent: root, or a registered child.
    pub fn agent(&self, agent_id: &str) -> Option<AgentIdentity> {
        if agent_id == ROOT_AGENT_ID {
            return Some(self.inner.root_identity.clone());
        }
        lock(&self.inner.agents)
            .get(agent_id)
            .map(|record| record.identity.clone())
    }

    /// Mint a child of `parent_agent_id` for `task_id`. The parent must be the
    /// root or a registered agent; subagent processes without a supervisor use
    /// the free [`mint_agent`] with the parent identity they were given.
    pub fn mint_agent(
        &self,
        parent_agent_id: &str,
        task_id: &str,
    ) -> Result<AgentIdentity, NativeError> {
        let Some(parent) = self.agent(parent_agent_id) else {
            return Err(NativeError::UnknownAgent(parent_agent_id.to_owned()));
        };
        Ok(mint_agent(&parent, task_id))
    }

    /// Whether this supervisor manages at least one helper.
    pub fn has_helpers(&self) -> bool {
        !self.inner.entries.is_empty()
    }

    /// Whether at least one managed helper is shared with subagents: only
    /// then do spawned children get agent env and the EnsureHelper relay
    /// (spec D5/D6).
    pub fn has_shared_helpers(&self) -> bool {
        self.inner
            .entries
            .values()
            .any(|entry| entry.helper.spec.shared)
    }

    /// The socket paths of the helpers currently running (started and not yet
    /// shut down), without starting anything. Used for the Shell tool's
    /// environment (spec D5): a socket appears once its helper has started.
    pub fn live_sockets(&self) -> HashMap<String, PathBuf> {
        let mut out = HashMap::new();
        for plugin in self.inner.entries.keys() {
            let Some(slot) = self.inner.slots.get(plugin) else {
                continue;
            };
            let Ok(state) = slot.state.try_lock() else {
                continue;
            };
            if let HelperState::Live { socket, .. } = &*state {
                out.insert(plugin.clone(), socket.clone());
            }
        }
        out
    }

    /// Start the helper for `plugin` on first use and return its socket path.
    /// Later calls return the running helper's path. Unix only.
    pub async fn ensure(&self, plugin: &str) -> Result<PathBuf, NativeError> {
        #[cfg(not(unix))]
        {
            let _ = plugin;
            return Err(NativeError::Unsupported(
                "tier C helpers need a Unix socket".to_owned(),
            ));
        }
        #[cfg(unix)]
        {
            self.ensure_unix(plugin).await
        }
    }

    /// Call `tool` for the root agent, starting the helper first. When the
    /// child exited or the connection broke, the helper restarts once and the
    /// call is retried once.
    pub async fn call_tool(
        &self,
        plugin: &str,
        tool: &str,
        args: Value,
    ) -> Result<ToolOutput, NativeError> {
        self.ensure(plugin).await?;
        let generation = self.generation(plugin);
        match self.do_call_bounded(plugin, tool, args.clone()).await {
            Err(error) => {
                // The error flavor depends on when the helper died: restart
                // when the transport broke, or when the child is gone.
                if !is_broken(&error) && self.child_alive(plugin).await {
                    return Err(error);
                }
                self.restart_for_retry(plugin, generation).await?;
                self.do_call_bounded(plugin, tool, args).await
            }
            ok => ok,
        }
    }

    /// Register a child agent with one helper: stores it (so a restart
    /// re-sends it) and sends `hoocode/agentStarted` over this process's
    /// connection, starting the helper first.
    pub async fn register_child(
        &self,
        plugin: &str,
        identity: &AgentIdentity,
    ) -> Result<(), NativeError> {
        if !self.inner.entries.contains_key(plugin) {
            return Err(NativeError::UnknownPlugin(plugin.to_owned()));
        }
        {
            let mut agents = lock(&self.inner.agents);
            agents
                .entry(identity.agent_id.clone())
                .or_insert_with(|| RegisteredAgent {
                    identity: identity.clone(),
                    plugins: HashSet::new(),
                })
                .plugins
                .insert(plugin.to_owned());
        }
        self.notify(plugin, "hoocode/agentStarted", identity.started_params())
            .await
    }

    /// Send `hoocode/agentEnded` to every helper the agent was registered
    /// with, and forget it. Helpers that never started, or whose child
    /// already exited, get nothing. Every send is bounded: ending an agent
    /// must not hang on the call timeout.
    pub async fn end_agent(&self, agent_id: &str) -> Result<(), NativeError> {
        let plugins = lock(&self.inner.agents)
            .remove(agent_id)
            .map(|record| record.plugins)
            .unwrap_or_default();
        for plugin in plugins {
            let client = {
                let Some(slot) = self.inner.slots.get(&plugin) else {
                    continue;
                };
                let mut state = slot.state.lock().await;
                let HelperState::Live { client, child, .. } = &mut *state else {
                    continue;
                };
                match child.try_wait() {
                    Ok(None) => client.clone(),
                    _ => continue,
                }
            };
            // Best effort: teardown must not fail the session.
            let send = client.notify(
                "hoocode/agentEnded",
                serde_json::json!({"agentId": agent_id}),
            );
            let _ = tokio::time::timeout(LIFECYCLE_NOTIFY_TIMEOUT, send).await;
        }
        Ok(())
    }

    /// End the session: send `hoocode/sessionEnding` to every started helper,
    /// wait up to `shutdown_timeout` for each exit, then kill what is left,
    /// and remove the runtime dir. Idempotent.
    pub async fn shutdown(&self) -> Result<(), NativeError> {
        if self.inner.shutdown.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        for slot in self.inner.slots.values() {
            let mut state = slot.state.lock().await;
            let HelperState::Live {
                client, mut child, ..
            } = std::mem::replace(&mut *state, HelperState::Idle)
            else {
                continue;
            };
            drop(state);
            // Bounded: a wedged helper must not hold shutdown past a beat.
            // Afterwards the child is killed either way.
            let send = client.notify("hoocode/sessionEnding", Value::Null);
            let _ = tokio::time::timeout(SHUTDOWN_NOTIFY_TIMEOUT, send).await;
            let done = tokio::time::timeout(self.inner.config.shutdown_timeout, child.wait())
                .await
                .is_ok();
            if !done {
                let _ = child.kill().await;
            }
            let _ = child.wait().await;
        }
        std::fs::remove_dir_all(&self.inner.runtime_dir).map_err(|error| {
            NativeError::Io(format!(
                "cannot remove runtime dir {}: {error}",
                self.inner.runtime_dir.display()
            ))
        })?;
        Ok(())
    }

    /// Run `do_call`, but stop waiting as soon as the helper child is gone:
    /// the connection dies with it, so the call can never answer. Without
    /// this a call in flight while the helper crashes waits out the whole
    /// call timeout before the supervisor restarts it.
    async fn do_call_bounded(
        &self,
        plugin: &str,
        tool: &str,
        args: Value,
    ) -> Result<ToolOutput, NativeError> {
        tokio::select! {
            output = self.do_call(plugin, tool, args) => output,
            () = self.wait_child_gone(plugin) => Err(child_died(plugin)),
        }
    }

    /// Resolves when the helper child for `plugin` is gone: it exited, or
    /// the slot left `Live` (restart, shutdown). Polls with `try_wait`, so
    /// it never blocks a start holding the state lock.
    async fn wait_child_gone(&self, plugin: &str) {
        loop {
            let gone = match self.inner.slots.get(plugin) {
                Some(slot) => match slot.state.try_lock() {
                    Ok(mut state) => match &mut *state {
                        HelperState::Live { child, .. } => {
                            matches!(child.try_wait(), Ok(Some(_)))
                        }
                        _ => true,
                    },
                    // A start holds the lock: the child is being replaced,
                    // and the caller's retry path will sort it out.
                    Err(_) => false,
                },
                None => true,
            };
            if gone {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn do_call(
        &self,
        plugin: &str,
        tool: &str,
        args: Value,
    ) -> Result<ToolOutput, NativeError> {
        let client = {
            let Some(slot) = self.inner.slots.get(plugin) else {
                return Err(NativeError::UnknownPlugin(plugin.to_owned()));
            };
            let mut state = slot.state.lock().await;
            match &mut *state {
                // A dead child means the connection is dead too: report it
                // as not running so the caller restarts immediately instead
                // of sending a call into the void and waiting out the call
                // timeout. `try_wait` also reaps the zombie here.
                HelperState::Live { client, child, .. } => match child.try_wait() {
                    Ok(None) => client.clone(),
                    _ => {
                        return Err(child_died(plugin));
                    }
                },
                HelperState::Down { reason } => {
                    return Err(NativeError::Down {
                        plugin: plugin.to_owned(),
                        reason: reason.clone(),
                    });
                }
                HelperState::Idle => {
                    return Err(not_running(plugin));
                }
            }
        };
        Ok(client.call_tool(tool, args, CallOptions::default()).await?)
    }

    /// Send a notification over the root connection, restarting once when the
    /// connection broke, the child is gone, or the helper never started.
    async fn notify(&self, plugin: &str, method: &str, params: Value) -> Result<(), NativeError> {
        self.ensure(plugin).await?;
        let generation = self.generation(plugin);
        match self.try_notify(plugin, method, params.clone()).await {
            Some(Ok(())) => return Ok(()),
            // A send failure on a live helper goes out: a second try would
            // hang the same way when the connection is wedged.
            Some(Err(error)) if !is_broken(&error) && self.child_alive(plugin).await => {
                return Err(error);
            }
            // Broken, dead, or never started: restart once, then retry once.
            Some(Err(_)) | None => {}
        }
        self.restart_for_retry(plugin, generation).await?;
        match self.try_notify(plugin, method, params).await {
            Some(result) => result,
            None => Err(not_running(plugin)),
        }
    }

    /// `None` when no live client exists right now (starting, idle, down,
    /// or its child already exited). The send itself is bounded: a wedged
    /// connection must not hang the caller on the 60 s call timeout.
    async fn try_notify(
        &self,
        plugin: &str,
        method: &str,
        params: Value,
    ) -> Option<Result<(), NativeError>> {
        let client = self.live_client(plugin)?;
        let send = client.notify(method, params);
        Some(
            match tokio::time::timeout(LIFECYCLE_NOTIFY_TIMEOUT, send).await {
                Ok(result) => result.map_err(NativeError::from),
                Err(_) => Err(NativeError::Timeout {
                    plugin: plugin.to_owned(),
                    after: LIFECYCLE_NOTIFY_TIMEOUT,
                }),
            },
        )
    }

    /// True when the helper's child is still running. A dead child is reaped.
    async fn child_alive(&self, plugin: &str) -> bool {
        let Some(slot) = self.inner.slots.get(plugin) else {
            return false;
        };
        let mut state = slot.state.lock().await;
        match &mut *state {
            HelperState::Live { child, .. } => matches!(child.try_wait(), Ok(None)),
            _ => false,
        }
    }

    fn live_client(&self, plugin: &str) -> Option<McpClient> {
        let slot = self.inner.slots.get(plugin)?;
        // The state lock is async; notify callers hold no lock across this, so
        // try_lock succeeds unless a start holds it (then there is no live
        // client to use anyway).
        match slot.state.try_lock() {
            Ok(mut state) => match &mut *state {
                // A dead child means the connection is dead too: no client.
                // The caller restarts instead of sending into the void.
                HelperState::Live {
                    socket,
                    client,
                    child,
                } => match child.try_wait() {
                    Ok(None) if socket.exists() => Some(client.clone()),
                    _ => None,
                },
                _ => None,
            },
            Err(_) => None,
        }
    }

    fn generation(&self, plugin: &str) -> u64 {
        self.inner
            .slots
            .get(plugin)
            .map(|slot| slot.generation.load(Ordering::SeqCst))
            .unwrap_or(0)
    }

    /// Restart when the generation still matches: someone else may have
    /// restarted already while this call waited.
    async fn restart_for_retry(&self, plugin: &str, generation: u64) -> Result<(), NativeError> {
        let Some(slot) = self.inner.slots.get(plugin) else {
            return Err(NativeError::UnknownPlugin(plugin.to_owned()));
        };
        let _guard = slot.start.lock().await;
        if slot.generation.load(Ordering::SeqCst) != generation {
            return Ok(());
        }
        let mut state = slot.state.lock().await;
        self.start_locked(plugin, slot, &mut state)
            .await
            .map(|_| ())
    }

    #[cfg(unix)]
    async fn ensure_unix(&self, plugin: &str) -> Result<PathBuf, NativeError> {
        if self.inner.shutdown.load(Ordering::SeqCst) {
            return Err(shut_down(plugin));
        }
        let Some(slot) = self.inner.slots.get(plugin) else {
            return Err(NativeError::UnknownPlugin(plugin.to_owned()));
        };
        let _guard = slot.start.lock().await;
        let mut state = slot.state.lock().await;
        match &mut *state {
            HelperState::Live { socket, child, .. } => match child.try_wait() {
                Ok(None) if socket.exists() => return Ok(socket.clone()),
                // Dead or unreachable: fall through to (re)start.
                _ => {}
            },
            HelperState::Down { reason } => {
                return Err(NativeError::Down {
                    plugin: plugin.to_owned(),
                    reason: reason.clone(),
                });
            }
            HelperState::Idle => {}
        }
        self.start_locked(plugin, slot, &mut state).await
    }

    /// Spawn (or respawn) the helper and wait for it to be ready. The caller
    /// holds the slot's start lock. On failure the slot goes back to `Idle`;
    /// on success every registered agent is re-sent.
    #[cfg(unix)]
    async fn start_locked(
        &self,
        plugin: &str,
        slot: &HelperSlot,
        state: &mut HelperState,
    ) -> Result<PathBuf, NativeError> {
        let entry = self
            .inner
            .entries
            .get(plugin)
            .cloned()
            .ok_or_else(|| NativeError::UnknownPlugin(plugin.to_owned()))?;
        // Retire whatever is there: a dead child, or a live one whose
        // connection broke.
        if let HelperState::Live { mut child, .. } = std::mem::replace(state, HelperState::Idle) {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        // The restart budget counts every spawn after the first, including
        // ones that then fail: a bad binary must not respawn forever.
        let previous = slot.spawn_count.fetch_add(1, Ordering::SeqCst);
        if previous > 0 {
            let mut restarts = lock(&slot.restarts);
            restarts.retain(|when| when.elapsed() < self.inner.config.restart_window);
            if restarts.len() >= self.inner.config.max_restarts as usize {
                let reason = format!(
                    "restarted {} times in the last {}s",
                    restarts.len(),
                    self.inner.config.restart_window.as_secs()
                );
                *state = HelperState::Down {
                    reason: reason.clone(),
                };
                return Err(NativeError::Down {
                    plugin: plugin.to_owned(),
                    reason,
                });
            }
            restarts.push_back(Instant::now());
        }
        let client = hoocode_ai_util::tls::http_client_builder()
            .build()
            .map_err(|error| {
                NativeError::Download(format!("cannot build the HTTP client: {error}"))
            })?;
        let executable =
            resolve_executable(&self.inner.home, &entry, &self.inner.overrides, &client).await?;
        let socket = socket_path_for(&self.inner.runtime_dir, plugin)?;
        let _ = std::fs::remove_file(&socket);
        let log = self.inner.runtime_dir.join(format!("{plugin}.log"));
        let mut child = spawn_helper(
            &executable,
            &entry,
            &socket,
            &self.inner.root_identity.token,
            &log,
        )
        .map_err(|error| {
            NativeError::Spawn(format!("cannot spawn {}: {error}", executable.display()))
        })?;
        slot.generation.fetch_add(1, Ordering::SeqCst);
        match wait_ready(
            plugin,
            &socket,
            &self.inner.root_identity,
            self.inner.config.ready_timeout,
        )
        .await
        {
            Ok(ready_client) => {
                // After a restart every live registered agent is re-sent.
                // The lock is scoped: the guard must not cross the awaits.
                let reregister: Vec<AgentIdentity> = {
                    lock(&self.inner.agents)
                        .values()
                        .filter(|record| record.plugins.contains(plugin))
                        .map(|record| record.identity.clone())
                        .collect()
                };
                for agent in reregister {
                    if let Err(error) = ready_client
                        .notify("hoocode/agentStarted", agent.started_params())
                        .await
                    {
                        let _ = child.kill().await;
                        let _ = child.wait().await;
                        return Err(error.into());
                    }
                }
                *state = HelperState::Live {
                    socket: socket.clone(),
                    client: ready_client,
                    child,
                };
                Ok(socket)
            }
            Err(error) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                Err(error)
            }
        }
    }
}

/// The helper child died under an in-flight call: the transport is gone
/// with it. Restartable, like any broken connection.
fn child_died(plugin: &str) -> NativeError {
    NativeError::Mcp(McpError::Disconnected(format!(
        "the {plugin} helper exited mid-call"
    )))
}

/// A transport-level failure: the helper is gone or unreachable. Tool-level
/// failures (`Rpc`, error results) and timeouts never restart.
fn is_broken(error: &NativeError) -> bool {
    matches!(
        error,
        NativeError::Mcp(McpError::Disconnected(_) | McpError::Connect(_))
    )
}

/// The helper is not running (idle, or mid-start): restart, don't report.
fn not_running(plugin: &str) -> NativeError {
    NativeError::Down {
        plugin: plugin.to_owned(),
        reason: "the helper is not running".to_owned(),
    }
}

/// Ready = the socket exists and an MCP `initialize` succeeds, within
/// `timeout`. Deterministic failures (bad config, cancel) return at once.
/// The stored client is connected afterwards with the default options, so
/// calls get the full deadline instead of the leftover ready budget.
#[cfg(unix)]
async fn wait_ready(
    plugin: &str,
    socket: &Path,
    root: &AgentIdentity,
    timeout: Duration,
) -> Result<McpClient, NativeError> {
    let deadline = Instant::now() + timeout;
    loop {
        if Instant::now() >= deadline {
            break;
        }
        if socket.exists() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match McpClient::connect(
                plugin,
                unix_config(socket, root),
                ClientOptions {
                    request_timeout: remaining,
                    ..ClientOptions::default()
                },
            )
            .await
            {
                Ok(probe) => {
                    probe.shutdown().await;
                    break;
                }
                Err(error @ (McpError::Config(_) | McpError::Cancelled)) => {
                    return Err(error.into());
                }
                Err(_) => {
                    // Not up yet (or gone again): keep polling to the deadline.
                }
            }
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        tokio::time::sleep(READY_POLL.min(remaining)).await;
    }
    if Instant::now() >= deadline {
        return Err(NativeError::Timeout {
            plugin: plugin.to_owned(),
            after: timeout,
        });
    }
    // The socket answered: connect for real. A failure here means the helper
    // died between probe and connect; the caller retires the child.
    let client = McpClient::connect(plugin, unix_config(socket, root), ClientOptions::default())
        .await
        .map_err(NativeError::from)?;
    client.set_call_meta(root.call_meta());
    Ok(client)
}

/// The root connection config for one socket.
#[cfg(unix)]
fn unix_config(socket: &Path, root: &AgentIdentity) -> McpServerConfig {
    McpServerConfig::Unix {
        path: socket.to_path_buf(),
        headers: BTreeMap::from([("Authorization".to_owned(), format!("Bearer {}", root.token))]),
    }
}

/// Spawn one helper: `<exe> <args...> --parent-pid <pid>` with the mcp.json
/// env plus the socket, the root token and `HOOCODE_AGENT_ID=root`. Stdout and
/// stderr append to the plugin log. Own process group, kill on drop.
#[cfg(unix)]
fn spawn_helper(
    exe: &Path,
    entry: &NativeHelperEntry,
    socket: &Path,
    root_token: &str,
    log_path: &Path,
) -> std::io::Result<tokio::process::Child> {
    let mut command = tokio::process::Command::new(exe);
    command.args(&entry.helper.launch.args);
    command
        .arg("--parent-pid")
        .arg(std::process::id().to_string());
    command.envs(&entry.helper.launch.env);
    command.env(HELPER_SOCKET_ENV, socket);
    command.env(HELPER_TOKEN_ENV, root_token);
    command.env(AGENT_ID_ENV, ROOT_AGENT_ID);
    if let Some(cwd) = &entry.helper.launch.cwd {
        command.current_dir(cwd);
    }
    command.stdin(std::process::Stdio::null());
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)?;
    command.stdout(log.try_clone()?);
    command.stderr(log);
    command.kill_on_drop(true);
    hoocode_runtime::spawn_in_new_session(command.as_std_mut());
    command.spawn()
}

/// The non-root side: one agent's connection to a helper, over its socket with
/// its own token. Subagent processes build this after the parent relays the
/// socket path (slice 4); this crate only connects.
#[derive(Clone)]
pub struct NativeClient {
    client: McpClient,
    identity: AgentIdentity,
}

impl std::fmt::Debug for NativeClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeClient")
            .field("identity", &self.identity)
            .finish_non_exhaustive()
    }
}

impl NativeClient {
    /// Connect to the helper at `socket` as `identity`: `Authorization: Bearer`
    /// plus the `_meta` agent identity on every call.
    pub async fn connect(
        socket: impl AsRef<Path>,
        plugin: &str,
        identity: &AgentIdentity,
    ) -> Result<Self, NativeError> {
        #[cfg(not(unix))]
        {
            let _ = (socket, plugin, identity);
            return Err(NativeError::Unsupported(
                "tier C helpers need a Unix socket".to_owned(),
            ));
        }
        #[cfg(unix)]
        {
            let config = McpServerConfig::Unix {
                path: socket.as_ref().to_path_buf(),
                headers: BTreeMap::from([(
                    "Authorization".to_owned(),
                    format!("Bearer {}", identity.token),
                )]),
            };
            let client = McpClient::connect(plugin, config, ClientOptions::default()).await?;
            client.set_call_meta(identity.call_meta());
            Ok(Self {
                client,
                identity: identity.clone(),
            })
        }
    }

    /// This connection's agent identity.
    pub fn identity(&self) -> &AgentIdentity {
        &self.identity
    }

    /// Register a child of this agent with the helper (`hoocode/agentStarted`).
    /// The helper accepts it only when `parentId` is this agent's id.
    pub async fn register_child(&self, child: &AgentIdentity) -> Result<(), NativeError> {
        Ok(self
            .client
            .notify("hoocode/agentStarted", child.started_params())
            .await?)
    }

    /// End one agent (`hoocode/agentEnded`).
    pub async fn end_agent(&self, agent_id: &str) -> Result<(), NativeError> {
        Ok(self
            .client
            .notify(
                "hoocode/agentEnded",
                serde_json::json!({"agentId": agent_id}),
            )
            .await?)
    }

    /// Call `tool` as this agent.
    pub async fn call_tool(&self, tool: &str, args: Value) -> Result<ToolOutput, NativeError> {
        Ok(self
            .client
            .call_tool(tool, args, CallOptions::default())
            .await?)
    }
}

/// Locks a std mutex, ignoring poisoning: the maps hold no invariants a panic
/// could break.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

//! The tier C supervisor (root process only): one helper process per plugin.
//!
//! Started on first use, never at session start (spec D4). `ensure` starts the
//! helper for a plugin and waits until its socket exists and an MCP
//! `initialize` succeeds; `call_tool` calls a tool for the root agent with a
//! [`NativeCallOptions`] cancellation token. At most `max_restarts` restarts
//! in any `restart_window`, then the helper is down until the session ends.
//! After a restart every live registered agent is re-sent. `shutdown` ends
//! the session gracefully and removes the runtime dir.
//!
//! Retry rule: a call is restarted and retried once only when the request
//! provably never reached the helper (not started, child already dead before
//! the send, or a connect failure). When the helper dies mid-call the call
//! fails with an error saying the call may or may not have run, and the next
//! call restarts the helper.
//!
//! [`NativeClient`] is the non-root side: subagent processes connect with
//! their own token. Slice 4 builds the "ask parent EnsureHelper" relay on top;
//! this crate only connects, given the socket.

#[cfg(unix)]
use std::collections::BTreeMap;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use hoocode_agent_mcp::{CallOptions, McpClient, McpError, ToolOutput};
#[cfg(unix)]
use hoocode_agent_mcp::{ClientOptions, McpServerConfig};
use hoocode_code_plugins::NativeHelperEntry;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

#[cfg(unix)]
use crate::agents::AGENT_ID_ENV;
use crate::agents::{mint_agent, AgentIdentity, ROOT_AGENT_ID};
use crate::error::{shut_down, NativeError};
#[cfg(unix)]
use crate::resolve::resolve_executable;
use crate::runtime_dir::create_runtime_dir;
#[cfg(unix)]
use crate::runtime_dir::{socket_path_for, validate_path_component};

/// Passed to the helper in its environment (spec D4).
pub const HELPER_SOCKET_ENV: &str = "HOOCODE_HELPER_SOCKET";
/// The root bearer token, in the helper's environment.
pub const HELPER_TOKEN_ENV: &str = "HOOCODE_HELPER_TOKEN";

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

/// How often the ready wait polls for the socket. Unix sockets are
/// Unix-only: this only runs there.
#[cfg(unix)]
const READY_POLL: Duration = Duration::from_millis(50);
/// A download that takes longer than this fails the call instead of hanging
/// it (spec D4 has no bound; five minutes is generous for a helper binary).
/// Downloads only run on Unix.
#[cfg(unix)]
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// How often the in-flight call checks whether its helper is still alive.
const WATCH_CHILD_POLL: Duration = Duration::from_millis(10);

/// Options for one tier C call through [`Supervisor::call_tool`] or
/// [`NativeClient::call_tool`], mirroring
/// [`CallOptions`](hoocode_agent_mcp::CallOptions).
#[derive(Debug, Clone, Default)]
pub struct NativeCallOptions {
    /// Cancelling it aborts the call: the call returns
    /// [`McpError::Cancelled`](hoocode_agent_mcp::McpError) promptly, the
    /// helper gets an MCP `notifications/cancelled`, and an in-progress
    /// download or start is abandoned. Fresh (never cancelled) by default.
    pub cancel: CancellationToken,
}

impl NativeCallOptions {
    /// Options that are already cancelled (tests): the call fails at once
    /// without starting anything.
    pub fn cancelled() -> Self {
        let cancel = CancellationToken::new();
        cancel.cancel();
        Self { cancel }
    }
}

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
    /// The session runtime dir, created lazily on first `ensure`: a session
    /// that never uses a helper creates nothing. Removed at session end
    /// when created.
    runtime_dir: Mutex<Option<PathBuf>>,
    slots: HashMap<String, HelperSlot>,
    agents: Mutex<HashMap<String, RegisteredAgent>>,
    shutdown: AtomicBool,
    /// Cancelled by `shutdown`: wakes calls waiting out a download or start.
    shutdown_token: CancellationToken,
}

impl Drop for SupervisorInner {
    fn drop(&mut self) {
        // Best effort: shutdown removes the dir properly; the children die
        // with the state (kill_on_drop) and the helper exits on --parent-pid.
        if let Some(dir) = lock(&self.runtime_dir).clone() {
            let _ = std::fs::remove_dir_all(dir);
        }
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
    /// Build the supervisor: mints the root token. The runtime dir is not
    /// created here but lazily on first `ensure`, so a session that never
    /// uses a helper creates nothing.
    /// `home` is the hoocode home dir (`~/.hoocode/plugin-bin` lives under it;
    /// see [`crate::resolve::default_home_dir`]), injectable for tests.
    pub fn new(
        home: impl Into<PathBuf>,
        helpers: Vec<NativeHelperEntry>,
        overrides: HashMap<String, PathBuf>,
        config: SupervisorConfig,
    ) -> Result<Self, NativeError> {
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
                runtime_dir: Mutex::new(None),
                slots,
                agents: Mutex::new(HashMap::new()),
                shutdown: AtomicBool::new(false),
                shutdown_token: CancellationToken::new(),
            }),
        })
    }

    /// The session runtime dir holding the sockets and logs, once the first
    /// `ensure` created it. `None` before first use.
    pub fn runtime_dir(&self) -> Option<PathBuf> {
        lock(&self.inner.runtime_dir).clone()
    }

    /// The runtime dir, creating it on first use: the test override when
    /// set, else a fresh random dir. Never eager: `new` creates nothing.
    fn runtime_dir_or_create(&self) -> Result<PathBuf, NativeError> {
        if let Some(dir) = lock(&self.inner.runtime_dir).clone() {
            return Ok(dir);
        }
        let dir = match crate::runtime_dir::internal_runtime_dir() {
            Some(dir) => dir,
            None => create_runtime_dir()?,
        };
        *lock(&self.inner.runtime_dir) = Some(dir.clone());
        Ok(dir)
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

    /// Whether `plugin` is a shared helper (spec D6): subagents get its
    /// tools. Unknown plugins and `shared: false` helpers are not shared:
    /// asking for their socket is refused before anything starts.
    pub fn is_shared(&self, plugin: &str) -> bool {
        self.inner
            .entries
            .get(plugin)
            .is_some_and(|entry| entry.helper.spec.shared)
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
        self.ensure_cancel(plugin, &CancellationToken::new()).await
    }

    /// Call `tool` for the root agent, starting the helper first. Cancelling
    /// `options.cancel` aborts the call promptly (the helper gets an MCP
    /// `notifications/cancelled`).
    ///
    /// Retry rule: the call is restarted and retried once only when the
    /// request provably never reached the helper (not started, child already
    /// dead before the send, or a connect failure). When the helper dies
    /// mid-call the call fails with an error saying the call may or may not
    /// have run, and the next call restarts the helper.
    pub async fn call_tool(
        &self,
        plugin: &str,
        tool: &str,
        args: Value,
        options: NativeCallOptions,
    ) -> Result<ToolOutput, NativeError> {
        self.ensure_cancel(plugin, &options.cancel).await?;
        let generation = self.generation(plugin);
        match self
            .do_call_bounded(plugin, tool, &args, &options.cancel)
            .await
        {
            Ok(output) => Ok(output),
            Err(CallFailure::NeverSent(_)) => {
                // The request never reached the helper: restarting and
                // retrying once cannot run it twice.
                self.restart_for_retry(plugin, generation, &options.cancel)
                    .await?;
                self.do_call_bounded(plugin, tool, &args, &options.cancel)
                    .await
                    .map_err(CallFailure::into_error)
            }
            Err(CallFailure::Final(error)) => Err(error),
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
    /// and remove the runtime dir when one was created. Idempotent.
    /// Proceeds even while a download or start is in flight: those hold no
    /// slot lock, and fail fast on the shutdown instead of installing
    /// anything.
    pub async fn shutdown(&self) -> Result<(), NativeError> {
        if self.inner.shutdown.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        self.inner.shutdown_token.cancel();
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
            let send = client.notify("hoocode/sessionEnding", serde_json::json!({}));
            let _ = tokio::time::timeout(SHUTDOWN_NOTIFY_TIMEOUT, send).await;
            let done = tokio::time::timeout(self.inner.config.shutdown_timeout, child.wait())
                .await
                .is_ok();
            if !done {
                let _ = child.kill().await;
            }
            let _ = child.wait().await;
        }
        if let Some(dir) = lock(&self.inner.runtime_dir).clone() {
            match std::fs::remove_dir_all(&dir) {
                Ok(()) => {}
                // Already gone (or never fully created): not an error.
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(NativeError::Io(format!(
                        "cannot remove runtime dir {}: {error}",
                        dir.display()
                    )));
                }
            }
        }
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
        args: &Value,
        cancel: &CancellationToken,
    ) -> Result<ToolOutput, CallFailure> {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(CallFailure::Final(cancelled())),
            _ = self.inner.shutdown_token.cancelled() => {
                Err(CallFailure::Final(shut_down(plugin)))
            }
            _ = self.wait_child_gone(plugin) => Err(CallFailure::Final(interrupted(plugin))),
            output = self.do_call(plugin, tool, args.clone(), cancel) => output,
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
            tokio::time::sleep(WATCH_CHILD_POLL).await;
        }
    }

    async fn do_call(
        &self,
        plugin: &str,
        tool: &str,
        args: Value,
        cancel: &CancellationToken,
    ) -> Result<ToolOutput, CallFailure> {
        let client = {
            let Some(slot) = self.inner.slots.get(plugin) else {
                return Err(CallFailure::Final(NativeError::UnknownPlugin(
                    plugin.to_owned(),
                )));
            };
            let mut state = slot.state.lock().await;
            match &mut *state {
                // A dead child means the connection is dead too, before
                // anything was sent: report it as not running so the caller
                // restarts immediately instead of sending a call into the
                // void and waiting out the call timeout. `try_wait` also
                // reaps the zombie here.
                HelperState::Live { client, child, .. } => match child.try_wait() {
                    Ok(None) => client.clone(),
                    _ => return Err(CallFailure::NeverSent(not_running(plugin))),
                },
                HelperState::Down { reason } => {
                    return Err(CallFailure::Final(NativeError::Down {
                        plugin: plugin.to_owned(),
                        reason: reason.clone(),
                    }));
                }
                HelperState::Idle => return Err(CallFailure::NeverSent(not_running(plugin))),
            }
        };
        let options = CallOptions {
            cancel: cancel.clone(),
            progress: None,
        };
        match client.call_tool(tool, args, options).await {
            Ok(output) => Ok(output),
            // A connect failure means the request never reached the helper.
            Err(McpError::Connect(_)) => Err(CallFailure::NeverSent(not_running(plugin))),
            Err(McpError::Cancelled) => Err(CallFailure::Final(cancelled())),
            Err(error @ McpError::Disconnected(_)) => {
                // Sent, then the transport died: only a dead child proves a
                // mid-call death. Under a live helper the connection dropped
                // on its own, which the error says as is.
                if !self.child_alive(plugin).await {
                    Err(CallFailure::Final(interrupted(plugin)))
                } else {
                    Err(CallFailure::Final(error.into()))
                }
            }
            Err(error) => Err(CallFailure::Final(error.into())),
        }
    }

    /// Send a notification over the root connection, restarting once when the
    /// connection broke, the child is gone, or the helper never started.
    /// Replaying a notification is safe: the helper dedupes by agent id.
    async fn notify(&self, plugin: &str, method: &str, params: Value) -> Result<(), NativeError> {
        self.ensure(plugin).await?;
        self.notify_cancel(plugin, method, params, &CancellationToken::new())
            .await
    }

    async fn notify_cancel(
        &self,
        plugin: &str,
        method: &str,
        params: Value,
        cancel: &CancellationToken,
    ) -> Result<(), NativeError> {
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
        self.restart_for_retry(plugin, generation, cancel).await?;
        match self.try_notify(plugin, method, params).await {
            Some(result) => result,
            None => Err(not_running(plugin)),
        }
    }

    /// `None` when no live client exists right now (idle, down, or its
    /// child already exited). The send itself is bounded: a wedged
    /// connection must not hang the caller on the 60 s call timeout.
    async fn try_notify(
        &self,
        plugin: &str,
        method: &str,
        params: Value,
    ) -> Option<Result<(), NativeError>> {
        let client = self.live_client(plugin).await?;
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

    /// The live connection, when the helper is really up: waits for the
    /// state lock instead of reporting none while a start is in flight, so
    /// a merely busy supervisor never looks like a dead helper (and a
    /// healthy helper is never killed for it).
    async fn live_client(&self, plugin: &str) -> Option<McpClient> {
        let slot = self.inner.slots.get(plugin)?;
        let mut state = slot.state.lock().await;
        match &mut *state {
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
    /// restarted already while this call waited. Only restarts when the
    /// helper is really down: with a live child and its socket present
    /// there is nothing to fix, and a healthy helper is never killed.
    /// Never resurrects a down helper or a shut-down session. Unix-only:
    /// anywhere else starting is [`NativeError::Unsupported`].
    async fn restart_for_retry(
        &self,
        plugin: &str,
        generation: u64,
        cancel: &CancellationToken,
    ) -> Result<(), NativeError> {
        #[cfg(not(unix))]
        {
            let _ = (plugin, generation, cancel);
            return Err(NativeError::Unsupported(
                "tier C helpers run on Unix sockets; this platform cannot start them".to_owned(),
            ));
        }
        #[cfg(unix)]
        {
            let Some(slot) = self.inner.slots.get(plugin) else {
                return Err(NativeError::UnknownPlugin(plugin.to_owned()));
            };
            let _guard = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(cancelled()),
                guard = slot.start.lock() => guard,
            };
            if slot.generation.load(Ordering::SeqCst) != generation {
                return Ok(());
            }
            if self.inner.shutdown.load(Ordering::SeqCst) {
                return Err(shut_down(plugin));
            }
            {
                let mut state = slot.state.lock().await;
                match &mut *state {
                    HelperState::Live { child, socket, .. } => match child.try_wait() {
                        // Someone else already fixed it: leave it alone.
                        Ok(None) if socket.exists() => return Ok(()),
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
            }
            self.start_locked(plugin, slot, cancel).await.map(|_| ())
        }
    }

    /// `ensure` with a cancellation token: a cancelled wait fails at once
    /// instead of riding out a download or start.
    async fn ensure_cancel(
        &self,
        plugin: &str,
        cancel: &CancellationToken,
    ) -> Result<PathBuf, NativeError> {
        #[cfg(not(unix))]
        {
            let _ = (plugin, cancel);
            return Err(NativeError::Unsupported(
                "tier C helpers run on Unix sockets; this platform is not supported".to_owned(),
            ));
        }
        #[cfg(unix)]
        {
            self.ensure_unix(plugin, cancel).await
        }
    }

    #[cfg(unix)]
    async fn ensure_unix(
        &self,
        plugin: &str,
        cancel: &CancellationToken,
    ) -> Result<PathBuf, NativeError> {
        if self.inner.shutdown.load(Ordering::SeqCst) {
            return Err(shut_down(plugin));
        }
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        let Some(slot) = self.inner.slots.get(plugin) else {
            return Err(NativeError::UnknownPlugin(plugin.to_owned()));
        };
        let _guard = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(cancelled()),
            guard = slot.start.lock() => guard,
        };
        {
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
        }
        self.start_locked(plugin, slot, cancel).await
    }

    /// Spawn (or respawn) the helper and wait for it to be ready. The caller
    /// holds the slot's start lock. On failure the slot goes back to `Idle`;
    /// on success every registered agent is re-sent.
    ///
    /// Holds no slot lock across the download or the ready wait, so
    /// `shutdown` proceeds while a start is in flight (the start then fails
    /// fast instead of installing a helper into a removed dir).
    #[cfg(unix)]
    async fn start_locked(
        &self,
        plugin: &str,
        slot: &HelperSlot,
        cancel: &CancellationToken,
    ) -> Result<PathBuf, NativeError> {
        if self.inner.shutdown.load(Ordering::SeqCst) {
            return Err(shut_down(plugin));
        }
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        // The plugin name lands in socket, log and plugin-bin paths: fail
        // fast when it is not a single safe component.
        validate_path_component("plugin", plugin)?;
        let entry = self
            .inner
            .entries
            .get(plugin)
            .cloned()
            .ok_or_else(|| NativeError::UnknownPlugin(plugin.to_owned()))?;
        // Retire whatever is there and count the spawn, under a brief lock:
        // a dead child, or a live one whose connection broke. A cancelled
        // or shut-down wait never reaches the spawn count.
        {
            let mut state = slot.state.lock().await;
            if let HelperState::Down { reason } = &*state {
                return Err(NativeError::Down {
                    plugin: plugin.to_owned(),
                    reason: reason.clone(),
                });
            }
            let retired = match std::mem::replace(&mut *state, HelperState::Idle) {
                HelperState::Live { child, .. } => Some(child),
                previous => {
                    *state = previous;
                    None
                }
            };
            drop(state);
            if let Some(mut child) = retired {
                let _ = child.kill().await;
                let _ = child.wait().await;
            }
            // The restart budget counts every spawn after the first,
            // including ones that then fail: a bad binary must not respawn
            // forever. The guard below never crosses an await.
            let previous = slot.spawn_count.fetch_add(1, Ordering::SeqCst);
            let mut over_budget = false;
            if previous > 0 {
                {
                    let mut restarts = lock(&slot.restarts);
                    restarts.retain(|when| when.elapsed() < self.inner.config.restart_window);
                    if restarts.len() >= self.inner.config.max_restarts as usize {
                        over_budget = true;
                    } else {
                        restarts.push_back(Instant::now());
                    }
                }
                if over_budget {
                    let reason = format!(
                        "restarted {} times in the last {}s",
                        self.inner.config.max_restarts,
                        self.inner.config.restart_window.as_secs()
                    );
                    *slot.state.lock().await = HelperState::Down {
                        reason: reason.clone(),
                    };
                    return Err(NativeError::Down {
                        plugin: plugin.to_owned(),
                        reason,
                    });
                }
            }
        }
        // Resolve the executable: binaryPath > downloaded binary > command.
        // No slot lock is held, so a slow download never blocks shutdown.
        // A cancelled, shut-down or over-long download fails the call
        // instead of hanging it.
        let dir = self.runtime_dir_or_create()?;
        let client = hoocode_ai_util::tls::http_client_builder()
            .build()
            .map_err(|error| {
                NativeError::Download(format!("cannot build the HTTP client: {error}"))
            })?;
        let executable = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(cancelled()),
            _ = self.inner.shutdown_token.cancelled() => return Err(shut_down(plugin)),
            resolved = tokio::time::timeout(
                DOWNLOAD_TIMEOUT,
                resolve_executable(
                    &self.inner.home,
                    &entry,
                    &self.inner.overrides,
                    &client,
                ),
            ) => match resolved {
                Ok(result) => result?,
                Err(_) => {
                    return Err(NativeError::Download(format!(
                        "{plugin}: download took longer than {:?}",
                        DOWNLOAD_TIMEOUT
                    )));
                }
            },
        };
        if self.inner.shutdown.load(Ordering::SeqCst) {
            return Err(shut_down(plugin));
        }
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        let socket = socket_path_for(&dir, plugin)?;
        // The log name embeds the plugin too: the socket check above
        // already validated it as a safe path component.
        let log = dir.join(format!("{plugin}.log"));
        let _ = std::fs::remove_file(&socket);
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
        let ready = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                return Err(cancelled());
            }
            _ = self.inner.shutdown_token.cancelled() => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                return Err(shut_down(plugin));
            }
            ready = wait_ready(
                plugin,
                &socket,
                &self.inner.root_identity,
                self.inner.config.ready_timeout,
            ) => ready,
        };
        match ready {
            Ok(ready_client) => {
                // After a restart every live registered agent is re-sent,
                // each send bounded: a wedged connection must not hang the
                // start on the 60 s call timeout.
                // The lock is scoped: the guard must not cross the awaits.
                let reregister: Vec<AgentIdentity> = {
                    lock(&self.inner.agents)
                        .values()
                        .filter(|record| record.plugins.contains(plugin))
                        .map(|record| record.identity.clone())
                        .collect()
                };
                for agent in reregister {
                    let send = ready_client.notify("hoocode/agentStarted", agent.started_params());
                    match tokio::time::timeout(LIFECYCLE_NOTIFY_TIMEOUT, send).await {
                        Ok(Ok(())) => {}
                        Ok(Err(error)) => {
                            let _ = child.kill().await;
                            let _ = child.wait().await;
                            return Err(error.into());
                        }
                        Err(_) => {
                            let _ = child.kill().await;
                            let _ = child.wait().await;
                            return Err(NativeError::Timeout {
                                plugin: plugin.to_owned(),
                                after: LIFECYCLE_NOTIFY_TIMEOUT,
                            });
                        }
                    }
                }
                let mut state = slot.state.lock().await;
                if self.inner.shutdown.load(Ordering::SeqCst) {
                    drop(state);
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                    return Err(shut_down(plugin));
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

/// What one call attempt produced: either the request provably never
/// reached the helper (safe to restart and retry once) or it failed after
/// that point (surfaces as is, never replayed).
enum CallFailure {
    /// Not started, child already dead before the send, or a connect
    /// failure: restarting and retrying once cannot run the call twice.
    NeverSent(NativeError),
    /// Anything else, including a mid-call death: the call may or may not
    /// have run.
    Final(NativeError),
}

impl CallFailure {
    fn into_error(self) -> NativeError {
        match self {
            Self::NeverSent(error) | Self::Final(error) => error,
        }
    }
}

/// The helper died after the request went out (or while it was going out):
/// the call may or may not have run. Never retried, never replayed.
fn interrupted(plugin: &str) -> NativeError {
    NativeError::Mcp(McpError::Disconnected(format!(
        "the {plugin} helper stopped during the call and the call may or may not have run"
    )))
}

/// The call's cancellation token fired: prompt, with the helper already
/// told to stop the work via an MCP `notifications/cancelled`.
fn cancelled() -> NativeError {
    NativeError::Mcp(McpError::Cancelled)
}

/// A transport-level failure: the helper is gone or unreachable.
/// Fire-and-forget lifecycle notifications (`agentStarted`/`agentEnded`)
/// restart on these; tool calls use [`CallFailure`] instead.
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

    /// Call `tool` as this agent. Cancelling `options.cancel` aborts the
    /// call promptly (the helper gets an MCP `notifications/cancelled`).
    pub async fn call_tool(
        &self,
        tool: &str,
        args: Value,
        options: NativeCallOptions,
    ) -> Result<ToolOutput, NativeError> {
        let call = CallOptions {
            cancel: options.cancel,
            progress: None,
        };
        Ok(self.client.call_tool(tool, args, call).await?)
    }
}

/// Locks a std mutex, ignoring poisoning: the maps hold no invariants a panic
/// could break.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

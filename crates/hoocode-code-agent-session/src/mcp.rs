//! MCP servers for one session (`docs/design/mcp.md`).
//!
//! The hub reads the `mcp.json` sources and the trust store (`hoocode-code-mcp`), starts the
//! trusted servers in the background on the `hoocode-io` runtime, and turns each connected
//! server's tools into [`ToolDefinition`]s named `mcp_<server>_<tool>`. Startup never waits for
//! a server: a server that fails becomes `Failed` and its tools are simply absent.
//!
//! The hub never prompts. The interactive mode asks about [`McpHub::pending_prompts`] and calls
//! [`McpHub::grant`]; print and rpc skip untrusted servers (fail closed).
//!
//! OAuth: a Streamable HTTP server that needs a login ends up `AuthNeeded`. The interactive mode
//! starts the login with [`McpHub::login`]; the hub opens the browser, waits for the redirect and
//! reconnects the server, so its tools appear at the next turn. Nothing here logs in by itself.
//! Elicitation: a server's questions go to the [`ElicitationHandler`] the mode installs
//! ([`McpHub::set_elicitation`]); without one they are declined.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use hoocode_agent_mcp::{
    begin_login, oauth, tool_name, CallOptions, ClientOptions, DeclineAll, McpClient, McpError,
    McpServerConfig, McpTool, ToolContent, ToolOutput, ToolProgress,
};
use hoocode_agent_types::{AgentToolResult, AgentToolUpdateCallback};
use hoocode_ai_types::{AbortSignal, Content, ImageContent};
use hoocode_code_mcp::{
    discover, load, ConfigSources, McpConfig, ServerState, Source, Transport, TrustStatus,
    TrustStore,
};
use hoocode_code_tool_api::{ToolDefinition, ToolError};
use hoocode_runtime::io_handle;
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;

pub use hoocode_agent_mcp::{ElicitationAnswer, ElicitationHandler, ElicitationRequest};
pub use hoocode_code_mcp::TrustPrompt;

/// How long [`McpHub::shutdown`] waits for the servers to close.
const SHUTDOWN_WAIT: Duration = Duration::from_secs(5);
/// How long a login waits for the browser to reach the redirect URI.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
/// How long print and rpc wait, before their first prompt, for the trusted servers to finish
/// connecting. A server still connecting after this is skipped for that prompt.
pub const STARTUP_WAIT: Duration = Duration::from_secs(10);

/// Receives a line of text for the user (a login link, a notice). Called off the UI thread.
pub type Notifier = Arc<dyn Fn(String) + Send + Sync>;

/// A server's state as `/mcp` shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum McpStatus {
    /// Trusted and starting.
    Connecting,
    Connected,
    /// The folder or plugin that declares it has no grant yet, or its grant is stale.
    NotTrusted,
    /// The server asked for a login this client cannot give.
    AuthNeeded,
    Failed(String),
    /// `"disabled": true` in its entry.
    Disabled,
}

/// One effective server, as `/mcp` lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpServerInfo {
    pub name: String,
    /// `user`, `project` or `plugin <id>`.
    pub source: String,
    pub status: McpStatus,
    pub tool_count: usize,
}

struct Entry {
    name: String,
    source: Source,
    /// The transport with `${VAR}` expanded. `None` when expansion failed or the entry is
    /// disabled: such an entry never starts, so no unexpanded text is ever sent.
    config: Option<McpServerConfig>,
    /// The values `${VAR}` expanded to. They are secrets: a failure reason has them removed.
    secrets: Vec<String>,
    status: McpStatus,
    client: Option<McpClient>,
    tools: Vec<ToolDefinition>,
}

#[derive(Default)]
struct HubState {
    config: McpConfig,
    entries: Vec<Entry>,
    prompts: Vec<TrustPrompt>,
    /// Bumped whenever the tool list changes.
    generation: u64,
    started: bool,
    shut_down: bool,
    /// Answers elicitation requests; `None` declines them all.
    elicitation: Option<Arc<dyn ElicitationHandler>>,
    /// Told when a server needs a login.
    notifier: Option<Notifier>,
    /// Servers with a login in progress.
    logins: Vec<String>,
}

/// The MCP servers of one session. Clones share the same servers.
#[derive(Clone)]
pub struct McpHub {
    trust_path: PathBuf,
    /// The hoocode data directory (the trust file's folder), where OAuth tokens are kept too.
    data_dir: PathBuf,
    state: Arc<Mutex<HubState>>,
    /// Signalled whenever a server finishes connecting or failing. Paired with `state`.
    changed: Arc<Condvar>,
}

impl McpHub {
    /// The hub for a folder: the user's `mcp.json`, the folder's `.agents/mcp.json`, and the
    /// trust store in the agent directory. No plugin sources yet.
    pub fn for_folder(folder: &Path) -> Self {
        let sources = ConfigSources::new(folder.to_path_buf(), Vec::new());
        Self::load(&sources, TrustStore::default_path())
    }

    /// Read the sources and the trust store, and mark every effective server. Nothing starts
    /// yet; call [`start`](Self::start). An unreadable trust file trusts nothing.
    pub fn load(sources: &ConfigSources, trust_path: impl Into<PathBuf>) -> Self {
        let trust_path = trust_path.into();
        let config = load(sources);
        let trust =
            TrustStore::load(&trust_path).unwrap_or_else(|_| TrustStore::empty(&trust_path));
        let discovery = discover(&config, &trust);
        let lookup = |name: &str| std::env::var(name).ok();
        let entries = discovery
            .servers
            .iter()
            .map(|server| {
                let status = match &server.state {
                    Some(ServerState::Disabled) => McpStatus::Disabled,
                    Some(ServerState::NotTrusted) => McpStatus::NotTrusted,
                    // discover() sets no other state; the client does.
                    Some(_) | None => McpStatus::Connecting,
                };
                // Expansion reads the environment now, after the trust check: the trust
                // fingerprint saw the text as written. A failure names the variable, not its
                // value, and the entry stays out of the start queue.
                let (config, secrets, status) =
                    match (server.transport.expanded_values(&lookup), status) {
                        (Ok((transport, secrets)), status) => {
                            (Some(to_client_config(&transport)), secrets, status)
                        }
                        (Err(_), McpStatus::Disabled) => (None, Vec::new(), McpStatus::Disabled),
                        (Err(error), _) => (
                            None,
                            Vec::new(),
                            McpStatus::Failed(format!("mcp.json: {error}")),
                        ),
                    };
                Entry {
                    name: server.name.clone(),
                    source: server.source.clone(),
                    config,
                    secrets,
                    status,
                    client: None,
                    tools: Vec::new(),
                }
            })
            .collect();
        let state = HubState {
            config,
            entries,
            prompts: discovery.prompts,
            ..Default::default()
        };
        let data_dir = trust_path
            .parent()
            .map_or_else(PathBuf::new, Path::to_path_buf);
        Self {
            trust_path,
            data_dir,
            state: Arc::new(Mutex::new(state)),
            changed: Arc::new(Condvar::new()),
        }
    }

    /// Answers the servers' elicitation requests with `handler`. Set it before
    /// [`start`](Self::start): a server that connects earlier keeps the handler it had.
    pub fn set_elicitation(&self, handler: Arc<dyn ElicitationHandler>) {
        lock(&self.state).elicitation = Some(handler);
    }

    /// Tells the user, through `notify`, when a server needs a login.
    pub fn set_notifier(&self, notify: impl Fn(String) + Send + Sync + 'static) {
        lock(&self.state).notifier = Some(Arc::new(notify));
    }

    /// Starts the login for the `AuthNeeded` HTTP server `name` and returns at once. In the
    /// background: the browser opens, `on_event` gets the login link (so it can be shown when the
    /// browser does not open), the redirect is awaited, and the server reconnects. `on_event` gets
    /// a final line with the outcome.
    ///
    /// Errors when the server is unknown, does not need a login, or already has one running.
    pub fn login(
        &self,
        name: &str,
        on_event: impl Fn(String) + Send + Sync + 'static,
    ) -> Result<(), String> {
        let url = {
            let mut state = lock(&self.state);
            if state.shut_down {
                return Err("MCP servers are shut down".to_owned());
            }
            let entry = state
                .entries
                .iter()
                .find(|e| e.name == name)
                .ok_or_else(|| format!("No MCP server named {name}."))?;
            if entry.status != McpStatus::AuthNeeded {
                return Err(format!("MCP server {name} does not need a login."));
            }
            let Some(McpServerConfig::Http { url, .. }) = &entry.config else {
                return Err(format!(
                    "MCP server {name} is not a Streamable HTTP server."
                ));
            };
            let url = url.clone();
            if state.logins.iter().any(|n| n == name) {
                return Err(format!("A login for {name} is already running."));
            }
            state.logins.push(name.to_owned());
            url
        };
        let hub = self.clone();
        let name = name.to_owned();
        let on_event: Notifier = Arc::new(on_event);
        hoocode_runtime::io_handle().spawn(async move {
            let outcome = hub.run_login(&name, &url, &on_event).await;
            lock(&hub.state).logins.retain(|n| n != &name);
            match outcome {
                Ok(()) => {
                    on_event(format!(
                        "Logged in to {name}. It connects in the background."
                    ));
                    hub.reconnect(&name);
                }
                Err(error) => on_event(format!("Login to {name} failed: {error}")),
            }
        });
        Ok(())
    }

    /// The login itself: begin, show the link, open the browser, wait for the redirect.
    async fn run_login(&self, name: &str, url: &str, on_event: &Notifier) -> Result<(), String> {
        let (authorize_url, handle) = begin_login(url, &self.data_dir)
            .await
            .map_err(|e| e.to_string())?;
        on_event(format!(
            "Log in to {name} in your browser. If it did not open, visit:\n{authorize_url}"
        ));
        oauth::open_browser(&authorize_url);
        handle
            .finish(LOGIN_TIMEOUT)
            .await
            .map_err(|e| e.to_string())
    }

    /// Connects an `AuthNeeded` server again after its login.
    fn reconnect(&self, name: &str) {
        let jobs = {
            let mut state = lock(&self.state);
            if !state.started || state.shut_down {
                return;
            }
            let Some(index) = state
                .entries
                .iter()
                .position(|e| e.name == name && e.status == McpStatus::AuthNeeded)
            else {
                return;
            };
            let entry = &mut state.entries[index];
            let Some(config) = entry.config.clone() else {
                return;
            };
            entry.status = McpStatus::Connecting;
            vec![(index, entry.name.clone(), config)]
        };
        self.spawn_jobs(jobs);
    }

    /// Start every trusted server in the background. Returns at once.
    pub fn start(&self) {
        let jobs = {
            let mut state = lock(&self.state);
            if state.started || state.shut_down {
                return;
            }
            state.started = true;
            connecting_jobs(&mut state)
        };
        self.spawn_jobs(jobs);
    }

    /// Source groups that still need a trust decision, in precedence order.
    pub fn pending_prompts(&self) -> Vec<TrustPrompt> {
        lock(&self.state).prompts.clone()
    }

    /// Record a grant for the given trust keys (folder paths or `plugin:<id>`), save the trust
    /// store, and start the servers those sources declare. Returns how many servers started.
    pub fn grant(&self, keys: &[String]) -> Result<usize, String> {
        let grants: Vec<(String, Vec<hoocode_code_mcp::ServerDef>)> = {
            let state = lock(&self.state);
            state
                .prompts
                .iter()
                .filter(|p| keys.contains(&p.key))
                .map(|p| (p.key.clone(), state.config.declared(&p.source).to_vec()))
                .collect()
        };
        if grants.is_empty() {
            return Ok(0);
        }
        TrustStore::update(&self.trust_path, |store| {
            for (key, declared) in &grants {
                store.grant(key, declared);
            }
        })
        .map_err(|e| e.to_string())?;

        let jobs = {
            let mut state = lock(&self.state);
            let granted: Vec<String> = grants.iter().map(|(k, _)| k.clone()).collect();
            state.prompts.retain(|p| !granted.contains(&p.key));
            let mut indices = Vec::new();
            for (index, entry) in state.entries.iter_mut().enumerate() {
                let key = entry.source.trust_key();
                let granted_here = key.is_some_and(|k| granted.contains(&k));
                if granted_here && entry.status == McpStatus::NotTrusted {
                    entry.status = McpStatus::Connecting;
                    indices.push(index);
                }
            }
            if !state.started || state.shut_down {
                Vec::new()
            } else {
                indices
                    .into_iter()
                    .filter_map(|i| start_job(&state.entries[i], i))
                    .collect()
            }
        };
        let started = jobs.len();
        self.spawn_jobs(jobs);
        Ok(started)
    }

    /// The tool list generation and the tools of every connected server.
    pub fn tool_definitions(&self) -> (u64, Vec<ToolDefinition>) {
        let state = lock(&self.state);
        let tools = state
            .entries
            .iter()
            .flat_map(|e| e.tools.iter().cloned())
            .collect();
        (state.generation, tools)
    }

    /// Every effective server with its state, in precedence order.
    pub fn servers(&self) -> Vec<McpServerInfo> {
        lock(&self.state).entries.iter().map(server_info).collect()
    }

    /// Stops every connected server. Waits at most a few seconds. Safe to call twice.
    pub fn shutdown(&self) {
        let clients: Vec<McpClient> = {
            let mut state = lock(&self.state);
            state.shut_down = true;
            state
                .entries
                .iter_mut()
                .filter_map(|e| e.client.take())
                .collect()
        };
        if clients.is_empty() {
            return;
        }
        let (done, wait) = std::sync::mpsc::channel();
        io_handle().spawn(async move {
            let mut set = tokio::task::JoinSet::new();
            for client in clients {
                set.spawn(async move { client.shutdown().await });
            }
            while set.join_next().await.is_some() {}
            let _ = done.send(());
        });
        let _ = wait.recv_timeout(SHUTDOWN_WAIT);
    }

    /// Blocks until no trusted server is still connecting, or `timeout` passes, and returns the
    /// servers still connecting (empty when all finished). Print and rpc call it once, before
    /// their first prompt, from the entry point, never from a runtime worker; the interactive
    /// mode does not wait. A server that connects later is picked up at the next turn.
    pub fn wait_for_startup(&self, timeout: Duration) -> Vec<McpServerInfo> {
        let deadline = Instant::now() + timeout;
        let mut state = lock(&self.state);
        loop {
            let pending: Vec<McpServerInfo> = if state.started {
                state
                    .entries
                    .iter()
                    .filter(|e| e.status == McpStatus::Connecting)
                    .map(server_info)
                    .collect()
            } else {
                Vec::new()
            };
            let now = Instant::now();
            if pending.is_empty() || state.shut_down || now >= deadline {
                return pending;
            }
            state = self
                .changed
                .wait_timeout(state, deadline - now)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    fn spawn_jobs(&self, jobs: Vec<(usize, String, McpServerConfig)>) {
        let (elicitation, notifier) = {
            let state = lock(&self.state);
            (state.elicitation.clone(), state.notifier.clone())
        };
        let changed = self.changed.clone();
        let elicitation: Arc<dyn ElicitationHandler> =
            elicitation.unwrap_or_else(|| Arc::new(DeclineAll));
        for (index, name, config) in jobs {
            let state = self.state.clone();
            let data_dir = self.data_dir.clone();
            let elicitation = elicitation.clone();
            let notifier = notifier.clone();
            let changed = changed.clone();
            io_handle().spawn(async move {
                let outcome = connect_and_list(&name, config, &data_dir, elicitation).await;
                if lock(&state).shut_down {
                    // The session ended while this server was starting.
                    if let Ok((client, _)) = outcome {
                        client.shutdown().await;
                    }
                    return;
                }
                let mut guard = lock(&state);
                match outcome {
                    Ok((client, tools)) => {
                        let definitions = definitions(&client, &name, tools);
                        let entry = &mut guard.entries[index];
                        entry.status = McpStatus::Connected;
                        entry.tools = definitions;
                        entry.client = Some(client);
                        guard.generation += 1;
                        changed.notify_all();
                    }
                    Err(error) => {
                        let secrets = guard.entries[index].secrets.clone();
                        let status = redact_status(status_for(&error), &secrets);
                        let needs_login = status == McpStatus::AuthNeeded;
                        guard.entries[index].status = status;
                        changed.notify_all();
                        drop(guard);
                        if let (true, Some(notify)) = (needs_login, notifier) {
                            notify(format!(
                                "MCP server {name} needs a login. In interactive mode, run /mcp login {name}. Its tools are unavailable."
                            ));
                        }
                    }
                }
            });
        }
    }
}

/// Entries that are trusted and not started yet, as spawn jobs. Marks them `Connecting`.
fn connecting_jobs(state: &mut HubState) -> Vec<(usize, String, McpServerConfig)> {
    state
        .entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.status == McpStatus::Connecting && e.client.is_none())
        .filter_map(|(i, e)| start_job(e, i))
        .collect()
}

/// The spawn job for one entry, if it has a usable config (an entry whose expansion failed
/// has none and never starts).
fn start_job(entry: &Entry, index: usize) -> Option<(usize, String, McpServerConfig)> {
    entry
        .config
        .clone()
        .map(|config| (index, entry.name.clone(), config))
}

async fn connect_and_list(
    name: &str,
    config: McpServerConfig,
    data_dir: &Path,
    elicitation: Arc<dyn ElicitationHandler>,
) -> Result<(McpClient, Vec<McpTool>), McpError> {
    let options = ClientOptions::default();
    // A Streamable HTTP server is reached with its stored OAuth token, when it has one.
    let client = match config {
        McpServerConfig::Http { url, headers } => {
            McpClient::connect_oauth_with_elicitation(
                name,
                url,
                headers,
                data_dir,
                options,
                elicitation,
            )
            .await?
        }
        stdio @ McpServerConfig::Stdio { .. } => {
            McpClient::connect_with_elicitation(name, stdio, options, elicitation).await?
        }
    };
    match client.list_tools().await {
        Ok(tools) => Ok((client, tools)),
        Err(error) => {
            client.shutdown().await;
            Err(error)
        }
    }
}

fn server_info(entry: &Entry) -> McpServerInfo {
    McpServerInfo {
        name: entry.name.clone(),
        source: entry.source.label(),
        status: entry.status.clone(),
        tool_count: entry.tools.len(),
    }
}

/// Removes the expanded `${VAR}` values from a failure reason. A reason can echo the command or
/// the URL it failed on, and those may carry a token.
fn redact_status(status: McpStatus, secrets: &[String]) -> McpStatus {
    match status {
        McpStatus::Failed(reason) => {
            McpStatus::Failed(secrets.iter().fold(reason, |text, secret| {
                text.replace(secret.as_str(), "<redacted>")
            }))
        }
        other => other,
    }
}

/// The state of a server whose connection failed: `AuthNeeded` when it wants a login (a 401 with
/// no usable token, or a token that could not be refreshed), else `Failed` with the reason.
pub(crate) fn status_for(error: &McpError) -> McpStatus {
    match error {
        McpError::AuthRequired { .. } | McpError::Auth(_) => McpStatus::AuthNeeded,
        other => McpStatus::Failed(other.to_string()),
    }
}

/// Elicitation in print and rpc modes: nobody can answer, so each request is declined and the
/// decline is reported through `report`.
pub struct DeclineElicitation {
    report: Notifier,
}

impl DeclineElicitation {
    pub fn new(report: impl Fn(String) + Send + Sync + 'static) -> Self {
        Self {
            report: Arc::new(report),
        }
    }
}

impl ElicitationHandler for DeclineElicitation {
    fn elicit<'a>(
        &'a self,
        server: &'a str,
        _request: ElicitationRequest,
    ) -> Pin<Box<dyn Future<Output = ElicitationAnswer> + Send + 'a>> {
        Box::pin(async move {
            (self.report)(format!(
                "MCP server {server} asked for input; declined, because this mode cannot ask. Use interactive mode."
            ));
            ElicitationAnswer::Decline
        })
    }
}

fn to_client_config(transport: &Transport) -> McpServerConfig {
    match transport {
        Transport::Stdio {
            command,
            args,
            env,
            cwd,
        } => McpServerConfig::Stdio {
            command: command.clone(),
            args: args.clone(),
            env: env.clone(),
            cwd: cwd.as_deref().map(PathBuf::from),
        },
        Transport::Http { url, headers } => McpServerConfig::Http {
            url: url.clone(),
            headers: headers.clone(),
        },
    }
}

/// One [`ToolDefinition`] per listed tool. Calls go to the client on the io runtime.
fn definitions(client: &McpClient, server: &str, tools: Vec<McpTool>) -> Vec<ToolDefinition> {
    tools
        .into_iter()
        .map(|tool| {
            let name = tool_name(server, &tool.name);
            let remote = tool.name.clone();
            let client = client.clone();
            let handle = io_handle();
            ToolDefinition {
                label: name.clone(),
                name,
                description: tool.description.clone().unwrap_or_default(),
                prompt_snippet: None,
                prompt_guidelines: Vec::new(),
                parameters: tool.input_schema,
                prepare_arguments: None,
                execution_mode: None,
                background: false,
                background_when: None,
                ordered_start: false,
                execute: Arc::new(move |_id, params, signal, on_update, _ctx| {
                    call_blocking(&handle, &client, &remote, params, signal, on_update)
                }),
            }
        })
        .collect()
}

/// Runs one `tools/call` from a tool's blocking thread. The turn's abort signal cancels the
/// call; progress reports go to `on_update` as text.
fn call_blocking(
    handle: &Handle,
    client: &McpClient,
    remote: &str,
    params: serde_json::Value,
    signal: Option<AbortSignal>,
    on_update: Option<AgentToolUpdateCallback>,
) -> Result<AgentToolResult, ToolError> {
    let cancel = CancellationToken::new();
    let (progress_tx, mut progress_rx) = tokio::sync::watch::channel(None::<ToolProgress>);
    let options = CallOptions {
        cancel: cancel.clone(),
        progress: Some(progress_tx),
    };
    let outcome = handle.block_on(async {
        let mut call = std::pin::pin!(client.call_tool(remote, params, options));
        let mut progress_open = true;
        let mut aborting = false;
        loop {
            tokio::select! {
                biased;
                result = &mut call => break result,
                _ = wait_for_abort(signal.as_ref()), if !aborting => {
                    aborting = true;
                    cancel.cancel();
                }
                changed = progress_rx.changed(), if progress_open => match changed {
                    Ok(()) => {
                        let latest = progress_rx.borrow_and_update().clone();
                        if let (Some(progress), Some(update)) = (latest, on_update.as_ref()) {
                            update(progress_result(&progress));
                        }
                    }
                    Err(_) => progress_open = false,
                },
            }
        }
    });
    match outcome {
        Err(error) => Err(error.to_string().into()),
        Ok(output) => output_result(output),
    }
}

async fn wait_for_abort(signal: Option<&AbortSignal>) {
    match signal {
        Some(signal) => signal.cancelled().await,
        None => std::future::pending().await,
    }
}

fn progress_result(progress: &ToolProgress) -> AgentToolResult {
    let mut text = match progress.total {
        Some(total) => format!("{}/{}", progress.progress, total),
        None => progress.progress.to_string(),
    };
    if let Some(message) = &progress.message {
        text.push(' ');
        text.push_str(message);
    }
    AgentToolResult {
        content: vec![Content::text(text)],
        details: serde_json::Value::Null,
        terminate: false,
    }
}

fn output_result(output: ToolOutput) -> Result<AgentToolResult, ToolError> {
    let content: Vec<Content> = output
        .content
        .into_iter()
        .map(|block| match block {
            ToolContent::Text(text) => Content::text(text),
            ToolContent::Image { mime_type, data } => Content::Image(ImageContent {
                data,
                media_type: mime_type,
            }),
        })
        .collect();
    if output.is_error {
        let text: String = content
            .iter()
            .filter_map(|c| match c {
                Content::Text(t) => Some(t.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        let text = if text.is_empty() {
            "the MCP tool reported an error".to_owned()
        } else {
            text
        };
        return Err(text.into());
    }
    Ok(AgentToolResult {
        content,
        details: serde_json::Value::Null,
        terminate: false,
    })
}

/// The question the trust prompt asks for one source: which servers, or what changed.
pub fn trust_question(prompt: &TrustPrompt, servers: &[McpServerInfo]) -> String {
    let names: Vec<&str> = servers
        .iter()
        .filter(|s| s.source == prompt.source.label())
        .map(|s| s.name.as_str())
        .collect();
    let listed = if names.is_empty() {
        String::new()
    } else {
        format!(": {}", names.join(", "))
    };
    match &prompt.status {
        TrustStatus::Changed(diff) => {
            let mut parts = Vec::new();
            if !diff.added.is_empty() {
                parts.push(format!("added {}", diff.added.join(", ")));
            }
            if !diff.changed.is_empty() {
                parts.push(format!("changed {}", diff.changed.join(", ")));
            }
            if !diff.removed.is_empty() {
                parts.push(format!("removed {}", diff.removed.join(", ")));
            }
            format!(
                "MCP servers from {} changed since you trusted them ({}). Trust the new list?",
                prompt.source.label(),
                parts.join("; ")
            )
        }
        _ => format!(
            "Trust MCP servers from {} ({}){}? Untrusted servers do not start.",
            prompt.source.label(),
            prompt.source.path().display(),
            listed
        ),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_login_error_maps_to_auth_needed_and_others_to_failed() {
        let auth = McpError::AuthRequired {
            server: "docs".to_owned(),
            www_authenticate: "Bearer".to_owned(),
        };
        assert_eq!(status_for(&auth), McpStatus::AuthNeeded);
        assert_eq!(
            status_for(&McpError::Auth("refresh rejected".to_owned())),
            McpStatus::AuthNeeded
        );
        // A 401 to the entry's own Authorization header is a failure a login cannot fix.
        let own = McpError::Unauthorized("check the Authorization header".to_owned());
        assert_eq!(
            status_for(&own),
            McpStatus::Failed("unauthorized: check the Authorization header".to_owned())
        );
        // The message text no longer decides: a connect failure that says "401" is just failed.
        let other = McpError::Connect("server said 401 unauthorized in its banner".to_owned());
        assert!(matches!(status_for(&other), McpStatus::Failed(_)));
    }
}

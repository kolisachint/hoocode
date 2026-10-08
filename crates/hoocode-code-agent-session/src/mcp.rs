//! MCP servers for one session (`docs/design/mcp.md`).
//!
//! The hub reads the `mcp.json` sources and the trust store (`hoocode-code-mcp`), starts the
//! trusted servers in the background on the `hoocode-io` runtime, and turns each connected
//! server's tools into [`ToolDefinition`]s named `mcp_<server>_<tool>`. Startup never waits for
//! a server: a server that fails becomes `Failed` and its tools are simply absent.
//!
//! The hub never prompts. The interactive mode asks about [`McpHub::pending_prompts`] and calls
//! [`McpHub::grant`]; print and rpc skip untrusted servers (fail closed).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use hoocode_agent_mcp::{
    tool_name, CallOptions, ClientOptions, McpClient, McpError, McpServerConfig, McpTool,
    ToolContent, ToolOutput, ToolProgress,
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

pub use hoocode_code_mcp::TrustPrompt;

/// How long [`McpHub::shutdown`] waits for the servers to close.
const SHUTDOWN_WAIT: Duration = Duration::from_secs(5);

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
    config: McpServerConfig,
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
}

/// The MCP servers of one session. Cheap to share through `Arc`.
pub struct McpHub {
    trust_path: PathBuf,
    state: Arc<Mutex<HubState>>,
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
                Entry {
                    name: server.name.clone(),
                    source: server.source.clone(),
                    config: to_client_config(&server.transport),
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
        Self {
            trust_path,
            state: Arc::new(Mutex::new(state)),
        }
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
                    .map(|i| {
                        (
                            i,
                            state.entries[i].name.clone(),
                            state.entries[i].config.clone(),
                        )
                    })
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
        lock(&self.state)
            .entries
            .iter()
            .map(|e| McpServerInfo {
                name: e.name.clone(),
                source: e.source.label(),
                status: e.status.clone(),
                tool_count: e.tools.len(),
            })
            .collect()
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

    fn spawn_jobs(&self, jobs: Vec<(usize, String, McpServerConfig)>) {
        for (index, name, config) in jobs {
            let state = self.state.clone();
            io_handle().spawn(async move {
                let outcome = connect_and_list(&name, config).await;
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
                    }
                    Err(error) => {
                        guard.entries[index].status = classify(&error);
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
        .map(|(i, e)| (i, e.name.clone(), e.config.clone()))
        .collect()
}

async fn connect_and_list(
    name: &str,
    config: McpServerConfig,
) -> Result<(McpClient, Vec<McpTool>), McpError> {
    let client = McpClient::connect(name, config, ClientOptions::default()).await?;
    match client.list_tools().await {
        Ok(tools) => Ok((client, tools)),
        Err(error) => {
            client.shutdown().await;
            Err(error)
        }
    }
}

/// `Failed` with the reason, or `AuthNeeded` when the server refused the connection for want of
/// a login. Heuristic: the client has no OAuth yet, so the message is all there is to go on.
fn classify(error: &McpError) -> McpStatus {
    let text = error.to_string();
    let lower = text.to_ascii_lowercase();
    let needs_login = [
        "401",
        "unauthorized",
        "authentication",
        "auth required",
        "oauth",
    ]
    .iter()
    .any(|needle| lower.contains(needle));
    if needs_login {
        McpStatus::AuthNeeded
    } else {
        McpStatus::Failed(text)
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

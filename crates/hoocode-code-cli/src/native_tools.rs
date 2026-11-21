//! Tier C native helper tools (PL-18 slice 4a, `pl18-spec.md` D2/D4/D5).
//!
//! A plugin whose `extensions."dev.hoocode".native` block validated declares helper tools.
//! They appear as model tools named as the plugin declares them (`echo`, say), with the origin
//! `Plugin { plugin, tool }` (the same origin as tier B WASM tools, so `plugin:<p>` selectors
//! match them). Registration shares the tier B collision rules: built-ins are never shadowed,
//! the first plugin in load order wins, names match case-insensitively, and the `mcp__`
//! prefix is refused. Only plugins from trusted marketplaces register tools, like tier B.
//!
//! A call runs through the session's [`Supervisor`]: the first call starts the helper process,
//! and the result maps to the agent tool result the same way MCP tool results map. No process
//! starts at session start or at registration. One supervisor lives per root session (see
//! [`NativeHolder`]); it is rebuilt when the plugin set changes, and shut down at session end.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::sync::{Arc, Mutex};

use hoocode_agent_mcp::{McpError, ToolContent, ToolOutput};
use hoocode_agent_types::{collision_notices, valid_plugin_tool_name, AgentToolResult, ToolOrigin};
use hoocode_ai_types::{AbortSignal, Content, ImageContent};
use hoocode_code_native::{
    binary_path_overrides, default_home_dir, helper_socket_env_name, NativeCallOptions,
    NativeError, Supervisor, SupervisorConfig, AGENT_ID_ENV, AGENT_TOKEN_ENV, ROOT_AGENT_ID,
};
use hoocode_code_permissions::{set_native_hints, NativeToolHint};
use hoocode_code_plugins::native::NativePermission;
use hoocode_code_plugins::{NativeHelperEntry, PluginSet};
use hoocode_code_settings::SettingsManager;
use hoocode_code_tool_api::{ToolDefinition, ToolError};
use hoocode_code_tool_bash::{BashSpawnContext, BashSpawnHook};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::args::DiagnosticKind;
use crate::runtime::Diagnostics;

/// The tier C supervisor of one root session, shared by its plugin tools and its Shell env.
///
/// Created from the loaded plugins without starting any process. [`sync`](Self::sync) rebuilds
/// it when the helper set changes (new helpers added, removed helpers' processes shut down);
/// [`shutdown`](Self::shutdown) ends it gracefully at session end (`Drop` stays the fallback).
pub(crate) struct NativeHolder {
    inner: Mutex<HolderInner>,
}

struct HolderInner {
    supervisor: Option<Arc<Supervisor>>,
    /// The sorted plugin names the active supervisor serves.
    plugins: Vec<String>,
    /// The user `binaryPath` overrides the active supervisor was built with.
    overrides: HashMap<String, std::path::PathBuf>,
}

impl NativeHolder {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(HolderInner {
                supervisor: None,
                plugins: Vec::new(),
                overrides: HashMap::new(),
            }),
        })
    }

    /// Bring the supervisor in line with `plugins`: trusted helpers only, user `binaryPath`
    /// overrides from `settings`. Never starts a process. Returns the problems found (one line
    /// each), for the session's diagnostics.
    pub(crate) fn sync(&self, plugins: &PluginSet, settings: &SettingsManager) -> Vec<String> {
        let trusted: HashSet<&str> = plugins
            .plugins
            .iter()
            .filter(|p| p.marketplace_trusted)
            .map(|p| p.name.as_str())
            .collect();
        let mut helpers: Vec<_> = plugins
            .native_helpers()
            .into_iter()
            .filter(|entry| trusted.contains(entry.plugin.as_str()))
            .collect();
        helpers.sort_by(|a, b| a.plugin.cmp(&b.plugin));
        let names: Vec<String> = helpers.iter().map(|e| e.plugin.clone()).collect();

        let raw = Value::Object(settings.global_settings().clone());
        // `binaryPath` is user-level only (spec D3): a cloned repo's project settings must
        // never point a trusted plugin's helper at its own binary.
        let (overrides, mut warnings) = binary_path_overrides(&raw);
        let same = {
            let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            inner.plugins == names
                && inner.overrides == overrides
                && inner.supervisor.is_some()
        };
        if same {
            // Neither the helper set nor the `binaryPath` overrides changed:
            // keep the running supervisor (and its processes). Publishing the
            // hints again is harmless.
            publish_hints(&helpers);
            return warnings;
        }
        if helpers.is_empty() {
            set_native_hints(HashMap::new());
            let old = self
                .inner
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .supervisor
                .take();
            if old.is_some() {
                // No helper left: the old supervisor's processes are shut down in the
                // background, like a swapped MCP hub, so the turn being prepared does not wait.
                hoocode_runtime::io_handle().spawn(async move {
                    if let Some(old) = old {
                        // Best effort: the turn being prepared does not wait for it.
                        let _ = old.shutdown().await;
                    }
                });
            }
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            inner.plugins = Vec::new();
            inner.overrides = HashMap::new();
            return warnings;
        }
        let supervisor = match Supervisor::new(
            default_home_dir(),
            helpers.clone(),
            overrides.clone(),
            SupervisorConfig::default(),
        ) {
            Ok(supervisor) => Arc::new(supervisor),
            Err(error) => {
                warnings.push(format!("native helpers off: {error}"));
                return warnings;
            }
        };
        publish_hints(&helpers);
        let old = {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            inner.plugins = names;
            inner.overrides = overrides;
            inner.supervisor.replace(supervisor)
        };
        if let Some(old) = old {
            hoocode_runtime::io_handle().spawn(async move {
                // Best effort: the turn being prepared does not wait for it.
                let _ = old.shutdown().await;
            });
        }
        warnings
    }

    /// The active supervisor, if the session has a tier C helper.
    pub(crate) fn supervisor(&self) -> Option<Arc<Supervisor>> {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .supervisor
            .clone()
    }

    /// The user `binaryPath` override for `plugin`, if one is set. Tests use
    /// it to prove a project-level value is ignored (spec D3).
    #[cfg(test)]
    pub(crate) fn override_for(&self, plugin: &str) -> Option<std::path::PathBuf> {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .overrides
            .get(plugin)
            .cloned()
    }

    /// Gracefully shut the supervisor down (bounded by its own timeout): `hoocode/sessionEnding`
    /// to every running helper, then kill. Safe to call twice, or with no supervisor.
    pub(crate) async fn shutdown(&self) {
        let supervisor = self
            .inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .supervisor
            .clone();
        if let Some(supervisor) = supervisor {
            // Best effort at session end: `Drop` stays the fallback.
            let _ = supervisor.shutdown().await;
        }
    }

    /// The extra Shell environment for one command (spec D5): `HOOCODE_AGENT_ID` (`root`) and
    /// `HOOCODE_AGENT_TOKEN` (the root token) when the session has a tier C helper, plus
    /// `HOOCODE_HELPER_SOCKET_<PLUGIN>` for every helper that has started. Empty when the
    /// session has no helper. Never logs, never reaches the model except through the command.
    pub(crate) fn shell_extra_env(&self) -> Vec<(OsString, OsString)> {
        let supervisor = self
            .inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .supervisor
            .clone();
        let Some(supervisor) = supervisor else {
            return Vec::new();
        };
        if !supervisor.has_helpers() {
            return Vec::new();
        }
        let mut env = vec![
            (OsString::from(AGENT_ID_ENV), OsString::from(ROOT_AGENT_ID)),
            (
                OsString::from(AGENT_TOKEN_ENV),
                OsString::from(supervisor.root_identity().token.clone()),
            ),
        ];
        let mut sockets: Vec<_> = supervisor.live_sockets().into_iter().collect();
        sockets.sort_by(|a, b| a.0.cmp(&b.0));
        for (plugin, socket) in sockets {
            env.push((
                OsString::from(helper_socket_env_name(&plugin)),
                OsString::from(socket),
            ));
        }
        env
    }
}

/// Which side runs a tier C tool: the session's supervisor (the root), or a
/// subagent child's cached connection after an EnsureHelper round-trip.
#[derive(Clone)]
pub(crate) enum NativeExecutor {
    Supervisor(Arc<Supervisor>),
    Child(Arc<super::native_child::ChildNativeState>),
}

/// Publish the declared tools' permission hints to the gate, keyed by plugin+tool
/// (origin-keyed, never name-keyed). Missing hints mean `act`.
pub(crate) fn publish_hints(helpers: &[hoocode_code_plugins::NativeHelperEntry]) {
    let mut hints = HashMap::new();
    for entry in helpers {
        for tool in &entry.helper.spec.tools {
            let hint = match tool.permission {
                NativePermission::Read => NativeToolHint::Read,
                NativePermission::Act => NativeToolHint::Act,
                NativePermission::Dangerous => NativeToolHint::Dangerous,
            };
            hints.insert((entry.plugin.clone(), tool.name.clone()), hint);
        }
    }
    set_native_hints(hints);
}

/// The session's current tier C holder, set when a session is assembled and shut down when the
/// session ends (see `runtime.rs`). Like `LAST_RESOURCES`: one root session drives it.
static CURRENT: Mutex<Option<Arc<NativeHolder>>> = Mutex::new(None);

/// Remember `holder` as the session's holder.
pub(crate) fn set_current(holder: Arc<NativeHolder>) {
    *CURRENT.lock().unwrap_or_else(|e| e.into_inner()) = Some(holder);
}

/// The session's holder, if it has one.
pub(crate) fn current() -> Option<Arc<NativeHolder>> {
    CURRENT.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Take the session's holder, leaving none behind. Used on in-process session replacement
/// (`/new`, `/resume`, rpc `new_session`, ...): the old session was already torn down, so the
/// caller shuts its supervisor down gracefully instead of relying on `Drop`.
pub(crate) fn take_current() -> Option<Arc<NativeHolder>> {
    CURRENT.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// The model tools of every trusted plugin's native helper: bare names with the
/// `Plugin { plugin, tool }` origin. A name the session already has (built-ins, the session's
/// own tools, the MCP tools, the tier B tools registered before these) is skipped with a
/// diagnostic, like tier B. Nothing here starts a process: the first call does.
pub(crate) fn build(
    plugins: &PluginSet,
    holder: &NativeHolder,
    taken: &[(String, ToolOrigin)],
    diagnostics: &mut Diagnostics,
) -> Vec<ToolDefinition> {
    let Some(supervisor) = holder.supervisor() else {
        return Vec::new();
    };
    let trusted: HashSet<&str> = plugins
        .plugins
        .iter()
        .filter(|p| p.marketplace_trusted)
        .map(|p| p.name.as_str())
        .collect();
    let entries: Vec<NativeHelperEntry> = plugins
        .native_helpers()
        .into_iter()
        .filter(|entry| trusted.contains(entry.plugin.as_str()))
        .collect();
    build_from_entries(
        entries,
        taken,
        diagnostics,
        NativeExecutor::Supervisor(supervisor),
    )
}

/// [`build`], over an already-filtered entry list with an explicit executor:
/// the root runs through its supervisor, a subagent child through its cached
/// connection (spec D6). Nothing here starts a process either way.
pub(crate) fn build_from_entries(
    entries: Vec<NativeHelperEntry>,
    taken: &[(String, ToolOrigin)],
    diagnostics: &mut Diagnostics,
    executor: NativeExecutor,
) -> Vec<ToolDefinition> {
    let mut out = Vec::new();
    // Every name the session has, with the origin that holds it, keyed by lowercase name: a
    // selector matches a name case-insensitively, so `Read` and `read` are one name.
    let mut holders: HashMap<String, ToolOrigin> = taken
        .iter()
        .map(|(name, origin)| (name.to_ascii_lowercase(), origin.clone()))
        .collect();
    let mut collisions = Vec::new();
    for entry in &entries {
        let label = format!("plugin {}", entry.plugin);
        for tool in &entry.helper.spec.tools {
            if !valid_plugin_tool_name(&tool.name) {
                diagnostics.push((
                    DiagnosticKind::Warning,
                    format!(
                        "{label}: tool {:?} not loaded: its name must match ^[A-Za-z][A-Za-z0-9_-]{{0,63}}$ and not start with mcp__",
                        tool.name
                    ),
                ));
                continue;
            }
            let origin = ToolOrigin::Plugin {
                plugin: entry.plugin.clone(),
                tool: tool.name.clone(),
            };
            let key = tool.name.to_ascii_lowercase();
            if let Some(existing) = holders.get(&key) {
                collisions.push(hoocode_agent_types::Collision {
                    name: tool.name.clone(),
                    origin,
                    existing: existing.clone(),
                });
                continue;
            }
            holders.insert(key, origin.clone());
            out.push(definition(
                origin,
                entry.plugin.clone(),
                tool.clone(),
                executor.clone(),
            ));
        }
    }
    for message in collision_notices(&collisions) {
        diagnostics.push((DiagnosticKind::Warning, message));
    }
    out
}

/// A call runs through the session's supervisor (starting the helper on first use) on the
/// io runtime, or — in a subagent child — through the child's cached connection after an
/// EnsureHelper round-trip; the result maps like an MCP tool result either way.
fn definition(
    origin: ToolOrigin,
    plugin: String,
    tool: hoocode_code_plugins::native::NativeTool,
    executor: NativeExecutor,
) -> ToolDefinition {
    let handle = hoocode_runtime::io_handle();
    let label = tool.name.clone();
    let name = tool.name.clone();
    let remote = tool.name.clone();
    ToolDefinition {
        origin,
        render: None,
        label,
        name,
        description: tool.description.clone(),
        prompt_snippet: None,
        prompt_guidelines: Vec::new(),
        parameters: tool.input_schema.clone(),
        prepare_arguments: None,
        execution_mode: None,
        background: false,
        background_when: None,
        background_placeholder: None,
        ordered_start: false,
        abort_grace: None,
        execute: Arc::new(move |_id, params, signal, _on_update, _ctx| {
            // Esc aborts the turn: the token fires (the helper sees a cancelled
            // call) and the wait ends at once, without waiting out
            // ensure/download/ready-wait. The dropped op may still have
            // started server-side, like a call cut by a crash.
            let cancel = CancellationToken::new();
            let options = NativeCallOptions {
                cancel: cancel.clone(),
            };
            let output = match &executor {
                NativeExecutor::Supervisor(supervisor) => {
                    let call = supervisor.call_tool(&plugin, &remote, params, options);
                    abortable(&handle, signal.as_ref(), cancel, call)
                        .map_err(|error| ToolError::from(error.to_string()))?
                }
                NativeExecutor::Child(state) => super::native_child::call_tool(
                    state,
                    &handle,
                    &plugin,
                    &remote,
                    params,
                    signal.as_ref(),
                    cancel,
                )
                .map_err(|error| ToolError::from(error.to_string()))?,
            };
            output_result(output)
        }),
    }
}

/// Run an async tier C op from a tool's blocking thread, wiring the turn's
/// abort signal to `cancel`: on abort the token fires and the wait ends at
/// once with `McpError::Cancelled` (see the `execute` above).
pub(crate) fn abortable<T>(
    handle: &tokio::runtime::Handle,
    signal: Option<&AbortSignal>,
    cancel: CancellationToken,
    op: impl std::future::Future<Output = Result<T, NativeError>>,
) -> Result<T, NativeError> {
    handle.block_on(async {
        tokio::pin!(op);
        tokio::select! {
            result = &mut op => result,
            _ = wait_for_abort(signal) => {
                cancel.cancel();
                Err(NativeError::Mcp(McpError::Cancelled))
            }
        }
    })
}

async fn wait_for_abort(signal: Option<&AbortSignal>) {
    match signal {
        Some(signal) => signal.cancelled().await,
        None => std::future::pending().await,
    }
}

/// An MCP-shaped helper result as the agent tool result, the way MCP tool results map
/// (`agent-session::mcp`): text and image blocks through, `is_error` as the tool error.
pub(crate) fn output_result(output: ToolOutput) -> Result<AgentToolResult, ToolError> {
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
            "the native helper tool reported an error".to_owned()
        } else {
            text
        };
        return Err(text.into());
    }
    Ok(AgentToolResult {
        content,
        details: Value::Null,
        terminate: false,
    })
}

/// The Shell tool's spawn hook for a session with tier C helpers (spec D5): every command gets
/// `HOOCODE_AGENT_ID`/`HOOCODE_AGENT_TOKEN` plus the started helpers' sockets. A session with
/// no helper gets no extra variable.
pub(crate) fn shell_spawn_hook(holder: Arc<NativeHolder>) -> BashSpawnHook {
    Arc::new(move |mut ctx: BashSpawnContext| {
        for (key, value) in holder.shell_extra_env() {
            ctx.env.insert(key, value);
        }
        ctx
    })
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // the abortable test runs on its own runtime, like the tool threads
mod tests {
    use super::*;
    use hoocode_code_mcp::TrustStore;
    use hoocode_code_plugins::{load_plugins, LoadOptions};
    use serde_json::json;
    use std::collections::BTreeSet;
    use std::path::Path;

    fn write_native_plugin(dir: &Path) {
        let root = dir.join("plugins").join("demo");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("plugin.json"),
            serde_json::json!({
                "$schema": "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json",
                "name": "demo",
                "version": "1.0.0",
                "description": "Tier C fixture",
                "extensions": {"dev.hoocode": {"native": {
                    "server": "echo",
                    "shared": true,
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
            r#"{"$schema": "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json",
                "mcpServers": {"echo": {"type": "stdio", "command": "never-started"}}}"#,
        )
        .unwrap();
    }

    fn settings_with(
        cwd: &Path,
        agent_dir: &Path,
        global_native: Option<serde_json::Value>,
        project_native: Option<serde_json::Value>,
    ) -> SettingsManager {
        std::fs::create_dir_all(agent_dir).unwrap();
        std::fs::create_dir_all(cwd.join(".hoocode")).unwrap();
        let mut global = serde_json::Map::new();
        if let Some(native) = global_native {
            global.insert("nativeHelpers".into(), native);
        }
        std::fs::write(
            agent_dir.join("settings.json"),
            serde_json::Value::Object(global).to_string(),
        )
        .unwrap();
        let mut project = serde_json::Map::new();
        if let Some(native) = project_native {
            project.insert("nativeHelpers".into(), native);
        }
        std::fs::write(
            cwd.join(".hoocode/settings.json"),
            serde_json::Value::Object(project).to_string(),
        )
        .unwrap();
        SettingsManager::create(cwd, agent_dir)
    }

    fn native_helpers_of(dir: &Path) -> PluginSet {
        std::fs::create_dir_all(dir.join("repo")).unwrap();
        write_native_plugin(dir);
        let trust = TrustStore::empty(dir.join("trust.json"));
        load_plugins(&LoadOptions {
            project_folder: dir.join("repo"),
            user_plugins_dir: dir.join("plugins"),
            data_root: dir.join("plugin-data"),
            trust: &trust,
            untrusted_installs: BTreeSet::new(),
            installs_unreadable: None,
        })
    }

    /// The abort wrapper ends the wait at once with `Cancelled`: Esc never
    /// waits out ensure/download/ready-wait/call. Runs on a plain thread
    /// like the tool blocking threads that call it (`block_on` cannot run
    /// inside an async test).
    #[test]
    fn abortable_returns_cancelled_promptly() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let signal = AbortSignal::new();
        signal.abort();
        let cancel = CancellationToken::new();
        let handle = runtime.handle().clone();
        let started = std::time::Instant::now();
        let result: Result<(), NativeError> =
            abortable(&handle, Some(&signal), cancel.clone(), async {
                let hung: Result<(), NativeError> = std::future::pending().await;
                hung
            });
        assert!(
            matches!(result, Err(NativeError::Mcp(McpError::Cancelled))),
            "an aborted wait reports cancellation: {result:?}"
        );
        assert!(cancel.is_cancelled(), "the op's token fires");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "the aborted wait ends at once"
        );
    }

    /// `binaryPath` is user-level only (spec D3): a project settings value is
    /// ignored, a global one is honored.
    #[test]
    fn binary_path_is_user_level_only() {
        let dir = tempfile::tempdir().unwrap();
        let set = native_helpers_of(dir.path());
        assert!(
            set.native_helpers().iter().any(|e| e.plugin == "demo"),
            "the fixture plugin parses its native block"
        );
        let mk = |name: &str| json!({name: {"binaryPath": "/abs/project-binary"}});
        // A project-only value is ignored: no override, no warning.
        let settings = settings_with(
            &dir.path().join("repo"),
            &dir.path().join("agent"),
            None,
            Some(mk("demo")),
        );
        assert!(settings.project_settings().contains_key("nativeHelpers"));
        let holder = NativeHolder::new();
        let warnings = holder.sync(&set, &settings);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(holder.override_for("demo"), None);
        // A global value is honored alongside an (ignored) project one.
        let settings = settings_with(
            &dir.path().join("repo"),
            &dir.path().join("agent"),
            Some(json!({"demo": {"binaryPath": "/abs/user-binary"}})),
            Some(mk("demo")),
        );
        let holder = NativeHolder::new();
        let warnings = holder.sync(&set, &settings);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            holder.override_for("demo"),
            Some(std::path::PathBuf::from("/abs/user-binary"))
        );
    }

    /// A changed `binaryPath` rebuilds the supervisor: the running one was
    /// built with the old overrides and never re-reads them, so keeping it
    /// would start helpers from the stale path. The old supervisor shuts
    /// down in the background like any other set change.
    #[test]
    fn changed_overrides_rebuild_the_supervisor() {
        let dir = tempfile::tempdir().unwrap();
        let set = native_helpers_of(dir.path());
        let settings = |binary: &str| {
            settings_with(
                &dir.path().join("repo"),
                &dir.path().join("agent"),
                Some(json!({"demo": {"binaryPath": binary}})),
                None,
            )
        };
        let holder = NativeHolder::new();
        assert!(holder.sync(&set, &settings("/abs/first")).is_empty());
        let first = holder.supervisor().expect("supervisor after first sync");
        assert_eq!(
            holder.override_for("demo"),
            Some(std::path::PathBuf::from("/abs/first"))
        );
        // Same helper set, only the override changed: the supervisor is
        // replaced, not kept with stale overrides.
        assert!(holder.sync(&set, &settings("/abs/second")).is_empty());
        let second = holder.supervisor().expect("supervisor after override change");
        assert_eq!(
            holder.override_for("demo"),
            Some(std::path::PathBuf::from("/abs/second"))
        );
        assert!(
            !Arc::ptr_eq(&first, &second),
            "the supervisor is rebuilt for new overrides"
        );
        // An identical re-sync keeps the running supervisor.
        assert!(holder.sync(&set, &settings("/abs/second")).is_empty());
        assert!(
            Arc::ptr_eq(&second, &holder.supervisor().expect("still there")),
            "an unchanged sync keeps the supervisor"
        );
    }
}

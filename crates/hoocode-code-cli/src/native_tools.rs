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

use hoocode_agent_mcp::{ToolContent, ToolOutput};
use hoocode_agent_types::{collision_notices, valid_plugin_tool_name, AgentToolResult, ToolOrigin};
use hoocode_ai_types::{Content, ImageContent};
use hoocode_code_native::{
    binary_path_overrides, default_home_dir, helper_socket_env_name, Supervisor, SupervisorConfig,
    AGENT_ID_ENV, AGENT_TOKEN_ENV, ROOT_AGENT_ID,
};
use hoocode_code_permissions::{set_native_hints, NativeToolHint};
use hoocode_code_plugins::native::NativePermission;
use hoocode_code_plugins::{NativeHelperEntry, PluginSet};
use hoocode_code_settings::SettingsManager;
use hoocode_code_tool_api::{ToolDefinition, ToolError};
use hoocode_code_tool_bash::{BashSpawnContext, BashSpawnHook};
use serde_json::Value;

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
}

impl NativeHolder {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(HolderInner {
                supervisor: None,
                plugins: Vec::new(),
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

        let raw = Value::Object(settings.settings().clone());
        let (overrides, mut warnings) = binary_path_overrides(&raw);
        let same = {
            let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            inner.plugins == names && inner.supervisor.is_some()
        };
        if same {
            // The helper set did not change: keep the running supervisor (and its processes).
            // Overrides are read again above so a changed `binaryPath` applies to the next
            // start; publishing the hints again is harmless.
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
            self.inner.lock().unwrap_or_else(|e| e.into_inner()).plugins = Vec::new();
            return warnings;
        }
        let supervisor = match Supervisor::new(
            default_home_dir(),
            helpers.clone(),
            overrides,
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
        execute: Arc::new(move |_id, params, _signal, _on_update, _ctx| {
            let output = match &executor {
                NativeExecutor::Supervisor(supervisor) => handle
                    .block_on(supervisor.call_tool(&plugin, &remote, params))
                    .map_err(|error| ToolError::from(error.to_string()))?,
                NativeExecutor::Child(state) => {
                    super::native_child::call_tool(state, &handle, &plugin, &remote, params)
                        .map_err(|error| ToolError::from(error.to_string()))?
                }
            };
            output_result(output)
        }),
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

//! Plugin WASM components as agent tools (`docs/design/plugins.md`, PL-9).
//!
//! Each tool a loaded `dev.hoocode/extension.wasm` lists becomes a model tool named as the
//! plugin declares it (`WebFetch`, say), with the origin `Plugin { plugin, tool }`
//! (`docs/design/plugins.md`, Naming and identity). A call runs the component in the host (`hoocode-code-wasm`) on a
//! blocking thread. Its network requests ask the session's permission gate as `WebFetch`, once
//! per origin (scheme, host, port) per tool call. The call itself is not gated.

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsStr;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use hoocode_agent_types::{
    collision_notice, AgentToolCall, AgentToolResult, PermissionDecision, PermissionGate,
    ToolOrigin,
};
use hoocode_ai_types::Content;
use hoocode_code_plugins::PluginSet;
use hoocode_code_settings::SettingsManager;
use hoocode_code_tool_api::{ToolDefinition, ToolError};
use hoocode_code_wasm::{
    CallEnv, ConfigLookup, Extension, ExtensionSpec, NetGate, ToolInfo, WasmError, WasmHost,
};
use hoocode_runtime::io_handle;
use serde_json::Value;
use tokio::runtime::Handle;
use url::Url;

use crate::args::DiagnosticKind;
use crate::runtime::Diagnostics;

/// The built-in tool a component's network request is asked as.
const NET_TOOL: &str = "WebFetch";

/// Internal (not in `--help`, tests only): when set to exactly `1`, the WASM host lets loopback
/// addresses through its SSRF block, so an integration test can serve a plugin's request from a
/// local server. It is read from the process environment only, never from plugin config, so a
/// plugin cannot switch it on. Never set it anywhere else.
pub const INTERNAL_WASM_LOOPBACK_ENV: &str = "HOOCODE_INTERNAL_WASM_ALLOW_LOOPBACK";

/// The `pluginConfig` values of each plugin, by plugin name. Copied out of the settings so the
/// caller can drop its settings lock before any component is built.
pub(crate) type PluginConfigs = HashMap<String, BTreeMap<String, String>>;

/// The config values of every plugin in `plugins`, read from `settings` now.
pub(crate) fn plugin_configs(plugins: &PluginSet, settings: &SettingsManager) -> PluginConfigs {
    plugins
        .plugins
        .iter()
        .map(|plugin| {
            (
                plugin.name.clone(),
                settings.plugin_config_values(&plugin.name),
            )
        })
        .collect()
}

/// The model tools of every loaded plugin component, with the cache folder the default host
/// uses (`<agent dir>/cache/wasm`). `configs` comes from [`plugin_configs`].
///
/// A plugin whose marketplace is not trusted is skipped, as its MCP servers would wait for a
/// grant: the component runs only under the same trust (`LoadedPlugin::marketplace_trusted`).
/// A component that fails to load, and a tool whose name is taken, are skipped with a
/// diagnostic. Nothing here fails the session.
pub(crate) fn build(
    plugins: &PluginSet,
    configs: &PluginConfigs,
    gate: Arc<dyn PermissionGate>,
    taken: &[(String, ToolOrigin)],
    diagnostics: &mut Diagnostics,
) -> Vec<ToolDefinition> {
    let cache_dir = hoocode_code_paths::agent_dir().join("cache").join("wasm");
    build_with_cache(plugins, configs, gate, taken, &cache_dir, diagnostics)
}

/// [`build`] with the component cache folder given.
fn build_with_cache(
    plugins: &PluginSet,
    configs: &PluginConfigs,
    gate: Arc<dyn PermissionGate>,
    taken: &[(String, ToolOrigin)],
    cache_dir: &Path,
    diagnostics: &mut Diagnostics,
) -> Vec<ToolDefinition> {
    let mut out = Vec::new();
    // Every name the session has, with the origin that holds it. A plugin tool is added here
    // as it is loaded, so a later plugin cannot take its name either.
    let mut holders: HashMap<String, ToolOrigin> = taken.iter().cloned().collect();
    let mut host: Option<WasmHost> = None;
    let trust = hoocode_ai_util::tls::installed_trust();
    for plugin in &plugins.plugins {
        let Some(wasm_path) = &plugin.extension_wasm else {
            continue;
        };
        let label = format!("plugin {}", plugin.name);
        if !plugin.marketplace_trusted {
            diagnostics.push((
                DiagnosticKind::Warning,
                format!("{label}: extension not loaded: plugin is not trusted"),
            ));
            continue;
        }
        if host.is_none() {
            match new_host(cache_dir, &trust) {
                Ok(new) => host = Some(new),
                Err(error) => {
                    diagnostics.push((
                        DiagnosticKind::Warning,
                        format!("{label}: extension not loaded: {error}"),
                    ));
                    return out;
                }
            }
        }
        let Some(wasm_host) = host.clone() else {
            return out;
        };
        let spec = ExtensionSpec {
            plugin: plugin.name.clone(),
            wasm_path: wasm_path.clone(),
            data_dir: plugin.data_dir.clone(),
        };
        let extension = match wasm_host.load(spec) {
            Ok(extension) => Arc::new(extension),
            Err(error) => {
                diagnostics.push((
                    DiagnosticKind::Warning,
                    format!("{label}: extension not loaded: {error}"),
                ));
                continue;
            }
        };
        // A plugin reads only `pluginConfig.<its name>`, taken when the session starts.
        let values = configs.get(&plugin.name).cloned().unwrap_or_default();
        let config: ConfigLookup = Arc::new(move |key: &str| values.get(key).cloned());
        for info in extension.tools() {
            if !valid_tool_name(&info.name) {
                diagnostics.push((
                    DiagnosticKind::Warning,
                    format!(
                        "{label}: tool {:?} not loaded: its name must match ^[A-Za-z][A-Za-z0-9_-]{{0,63}}$",
                        info.name
                    ),
                ));
                continue;
            }
            let origin = ToolOrigin::Plugin {
                plugin: plugin.name.clone(),
                tool: info.name.clone(),
            };
            if let Some(existing) = holders.get(&info.name) {
                diagnostics.push((
                    DiagnosticKind::Warning,
                    collision_notice(&info.name, &info.name, &origin, existing),
                ));
                continue;
            }
            holders.insert(info.name.clone(), origin.clone());
            out.push(definition(
                origin,
                info,
                extension.clone(),
                config.clone(),
                gate.clone(),
            ));
        }
    }
    out
}

/// The model-facing name a plugin tool may declare: `^[A-Za-z][A-Za-z0-9_-]{0,63}$`.
fn valid_tool_name(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= 64
        && chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// A host for `cache_dir`, with the test-only loopback switch applied when its variable is `1`.
fn new_host(
    cache_dir: &Path,
    trust: &hoocode_ai_util::tls::TrustedCas,
) -> Result<WasmHost, WasmError> {
    let host = WasmHost::new(cache_dir.to_path_buf())?.with_tls_trust(trust)?;
    let allow = loopback_switch_on(std::env::var_os(INTERNAL_WASM_LOOPBACK_ENV).as_deref());
    Ok(if allow {
        host.with_loopback_allowed()
    } else {
        host
    })
}

/// Whether a value of [`INTERNAL_WASM_LOOPBACK_ENV`] turns the switch on: exactly `1`.
fn loopback_switch_on(value: Option<&OsStr>) -> bool {
    value.is_some_and(|v| v == "1")
}

/// One model tool for `info`, run through `extension`. Its model-facing name is the name the
/// plugin declares.
fn definition(
    origin: ToolOrigin,
    info: &ToolInfo,
    extension: Arc<Extension>,
    config: ConfigLookup,
    gate: Arc<dyn PermissionGate>,
) -> ToolDefinition {
    let handle = io_handle();
    let tool = info.name.clone();
    ToolDefinition {
        origin,
        label: info.name.clone(),
        name: info.name.clone(),
        description: info.description.clone(),
        prompt_snippet: None,
        prompt_guidelines: Vec::new(),
        parameters: info.input_schema.clone(),
        prepare_arguments: None,
        execution_mode: None,
        background: false,
        background_when: None,
        background_placeholder: None,
        ordered_start: false,
        abort_grace: None,
        execute: Arc::new(move |_id, params, signal, _on_update, _ctx| {
            // One net gate per call: its decisions and its last refusal are this call's alone.
            let net = Arc::new(ToolNetGate::new(gate.clone()));
            let env = CallEnv {
                config: config.clone(),
                net: net.clone(),
                abort: signal,
            };
            call_blocking(&handle, &extension, &tool, params, env, &net)
        }),
    }
}

/// The code a refused request shows the guest, for every refusal alike (`http-request-denied`).
const REFUSED_CODE: &str = "HttpRequestDenied";

/// A failed call's text, with the reason of the call's last refused request added when the
/// failure is that refusal. The guest sees only the code, so without this the model could not
/// tell a user's "no", a private address and a missing permission apart.
fn with_refusal(error: String, refusal: Option<String>) -> String {
    match refusal {
        Some(reason) if error.contains(REFUSED_CODE) => format!("{error}: {reason}"),
        _ => error,
    }
}

/// Runs one call from the tool's blocking thread. The result is the component's JSON as text
/// (pretty-printed for an object). A failure becomes the tool error, with its own message, and
/// a refusal with the reason it was refused for (`net`).
fn call_blocking(
    handle: &Handle,
    extension: &Extension,
    tool: &str,
    params: Value,
    env: CallEnv,
    net: &ToolNetGate,
) -> Result<AgentToolResult, ToolError> {
    let value = handle
        .block_on(extension.call(tool, params, env))
        .map_err(|error| ToolError::from(with_refusal(error.to_string(), net.last_refusal())))?;
    let text = match &value {
        Value::Object(_) => serde_json::to_string_pretty(&value),
        _ => serde_json::to_string(&value),
    }
    .map_err(|error| ToolError::from(error.to_string()))?;
    Ok(AgentToolResult {
        content: vec![Content::text(text)],
        details: Value::Null,
        terminate: false,
    })
}

/// An origin: scheme, host and port. Two URLs share one only if all three match.
type Origin = (String, String, u16);

/// The origin of an absolute URL with a host, or `None` when it has none.
fn origin_of(url: &str) -> Option<Origin> {
    let parsed = Url::parse(url).ok()?;
    let host = parsed.host_str()?.to_owned();
    let port = parsed.port_or_known_default()?;
    Some((parsed.scheme().to_owned(), host, port))
}

/// The network gate of one tool call. The first request to an origin asks the session's gate as
/// `WebFetch`; that answer, allow or deny, holds for later requests to the same origin in the
/// call, so a redirect hop on the same origin does not ask again. A request to another origin
/// asks again. Each request still passes the host's own checks before it reaches this gate.
pub(crate) struct ToolNetGate {
    gate: Arc<dyn PermissionGate>,
    decisions: Mutex<HashMap<Origin, Result<(), String>>>,
    /// The reason of the call's last refused request (the gate's, or a host check's).
    refusal: Mutex<Option<String>>,
}

impl ToolNetGate {
    pub(crate) fn new(gate: Arc<dyn PermissionGate>) -> Self {
        Self {
            gate,
            decisions: Mutex::new(HashMap::new()),
            refusal: Mutex::new(None),
        }
    }

    /// The reason of the call's last refused request, if any.
    fn last_refusal(&self) -> Option<String> {
        self.refusal
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn ask(&self, url: &str) -> Result<(), String> {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        let call = AgentToolCall {
            id: format!("ext-net-{}", NEXT_ID.fetch_add(1, Ordering::Relaxed)),
            name: NET_TOOL.to_owned(),
            arguments: serde_json::json!({ "url": url }),
        };
        match self.gate.request(&call) {
            PermissionDecision::Grant | PermissionDecision::GrantAlways => Ok(()),
            PermissionDecision::Deny { reason } => Err(reason),
        }
    }
}

impl NetGate for ToolNetGate {
    fn refused(&self, detail: &str) {
        *self.refusal.lock().unwrap_or_else(|e| e.into_inner()) = Some(detail.to_owned());
    }

    fn check(&self, url: &str) -> Result<(), String> {
        // A URL with no origin is refused without asking.
        let Some(origin) = origin_of(url) else {
            return Err("URL has no origin".to_owned());
        };
        // Held across the ask, so two requests to one origin cannot both ask.
        let mut decisions = self.decisions.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(decision) = decisions.get(&origin) {
            return decision.clone();
        }
        let decision = self.ask(url);
        decisions.insert(origin, decision.clone());
        decision
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::PathBuf;
    use std::sync::Mutex;

    use hoocode_code_mcp::TrustStore;
    use hoocode_code_plugins::{load_plugins, LoadOptions};
    use serde_json::json;

    use super::*;

    /// The fixture component (`hoocode-code-wasm`'s echo extension).
    const FIXTURE: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../hoocode-code-wasm/tests/fixtures/echo-ext.wasm"
    );

    /// A gate that answers from a script and records what it was asked.
    struct ScriptedGate {
        answers: Mutex<Vec<PermissionDecision>>,
        asked: Mutex<Vec<AgentToolCall>>,
    }

    impl ScriptedGate {
        fn new(answers: Vec<PermissionDecision>) -> Arc<Self> {
            Arc::new(Self {
                answers: Mutex::new(answers),
                asked: Mutex::new(Vec::new()),
            })
        }

        fn asked(&self) -> Vec<AgentToolCall> {
            self.asked.lock().unwrap().clone()
        }
    }

    impl PermissionGate for ScriptedGate {
        fn request(&self, tool_call: &AgentToolCall) -> PermissionDecision {
            self.asked.lock().unwrap().push(tool_call.clone());
            let mut answers = self.answers.lock().unwrap();
            if answers.is_empty() {
                PermissionDecision::Deny {
                    reason: "no answer scripted".into(),
                }
            } else {
                answers.remove(0)
            }
        }
    }

    /// A temp folder with a user plugin folder, a plugin data root and a project folder.
    struct Layout {
        _dir: tempfile::TempDir,
        user_plugins: PathBuf,
        data_root: PathBuf,
        cwd: PathBuf,
    }

    impl Layout {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().canonicalize().unwrap();
            let layout = Self {
                user_plugins: root.join("plugins"),
                data_root: root.join("plugin-data"),
                cwd: root.join("repo"),
                _dir: dir,
            };
            std::fs::create_dir_all(&layout.cwd).unwrap();
            layout
        }

        /// Writes plugin `name` with `component` as its `dev.hoocode/extension.wasm`.
        fn add_plugin(&self, name: &str, component: &[u8]) -> PathBuf {
            let root = self.user_plugins.join(name);
            std::fs::create_dir_all(root.join("dev.hoocode")).unwrap();
            std::fs::write(
                root.join("plugin.json"),
                format!(
                    r#"{{"$schema": "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json", "name": "{name}", "version": "1.0.0", "description": "Test plugin"}}"#
                ),
            )
            .unwrap();
            std::fs::write(root.join("dev.hoocode/extension.wasm"), component).unwrap();
            root.canonicalize().unwrap()
        }

        fn load(&self, untrusted: &[PathBuf]) -> PluginSet {
            let trust = TrustStore::empty(self.data_root.join("trust.json"));
            load_plugins(&LoadOptions {
                project_folder: self.cwd.clone(),
                user_plugins_dir: self.user_plugins.clone(),
                data_root: self.data_root.clone(),
                trust: &trust,
                untrusted_installs: untrusted.iter().cloned().collect::<BTreeSet<_>>(),
                installs_unreadable: None,
            })
        }

        fn cache(&self) -> PathBuf {
            self.data_root.join("cache-wasm")
        }
    }

    fn fixture_bytes() -> Vec<u8> {
        std::fs::read(FIXTURE).expect("fixture component exists")
    }

    fn settings(json: Value) -> SettingsManager {
        SettingsManager::in_memory(json.as_object().unwrap().clone())
    }

    fn run(def: &ToolDefinition, params: Value) -> Result<String, String> {
        let result =
            (def.execute)("call-1".into(), params, None, None, None).map_err(|e| e.to_string())?;
        match &result.content[0] {
            Content::Text(text) => Ok(text.text.clone()),
            other => panic!("expected text content, got {other:?}"),
        }
    }

    fn build_for(
        layout: &Layout,
        set: &PluginSet,
        settings: &SettingsManager,
        gate: Arc<dyn PermissionGate>,
        taken: &[(String, ToolOrigin)],
    ) -> (Vec<ToolDefinition>, Diagnostics) {
        let mut diagnostics = Diagnostics::new();
        let tools = build_with_cache(
            set,
            &plugin_configs(set, settings),
            gate,
            taken,
            &layout.cache(),
            &mut diagnostics,
        );
        (tools, diagnostics)
    }

    fn names(tools: &[ToolDefinition]) -> Vec<&str> {
        let mut names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        names.sort_unstable();
        names
    }

    #[test]
    fn the_loopback_switch_is_on_only_for_exactly_one() {
        assert!(!loopback_switch_on(None));
        assert!(loopback_switch_on(Some(OsStr::new("1"))));
        for off in ["", "0", "true", "yes", " 1"] {
            assert!(!loopback_switch_on(Some(OsStr::new(off))), "{off:?}");
        }
    }

    #[test]
    fn tools_are_named_ext_plugin_tool_and_carry_the_listed_schema() {
        let layout = Layout::new();
        layout.add_plugin("demo", &fixture_bytes());
        let set = layout.load(&[]);
        let gate = ScriptedGate::new(vec![]);
        let (tools, diagnostics) = build_for(&layout, &set, &settings(json!({})), gate, &[]);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(
            names(&tools),
            [
                "alloc",
                "chatter",
                "config",
                "echo",
                "fail",
                "fetch",
                "spin",
                "write-data",
            ]
        );
        let echo = tools.iter().find(|t| t.name == "echo").unwrap();
        assert_eq!(echo.label, "echo");
        assert_eq!(echo.parameters["type"], "object");
        assert!(!echo.description.is_empty());
    }

    #[test]
    fn echo_round_trips_arguments_through_the_component() {
        let layout = Layout::new();
        layout.add_plugin("demo", &fixture_bytes());
        let set = layout.load(&[]);
        let (tools, _) = build_for(
            &layout,
            &set,
            &settings(json!({})),
            ScriptedGate::new(vec![]),
            &[],
        );
        let echo = tools.iter().find(|t| t.name == "echo").unwrap();
        let text = run(echo, json!({"x": 1, "list": [2, 3]})).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&text).unwrap(),
            json!({"x": 1, "list": [2, 3]})
        );
        assert!(text.contains('\n'), "an object result is pretty-printed");
    }

    #[test]
    fn config_tool_reads_only_this_plugins_settings() {
        let layout = Layout::new();
        layout.add_plugin("demo", &fixture_bytes());
        layout.add_plugin("other", &fixture_bytes());
        let set = layout.load(&[]);
        let settings = settings(json!({
            "pluginConfig": {
                "demo": {"apiKey": "from-settings"},
                "other": {"apiKey": "not-for-demo"}
            }
        }));
        let (tools, _) = build_for(&layout, &set, &settings, ScriptedGate::new(vec![]), &[]);
        let config = tools.iter().find(|t| t.name == "config").unwrap();
        let read = |key: &str| -> Value {
            serde_json::from_str(&run(config, json!({ "key": key })).unwrap()).unwrap()
        };
        assert_eq!(read("apiKey"), json!({"value": "from-settings"}));
        assert_eq!(read("missing"), json!({"value": null}));
    }

    #[test]
    fn untrusted_plugin_is_skipped_with_a_diagnostic() {
        let layout = Layout::new();
        let root = layout.add_plugin("demo", &fixture_bytes());
        let set = layout.load(&[root]);
        let (tools, diagnostics) = build_for(
            &layout,
            &set,
            &settings(json!({})),
            ScriptedGate::new(vec![]),
            &[],
        );
        assert!(tools.is_empty());
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert!(matches!(diagnostics[0].0, DiagnosticKind::Warning));
        assert!(
            diagnostics[0]
                .1
                .contains("plugin demo: extension not loaded: plugin is not trusted"),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn a_component_that_does_not_load_is_a_diagnostic_not_a_crash() {
        let layout = Layout::new();
        layout.add_plugin("broken", b"not a wasm component");
        let set = layout.load(&[]);
        let (tools, diagnostics) = build_for(
            &layout,
            &set,
            &settings(json!({})),
            ScriptedGate::new(vec![]),
            &[],
        );
        assert!(tools.is_empty());
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert!(diagnostics[0]
            .1
            .contains("plugin broken: extension not loaded:"));
    }

    #[test]
    fn a_name_already_taken_is_skipped_and_never_shadows() {
        let layout = Layout::new();
        layout.add_plugin("demo", &fixture_bytes());
        let set = layout.load(&[]);
        let taken = vec![("echo".to_owned(), ToolOrigin::BuiltIn)];
        let (tools, diagnostics) = build_for(
            &layout,
            &set,
            &settings(json!({})),
            ScriptedGate::new(vec![]),
            &taken,
        );
        assert!(!tools.iter().any(|t| t.name == "echo"));
        assert!(tools.iter().any(|t| t.name == "config"));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(
            diagnostics[0].1,
            "plugin demo tool echo not loaded: echo is already taken by built-in tool"
        );
    }

    #[test]
    fn a_collision_with_another_plugin_names_both_origins() {
        let layout = Layout::new();
        layout.add_plugin("demo", &fixture_bytes());
        let set = layout.load(&[]);
        let other = ToolOrigin::Plugin {
            plugin: "docs".into(),
            tool: "Echo".into(),
        };
        let taken = vec![("echo".to_owned(), other)];
        let (_, diagnostics) = build_for(
            &layout,
            &set,
            &settings(json!({})),
            ScriptedGate::new(vec![]),
            &taken,
        );
        assert!(
            diagnostics.iter().any(|(_, m)| m
                == "plugin demo tool echo not loaded: echo is already taken by plugin docs"),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn a_plugin_tool_name_must_match_the_model_name_rule() {
        assert!(valid_tool_name("WebFetch"));
        assert!(valid_tool_name("write-data"));
        assert!(valid_tool_name(&format!("A{}", "b".repeat(63))));
        assert!(!valid_tool_name(""));
        assert!(!valid_tool_name("1st"));
        assert!(!valid_tool_name("_x"));
        assert!(!valid_tool_name("a.b"));
        assert!(!valid_tool_name("a b"));
        assert!(!valid_tool_name(&format!("A{}", "b".repeat(64))));
    }

    #[test]
    fn net_gate_denial_is_an_error_and_is_asked_once() {
        let gate = ScriptedGate::new(vec![PermissionDecision::Deny {
            reason: "no network for you".into(),
        }]);
        let net = ToolNetGate::new(gate.clone());
        assert_eq!(
            net.check("https://example.com/a"),
            Err("no network for you".into())
        );
        // The same call's next request is refused without asking again.
        assert_eq!(
            net.check("https://example.com/b"),
            Err("no network for you".into())
        );
        let asked = gate.asked();
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].name, "WebFetch");
        assert_eq!(asked[0].arguments, json!({"url": "https://example.com/a"}));
    }

    #[test]
    fn net_gate_grant_lets_later_requests_of_the_call_through_without_asking() {
        let gate = ScriptedGate::new(vec![PermissionDecision::Grant]);
        let net = ToolNetGate::new(gate.clone());
        assert_eq!(net.check("https://example.com/a"), Ok(()));
        assert_eq!(net.check("https://example.com/redirect"), Ok(()));
        assert_eq!(gate.asked().len(), 1);
        // A new call gets a new gate, and asks again.
        let next = ToolNetGate::new(gate.clone());
        assert_eq!(
            next.check("https://example.com/c"),
            Err("no answer scripted".into())
        );
        assert_eq!(gate.asked().len(), 2);
    }

    #[test]
    fn net_gate_grant_for_one_origin_does_not_cover_another() {
        let gate = ScriptedGate::new(vec![
            PermissionDecision::Grant,
            PermissionDecision::Deny {
                reason: "not api.other".into(),
            },
        ]);
        let net = ToolNetGate::new(gate.clone());
        assert_eq!(net.check("https://api.example/a"), Ok(()));
        assert_eq!(net.check("https://api.example/b?x=1"), Ok(()));
        assert_eq!(gate.asked().len(), 1);
        // Another host asks again, and is refused.
        assert_eq!(
            net.check("https://api.other/a"),
            Err("not api.other".into())
        );
        assert_eq!(gate.asked().len(), 2);
        assert_eq!(
            gate.asked()[1].arguments,
            json!({"url": "https://api.other/a"})
        );
    }

    #[test]
    fn net_gate_origin_includes_scheme_and_port() {
        let gate = ScriptedGate::new(vec![
            PermissionDecision::Grant,
            PermissionDecision::Deny {
                reason: "other scheme".into(),
            },
            PermissionDecision::Deny {
                reason: "other port".into(),
            },
        ]);
        let net = ToolNetGate::new(gate.clone());
        assert_eq!(net.check("https://api.example/a"), Ok(()));
        // Same host, other scheme: a different origin.
        assert_eq!(
            net.check("http://api.example/a"),
            Err("other scheme".into())
        );
        // Same host and scheme, other port: a different origin.
        assert_eq!(
            net.check("https://api.example:8443/a"),
            Err("other port".into())
        );
        assert_eq!(gate.asked().len(), 3);
        // The default port is the https port: the same origin as the first request.
        assert_eq!(net.check("https://api.example:443/b"), Ok(()));
        assert_eq!(gate.asked().len(), 3);
    }

    #[test]
    fn net_gate_deny_for_one_origin_is_remembered_for_that_origin_only() {
        let gate = ScriptedGate::new(vec![
            PermissionDecision::Deny {
                reason: "no a".into(),
            },
            PermissionDecision::Grant,
        ]);
        let net = ToolNetGate::new(gate.clone());
        assert_eq!(net.check("https://a.example/1"), Err("no a".into()));
        // Refused again for A without asking.
        assert_eq!(net.check("https://a.example/2"), Err("no a".into()));
        assert_eq!(gate.asked().len(), 1);
        // B is a new origin: it asks, and is granted.
        assert_eq!(net.check("https://b.example/1"), Ok(()));
        assert_eq!(gate.asked().len(), 2);
    }

    #[test]
    fn net_gate_refuses_an_unparseable_url_without_asking() {
        let gate = ScriptedGate::new(vec![PermissionDecision::Grant]);
        let net = ToolNetGate::new(gate.clone());
        assert!(net.check("not a url").is_err());
        assert!(net.check("").is_err());
        assert!(gate.asked().is_empty());
        // The gate still answers a real request.
        assert_eq!(net.check("https://example.com/"), Ok(()));
        assert_eq!(gate.asked().len(), 1);
    }

    #[test]
    fn fetch_through_the_component_asks_the_gate_before_any_network() {
        // A public IP literal: the host's SSRF check passes without DNS, the gate denies, and
        // nothing connects.
        let layout = Layout::new();
        layout.add_plugin("demo", &fixture_bytes());
        let set = layout.load(&[]);
        let gate = ScriptedGate::new(vec![PermissionDecision::Deny {
            reason: "refused in test".into(),
        }]);
        let (tools, _) = build_for(&layout, &set, &settings(json!({})), gate.clone(), &[]);
        let fetch = tools.iter().find(|t| t.name == "fetch").unwrap();
        let error = run(fetch, json!({"url": "https://93.184.216.34/"})).unwrap_err();
        // The component sees the refusal as its HTTP error code, and the gate's reason is added to
        // the tool error, so the model can tell why.
        assert!(error.contains("HttpRequestDenied"), "{error}");
        assert!(error.contains("refused in test"), "{error}");
        let asked = gate.asked();
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].arguments, json!({"url": "https://93.184.216.34/"}));
    }

    #[test]
    fn fetch_refused_by_the_host_names_the_block_without_asking() {
        // A private literal IP is refused by the host's own check, before the gate is asked.
        let layout = Layout::new();
        layout.add_plugin("demo", &fixture_bytes());
        let set = layout.load(&[]);
        let gate = ScriptedGate::new(vec![]);
        let (tools, _) = build_for(&layout, &set, &settings(json!({})), gate.clone(), &[]);
        let fetch = tools.iter().find(|t| t.name == "fetch").unwrap();
        let error = run(fetch, json!({"url": "http://10.1.2.3/"})).unwrap_err();
        assert!(error.contains("HttpRequestDenied"), "{error}");
        assert!(error.contains("10.1.2.3 is private"), "{error}");
        assert!(gate.asked().is_empty());
    }

    #[test]
    fn a_refusal_reason_is_added_only_to_a_refusal() {
        let refused = "HttpRequestDenied".to_owned();
        assert_eq!(
            with_refusal(refused.clone(), Some("user said no".into())),
            "HttpRequestDenied: user said no"
        );
        // No refusal in this call: the error is unchanged.
        assert_eq!(with_refusal(refused.clone(), None), refused);
        // A failure that is not a refusal keeps its own text.
        assert_eq!(
            with_refusal("plugin crashed".into(), Some("user said no".into())),
            "plugin crashed"
        );
    }

    #[test]
    fn a_refusal_reason_stays_with_the_call_that_got_it() {
        let gate = ScriptedGate::new(vec![]);
        let first = ToolNetGate::new(gate.clone());
        let second = ToolNetGate::new(gate.clone());
        first.refused("https://a.example/: a is private");
        assert_eq!(
            first.last_refusal().as_deref(),
            Some("https://a.example/: a is private")
        );
        // A concurrent call has its own: it sees none, and a later refusal replaces the first.
        assert_eq!(second.last_refusal(), None);
        second.refused("https://b.example/: no");
        first.refused("https://a.example/2: second");
        assert_eq!(
            first.last_refusal().as_deref(),
            Some("https://a.example/2: second")
        );
        assert_eq!(
            second.last_refusal().as_deref(),
            Some("https://b.example/: no")
        );
    }

    /// A PEM CA certificate generated for the test (not an invalid placeholder).
    fn test_ca_pem() -> String {
        use rcgen::{BasicConstraints, CertificateParams, CertifiedIssuer, IsCa, KeyPair};
        let mut params = CertificateParams::new(Vec::new()).expect("ca params");
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let key = KeyPair::generate().expect("ca key");
        CertifiedIssuer::self_signed(params, key).expect("ca").pem()
    }

    /// The host the CLI builds from the TLS sources, through the same resolution
    /// the providers use. `configure_global_tls` is process-wide, so the test
    /// resolves and builds directly instead of calling it.
    fn trust_host_for(
        cache: &Path,
        sources: &hoocode_ai_util::tls::TlsSources,
    ) -> Result<WasmHost, WasmError> {
        new_host(cache, &hoocode_ai_util::tls::resolve_trusted_cas(sources))
    }

    /// `--ca-cert <file>`, `HOOCODE_CA_CERT` and `NODE_EXTRA_CA_CERTS` reach the
    /// plugin host's trust set as the generated certificate; the flag beats the
    /// env; an unreadable path warns and leaves the bundled roots only.
    #[test]
    fn ca_cert_sources_reach_the_plugin_host_trust_set() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("cache");
        let pem = test_ca_pem();
        let other = test_ca_pem();
        let ca = dir.path().join("proxy-ca.pem");
        std::fs::write(&ca, &pem).unwrap();
        let other_ca = dir.path().join("other-ca.pem");
        std::fs::write(&other_ca, &other).unwrap();
        let path = ca.display().to_string();
        let other_path = other_ca.display().to_string();
        let expected = vec![hoocode_ai_util::tls::ExtraCa::Pem(pem.clone())];

        // `--ca-cert` through the argv pre-scan.
        let argv = vec!["--ca-cert".to_owned(), path.clone()];
        let flag = hoocode_ai_util::tls::TlsSources::from_args_and_env(&argv, |_| None);
        let host = trust_host_for(&cache, &flag).unwrap();
        assert_eq!(host.tls_trust().extra, expected);
        assert_eq!(host.tls_trust().certificates().len(), 1);

        // `HOOCODE_CA_CERT`.
        let env = hoocode_ai_util::tls::TlsSources::from_args_and_env(&[], |k| {
            (k == "HOOCODE_CA_CERT").then(|| path.clone())
        });
        assert_eq!(
            trust_host_for(&cache, &env).unwrap().tls_trust().extra,
            expected
        );

        // `NODE_EXTRA_CA_CERTS`, the lowest-precedence source.
        let node = hoocode_ai_util::tls::TlsSources::from_args_and_env(&[], |k| {
            (k == "NODE_EXTRA_CA_CERTS").then(|| path.clone())
        });
        assert_eq!(
            trust_host_for(&cache, &node).unwrap().tls_trust().extra,
            expected
        );

        // The flag wins over an env value that points elsewhere.
        let both = hoocode_ai_util::tls::TlsSources::from_args_and_env(&argv, |k| {
            (k == "HOOCODE_CA_CERT").then(|| other_path.clone())
        });
        assert_eq!(
            trust_host_for(&cache, &both).unwrap().tls_trust().extra,
            expected
        );

        // An unreadable path: the host still builds, with the bundled roots only.
        let missing = dir.path().join("missing.pem").display().to_string();
        let bad = hoocode_ai_util::tls::TlsSources::from_args_and_env(&[], |k| {
            (k == "HOOCODE_CA_CERT").then(|| missing.clone())
        });
        assert!(trust_host_for(&cache, &bad)
            .unwrap()
            .tls_trust()
            .extra
            .is_empty());
    }
}

//! Plugin WASM components as agent tools (`docs/design/plugins.md`, PL-9).
//!
//! Each tool a loaded `dev.hoocode/extension.wasm` lists becomes a model tool named
//! `ext_<plugin>_<tool>`. A call runs the component in the host (`hoocode-code-wasm`) on a
//! blocking thread. Its network requests ask the session's permission gate as `WebFetch`, once
//! per origin (scheme, host, port) per tool call. The call itself is not gated.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::OsStr;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use hoocode_agent_mcp::ext_tool_name;
use hoocode_agent_types::{AgentToolCall, AgentToolResult, PermissionDecision, PermissionGate};
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
    taken: &[String],
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
    taken: &[String],
    cache_dir: &Path,
    diagnostics: &mut Diagnostics,
) -> Vec<ToolDefinition> {
    let mut out = Vec::new();
    let mut names: HashSet<String> = taken.iter().cloned().collect();
    let mut host: Option<WasmHost> = None;
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
            match new_host(cache_dir) {
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
            let name = ext_tool_name(&plugin.name, &info.name);
            if !names.insert(name.clone()) {
                diagnostics.push((
                    DiagnosticKind::Warning,
                    format!(
                        "{label}: tool {} not loaded: {name} is already a tool",
                        info.name
                    ),
                ));
                continue;
            }
            out.push(definition(
                name,
                info,
                extension.clone(),
                config.clone(),
                gate.clone(),
            ));
        }
    }
    out
}

/// A host for `cache_dir`, with the test-only loopback switch applied when its variable is `1`.
fn new_host(cache_dir: &Path) -> Result<WasmHost, WasmError> {
    let host = WasmHost::new(cache_dir.to_path_buf())?;
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

/// One model tool for `info`, run through `extension`.
fn definition(
    name: String,
    info: &ToolInfo,
    extension: Arc<Extension>,
    config: ConfigLookup,
    gate: Arc<dyn PermissionGate>,
) -> ToolDefinition {
    let handle = io_handle();
    let tool = info.name.clone();
    ToolDefinition {
        label: name.clone(),
        name,
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
            let env = CallEnv {
                config: config.clone(),
                net: Arc::new(ToolNetGate::new(gate.clone())),
                abort: signal,
            };
            call_blocking(&handle, &extension, &tool, params, env)
        }),
    }
}

/// Runs one call from the tool's blocking thread. The result is the component's JSON as text
/// (pretty-printed for an object). A failure becomes the tool error, with its own message.
fn call_blocking(
    handle: &Handle,
    extension: &Extension,
    tool: &str,
    params: Value,
    env: CallEnv,
) -> Result<AgentToolResult, ToolError> {
    let value = handle
        .block_on(extension.call(tool, params, env))
        .map_err(|error| ToolError::from(error.to_string()))?;
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
}

impl ToolNetGate {
    pub(crate) fn new(gate: Arc<dyn PermissionGate>) -> Self {
        Self {
            gate,
            decisions: Mutex::new(HashMap::new()),
        }
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
        taken: &[String],
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
                "ext_demo_alloc",
                "ext_demo_config",
                "ext_demo_echo",
                "ext_demo_fail",
                "ext_demo_fetch",
                "ext_demo_spin",
                "ext_demo_write-data",
            ]
        );
        let echo = tools.iter().find(|t| t.name == "ext_demo_echo").unwrap();
        assert_eq!(echo.label, "ext_demo_echo");
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
        let echo = tools.iter().find(|t| t.name == "ext_demo_echo").unwrap();
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
        let config = tools.iter().find(|t| t.name == "ext_demo_config").unwrap();
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
        let taken = vec!["ext_demo_echo".to_owned()];
        let (tools, diagnostics) = build_for(
            &layout,
            &set,
            &settings(json!({})),
            ScriptedGate::new(vec![]),
            &taken,
        );
        assert!(!tools.iter().any(|t| t.name == "ext_demo_echo"));
        assert!(tools.iter().any(|t| t.name == "ext_demo_config"));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert!(diagnostics[0].1.contains("tool echo not loaded"));
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
        let fetch = tools.iter().find(|t| t.name == "ext_demo_fetch").unwrap();
        let error = run(fetch, json!({"url": "https://93.184.216.34/"})).unwrap_err();
        // The component sees the refusal as its HTTP error code; the gate's reason goes to the
        // host log, not into the tool result.
        assert!(error.contains("HttpRequestDenied"), "{error}");
        let asked = gate.asked();
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].arguments, json!({"url": "https://93.184.216.34/"}));
    }
}

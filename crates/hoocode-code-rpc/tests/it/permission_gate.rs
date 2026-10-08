#![allow(clippy::disallowed_methods)] // test code: #[tokio::test] expands to a runtime builder
//! rpc mode fails closed (reliability 1.1): a gated tool call that needs
//! approval is denied with a message the model sees, and the tool never runs.
//! The session uses the real `HooPermissionGate` in its fail-closed headless
//! channel, which is what the CLI builds for `--mode rpc`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, Once};
use std::time::Duration;

use hoocode_agent_types::{AgentToolResult, PermissionGate};
use hoocode_ai_provider_faux::{
    faux_assistant_message, faux_tool_call, register_faux_provider, FauxProvider,
    FauxProviderRegistration, FauxResponseStep, RegisterFauxProviderOptions,
};
use hoocode_ai_types::Content;
use hoocode_code_agent_session::{
    create_agent_session, create_agent_session_runtime, AgentSessionServices, BaseTools,
    CreateAgentSessionOptions, CreatedRuntime, RuntimeFactory, RuntimeRequest,
    StaticResourceLoader,
};
use hoocode_code_models::{AuthLookup, ModelRegistry};
use hoocode_code_permissions::{ApprovalChannel, HooPermissionGate};
use hoocode_code_rpc::client::{RpcClient, RpcClientOptions};
use hoocode_code_rpc::{run_rpc_mode, serialize_json_line, RuntimeHost};
use hoocode_code_session::SessionManager;
use hoocode_code_settings::SettingsManager;
use hoocode_code_tool_api::ToolDefinition;
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;

const WAIT: Duration = Duration::from_secs(10);
const DENIAL: &str = "needs approval, but no client can answer in this session";

fn isolate_agent_dir() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let dir =
            std::env::temp_dir().join(format!("hoocode-rpc-gate-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("HOOCODE_CODING_AGENT_DIR", &dir);
    });
}

struct FauxAuth(String);

impl AuthLookup for FauxAuth {
    fn api_key(&self, provider: &str) -> Option<String> {
        (provider == self.0).then(|| "faux-key".to_string())
    }
}

/// A `bash` tool that records that it ran, so a test can see a denied call
/// never reached it.
fn recording_bash(ran: Arc<Mutex<Vec<String>>>) -> ToolDefinition {
    ToolDefinition {
        ordered_start: false,
        background_when: None,
        name: "Shell".into(),
        label: "Shell".into(),
        description: "bash tool".into(),
        prompt_snippet: None,
        prompt_guidelines: vec![],
        parameters: json!({"type": "object", "properties": {"command": {"type": "string"}}}),
        prepare_arguments: None,
        execution_mode: None,
        background: false,
        execute: Arc::new(move |_id, params, _signal, _update, _ctx| {
            let command = params["command"].as_str().unwrap_or_default().to_string();
            ran.lock().unwrap().push(command.clone());
            Ok(AgentToolResult {
                content: vec![Content::text(format!("ran {command}"))],
                details: json!({}),
                terminate: false,
            })
        }),
    }
}

struct Setup {
    client: RpcClient,
    ran: Arc<Mutex<Vec<String>>>,
    _registration: FauxProviderRegistration,
    _temp: tempfile::TempDir,
}

/// RPC mode on the faux provider with the rpc gate (fail closed, no UI).
/// `faux_steps` are the model's responses in order.
async fn setup(faux_steps: Vec<FauxResponseStep>, gate: Arc<dyn PermissionGate>) -> Setup {
    isolate_agent_dir();
    let temp = tempfile::tempdir().unwrap();
    let dir: PathBuf = temp.path().canonicalize().unwrap();
    let registration = register_faux_provider(RegisterFauxProviderOptions::default());
    let faux: Arc<FauxProvider> = registration.provider().clone();
    faux.set_responses(faux_steps);
    let ran = Arc::new(Mutex::new(Vec::new()));
    let factory: RuntimeFactory = {
        let faux = faux.clone();
        let agent_dir = dir.clone();
        let ran = ran.clone();
        Arc::new(move |request: RuntimeRequest| {
            let faux = faux.clone();
            let agent_dir = agent_dir.clone();
            let ran = ran.clone();
            let gate = gate.clone();
            Box::pin(async move {
                let model = faux.get_model();
                let services = AgentSessionServices {
                    cwd: request.cwd.clone(),
                    agent_dir,
                    auth: Arc::new(FauxAuth(model.provider.clone())),
                    settings: Arc::new(Mutex::new(SettingsManager::in_memory(Default::default()))),
                    model_registry: Arc::new(ModelRegistry::in_memory()),
                    resource_loader: Arc::new(StaticResourceLoader::default()),
                    diagnostics: vec![],
                };
                let created = create_agent_session(
                    &services,
                    request.session_manager,
                    CreateAgentSessionOptions {
                        model: Some(model),
                        base_tools: Some(BaseTools::Override(vec![])),
                        custom_tools: vec![recording_bash(ran)],
                        permission_gate: Some(gate),
                        stream_fn: Some(Arc::new(faux.stream_fn())),
                        session_start_event: request.session_start_event,
                        ..Default::default()
                    },
                );
                Ok(CreatedRuntime {
                    session: created.session,
                    diagnostics: vec![],
                    services,
                    model_fallback_message: created.model_fallback_message,
                })
            })
        })
    };
    let session_manager = SessionManager::create(dir.to_string_lossy(), Some(dir.join("sessions")));
    let runtime = create_agent_session_runtime(
        factory,
        RuntimeRequest {
            cwd: dir.clone(),
            agent_dir: dir,
            session_manager,
            session_start_event: None,
        },
    )
    .await
    .unwrap();

    let (client_writer, agent_reader) = tokio::io::duplex(1 << 16);
    let (agent_writer, client_reader) = tokio::io::duplex(1 << 16);
    let (line_tx, mut line_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        let mut agent_writer = agent_writer;
        while let Some(line) = line_rx.recv().await {
            if agent_writer.write_all(line.as_bytes()).await.is_err() {
                break;
            }
        }
    });
    tokio::spawn(run_rpc_mode(
        Arc::new(RuntimeHost::new(runtime)),
        agent_reader,
        Arc::new(move |value: &Value| {
            let _ = line_tx.send(serialize_json_line(value));
        }),
    ));
    let client = RpcClient::attach(RpcClientOptions::default(), client_reader, client_writer);
    Setup {
        client,
        ran,
        _registration: registration,
        _temp: temp,
    }
}

fn bash_call(command: &str) -> FauxResponseStep {
    FauxResponseStep::Message(faux_assistant_message(
        vec![faux_tool_call(
            "Shell",
            json!({"command": command}),
            Some("call-1".into()),
        )],
        Default::default(),
    ))
}

fn reply(text: &str) -> FauxResponseStep {
    FauxResponseStep::Message(faux_assistant_message(text, Default::default()))
}

fn rpc_gate(cwd: &std::path::Path) -> Arc<dyn PermissionGate> {
    Arc::new(HooPermissionGate::new(
        cwd.to_path_buf(),
        ApprovalChannel::Headless { fail_closed: true },
        None,
    ))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_gated_tool_call_is_denied_in_rpc_mode_and_never_runs() {
    // The gate reads the project config from its cwd: an empty dir has none.
    let cwd = tempfile::tempdir().unwrap();
    let s = setup(
        vec![bash_call("echo hi"), reply("I could not run it.")],
        rpc_gate(cwd.path()),
    )
    .await;

    let events = s
        .client
        .prompt_and_wait("run echo", None, WAIT)
        .await
        .unwrap();
    assert_eq!(events.last().unwrap()["type"], "agent_end");

    assert!(s.ran.lock().unwrap().is_empty(), "the denied tool ran");
    let messages = serde_json::to_string(&s.client.get_messages().await.unwrap()).unwrap();
    assert!(
        messages.contains(DENIAL),
        "denial not returned to the model: {messages}"
    );
    assert!(messages.contains("Add it to auto_allow"), "{messages}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_auto_allowed_tool_still_runs_in_rpc_mode() {
    let cwd = tempfile::tempdir().unwrap();
    let project = cwd.path().join(hoocode_code_paths::CONFIG_DIR_NAME);
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("hoo-config.json"),
        r#"{"active_mode":"build","modes":{"build":{"auto_allow":["Shell"]}}}"#,
    )
    .unwrap();
    let s = setup(
        vec![bash_call("echo hi"), reply("done")],
        rpc_gate(cwd.path()),
    )
    .await;

    s.client
        .prompt_and_wait("run echo", None, WAIT)
        .await
        .unwrap();
    assert_eq!(*s.ran.lock().unwrap(), vec!["echo hi".to_string()]);
}

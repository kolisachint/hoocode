//! Shared helpers for the `Mode` tests: a `Mode` over a session whose agent never runs.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use hoocode_code_agent_session::{
    AgentSession, AgentSessionConfig, BaseTools, StaticResourceLoader,
};
use hoocode_code_session::SessionManager;

use super::*;

/// A terminal that draws nothing: the tests read the component tree.
pub(super) struct NullTerminal;

impl hoocode_tui_terminal::Terminal for NullTerminal {
    fn start(&mut self, _: Box<dyn FnMut(&str) + Send>, _: Box<dyn FnMut() + Send>) {}
    fn stop(&mut self) {}
    fn drain_input(&mut self, _: std::time::Duration, _: std::time::Duration) {}
    fn write(&mut self, _: &str) {}
    fn columns(&self) -> u16 {
        100
    }
    fn rows(&self) -> u16 {
        40
    }
    fn kitty_protocol_active(&self) -> bool {
        false
    }
    fn move_by(&mut self, _: i32) {}
    fn hide_cursor(&mut self) {}
    fn show_cursor(&mut self) {}
    fn clear_line(&mut self) {}
    fn clear_from_cursor(&mut self) {}
    fn clear_screen(&mut self) {}
    fn set_title(&mut self, _: &str) {}
    fn set_progress(&mut self, _: bool) {}
}

pub(super) struct NoAuth;

impl hoocode_code_models::AuthLookup for NoAuth {
    fn api_key(&self, _: &str) -> Option<String> {
        None
    }
}

/// A `Mode` over a session whose agent never runs. With `session_file`, the
/// session is persisted there, so its runs can be attached to.
pub(super) fn test_mode(session_file: Option<&Path>) -> Mode {
    test_mode_in(session_file, std::env::temp_dir())
}

/// [`test_mode`] with the session working in `cwd` (the scheduler's store lives there).
pub(super) fn test_mode_in(session_file: Option<&Path>, cwd: PathBuf) -> Mode {
    let agent = Arc::new(hoocode_agent_core::Agent::with_options(
        hoocode_agent_core::AgentOptions::default(),
    ));
    mode_over(agent, Arc::new(NoAuth), session_file, cwd)
}

/// Auth for the faux model only: every provider has a key.
pub(super) struct FauxAuth;

impl hoocode_code_models::AuthLookup for FauxAuth {
    fn api_key(&self, _: &str) -> Option<String> {
        Some("faux-key".into())
    }
}

/// A `Mode` whose agent streams one turn (`prompt("start")`). The faux model
/// answers only when `gate` is notified, so the session stays busy until then.
pub(super) fn streaming_mode(gate: Arc<tokio::sync::Notify>) -> Mode {
    let faux = hoocode_ai_provider_faux::FauxProvider::new();
    faux.set_responses(vec![
        hoocode_ai_provider_faux::FauxResponseStep::async_factory(move |_, _, _, _| async move {
            gate.notified().await;
            Ok(hoocode_ai_provider_faux::faux_assistant_message(
                "done",
                Default::default(),
            ))
        }),
    ]);
    let agent = Arc::new(hoocode_agent_core::Agent::with_options(
        hoocode_agent_core::AgentOptions {
            initial_state: Some(hoocode_agent_types::AgentState {
                system_prompt: "You are a test assistant.".into(),
                model: faux.get_model(),
                thinking_level: hoocode_ai_types::ThinkingLevel::Off,
                tools: hoocode_agent_types::AgentTools::new(vec![]),
                messages: vec![],
                is_streaming: false,
                streaming_message: None,
                pending_tool_calls: Default::default(),
                error_message: None,
            }),
            stream_fn: Some(Arc::new(faux.stream_fn())),
            api_key: Some("faux-key".into()),
            ..Default::default()
        },
    ));
    mode_over(agent, Arc::new(FauxAuth), None, std::env::temp_dir())
}

/// The `Mode` over `agent`, with `auth` for its model and the session in `cwd`.
fn mode_over(
    agent: Arc<hoocode_agent_core::Agent>,
    auth: Arc<dyn hoocode_code_models::AuthLookup + Send + Sync>,
    session_file: Option<&Path>,
    cwd: PathBuf,
) -> Mode {
    let session_manager = match session_file {
        Some(dir) => SessionManager::create("/work", Some(dir.into())),
        None => SessionManager::in_memory("/work"),
    };
    // The theme is `dark`, as the layer goldens load it: `Mode::new` loads
    // the settings' theme into the process-wide state.
    let settings = hoocode_code_settings::SettingsManager::in_memory(
        serde_json::json!({"theme": "dark"})
            .as_object()
            .cloned()
            .unwrap_or_default(),
    );
    let session = AgentSession::new(AgentSessionConfig {
        agent,
        session_manager,
        settings: Arc::new(Mutex::new(settings)),
        cwd,
        scoped_models: vec![],
        resource_loader: Arc::new(StaticResourceLoader::default()),
        custom_tools: vec![],
        model_registry: Arc::new(hoocode_code_models::ModelRegistry::in_memory()),
        auth,
        initial_active_tool_names: None,
        allowed_tool_names: None,
        disallowed_tool_names: None,
        base_tools: BaseTools::Override(vec![]),
        extensions: None,
        session_start_event: None,
    });
    Mode::new(InteractiveOptions {
        session,
        session_runtime: None,
        runtime: hoocode_runtime::io_handle(),
        listing: Box::new(|_| Default::default()),
        auth_storage: Arc::new(hoocode_code_auth::AuthStorage::in_memory([])),
        version: "test".into(),
        verbose: false,
        initial_message: None,
        initial_images: vec![],
        initial_messages: vec![],
        model_fallback_message: None,
        startup_warnings: vec![],
        terminal: Some(Box::new(NullTerminal)),
        perf_log: None,
        update_notice: None,
    })
}

#![allow(clippy::disallowed_methods)] // test code: #[tokio::test] expands to a runtime builder
//! Waking an idle agent for finished background shell jobs: the interactive app
//! asks the session whether a wake is due (`job_wake_pending`) and sends the
//! generic wake message as a prompt. Nothing is taken at that point: the
//! prompt's model call carries the notices, so a completion reaches the model
//! once, and a wake that never reaches a model call loses nothing.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, Instant};

use hoocode_ai_provider_faux::{
    faux_assistant_message, register_faux_provider, FauxMessageOptions, FauxModelDefinition,
    FauxProvider, FauxProviderRegistration, FauxResponseStep, RegisterFauxProviderOptions,
};
use hoocode_ai_types::{Content, Message, StopReason};
use hoocode_code_agent_session::{
    create_agent_session, AgentSession, AgentSessionServices, BaseTools, CreateAgentSessionOptions,
    PromptOptions, StaticResourceLoader,
};
use hoocode_code_models::{AuthLookup, ModelRegistry};
use hoocode_code_session::SessionManager;
use hoocode_code_settings::SettingsManager;
use hoocode_code_tool_bash::{get_shell_env, JobRegistry, LocalBashOperations};

fn isolate_agent_dir() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let dir =
            std::env::temp_dir().join(format!("hoocode-job-wake-test-{}", std::process::id()));
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

struct Setup {
    session: AgentSession,
    jobs: JobRegistry,
    faux: Arc<FauxProvider>,
    dir: PathBuf,
    _temp: tempfile::TempDir,
    _registration: FauxProviderRegistration,
}

fn write_models_json(dir: &Path, faux: &FauxProvider) {
    let model = faux.get_model();
    let models: Vec<_> = faux
        .models()
        .iter()
        .map(|m| {
            serde_json::json!({
                "id": m.id, "name": m.name, "reasoning": m.reasoning, "input": m.input,
                "contextWindow": m.context_window, "maxTokens": m.max_tokens,
            })
        })
        .collect();
    let config = serde_json::json!({"providers": {model.provider.clone(): {
        "baseUrl": model.base_url, "apiKey": "faux-key", "api": faux.api(), "models": models,
    }}});
    std::fs::write(dir.join("models.json"), config.to_string()).unwrap();
}

fn setup(with_jobs: bool) -> Setup {
    isolate_agent_dir();
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().canonicalize().unwrap();
    let registration = register_faux_provider(RegisterFauxProviderOptions {
        models: vec![FauxModelDefinition {
            id: "faux-1".into(),
            ..Default::default()
        }],
        ..Default::default()
    });
    let faux = registration.provider().clone();
    write_models_json(&dir, &faux);
    let services = AgentSessionServices {
        cwd: dir.clone(),
        agent_dir: dir.clone(),
        auth: Arc::new(FauxAuth(faux.get_model().provider)),
        settings: Arc::new(Mutex::new(SettingsManager::in_memory(Default::default()))),
        model_registry: Arc::new(ModelRegistry::create(dir.join("models.json"))),
        resource_loader: Arc::new(StaticResourceLoader::default()),
        diagnostics: vec![],
    };
    let jobs = JobRegistry::new(dir.join("jobs"), LocalBashOperations::new(None));
    let created = create_agent_session(
        &services,
        SessionManager::in_memory(dir.to_string_lossy()),
        CreateAgentSessionOptions {
            model: Some(faux.get_model()),
            base_tools: Some(BaseTools::Override(vec![])),
            background_jobs: with_jobs.then(|| jobs.clone()),
            stream_fn: Some(Arc::new(faux.stream_fn())),
            ..Default::default()
        },
    );
    Setup {
        session: created.session,
        jobs,
        faux,
        dir,
        _temp: temp,
        _registration: registration,
    }
}

/// Runs `command` as a job to its end and returns its id.
fn finished_job(s: &Setup, command: &str) -> String {
    let id = s
        .jobs
        .spawn(command, &s.dir, get_shell_env())
        .expect("spawn")
        .id;
    let deadline = Instant::now() + Duration::from_secs(15);
    while s.jobs.info(&id).is_some_and(|i| i.state.is_running()) {
        assert!(Instant::now() < deadline, "job {id} did not finish");
        std::thread::sleep(Duration::from_millis(10));
    }
    id
}

/// Prompts the session once and returns the user texts the model call saw.
async fn prompt_and_see_context(s: &Setup, text: &str) -> Vec<String> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    s.faux
        .set_responses(vec![FauxResponseStep::factory(move |context, _, _, _| {
            let texts = context
                .messages
                .iter()
                .filter_map(|m| match m {
                    Message::User(u) => Some(
                        u.content
                            .blocks()
                            .iter()
                            .filter_map(|c| match c {
                                Content::Text(t) => Some(t.text.clone()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n"),
                    ),
                    _ => None,
                })
                .collect();
            *sink.lock().unwrap() = texts;
            Ok(faux_assistant_message("ok", Default::default()))
        })]);
    s.session
        .prompt(text, PromptOptions::default())
        .await
        .expect("prompt");
    let texts = seen.lock().unwrap().clone();
    texts
}

fn mentions(texts: &[String], id: &str) -> usize {
    texts
        .iter()
        .filter(|t| t.contains(&format!("Background shell job {id} ")))
        .count()
}

const WAKE: &str = hoocode_code_agent_session::JOB_WAKE_MESSAGE;

#[tokio::test(flavor = "multi_thread")]
async fn a_finished_job_wakes_an_idle_agent_and_the_wake_call_carries_the_notice_once() {
    let s = setup(true);
    let id = finished_job(&s, "echo done");

    assert!(WAKE.contains("not typed by the user"), "{WAKE}");
    assert!(s.session.job_wake_pending());
    // Asking takes nothing: still due until a model call carries the notice.
    assert!(s.session.job_wake_pending());

    let seen = prompt_and_see_context(&s, WAKE).await;
    assert_eq!(mentions(&seen, &id), 1, "{seen:?}");
    assert!(
        seen.iter()
            .any(|t| t.contains(&format!("Background shell job {id} finished (exit 0)"))),
        "{seen:?}"
    );
    assert!(!s.session.job_wake_pending());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wake_call_that_fails_does_not_use_up_the_notice() {
    let s = setup(true);
    let id = finished_job(&s, "true");

    s.faux
        .set_responses(vec![FauxResponseStep::Message(faux_assistant_message(
            "boom",
            FauxMessageOptions {
                stop_reason: Some(StopReason::Error),
                error_message: Some("boom".into()),
                ..Default::default()
            },
        ))]);
    let _ = s.session.prompt(WAKE, PromptOptions::default()).await;

    let seen = prompt_and_see_context(&s, "try again").await;
    assert_eq!(mentions(&seen, &id), 1, "{seen:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_notice_a_model_call_already_took_does_not_wake_the_agent_later() {
    let s = setup(true);
    let id = finished_job(&s, "true");

    // The job finished during a turn: the next model call carries the notice.
    let seen = prompt_and_see_context(&s, "go on").await;
    assert_eq!(mentions(&seen, &id), 1, "{seen:?}");
    assert!(!s.session.job_wake_pending());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_killed_job_does_not_wake_the_agent_but_its_notice_still_reaches_the_next_call() {
    let s = setup(true);
    let id = s
        .jobs
        .spawn("sleep 30", &s.dir, get_shell_env())
        .expect("spawn")
        .id;
    s.jobs.kill(&id).expect("kill");

    assert!(!s.session.job_wake_pending());
    let seen = prompt_and_see_context(&s, "next").await;
    assert_eq!(mentions(&seen, &id), 1, "{seen:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_running_job_and_a_session_without_jobs_never_wake() {
    let s = setup(true);
    let id = s
        .jobs
        .spawn("sleep 30", &s.dir, get_shell_env())
        .expect("spawn")
        .id;
    assert!(!s.session.job_wake_pending());
    s.jobs.kill(&id).unwrap();

    let plain = setup(false);
    assert!(!plain.session.job_wake_pending());
}

#[tokio::test(flavor = "multi_thread")]
async fn two_jobs_finishing_before_the_wake_arrive_in_finish_order_in_one_call() {
    let s = setup(true);
    let a = finished_job(&s, "true");
    let b = finished_job(&s, "exit 3");

    assert!(s.session.job_wake_pending());
    let seen = prompt_and_see_context(&s, WAKE).await;
    let joined = seen.join("\n");
    let (at_a, at_b) = (
        joined.find(&format!("job {a} ")).expect("a"),
        joined.find(&format!("job {b} ")).expect("b"),
    );
    assert!(at_a < at_b, "{joined}");
    assert!(joined.contains("exit 3"), "{joined}");
}

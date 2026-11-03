#![allow(clippy::disallowed_methods)] // test code: #[tokio::test] expands to a runtime builder
//! Default-model fallback (`model_fallback.rs`) end to end on the faux provider.

use crate::common::{Harness, HarnessOptions};
use hoocode_ai_provider_faux::{
    faux_assistant_message, FauxMessageOptions, FauxModelDefinition, FauxResponseStep,
};
use hoocode_ai_types::StopReason;
use hoocode_code_agent_session::{AgentSessionEvent, CycleDirection, PromptOptions};
use hoocode_code_models::ScopedModel;

const REFUSED: &str = "429 Free usage limit reached. Free models work in OpenCode";

fn harness(retry_enabled: bool) -> Harness {
    let mut settings = serde_json::Map::new();
    settings.insert(
        "retry".into(),
        serde_json::json!({"enabled": retry_enabled, "maxRetries": 3, "baseDelayMs": 1}),
    );
    let h = Harness::new(HarnessOptions {
        settings,
        models: vec![
            FauxModelDefinition::new("picked"),
            FauxModelDefinition::new("next"),
        ],
        ..Default::default()
    });
    let next = h.faux.get_model_by_id("next").unwrap();
    h.session.set_model_fallbacks(vec![next]);
    h
}

fn error(message: &str) -> FauxResponseStep {
    faux_assistant_message(
        "",
        FauxMessageOptions {
            stop_reason: Some(StopReason::Error),
            error_message: Some(message.into()),
            ..Default::default()
        },
    )
    .into()
}

fn text(t: &str) -> FauxResponseStep {
    faux_assistant_message(t, Default::default()).into()
}

fn fallbacks(h: &Harness) -> Vec<(String, String)> {
    h.events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            AgentSessionEvent::ModelFallback { from, to, .. } => Some((from.clone(), to.clone())),
            _ => None,
        })
        .collect()
}

fn retry_starts(h: &Harness) -> usize {
    h.events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| matches!(e, AgentSessionEvent::AutoRetryStart { .. }))
        .count()
}

async fn check_switches_and_waits(retry_enabled: bool) {
    let h = harness(retry_enabled);
    let provider = h.faux.get_model().provider;
    h.set_responses(vec![error(REFUSED), text("ok")]);
    h.session
        .prompt("hi", PromptOptions::default())
        .await
        .unwrap();
    // `prompt` returned only after the switched model answered.
    assert_eq!(h.assistant_texts(), ["ok"]);
    assert_eq!(h.faux.call_count(), 2);
    assert_eq!(retry_starts(&h), 0, "a refused model is not retried");
    assert_eq!(
        fallbacks(&h),
        [(format!("{provider}/picked"), format!("{provider}/next"))]
    );
    assert_eq!(h.session.model().unwrap().id, "next");
    assert!(!h.session.is_retrying());
    assert!(!h.session.has_model_fallbacks());
    let settings = h.session.settings();
    assert_eq!(
        settings.default_provider().as_deref(),
        Some(provider.as_str())
    );
    assert_eq!(settings.default_model().as_deref(), Some("next"));
}

#[tokio::test(flavor = "multi_thread")]
async fn switches_and_waits_for_the_answer_with_retry_disabled() {
    check_switches_and_waits(false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn switches_before_retrying_a_refused_model() {
    check_switches_and_waits(true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn cycling_the_model_turns_the_fallback_off() {
    let h = harness(false);
    let models: Vec<ScopedModel> = ["picked", "next"]
        .iter()
        .map(|id| ScopedModel {
            model: h.faux.get_model_by_id(id).unwrap(),
            thinking_level: None,
        })
        .collect();
    h.session.set_scoped_models(models);
    assert!(h.session.has_model_fallbacks());
    h.session.cycle_model(CycleDirection::Forward).unwrap();
    assert!(!h.session.has_model_fallbacks());
    // The model the user cycled to is not switched away from, nor saved.
    h.set_responses(vec![error(REFUSED)]);
    h.session
        .prompt("hi", PromptOptions::default())
        .await
        .unwrap();
    assert!(fallbacks(&h).is_empty());
    assert_eq!(h.session.settings().default_model(), None);
}

#![allow(clippy::disallowed_methods)] // test code: #[tokio::test] expands to a runtime builder
//! RT-39 session naming: a cheap model names the session after a run. Covers
//! the user-name guard, failures, the KEEP and new-name replies, the
//! interactive first-run trigger, and the two pure helpers.

use std::time::Duration;

use crate::common::{Harness, HarnessOptions};
use hoocode_ai_provider_faux::{faux_assistant_message, FauxMessageOptions, FauxResponseStep};
use hoocode_ai_types::{Model, StopReason};
use hoocode_code_agent_session::{
    parse_name_reply, pick_naming_model, AutoNameMode, PromptOptions,
};
use hoocode_code_models::ResolvedScoped;
use hoocode_code_session::{FileEntry, NameSource, SessionManager};

fn text(t: &str) -> FauxResponseStep {
    faux_assistant_message(t, Default::default()).into()
}

fn failing() -> FauxResponseStep {
    faux_assistant_message(
        "",
        FauxMessageOptions {
            stop_reason: Some(StopReason::Error),
            error_message: Some("provider exploded".into()),
            ..Default::default()
        },
    )
    .into()
}

/// A naming session in `mode` that persists to a session file in the temp dir.
fn persisted(mode: AutoNameMode) -> HarnessOptions {
    HarnessOptions {
        auto_name: Some(mode),
        session_manager: Some(Box::new(|dir: &std::path::Path| {
            SessionManager::create(dir.to_string_lossy(), Some(dir.join("sessions")))
        })),
        ..Default::default()
    }
}

fn headless() -> HarnessOptions {
    persisted(AutoNameMode::Headless)
}

/// The `source` of every entry that defines a name, oldest first.
fn name_sources(h: &Harness) -> Vec<Option<String>> {
    h.session
        .session_manager()
        .entries()
        .iter()
        .filter_map(|e| match e {
            FileEntry::SessionInfo {
                name: Some(_),
                source,
                ..
            } => Some(source.clone()),
            _ => None,
        })
        .collect()
}

async fn wait_for_name(h: &Harness) {
    for _ in 0..200 {
        if h.session.session_name().is_some() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the session was never named");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_user_name_is_kept_without_a_model_call() {
    let h = Harness::new(headless());
    h.set_responses(vec![text("one"), text("Should not be used")]);
    h.session
        .prompt("one", PromptOptions::default())
        .await
        .unwrap();
    h.session.set_session_name("My name");
    let pending = h.pending_response_count();
    let entries = name_sources(&h).len();

    h.session.auto_name_now().await;

    assert_eq!(h.pending_response_count(), pending, "a model call was made");
    assert_eq!(name_sources(&h).len(), entries, "an entry was written");
    assert_eq!(h.session.session_name().as_deref(), Some("My name"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_provider_error_changes_nothing() {
    let h = Harness::new(headless());
    h.set_responses(vec![text("one"), failing()]);
    h.session
        .prompt("one", PromptOptions::default())
        .await
        .unwrap();

    h.session.auto_name_now().await;

    assert_eq!(
        h.pending_response_count(),
        0,
        "the naming call was not made"
    );
    assert!(name_sources(&h).is_empty());
    assert_eq!(h.session.session_name(), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn keep_writes_nothing_and_a_new_name_is_stored_as_auto() {
    let h = Harness::new(headless());
    h.set_responses(vec![text("one"), text("KEEP"), text("Fix login bug")]);
    h.session
        .prompt("one", PromptOptions::default())
        .await
        .unwrap();

    h.session.auto_name_now().await;
    assert!(name_sources(&h).is_empty(), "KEEP wrote an entry");

    h.session.auto_name_now().await;
    assert_eq!(name_sources(&h), [Some("auto".to_string())]);
    assert_eq!(h.session.session_name().as_deref(), Some("Fix login bug"));
    assert_eq!(
        h.session.session_manager().session_name_source(),
        Some(NameSource::Auto)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_interactive_session_names_after_its_first_run_only() {
    let h = Harness::new(persisted(AutoNameMode::Interactive));
    h.set_responses(vec![
        text("one"),
        text("First name"),
        text("two"),
        // Left unused when the second run does not trigger naming.
        text("Second name"),
    ]);
    h.session
        .prompt("one", PromptOptions::default())
        .await
        .unwrap();
    wait_for_name(&h).await;
    assert_eq!(h.session.session_name().as_deref(), Some("First name"));

    h.session
        .prompt("two", PromptOptions::default())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;

    assert_eq!(h.pending_response_count(), 1, "a second naming call ran");
    assert_eq!(h.session.session_name().as_deref(), Some("First name"));
}

#[test]
fn picks_the_cheapest_scoped_model_on_the_same_provider() {
    let h = Harness::new(HarnessOptions::default());
    let model = |id: &str, provider: &str, cost: f64| {
        let mut m: Model = h.faux.get_model();
        m.id = id.into();
        m.provider = provider.into();
        m.cost.input = cost;
        m.cost.output = cost;
        m
    };
    let scoped = |m: Model| ResolvedScoped {
        model: m,
        alias: None,
        category: None,
        effort: None,
    };
    let current = model("big", "acme", 10.0);
    let scope = vec![
        scoped(model("elsewhere", "other", 0.1)),
        scoped(model("mid", "acme", 3.0)),
        scoped(model("small", "acme", 1.0)),
    ];
    assert_eq!(pick_naming_model(&current, &scope).id, "small");
    assert_eq!(pick_naming_model(&current, &[]).id, "big");
    let only_other = vec![scoped(model("elsewhere", "other", 0.1))];
    assert_eq!(pick_naming_model(&current, &only_other).id, "big");
}

#[test]
fn parses_replies_into_names() {
    assert_eq!(parse_name_reply("KEEP", None), None);
    assert_eq!(parse_name_reply("keep", None), None);
    assert_eq!(parse_name_reply("   ", None), None);
    assert_eq!(parse_name_reply("", None), None);
    assert_eq!(
        parse_name_reply("  \"Fix login bug.\"  ", None).as_deref(),
        Some("Fix login bug")
    );
    assert_eq!(
        parse_name_reply("`Fix login bug`", None).as_deref(),
        Some("Fix login bug")
    );
    assert_eq!(
        parse_name_reply("\n\nFix login bug\nsecond line", None).as_deref(),
        Some("Fix login bug")
    );
    assert_eq!(
        parse_name_reply("Fix login bug", Some("Fix login bug")),
        None
    );
}

#[test]
fn caps_long_names_on_a_char_boundary() {
    let long = "é".repeat(70);
    let name = parse_name_reply(&long, None).unwrap();
    assert_eq!(name.chars().count(), 60);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_in_memory_session_makes_no_naming_call() {
    // `--no-session`: nothing is persisted, so the name could not be kept. No model call.
    let h = Harness::new(HarnessOptions {
        auto_name: Some(AutoNameMode::Headless),
        ..Default::default()
    });
    h.set_responses(vec![text("one"), text("Should not be used")]);
    h.session
        .prompt("one", PromptOptions::default())
        .await
        .unwrap();
    let pending = h.pending_response_count();

    h.session.auto_name_now().await;

    assert_eq!(h.pending_response_count(), pending, "a model call was made");
    assert!(name_sources(&h).is_empty());
    assert_eq!(h.session.session_name(), None);
}

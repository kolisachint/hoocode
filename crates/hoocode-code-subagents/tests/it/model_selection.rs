//! Scoped-model selection for the Agent tool (`select_model`): ask, pin,
//! default, tier fallback, the hard scope limit, and effort.

use hoocode_ai_types::{Model, ThinkingLevel};
use hoocode_code_models::{clamp_effort, resolve_scoped_models, ResolvedScoped};
use hoocode_code_settings::{ModelCategoryName, ScopedModel};
use hoocode_code_subagents::instance::{
    current_scope, scoped_models_override_active, scoped_models_override_flag,
    set_scoped_models_override,
};
use hoocode_code_subagents::model_categories::{select_model, ModelRequest};
use hoocode_code_subagents::tools::create_task_tool_definition;
use serde_json::json;

fn model(provider: &str, id: &str, reasoning: bool, price: f64) -> Model {
    serde_json::from_value(json!({
        "id": id, "name": id, "api": "anthropic-messages", "provider": provider,
        "baseUrl": "https://example.test", "reasoning": reasoning, "input": ["text"],
        "cost": {"input": price, "output": price, "cacheRead": 0, "cacheWrite": 0},
        "contextWindow": 128000, "maxTokens": 8192,
    }))
    .unwrap()
}

/// By combined price: tiny(1) < mid(6) < big(30); `solo` is another provider.
fn available() -> Vec<Model> {
    vec![
        model("acme", "tiny", false, 0.5),
        model("acme", "mid", false, 3.0),
        model("acme", "big", true, 15.0),
        model("other", "solo", false, 1.0),
    ]
}

fn entry(
    model: &str,
    category: Option<ModelCategoryName>,
    alias: Option<&str>,
    effort: Option<&str>,
) -> ScopedModel {
    ScopedModel {
        model: model.into(),
        effort: effort.map(String::from),
        category,
        alias: alias.map(String::from),
    }
}

fn scope(entries: &[ScopedModel]) -> Vec<ResolvedScoped> {
    resolve_scoped_models(entries, &available())
}

fn req(ask: Option<&str>, pin: Option<&str>, inherited: Option<&str>) -> ModelRequest {
    ModelRequest {
        ask: ask.map(String::from),
        effort: None,
        pin: pin.map(String::from),
        inherited: inherited.map(String::from),
    }
}

fn pick(
    request: &ModelRequest,
    scoped: &[ResolvedScoped],
) -> hoocode_code_subagents::model_categories::ModelSelection {
    select_model(request, None, &available(), scoped)
}

// --- schema ----------------------------------------------------------------

#[test]
fn the_agent_schema_has_model_and_effort_and_no_complexity() {
    let dir = tempfile::tempdir().unwrap();
    let tool = create_task_tool_definition(dir.path());
    let props = tool.parameters["properties"].as_object().unwrap();
    assert!(props.contains_key("model"));
    assert!(props.contains_key("effort"));
    assert!(
        !props.contains_key("complexity"),
        "complexity is removed, not aliased"
    );
    assert_eq!(
        props["effort"]["enum"],
        json!(["off", "minimal", "low", "medium", "high", "xhigh"])
    );
}

// --- ask, pin, default -------------------------------------------------------

#[test]
fn an_explicit_ask_beats_the_frontmatter_pin() {
    let s = select_model(
        &req(Some("acme/mid"), Some("acme/big"), None),
        None,
        &available(),
        &[],
    );
    assert_eq!(s.model.as_deref(), Some("acme/mid"));
    assert!(s.error.is_none());
}

#[test]
fn the_pin_is_used_when_nothing_is_asked() {
    let s = select_model(
        &req(None, Some("acme/big"), Some("other/solo")),
        None,
        &available(),
        &[],
    );
    assert_eq!(s.model.as_deref(), Some("acme/big"));
}

#[test]
fn an_ask_beats_the_pin_inside_a_scope_too() {
    let scoped = scope(&[
        entry(
            "acme/tiny",
            Some(ModelCategoryName::Cheap),
            Some("tiny"),
            None,
        ),
        entry(
            "acme/big",
            Some(ModelCategoryName::Capable),
            Some("big"),
            None,
        ),
    ]);
    let s = pick(&req(Some("tiny"), Some("big"), None), &scoped);
    assert_eq!(s.model.as_deref(), Some("acme/tiny"));
    assert_eq!(s.alias.as_deref(), Some("tiny"));
}

#[test]
fn a_pin_outside_the_scope_falls_back_within_the_scope_and_says_so() {
    let scoped = scope(&[entry("acme/tiny", None, Some("tiny"), None)]);
    let s = pick(&req(None, Some("acme/big"), Some("acme/mid")), &scoped);
    assert_eq!(s.model.as_deref(), Some("acme/tiny"));
    let note = s.note.expect("the fallback is noted");
    assert!(note.contains("acme/big"), "{note}");
}

#[test]
fn a_category_pin_resolves_inside_the_scope() {
    let scoped = scope(&[entry(
        "acme/mid",
        Some(ModelCategoryName::Standard),
        Some("mid"),
        None,
    )]);
    let s = pick(&req(None, Some("standard"), None), &scoped);
    assert_eq!(s.model.as_deref(), Some("acme/mid"));
}

// --- categories -----------------------------------------------------------------

#[test]
fn a_category_tie_takes_the_first_in_list_order() {
    let scoped = scope(&[
        entry(
            "acme/mid",
            Some(ModelCategoryName::Fast),
            Some("first"),
            None,
        ),
        entry(
            "acme/tiny",
            Some(ModelCategoryName::Fast),
            Some("second"),
            None,
        ),
    ]);
    let s = pick(&req(Some("fast"), None, None), &scoped);
    assert_eq!(s.alias.as_deref(), Some("first"));
    assert!(s.note.is_none(), "no tier fallback, so no note");
}

#[test]
fn an_empty_tier_falls_back_to_the_nearest_tier_and_the_result_names_it() {
    // `standard` is empty: the lower `cheap` is nearer than `capable`.
    let scoped = scope(&[
        entry(
            "acme/tiny",
            Some(ModelCategoryName::Cheap),
            Some("tiny"),
            None,
        ),
        entry(
            "acme/big",
            Some(ModelCategoryName::Capable),
            Some("big"),
            None,
        ),
    ]);
    let s = pick(&req(Some("standard"), None, None), &scoped);
    assert_eq!(s.model.as_deref(), Some("acme/tiny"));
    let note = s.note.expect("a tier fallback is noted");
    assert!(note.contains("cheap"), "{note}");
    assert!(note.contains("standard"), "{note}");
}

#[test]
fn the_lowest_tier_steps_up_when_it_is_empty() {
    let scoped = scope(&[entry(
        "acme/big",
        Some(ModelCategoryName::Capable),
        Some("big"),
        None,
    )]);
    let s = pick(&req(Some("cheap"), None, None), &scoped);
    assert_eq!(s.model.as_deref(), Some("acme/big"));
    assert!(s.note.is_some());
}

#[test]
fn an_untagged_model_is_never_picked_for_a_category() {
    let scoped = scope(&[
        entry("acme/tiny", None, Some("plain"), None),
        entry(
            "acme/big",
            Some(ModelCategoryName::Capable),
            Some("big"),
            None,
        ),
    ]);
    let s = pick(&req(Some("fast"), None, None), &scoped);
    assert_eq!(s.alias.as_deref(), Some("big"));
    assert!(s.error.is_none());
}

#[test]
fn a_category_over_a_scope_with_no_tags_derives_the_tier_from_the_scope() {
    // No entry is tagged (e.g. a `--models` list): the tier comes from the
    // scoped models alone, so the only scoped model is the fast one.
    let scoped = scope(&[entry("acme/tiny", None, Some("plain"), None)]);
    let s = pick(&req(Some("fast"), None, Some("acme/big")), &scoped);
    assert!(s.error.is_none(), "{:?}", s.error);
    assert_eq!(s.model.as_deref(), Some("acme/tiny"));
    assert_eq!(s.alias.as_deref(), Some("plain"));
}

#[test]
fn an_untagged_scope_derives_each_tier_from_its_own_models_only() {
    // acme/tiny is the cheapest model overall, but it is outside the scope:
    // the fast tier is the cheapest scoped model, acme/mid.
    let scoped = scope(&[
        entry("acme/mid", None, None, None),
        entry("acme/big", None, None, None),
    ]);
    let fast = pick(&req(Some("fast"), None, None), &scoped);
    assert_eq!(fast.model.as_deref(), Some("acme/mid"));
    assert!(fast.error.is_none(), "{:?}", fast.error);
    let capable = pick(&req(Some("capable"), None, None), &scoped);
    assert_eq!(capable.model.as_deref(), Some("acme/big"));
}

#[test]
fn a_tagged_scope_keeps_its_tags_for_category_asks() {
    // One tagged entry means the tags decide: the untagged mid is never a fast pick.
    let scoped = scope(&[
        entry("acme/mid", None, Some("plain"), None),
        entry(
            "acme/big",
            Some(ModelCategoryName::Capable),
            Some("big"),
            None,
        ),
    ]);
    let s = pick(&req(Some("fast"), None, None), &scoped);
    assert_eq!(s.alias.as_deref(), Some("big"));
    assert!(s.note.is_some(), "the nearest tier is named");
}

#[test]
fn cheap_without_a_scope_behaves_as_fast() {
    let cheap = select_model(&req(Some("cheap"), None, None), None, &available(), &[]);
    let fast = select_model(&req(Some("fast"), None, None), None, &available(), &[]);
    assert_eq!(cheap.model.as_deref(), Some("acme/tiny"));
    assert_eq!(cheap.model, fast.model);
    assert!(cheap.error.is_none(), "{:?}", cheap.error);
}

#[test]
fn a_cheap_pin_without_a_scope_reads_as_fast() {
    // The agent's `model: cheap` used to fall through to the inherited model,
    // because only fast, standard and capable were parsed for a pin.
    let s = select_model(
        &req(None, Some("cheap"), Some("other/solo")),
        None,
        &available(),
        &[],
    );
    assert_eq!(s.model.as_deref(), Some("acme/tiny"));
}

// --- hard limit and empty scope ---------------------------------------------------

#[test]
fn with_a_scope_a_subagent_never_gets_an_unscoped_model() {
    let scoped = scope(&[entry("acme/mid", None, Some("mid"), None)]);
    // The session's own model is not scoped: the default is a scoped model.
    let s = pick(&req(None, None, Some("acme/big")), &scoped);
    assert_eq!(s.model.as_deref(), Some("acme/mid"));
    assert!(s.note.unwrap().contains("acme/big"));
    // An ask for an unscoped model is refused, and the default still stays scoped.
    let s = pick(&req(Some("acme/big"), None, None), &scoped);
    assert!(s.error.is_some());
    assert_eq!(s.model.as_deref(), Some("acme/mid"));
}

#[test]
fn an_empty_scope_keeps_every_logged_in_model() {
    let s = pick(&req(None, None, Some("acme/big")), &[]);
    assert_eq!(s.model.as_deref(), Some("acme/big"));
    assert!(s.note.is_none());
}

#[test]
fn an_unknown_model_name_is_refused_when_models_are_known() {
    let s = pick(&req(Some("fastt"), None, None), &[]);
    assert!(s.error.is_some());
    assert_eq!(s.model, None);
}

// --- effort ---------------------------------------------------------------------------

fn with_effort(ask: Option<&str>, effort: &str) -> ModelRequest {
    ModelRequest {
        effort: Some(effort.into()),
        ..req(ask, None, None)
    }
}

#[test]
fn the_explicit_effort_overrides_the_scoped_effort() {
    let scoped = scope(&[entry("acme/big", None, Some("big"), Some("low"))]);
    let s = select_model(
        &with_effort(Some("big"), "high"),
        None,
        &available(),
        &scoped,
    );
    let big = &available()[2];
    assert_eq!(s.effort, Some(clamp_effort(ThinkingLevel::High, big)));
}

#[test]
fn the_scoped_effort_is_used_when_no_effort_is_asked() {
    let scoped = scope(&[entry("acme/big", None, Some("big"), Some("low"))]);
    let s = pick(&req(Some("big"), None, None), &scoped);
    let big = &available()[2];
    assert_eq!(s.effort, Some(clamp_effort(ThinkingLevel::Low, big)));
}

#[test]
fn an_effort_the_model_does_not_support_is_clamped() {
    let scoped = scope(&[entry("acme/tiny", None, Some("tiny"), Some("xhigh"))]);
    let s = pick(&req(Some("tiny"), None, None), &scoped);
    let tiny = &available()[0];
    assert_eq!(s.effort, Some(clamp_effort(ThinkingLevel::XHigh, tiny)));
}

#[test]
fn no_effort_when_nothing_sets_one() {
    let s = pick(&req(Some("acme/mid"), None, None), &[]);
    assert_eq!(s.effort, None);
}

// --- the `--models` override (process-wide) ---------------------------------------

/// The override is one process-wide slot. The test holds the shared SERIAL lock
/// (the tool tests take it too) and always resets the slot, even on failure.
#[test]
fn the_models_override_replaces_the_scope_and_is_reset_after_use() {
    let _serial = crate::SERIAL.blocking_lock();
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            set_scoped_models_override(None);
        }
    }
    let _reset = Reset;
    assert!(!scoped_models_override_active());
    assert_eq!(scoped_models_override_flag(), None);

    let mid_high = entry("acme/mid", None, None, Some("high"));
    set_scoped_models_override(Some(vec![mid_high, entry("other/solo", None, None, None)]));
    assert!(scoped_models_override_active());
    // The flag form round-trips: `id:effort`, comma separated, no categories.
    assert_eq!(
        scoped_models_override_flag().as_deref(),
        Some("acme/mid:high,other/solo")
    );
    // The override is the scope, whatever the settings say; the cwd is never read.
    let scope = current_scope(std::path::Path::new("/nonexistent-cwd"), &available());
    let ids: Vec<String> = scope
        .iter()
        .map(|s| format!("{}/{}", s.model.provider, s.model.id))
        .collect();
    assert_eq!(ids, vec!["acme/mid".to_string(), "other/solo".to_string()]);
    assert_eq!(scope[0].effort, Some(ThinkingLevel::High));

    set_scoped_models_override(None);
    assert!(!scoped_models_override_active());
    assert_eq!(scoped_models_override_flag(), None);
}

#[test]
fn an_empty_models_override_has_no_flag_value() {
    let _serial = crate::SERIAL.blocking_lock();
    set_scoped_models_override(Some(Vec::new()));
    assert!(scoped_models_override_active());
    assert_eq!(scoped_models_override_flag(), None);
    set_scoped_models_override(None);
}

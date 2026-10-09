//! Scoped-model selection for the Agent tool (`select_model`): ask, pin,
//! default, tier fallback, the hard scope limit, and effort.

use hoocode_ai_types::{Model, ThinkingLevel};
use hoocode_code_models::{clamp_effort, resolve_scoped_models, ResolvedScoped};
use hoocode_code_settings::{ModelCategories, ModelCategoryName, ScopedModel};
use hoocode_code_subagents::model_categories::{
    scoped_models_prompt_section, select_model, CategorySettings, ModelRequest,
};
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
fn a_category_with_no_tagged_model_is_an_error_and_the_default_stays_in_scope() {
    let scoped = scope(&[entry("acme/tiny", None, Some("plain"), None)]);
    let s = pick(&req(Some("fast"), None, Some("acme/big")), &scoped);
    assert!(
        s.error.as_deref().unwrap_or("").contains("fast"),
        "{:?}",
        s.error
    );
    assert_eq!(s.model.as_deref(), Some("acme/tiny"));
}

#[test]
fn cheap_without_a_scope_behaves_as_fast() {
    let settings = CategorySettings {
        model_categories: Some(ModelCategories {
            fast: Some("acme/tiny".into()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let cheap = select_model(
        &req(Some("cheap"), None, None),
        Some(&settings),
        &available(),
        &[],
    );
    let fast = select_model(
        &req(Some("fast"), None, None),
        Some(&settings),
        &available(),
        &[],
    );
    assert_eq!(cheap.model.as_deref(), Some("acme/tiny"));
    assert_eq!(cheap.model, fast.model);
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

// --- prompt section --------------------------------------------------------------------

#[test]
fn the_prompt_section_is_empty_without_a_scope() {
    assert_eq!(scoped_models_prompt_section(&[]), "");
}

#[test]
fn the_prompt_section_lists_alias_or_id_effort_and_category() {
    let scoped = scope(&[
        entry(
            "acme/tiny",
            Some(ModelCategoryName::Cheap),
            Some("tiny"),
            Some("off"),
        ),
        entry("other/solo", None, None, None),
    ]);
    let text = scoped_models_prompt_section(&scoped);
    assert!(
        text.contains("tiny (acme/tiny): effort off, category cheap"),
        "{text}"
    );
    assert!(
        text.contains("- other/solo: effort default, category none"),
        "{text}"
    );
}

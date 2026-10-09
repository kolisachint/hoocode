//! Scoped-model resolution: list order, name matching, category pick with
//! nearest-tier fallback, and effort clamping (scoped-models design, decisions
//! 7, 9, 10, 11, 14, 17).

use hoocode_ai_types::{Model, ModelCost, ThinkingLevel};
use hoocode_code_models::{
    clamp_effort, match_scoped_model, pick_by_category, resolve_scoped_models, ResolvedScoped,
};
use hoocode_code_settings::{ModelCategoryName, ScopedModel};

fn model(provider: &str, id: &str, reasoning: bool) -> Model {
    Model {
        id: id.into(),
        name: id.into(),
        api: "openai-completions".into(),
        provider: provider.into(),
        base_url: "https://example.test".into(),
        reasoning,
        thinking_level_map: None,
        input: vec!["text".into()],
        cost: ModelCost::default(),
        context_window: 128_000,
        max_tokens: 8_192,
        headers: None,
        compat: None,
    }
}

fn available() -> Vec<Model> {
    vec![
        model("openai", "gpt-5", true),
        model("openai", "gpt-5-mini", true),
        model("anthropic", "claude-opus", true),
        model("anthropic", "claude-haiku", false),
    ]
}

fn entry(model: &str) -> ScopedModel {
    ScopedModel {
        model: model.into(),
        effort: None,
        category: None,
        alias: None,
    }
}

fn scoped(id: &str, category: Option<ModelCategoryName>, alias: Option<&str>) -> ResolvedScoped {
    let (provider, id) = match id.split_once('/') {
        Some((p, i)) => (p, i),
        None => ("test", id),
    };
    ResolvedScoped {
        model: model(provider, id, true),
        alias: alias.map(str::to_owned),
        category,
        effort: None,
    }
}

fn ids(list: &[ResolvedScoped]) -> Vec<String> {
    list.iter().map(|s| s.model.id.clone()).collect()
}

#[test]
fn resolves_in_list_order_with_alias_category_and_effort() {
    let entries = vec![
        ScopedModel {
            model: "anthropic/claude-opus".into(),
            effort: Some("high".into()),
            category: Some(ModelCategoryName::Capable),
            alias: Some("opus".into()),
        },
        ScopedModel {
            model: "openai/gpt-5-mini".into(),
            effort: None,
            category: Some(ModelCategoryName::Fast),
            alias: None,
        },
        entry("anthropic/claude-haiku"),
    ];
    let resolved = resolve_scoped_models(&entries, &available());
    assert_eq!(
        ids(&resolved),
        ["claude-opus", "gpt-5-mini", "claude-haiku"]
    );
    assert_eq!(resolved[0].effort, Some(ThinkingLevel::High));
    assert_eq!(resolved[0].alias.as_deref(), Some("opus"));
    assert_eq!(resolved[0].category, Some(ModelCategoryName::Capable));
    assert_eq!(resolved[1].effort, None);
    assert_eq!(resolved[1].category, Some(ModelCategoryName::Fast));
}

#[test]
fn explicit_effort_beats_level_suffix_and_suffix_is_used_otherwise() {
    let entries = vec![
        entry("openai/gpt-5:low"),
        ScopedModel {
            effort: Some("xhigh".into()),
            ..entry("anthropic/claude-opus:low")
        },
    ];
    let resolved = resolve_scoped_models(&entries, &available());
    assert_eq!(resolved[0].effort, Some(ThinkingLevel::Low));
    assert_eq!(resolved[1].effort, Some(ThinkingLevel::XHigh));
}

#[test]
fn glob_entries_expand_and_share_their_metadata() {
    let entries = vec![ScopedModel {
        category: Some(ModelCategoryName::Standard),
        ..entry("anthropic/*")
    }];
    let resolved = resolve_scoped_models(&entries, &available());
    assert_eq!(ids(&resolved), ["claude-opus", "claude-haiku"]);
    assert!(resolved
        .iter()
        .all(|s| s.category == Some(ModelCategoryName::Standard)));
}

#[test]
fn entries_matching_no_model_are_dropped() {
    let entries = vec![entry("mistral/large"), entry("openai/gpt-5")];
    let resolved = resolve_scoped_models(&entries, &available());
    assert_eq!(ids(&resolved), ["gpt-5"]);
}

#[test]
fn match_by_alias_exact_only() {
    let list = vec![ScopedModel {
        alias: Some("opus".into()),
        ..entry("anthropic/claude-opus")
    }];
    let resolved = resolve_scoped_models(&list, &available());
    assert_eq!(
        match_scoped_model("opus", &resolved).unwrap().model.id,
        "claude-opus"
    );
    assert!(
        match_scoped_model("op", &resolved).is_ok(),
        "substring of the id still matches"
    );
    let resolved_alias_only = vec![ResolvedScoped {
        alias: Some("fast-one".into()),
        ..scoped("openai/gpt-5-mini", None, None)
    }];
    assert!(
        match_scoped_model("fast", &resolved_alias_only).is_err(),
        "an alias is matched exactly, not as a prefix"
    );
    assert_eq!(
        match_scoped_model("fast-one", &resolved_alias_only)
            .unwrap()
            .model
            .id,
        "gpt-5-mini"
    );
}

#[test]
fn match_by_exact_id_with_or_without_provider() {
    let list = vec![scoped("anthropic/claude-opus", None, None)];
    assert_eq!(
        match_scoped_model("claude-opus", &list).unwrap().model.id,
        "claude-opus"
    );
    assert_eq!(
        match_scoped_model("anthropic/claude-opus", &list)
            .unwrap()
            .model
            .id,
        "claude-opus"
    );
}

#[test]
fn match_exact_id_beats_substring_matches() {
    let list = vec![
        scoped("openai/gpt-5", None, None),
        scoped("openai/gpt-5-mini", None, None),
    ];
    assert_eq!(
        match_scoped_model("gpt-5", &list).unwrap().model.id,
        "gpt-5"
    );
}

#[test]
fn match_unique_substring_of_id() {
    let list = vec![
        scoped("openai/gpt-5", None, None),
        scoped("anthropic/claude-haiku", None, None),
    ];
    assert_eq!(
        match_scoped_model("haiku", &list).unwrap().model.id,
        "claude-haiku"
    );
}

#[test]
fn ambiguous_substring_lists_candidates() {
    let list = vec![
        scoped("openai/gpt-5", None, None),
        scoped("openai/gpt-5-mini", None, None),
    ];
    let err = match_scoped_model("gpt", &list).unwrap_err();
    assert!(
        err.contains("openai/gpt-5") && err.contains("openai/gpt-5-mini"),
        "{err}"
    );
}

#[test]
fn ambiguous_exact_id_across_providers_is_an_error() {
    let list = vec![
        scoped("openai/shared-id", None, None),
        scoped("anthropic/shared-id", None, None),
    ];
    let err = match_scoped_model("shared-id", &list).unwrap_err();
    assert!(
        err.contains("openai/shared-id") && err.contains("anthropic/shared-id"),
        "{err}"
    );
    assert!(match_scoped_model("openai/shared-id", &list).is_ok());
}

#[test]
fn same_model_listed_twice_is_one_match() {
    let list = vec![
        scoped("openai/gpt-5", Some(ModelCategoryName::Fast), None),
        scoped("openai/gpt-5", Some(ModelCategoryName::Capable), None),
    ];
    assert_eq!(
        match_scoped_model("gpt-5", &list).unwrap().category,
        Some(ModelCategoryName::Fast)
    );
}

#[test]
fn no_match_lists_available_models() {
    let list = vec![scoped("openai/gpt-5", None, Some("big"))];
    let err = match_scoped_model("claude", &list).unwrap_err();
    assert!(err.contains("big (openai/gpt-5)"), "{err}");
    let empty: Vec<ResolvedScoped> = Vec::new();
    assert!(match_scoped_model("claude", &empty)
        .unwrap_err()
        .contains("no scoped models"));
}

#[test]
fn alias_beats_exact_id_of_another_model() {
    let list = vec![
        scoped("openai/gpt-5", None, None),
        scoped("anthropic/claude-haiku", None, Some("gpt-5")),
    ];
    assert_eq!(
        match_scoped_model("gpt-5", &list).unwrap().model.id,
        "claude-haiku"
    );
}

#[test]
fn pick_takes_first_in_list_order() {
    let list = vec![
        scoped("a/first", Some(ModelCategoryName::Fast), None),
        scoped("a/second", Some(ModelCategoryName::Fast), None),
    ];
    let (picked, fallback) = pick_by_category(ModelCategoryName::Fast, &list).unwrap();
    assert_eq!(picked.model.id, "first");
    assert!(!fallback);
}

#[test]
fn untagged_models_are_never_picked_for_a_category() {
    let list = vec![scoped("a/untagged", None, None)];
    assert!(pick_by_category(ModelCategoryName::Fast, &list).is_none());
    assert!(pick_by_category(ModelCategoryName::Cheap, &list).is_none());
    assert!(pick_by_category(ModelCategoryName::Capable, &[]).is_none());
}

#[test]
fn empty_tier_steps_down_to_the_nearest_lower_tier_first() {
    let list = vec![
        scoped("a/cheapest", Some(ModelCategoryName::Cheap), None),
        scoped("a/standard", Some(ModelCategoryName::Standard), None),
    ];
    // capable is empty; standard is the nearest lower tier, so it wins over cheap.
    let (picked, fallback) = pick_by_category(ModelCategoryName::Capable, &list).unwrap();
    assert_eq!(picked.model.id, "standard");
    assert!(fallback);
}

#[test]
fn empty_tier_steps_down_past_empty_tiers() {
    let list = vec![scoped("a/cheapest", Some(ModelCategoryName::Cheap), None)];
    let (picked, fallback) = pick_by_category(ModelCategoryName::Standard, &list).unwrap();
    assert_eq!(picked.model.id, "cheapest");
    assert!(fallback);
}

#[test]
fn empty_lowest_tier_steps_up_and_names_the_fallback() {
    // No cheap model: cheap steps up to fast, then standard.
    let list = vec![
        scoped("a/standard", Some(ModelCategoryName::Standard), None),
        scoped("a/capable", Some(ModelCategoryName::Capable), None),
    ];
    let (picked, fallback) = pick_by_category(ModelCategoryName::Cheap, &list).unwrap();
    assert_eq!(picked.model.id, "standard");
    assert!(
        fallback,
        "the caller reports that a different tier was used"
    );

    let list = vec![scoped("a/capable", Some(ModelCategoryName::Capable), None)];
    let (picked, fallback) = pick_by_category(ModelCategoryName::Standard, &list).unwrap();
    assert_eq!(picked.model.id, "capable");
    assert!(fallback);
}

#[test]
fn requested_tier_present_is_not_a_fallback() {
    let list = vec![
        scoped("a/fast", Some(ModelCategoryName::Fast), None),
        scoped("a/capable", Some(ModelCategoryName::Capable), None),
    ];
    let (picked, fallback) = pick_by_category(ModelCategoryName::Capable, &list).unwrap();
    assert_eq!(picked.model.id, "capable");
    assert!(!fallback);
}

#[test]
fn clamp_keeps_a_supported_level() {
    let reasoning = model("openai", "gpt-5", true);
    assert_eq!(
        clamp_effort(ThinkingLevel::Medium, &reasoning),
        ThinkingLevel::Medium
    );
}

#[test]
fn clamp_moves_an_unsupported_level_to_the_closest_supported_one() {
    // xhigh needs an explicit mapping, so the closest supported level is high.
    let reasoning = model("openai", "gpt-5", true);
    assert_eq!(
        clamp_effort(ThinkingLevel::XHigh, &reasoning),
        ThinkingLevel::High
    );
}

#[test]
fn clamp_on_a_non_reasoning_model_is_off() {
    let plain = model("anthropic", "claude-haiku", false);
    assert_eq!(
        clamp_effort(ThinkingLevel::High, &plain),
        ThinkingLevel::Off
    );
}

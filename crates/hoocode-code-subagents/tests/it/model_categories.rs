//! model-categories.test.ts.

use hoocode_ai_types::Model;
use hoocode_code_subagents::model_categories::*;
use serde_json::json;

fn model(provider: &str, id: &str, price_in: f64, price_out: Option<f64>) -> Model {
    serde_json::from_value(json!({
        "id": id, "name": id, "api": "anthropic-messages", "provider": provider,
        "baseUrl": "https://example.test", "reasoning": false, "input": ["text"],
        "cost": {"input": price_in, "output": price_out.unwrap_or(price_in), "cacheRead": 0, "cacheWrite": 0},
        "contextWindow": 128000, "maxTokens": 8192,
    }))
    .unwrap()
}

// By combined price: tiny(1) < solo(4) < mid(6) < big(30).
fn available() -> Vec<Model> {
    vec![
        model("acme", "tiny", 0.5, None),
        model("acme", "mid", 3.0, None),
        model("acme", "big", 15.0, None),
        model("other", "solo", 2.0, None),
    ]
}

fn s(v: &str) -> Option<String> {
    Some(v.to_string())
}

#[test]
fn recognizes_the_three_category_names_and_nothing_else() {
    assert!(is_model_category("fast"));
    assert!(is_model_category("standard"));
    assert!(is_model_category("capable"));
    assert!(!is_model_category("opus"));
    assert!(!is_model_category("anthropic/claude-haiku"));
}

#[test]
fn resolves_a_category_to_its_derived_model_id() {
    let models = available();
    assert_eq!(
        resolve_model_category(ModelCategory::Fast, None, Some(&models)),
        s("acme/tiny")
    );
    assert_eq!(
        resolve_model_category(ModelCategory::Capable, None, Some(&models)),
        s("acme/big")
    );
}

#[test]
fn unconfigured_category_is_none_without_available_models() {
    assert_eq!(
        resolve_model_category(
            ModelCategory::Standard,
            Some(&CategorySettings::default()),
            None
        ),
        None
    );
    assert_eq!(
        resolve_model_category(
            ModelCategory::Fast,
            Some(&CategorySettings::default()),
            None
        ),
        None
    );
    assert_eq!(
        resolve_model_category(ModelCategory::Capable, None, None),
        None
    );
}

#[test]
fn passes_non_category_references_through() {
    assert_eq!(
        resolve_model_reference(
            "anthropic/claude-sonnet",
            Some(&CategorySettings::default()),
            None
        ),
        s("anthropic/claude-sonnet")
    );
    assert_eq!(resolve_model_reference("gpt-4o", None, None), s("gpt-4o"));
}

#[test]
fn resolves_category_references_via_derived_defaults() {
    let models = available();
    assert_eq!(
        resolve_model_reference("standard", None, Some(&models)),
        s("acme/mid")
    );
    assert_eq!(resolve_model_reference("fast", None, None), None);
}

#[test]
fn derives_a_default_per_tier_from_the_available_set() {
    assert_eq!(
        derive_default_model_categories(&available(), None),
        DerivedCategories {
            capable: s("acme/big"),
            fast: s("acme/tiny"),
            standard: s("acme/mid")
        }
    );
}

#[test]
fn resolves_an_unconfigured_tier_to_its_derived_default() {
    let models = available();
    let empty = CategorySettings::default();
    assert_eq!(
        resolve_model_category(ModelCategory::Fast, Some(&empty), Some(&models)),
        s("acme/tiny")
    );
    assert_eq!(
        resolve_model_category(ModelCategory::Standard, Some(&empty), Some(&models)),
        s("acme/mid")
    );
    assert_eq!(
        resolve_model_category(ModelCategory::Capable, Some(&empty), Some(&models)),
        s("acme/big")
    );
    assert_eq!(
        resolve_model_reference("capable", None, Some(&models)),
        s("acme/big")
    );
}

#[test]
fn anchors_capable_to_the_configured_default_model() {
    let models = available();
    let settings = CategorySettings {
        default_provider: s("other"),
        default_model: s("solo"),
    };
    assert_eq!(
        resolve_model_category(ModelCategory::Capable, Some(&settings), Some(&models)),
        s("other/solo")
    );
    assert_eq!(
        resolve_model_category(ModelCategory::Fast, Some(&settings), Some(&models)),
        s("acme/tiny")
    );
    assert_eq!(
        resolve_model_category(ModelCategory::Standard, Some(&settings), Some(&models)),
        s("other/solo")
    );
}

#[test]
fn keeps_tiers_monotonic_across_providers() {
    let mixed = vec![
        model("oai", "mini", 0.15, Some(0.6)),
        model("oai", "gpt", 2.0, Some(8.0)),
        model("ant", "opus", 15.0, Some(75.0)),
    ];
    assert_eq!(
        derive_default_model_categories(&mixed, None),
        DerivedCategories {
            fast: s("oai/mini"),
            standard: s("oai/gpt"),
            capable: s("ant/opus")
        }
    );
}

#[test]
fn the_retired_model_categories_key_is_not_read() {
    // `modelCategories` was migrated into `scopedModels`; a leftover key does
    // not pin a tier any more.
    let settings = CategorySettings::from_settings(
        json!({"modelCategories": {"fast": "explicit/pin"}})
            .as_object()
            .unwrap(),
    );
    assert_eq!(settings, CategorySettings::default());
    assert_eq!(
        resolve_model_category(ModelCategory::Fast, Some(&settings), Some(&available())),
        s("acme/tiny")
    );
}

#[test]
fn every_tier_is_none_without_available_models() {
    let empty = CategorySettings::default();
    assert_eq!(
        derive_default_model_categories(&[], None),
        DerivedCategories::default()
    );
    for c in [
        ModelCategory::Fast,
        ModelCategory::Standard,
        ModelCategory::Capable,
    ] {
        assert_eq!(resolve_model_category(c, Some(&empty), Some(&[])), None);
    }
    assert_eq!(
        resolve_model_reference("capable", Some(&empty), Some(&[])),
        None
    );
}

#[test]
fn is_deterministic_regardless_of_order() {
    let a = available();
    let shuffled = vec![a[2].clone(), a[0].clone(), a[3].clone(), a[1].clone()];
    assert_eq!(
        derive_default_model_categories(&shuffled, None),
        derive_default_model_categories(&a, None)
    );
}

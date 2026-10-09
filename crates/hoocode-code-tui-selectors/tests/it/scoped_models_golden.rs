//! Component golden: the `/scoped-models` picker with its effort and category
//! columns, at 100 and 80 columns. Accept a deliberate change with
//! `UPDATE_GOLDENS=1 cargo test -p hoocode-code-tui-selectors scoped_models`.

use hoocode_ai_types::Model;
use hoocode_code_settings::{ModelCategoryName, ScopedModel};
use hoocode_code_tui_selectors::scoped_models_selector::ScopedModelsSelectorComponent;
use hoocode_tui_render::assert_golden;
use hoocode_tui_render::golden::render_golden;
use serde_json::json;

use crate::support::lock;

fn model(provider: &str, id: &str, name: &str) -> Model {
    serde_json::from_value(json!({
        "id": id, "name": name, "api": "openai-completions", "provider": provider,
        "baseUrl": "https://example.test", "reasoning": true, "input": ["text"],
        "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0},
        "contextWindow": 128000, "maxTokens": 8192,
    }))
    .unwrap()
}

/// Two scoped models with different columns set, and one model left out.
fn fixture() -> (Vec<Model>, Vec<ScopedModel>) {
    let models = vec![
        model("openai", "gpt-5-mini", "GPT-5 mini"),
        model("anthropic", "claude-haiku-5-5", "Claude Haiku 5.5"),
        model("openai", "gpt-5", "GPT-5"),
    ];
    let scoped = vec![
        ScopedModel {
            model: "anthropic/claude-haiku-5-5".into(),
            effort: Some("low".into()),
            category: Some(ModelCategoryName::Cheap),
            alias: Some("haiku".into()),
        },
        ScopedModel {
            model: "openai/gpt-5-mini".into(),
            effort: None,
            category: Some(ModelCategoryName::Standard),
            alias: None,
        },
    ];
    (models, scoped)
}

#[test]
fn scoped_models_picker_100() {
    let _g = lock();
    let (models, scoped) = fixture();
    let mut selector = ScopedModelsSelectorComponent::new(models, Some(scoped));
    assert_golden!(
        "scoped_models_picker_100",
        render_golden(&mut selector, 100)
    );
}

#[test]
fn scoped_models_picker_80() {
    let _g = lock();
    let (models, scoped) = fixture();
    let mut selector = ScopedModelsSelectorComponent::new(models, Some(scoped));
    assert_golden!("scoped_models_picker_80", render_golden(&mut selector, 80));
}

//! Two providers can offer the same model id. Both pickers show the provider
//! on every row, so the rows differ; the provider is a primary fact, so it
//! survives a narrow width (the id cuts first).

use hoocode_ai_types::Model;
use hoocode_code_settings::ScopedModel;
use hoocode_code_tui_selectors::model_selector::ModelSelectorComponent;
use hoocode_code_tui_selectors::scoped_models_selector::ScopedModelsSelectorComponent;
use hoocode_tui_render::Component;
use serde_json::json;

use crate::support::{lock, strip};

fn model(provider: &str, id: &str, name: &str) -> Model {
    serde_json::from_value(json!({
        "id": id, "name": name, "api": "openai-completions", "provider": provider,
        "baseUrl": "https://example.test", "reasoning": true, "input": ["text"],
        "cost": {"input": 3, "output": 15, "cacheRead": 0, "cacheWrite": 0},
        "contextWindow": 200000, "maxTokens": 8192,
    }))
    .unwrap()
}

/// The same id from two providers.
fn twins() -> Vec<Model> {
    vec![
        model("anthropic", "claude-sonnet-5-5", "Claude Sonnet 5.5"),
        model("openrouter", "claude-sonnet-5-5", "Claude Sonnet 5.5"),
    ]
}

/// The rendered rows that carry `provider` as their first fact: the provider
/// sits after the id (or its `…` cut) and before the dim `·` facts. The
/// detail column's `provider  name` line never has the `·` after it.
fn rows_of(rendered: &str, provider: &str) -> Vec<String> {
    let fact = format!("{provider} · ");
    rendered
        .lines()
        .filter(|line| line.contains(&fact))
        .map(|line| line.trim_matches(['│', '┃', ' ']).to_string())
        .collect()
}

#[test]
fn model_picker_shows_the_provider_on_each_twin_row() {
    let _g = lock();
    for width in [100, 60] {
        let mut selector = ModelSelectorComponent::new(None, Ok(twins()), None, vec![], None);
        let rendered = strip(&selector.render(width));
        let anthropic = rows_of(&rendered, "anthropic");
        let openrouter = rows_of(&rendered, "openrouter");
        assert_eq!(anthropic.len(), 1, "width {width}:\n{rendered}");
        assert_eq!(openrouter.len(), 1, "width {width}:\n{rendered}");
        assert_ne!(anthropic, openrouter, "width {width}:\n{rendered}");
    }
}

#[test]
fn scoped_models_picker_shows_the_provider_on_each_twin_row() {
    let _g = lock();
    for width in [100, 60] {
        let scoped = vec![
            ScopedModel {
                model: "anthropic/claude-sonnet-5-5".into(),
                effort: None,
                category: None,
                alias: None,
            },
            ScopedModel {
                model: "openrouter/claude-sonnet-5-5".into(),
                effort: None,
                category: None,
                alias: None,
            },
        ];
        let mut selector = ScopedModelsSelectorComponent::new(twins(), Some(scoped));
        let rendered = strip(&selector.render(width));
        let anthropic = rows_of(&rendered, "anthropic");
        let openrouter = rows_of(&rendered, "openrouter");
        assert_eq!(anthropic.len(), 1, "width {width}:\n{rendered}");
        assert_eq!(openrouter.len(), 1, "width {width}:\n{rendered}");
        assert_ne!(anthropic, openrouter, "width {width}:\n{rendered}");
    }
}

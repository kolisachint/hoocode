//! Two providers can offer the same model id. Both pickers show the provider
//! on every row, so the rows differ. The provider is a primary fact and the
//! id cuts only after the secondary facts are gone, so at 100 columns (detail
//! column open) and at 60 the id reads whole, followed by the provider.

use hoocode_ai_types::Model;
use hoocode_code_settings::ScopedModel;
use hoocode_code_tui_selectors::model_selector::ModelSelectorComponent;
use hoocode_code_tui_selectors::scoped_models_selector::ScopedModelsSelectorComponent;
use hoocode_tui_render::Component;
use serde_json::json;

use crate::support::{lock, strip};

const ID: &str = "claude-sonnet-5-5";

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
        model("anthropic", ID, "Claude Sonnet 5.5"),
        model("openrouter", ID, "Claude Sonnet 5.5"),
    ]
}

/// Exactly one list row shows the whole id, then the provider. The detail
/// column prints `claude-sonnet-5-5` on its own, never before a provider.
fn assert_full_id_row(rendered: &str, provider: &str, width: u16) {
    let row = format!("{ID}  {provider}");
    let hits = rendered.lines().filter(|line| line.contains(&row)).count();
    assert_eq!(hits, 1, "width {width}, {provider}:\n{rendered}");
}

#[test]
fn model_picker_shows_the_full_id_and_provider_on_each_twin_row() {
    let _g = lock();
    for width in [100, 60] {
        let mut selector = ModelSelectorComponent::new(None, Ok(twins()), None, vec![], None);
        let rendered = strip(&selector.render(width));
        assert_full_id_row(&rendered, "anthropic", width);
        assert_full_id_row(&rendered, "openrouter", width);
    }
}

#[test]
fn scoped_models_picker_shows_the_full_id_and_provider_on_each_twin_row() {
    let _g = lock();
    for width in [100, 60] {
        let scoped = vec![
            ScopedModel {
                model: format!("anthropic/{ID}"),
                effort: None,
                category: None,
                alias: None,
            },
            ScopedModel {
                model: format!("openrouter/{ID}"),
                effort: None,
                category: None,
                alias: None,
            },
        ];
        let mut selector = ScopedModelsSelectorComponent::new(twins(), Some(scoped));
        let rendered = strip(&selector.render(width));
        assert_full_id_row(&rendered, "anthropic", width);
        assert_full_id_row(&rendered, "openrouter", width);
    }
}

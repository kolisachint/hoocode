//! Component goldens: the `/model` picker as the HANGAR console, at 100 and
//! 80 columns. Hints use literal keys (no `alt`), so the bytes hold on every
//! platform. Accept a deliberate change with
//! `UPDATE_GOLDENS=1 cargo nextest run -p hoocode-code-tui-selectors hangar`.

use hoocode_ai_types::Model;
use hoocode_code_tui_selectors::model_selector::ModelSelectorComponent;
use hoocode_tui_render::assert_golden;
use hoocode_tui_render::golden::render_golden;
use hoocode_tui_render::Component;
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

/// The current ship plus two more, the first two scoped: the console opens on
/// the scoped tab counting two ships.
fn fixture() -> (Vec<Model>, Vec<Model>) {
    let models = vec![
        model("mock", "mock-large", "Mock Large"),
        model("mock", "mock-fast", "Mock Fast"),
        model("ollama", "mock-local", "Mock Local"),
    ];
    let scoped = vec![models[0].clone(), models[1].clone()];
    (models, scoped)
}

fn check(name: &str, width: u16) {
    let (models, scoped) = fixture();
    let mut selector =
        ModelSelectorComponent::new(Some(models[0].clone()), Ok(models), None, scoped, None);
    let rendered = render_golden(&mut selector, width);
    let text = rendered.split("--- styles ---").next().unwrap_or_default();
    assert!(
        text.contains("HANGAR · /model  2 ships"),
        "{name}: missing title in\n{text}"
    );
    assert!(
        text.lines()
            .any(|l| l.contains('›') && l.contains("mock-large")),
        "{name}: missing selected ship in\n{text}"
    );
    assert!(
        text.lines().any(|l| l.contains('▸')),
        "{name}: missing query row in\n{text}"
    );
    assert!(
        text.contains("enter fly it"),
        "{name}: missing hint foot in\n{text}"
    );
    assert_golden!(name, rendered);
}

#[test]
fn hangar_100() {
    let _g = lock();
    check("hangar_100", 100);
}

#[test]
fn hangar_80() {
    let _g = lock();
    check("hangar_80", 80);
}

/// Typing narrows the list and the title counts the match.
#[test]
fn hangar_search_counts_the_match() {
    let _g = lock();
    let (models, scoped) = fixture();
    let mut selector =
        ModelSelectorComponent::new(Some(models[0].clone()), Ok(models), None, scoped, None);
    selector.handle_input("fast");
    let rendered = render_golden(&mut selector, 100);
    let text = rendered.split("--- styles ---").next().unwrap_or_default();
    assert!(
        text.contains("1 ship of 2"),
        "missing filtered count in\n{text}"
    );
    assert!(
        !text.contains("mock-large"),
        "unfiltered ship still listed in\n{text}"
    );
}

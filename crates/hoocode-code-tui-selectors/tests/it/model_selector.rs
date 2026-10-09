//! Port of `suite/regressions/3217-scoped-model-order.test.ts`, plus the
//! pickers' own behaviour (no TS tests of their own).

use hoocode_ai_types::Model;
use hoocode_code_settings::{ModelCategoryName, ScopedModel};
use hoocode_code_tui_selectors::model_selector::{ModelSelectorComponent, ModelSelectorEvent};
use hoocode_code_tui_selectors::scoped_models_selector::{
    clear_all, enable_all, move_id, next_category, next_effort, toggle, ScopedModelsEvent,
    ScopedModelsSelectorComponent,
};

/// Raw input for the picker's effort (tab) and category (alt+j) keys.
const EFFORT_INPUT: &str = "\t";
const CATEGORY_INPUT: &str = "\x1bj";
use hoocode_code_tui_theme::SELECT_CURSOR;
use hoocode_tui_render::Component;
use serde_json::json;

use crate::support::{lock, strip};

fn model(provider: &str, id: &str, name: &str) -> Model {
    serde_json::from_value(json!({
        "id": id, "name": name, "api": "openai-completions", "provider": provider,
        "baseUrl": "https://example.test", "reasoning": true, "input": ["text"],
        "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0},
        "contextWindow": 128000, "maxTokens": 8192,
    }))
    .unwrap()
}

fn faux() -> Vec<Model> {
    vec![
        model("faux", "faux-1", "One"),
        model("faux", "faux-2", "Two"),
        model("faux", "faux-3", "Three"),
    ]
}

/// A saved entry with no effort, category or alias.
fn plain(model: &str) -> ScopedModel {
    entry(model, None, None, None)
}

fn entry(
    model: &str,
    effort: Option<&str>,
    category: Option<ModelCategoryName>,
    alias: Option<&str>,
) -> ScopedModel {
    ScopedModel {
        model: model.into(),
        effort: effort.map(String::from),
        category,
        alias: alias.map(String::from),
    }
}

fn full_ids(models: &[Model]) -> Vec<String> {
    models
        .iter()
        .map(|m| format!("{}/{}", m.provider, m.id))
        .collect()
}

/// The model id on each list row, in order.
fn listed_ids(rendered: &str, provider: &str) -> Vec<String> {
    rendered
        .lines()
        .filter(|line| line.contains(&format!("[{provider}]")))
        .map(|line| {
            let trimmed = line.trim();
            let trimmed = trimmed.strip_prefix('│').unwrap_or(trimmed);
            let trimmed = trimmed.strip_suffix('│').unwrap_or(trimmed).trim();
            let trimmed = trimmed.strip_prefix(SELECT_CURSOR).unwrap_or(trimmed);
            trimmed.split(" [").next().unwrap().trim().to_string()
        })
        .collect()
}

#[test]
fn propagates_reordered_scoped_models_back_to_the_session_state() {
    let _g = lock();
    let models = faux();
    let ordered = full_ids(&models);
    let saved = ordered.iter().map(|id| plain(id)).collect();
    let mut selector = ScopedModelsSelectorComponent::new(models, Some(saved));
    selector.handle_input("\x1b[1;3B");
    assert_eq!(
        selector.take_events(),
        vec![ScopedModelsEvent::Change(Some(vec![
            ordered[1].clone(),
            ordered[0].clone(),
            ordered[2].clone(),
        ]))]
    );
}

#[test]
fn preserves_scoped_model_order_in_the_model_scoped_tab() {
    let _g = lock();
    let models = faux();
    let (one, two, three) = (models[0].clone(), models[1].clone(), models[2].clone());
    let mut selector = ModelSelectorComponent::new(
        Some(one.clone()),
        Ok(models),
        None,
        vec![two.clone(), one.clone(), three.clone()],
        None,
    );
    let rendered = strip(&selector.render(120));
    let ids = listed_ids(&rendered, "faux");
    assert_eq!(ids[..3], [two.id, one.id, three.id]);
    assert!(rendered.contains("Scope: all | scoped"));
}

#[test]
fn lists_the_current_model_first_ticked_then_by_provider() {
    let _g = lock();
    let models = vec![
        model("zeta", "z-1", "Z"),
        model("alpha", "a-1", "A"),
        model("mid", "m-1", "M"),
    ];
    let current = models[2].clone();
    let mut selector = ModelSelectorComponent::new(Some(current), Ok(models), None, vec![], None);
    let rendered = strip(&selector.render(100));
    assert!(rendered.contains("Only showing models from configured providers"));
    assert!(rendered.contains("m-1 [mid] ✓"));
    assert!(rendered.contains("Model Name: M"));
    let order: Vec<_> = rendered
        .lines()
        .filter_map(|l| ["z-1", "a-1", "m-1"].into_iter().find(|id| l.contains(id)))
        .collect();
    assert_eq!(order, ["m-1", "a-1", "z-1"]);
}

#[test]
fn typing_narrows_and_enter_selects_the_highlighted_model() {
    let _g = lock();
    let models = faux();
    let mut selector =
        ModelSelectorComponent::new(Some(models[0].clone()), Ok(models), None, vec![], None);
    selector.handle_input("2");
    let rendered = strip(&selector.render(100));
    assert_eq!(listed_ids(&rendered, "faux"), ["faux-2"]);
    selector.handle_input("\r");
    match selector.take_events().as_slice() {
        [ModelSelectorEvent::Select(m)] => assert_eq!(m.id, "faux-2"),
        other => panic!("unexpected {other:?}"),
    }
    selector.handle_input("\x1b");
    assert_eq!(selector.take_events(), vec![ModelSelectorEvent::Cancel]);
}

#[test]
fn up_and_down_wrap_and_tab_switches_scope() {
    let _g = lock();
    let models = faux();
    let mut selector = ModelSelectorComponent::new(
        Some(models[0].clone()),
        Ok(models.clone()),
        None,
        vec![models[2].clone()],
        None,
    );
    // Scoped first: only faux-3.
    assert_eq!(
        listed_ids(&strip(&selector.render(100)), "faux"),
        ["faux-3"]
    );
    selector.handle_input("\t");
    let rendered = strip(&selector.render(100));
    assert_eq!(listed_ids(&rendered, "faux").len(), 3);
    selector.handle_input("\x1b[A");
    assert!(strip(&selector.render(100)).contains("Model Name: Three"));
    selector.handle_input("\x1b[B");
    assert!(strip(&selector.render(100)).contains("Model Name: One"));
}

#[test]
fn an_unlistable_registry_shows_its_error() {
    let _g = lock();
    let mut selector =
        ModelSelectorComponent::new(None, Err("boom\nsecond".into()), None, vec![], None);
    let rendered = strip(&selector.render(100));
    assert!(rendered.contains("boom"));
    assert!(rendered.contains("second"));
    assert!(!rendered.contains("No matching models"));
}

#[test]
fn toggle_from_all_enabled_keeps_only_that_model_and_marks_unsaved() {
    let _g = lock();
    let models = faux();
    let ids = full_ids(&models);
    let mut selector = ScopedModelsSelectorComponent::new(models, None);
    // The footer lists five hotkeys. On macOS `alt` is printed as `option`, so
    // the footer is about 20 columns wider than on Linux and wraps at 120,
    // splitting "all enabled". Render wide enough for both platforms.
    let rendered = strip(&selector.render(200));
    // "option+s" on macOS, like the pin's formatKeyText.
    let save = hoocode_code_tui_keybindings::format_key_text("alt+s", false);
    assert!(rendered.contains(&format!("Session-only. {save} to save to settings.")));
    assert!(rendered.contains("all enabled"));
    selector.handle_input("\r");
    assert_eq!(
        selector.take_events(),
        vec![ScopedModelsEvent::Change(Some(vec![ids[0].clone()]))]
    );
    let rendered = strip(&selector.render(200));
    assert!(rendered.contains("1/3 enabled (unsaved)"));
    assert!(rendered.contains("faux-2 [faux] ✗"));
    // alt+s saves and clears the mark.
    selector.handle_input("\x1bs");
    assert_eq!(
        selector.take_events(),
        vec![ScopedModelsEvent::Persist(vec![plain(&ids[0])])]
    );
    assert!(!strip(&selector.render(200)).contains("(unsaved)"));
}

#[test]
fn ctrl_c_clears_the_search_before_cancelling() {
    let _g = lock();
    let mut selector = ScopedModelsSelectorComponent::new(faux(), None);
    selector.handle_input("x");
    selector.handle_input("y");
    assert!(strip(&selector.render(100)).contains("No matching models"));
    selector.handle_input("\x03");
    assert!(selector.take_events().is_empty());
    assert!(!strip(&selector.render(100)).contains("No matching models"));
    selector.handle_input("\x03");
    assert_eq!(selector.take_events(), vec![ScopedModelsEvent::Cancel]);
}

#[test]
fn enabled_set_helpers_follow_the_original() {
    let all: Vec<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
    let s = |v: &[&str]| Some(v.iter().map(|x| x.to_string()).collect::<Vec<_>>());
    assert_eq!(toggle(&None, "b"), s(&["b"]));
    assert_eq!(toggle(&s(&["a", "b"]), "a"), s(&["b"]));
    assert_eq!(toggle(&s(&["a"]), "c"), s(&["a", "c"]));
    assert_eq!(enable_all(&None, &all, None), None);
    assert_eq!(enable_all(&s(&["a"]), &all, None), None);
    assert_eq!(
        enable_all(&s(&["a"]), &all, Some(&["c".to_string()])),
        s(&["a", "c"])
    );
    assert_eq!(clear_all(&None, &all, None), s(&[]));
    assert_eq!(
        clear_all(&None, &all, Some(&["b".to_string()])),
        s(&["a", "c"])
    );
    assert_eq!(clear_all(&s(&["a", "b"]), &all, None), s(&[]));
    assert_eq!(move_id(&s(&["a", "b"]), "b", -1), s(&["b", "a"]));
    assert_eq!(move_id(&s(&["a", "b"]), "a", -1), s(&["a", "b"]));
    assert_eq!(move_id(&None, "a", 1), None);
}

#[test]
fn picker_saves_effort_and_category_per_entry_not_ids_only() {
    let _g = lock();
    let models = faux();
    let ids = full_ids(&models);
    let saved = vec![
        entry(
            &ids[1],
            Some("high"),
            Some(ModelCategoryName::Capable),
            None,
        ),
        entry(&ids[0], None, None, Some("one")),
    ];
    let mut selector = ScopedModelsSelectorComponent::new(models, Some(saved));
    // Enabled order is the priority: faux-2 leads, faux-1 follows. "high" is
    // the model's last level, so one step unsets it and the next starts over.
    selector.handle_input(EFFORT_INPUT);
    selector.handle_input(EFFORT_INPUT);
    selector.handle_input(CATEGORY_INPUT);
    selector.handle_input(CATEGORY_INPUT);
    // Editing the columns is not a change to the enabled set.
    assert!(selector.take_events().is_empty());
    selector.handle_input("\x1b[B");
    selector.handle_input(EFFORT_INPUT);
    selector.handle_input(CATEGORY_INPUT);
    selector.handle_input("\x1bs");
    assert_eq!(
        selector.take_events(),
        vec![ScopedModelsEvent::Persist(vec![
            entry(&ids[1], Some("off"), Some(ModelCategoryName::Cheap), None),
            entry(
                &ids[0],
                Some("off"),
                Some(ModelCategoryName::Cheap),
                Some("one")
            ),
        ])]
    );
}

#[test]
fn saved_effort_is_clamped_to_what_the_model_supports() {
    let _g = lock();
    let models = faux();
    let ids = full_ids(&models);
    // The faux models have no xhigh mapping, so xhigh clamps to the nearest lower level.
    let saved = vec![entry(&ids[2], Some("xhigh"), None, None)];
    let mut selector = ScopedModelsSelectorComponent::new(models, Some(saved));
    assert!(strip(&selector.render(200)).contains("high"));
    selector.handle_input("\x1bs");
    assert_eq!(
        selector.take_events(),
        vec![ScopedModelsEvent::Persist(vec![entry(
            &ids[2],
            Some("high"),
            None,
            None
        )])]
    );
}

#[test]
fn all_enabled_with_no_effort_or_category_saves_as_the_clear() {
    let _g = lock();
    let mut selector = ScopedModelsSelectorComponent::new(faux(), None);
    selector.handle_input("\x1bs");
    assert_eq!(
        selector.take_events(),
        vec![ScopedModelsEvent::Persist(Vec::new())]
    );
}

#[test]
fn all_enabled_writes_every_model_once_a_column_is_set() {
    let _g = lock();
    let models = faux();
    let ids = full_ids(&models);
    let mut selector = ScopedModelsSelectorComponent::new(models, None);
    selector.handle_input(CATEGORY_INPUT);
    selector.handle_input("\x1bs");
    match selector.take_events().as_slice() {
        [ScopedModelsEvent::Persist(entries)] => {
            let saved: Vec<&str> = entries.iter().map(|e| e.model.as_str()).collect();
            assert_eq!(saved, ids.iter().map(String::as_str).collect::<Vec<_>>());
            assert_eq!(entries[0].category, Some(ModelCategoryName::Cheap));
            assert_eq!(entries[1].category, None);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn effort_and_category_cycles_step_then_unset() {
    let levels = ["off", "low", "high"];
    assert_eq!(next_effort(None, &levels), Some("off".into()));
    assert_eq!(next_effort(Some("off"), &levels), Some("low".into()));
    assert_eq!(next_effort(Some("high"), &levels), None);
    // A value the model does not list starts the cycle again.
    assert_eq!(next_effort(Some("minimal"), &levels), Some("off".into()));
    assert_eq!(next_effort(None, &[]), None);

    let mut category = None;
    let mut seen = Vec::new();
    for _ in 0..5 {
        category = next_category(category);
        seen.push(category.map(|c| c.as_str()));
    }
    assert_eq!(
        seen,
        [
            Some("cheap"),
            Some("fast"),
            Some("standard"),
            Some("capable"),
            None
        ]
    );
}

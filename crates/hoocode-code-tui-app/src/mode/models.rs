//! Models and providers: `/model`, `/scoped-models`, cycling, and the Anthropic subscription notice.

use std::collections::HashSet;

use hoocode_ai_types::{Model, ThinkingLevel, Transport};
use hoocode_code_models::{find_exact_model_reference_match, resolve_scoped_models};
use hoocode_code_settings::{ScopedModel, SettingsManager};
use hoocode_code_tui_selectors::model_selector::{ModelSelectorComponent, ModelSelectorEvent};
use hoocode_code_tui_selectors::scoped_models_selector::{
    ScopedModelsEvent, ScopedModelsSelectorComponent,
};
use hoocode_tui_render::Component;

use super::*;

/// A transport's settings.json name.
pub(super) fn transport_name(transport: Transport) -> String {
    serde_json::to_value(transport)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| "auto".into())
}

/// The saved `scopedModels` as concrete `provider/id` entries in list order,
/// one per model (a glob entry expands to its matches). Each entry's effort,
/// category and alias carry over; the alias goes to the first model it names.
/// A model named twice keeps one row, and the later entry fills in what the
/// first left unset (alias, category, effort), so nothing it carried is lost.
fn concrete_scoped_entries(entries: &[ScopedModel], available: &[Model]) -> Vec<ScopedModel> {
    let mut concrete: Vec<ScopedModel> = Vec::new();
    for entry in entries {
        let mut alias = entry.alias.clone();
        for resolved in resolve_scoped_models(std::slice::from_ref(entry), available) {
            let model = format!("{}/{}", resolved.model.provider, resolved.model.id);
            let effort = resolved.effort.map(|level| level.as_str().to_string());
            let alias_here = alias.take();
            match concrete.iter_mut().find(|c| c.model == model) {
                Some(existing) => {
                    existing.effort = existing.effort.take().or(effort);
                    existing.category = existing.category.or(resolved.category);
                    existing.alias = existing.alias.take().or(alias_here);
                }
                None => concrete.push(ScopedModel {
                    model,
                    effort,
                    category: resolved.category,
                    alias: alias_here,
                }),
            }
        }
    }
    concrete
}

/// The saved entries the picker cannot show because their models are not
/// available now (e.g. logged out). A save keeps them verbatim, so saving never
/// drops a scope entry the picker could not display.
fn unavailable_entries(saved: &[ScopedModel], available: &[Model]) -> Vec<ScopedModel> {
    saved
        .iter()
        .filter(|entry| resolve_scoped_models(std::slice::from_ref(entry), available).is_empty())
        .cloned()
        .collect()
}

/// What a picker save writes: the picked entries in priority order, then the
/// unavailable saved entries in their saved order.
fn entries_to_save(
    picked: Vec<ScopedModel>,
    saved: &[ScopedModel],
    available: &[Model],
) -> Vec<ScopedModel> {
    let mut all = picked;
    all.extend(unavailable_entries(saved, available));
    all
}

/// The notice for Anthropic subscription auth (`ANTHROPIC_SUBSCRIPTION_AUTH_*`).
pub const ANTHROPIC_SUBSCRIPTION_AUTH_TITLE: &str = "Anthropic subscription";

pub const ANTHROPIC_SUBSCRIPTION_AUTH_BODY: &[&str] = &[
    "Billed per token as extra usage, not against plan limits.",
    "Turn off in /settings → Anthropic extra usage.",
];

/// The once-per-session latch of `maybeWarnAboutAnthropicSubscriptionAuth`:
/// true when the notice should show now. `uses_subscription_auth` is asked
/// only while the latch is open, and a `false` leaves it open.
pub fn claim_anthropic_subscription_warning(
    shown: &mut bool,
    uses_subscription_auth: impl FnOnce() -> bool,
) -> bool {
    if *shown || !uses_subscription_auth() {
        return false;
    }
    *shown = true;
    true
}

impl Mode {
    /// `getModelCandidates`: the model scope when set, else every model with
    /// configured auth.
    fn model_candidates(&self) -> Vec<Model> {
        let scoped = self.session.scoped_models();
        if scoped.is_empty() {
            self.session.get_available_models()
        } else {
            scoped.into_iter().map(|s| s.model).collect()
        }
    }

    /// `updateAvailableProviderCount`: the footer names the provider once
    /// there is more than one.
    pub(super) fn update_available_provider_count(&mut self) {
        let providers: HashSet<String> = self
            .model_candidates()
            .into_iter()
            .map(|m| m.provider)
            .collect();
        self.footer_data
            .set_available_provider_count(providers.len());
        self.dirty.set(true);
    }

    /// `maybeWarnAboutAnthropicSubscriptionAuth`: once per session.
    pub(super) fn maybe_warn_about_anthropic_subscription_auth(&mut self, model: Option<Model>) {
        let Some(model) = model.or_else(|| self.session.model()) else {
            return;
        };
        let session = self.session.clone();
        if claim_anthropic_subscription_warning(&mut self.anthropic_warning_shown, || {
            session.uses_anthropic_subscription_auth(&model)
        }) {
            self.show_notice(
                ANTHROPIC_SUBSCRIPTION_AUTH_TITLE,
                ANTHROPIC_SUBSCRIPTION_AUTH_BODY,
            );
        }
    }

    /// `cycleModel`.
    pub(super) fn cycle_model(&mut self, forward: bool) {
        let direction = if forward {
            hoocode_code_agent_session::CycleDirection::Forward
        } else {
            hoocode_code_agent_session::CycleDirection::Backward
        };
        match self.session.cycle_model(direction) {
            None => {
                let message = if self.session.scoped_models().is_empty() {
                    "Only one model available"
                } else {
                    "Only one model in scope"
                };
                self.show_status(message);
            }
            Some(result) => {
                self.footer.borrow_mut().invalidate();
                self.update_editor_border_color();
                let thinking =
                    if result.model.reasoning && result.thinking_level != ThinkingLevel::Off {
                        format!(" (thinking: {})", result.thinking_level.as_str())
                    } else {
                        String::new()
                    };
                let name = if result.model.name.is_empty() {
                    &result.model.id
                } else {
                    &result.model.name
                };
                self.show_dial_step(
                    if forward {
                        "app.model.cycleBackward"
                    } else {
                        "app.model.cycleForward"
                    },
                    &format!("Switched to {name}{thinking}"),
                );
                self.maybe_warn_about_anthropic_subscription_auth(Some(result.model));
            }
        }
    }

    /// `handleModel`: `/model` opens the picker; `/model <ref>` switches on
    /// an exact match, else opens the picker searching for it.
    pub(super) fn handle_model_command(&mut self, search: Option<String>) {
        let Some(search) = search else {
            self.show_model_selector(None);
            return;
        };
        let candidates = self.model_candidates();
        let Some(model) = find_exact_model_reference_match(&search, &candidates).cloned() else {
            self.show_model_selector(Some(&search));
            return;
        };
        self.switch_model(model);
    }

    /// `session.setModel` and what the chrome shows about it.
    fn switch_model(&mut self, model: Model) {
        match self.session.set_model(model.clone()) {
            Ok(()) => {
                self.footer.borrow_mut().invalidate();
                self.update_editor_border_color();
                self.show_status(&format!("Model: {}", model.id));
                self.maybe_warn_about_anthropic_subscription_auth(Some(model));
            }
            Err(error) => self.show_error(&error.to_string()),
        }
    }

    /// `showModelSelector`.
    pub(super) fn show_model_selector(&mut self, initial_search: Option<&str>) {
        let load_error = self.session.model_registry().error().map(String::from);
        let selector = handle(ModelSelectorComponent::new(
            self.session.model(),
            Ok(self.session.get_available_models()),
            load_error,
            self.session
                .scoped_models()
                .into_iter()
                .map(|s| s.model)
                .collect(),
            initial_search,
        ));
        {
            let mut container = self.editor_container.borrow_mut();
            container.clear();
            container.add_child(as_component(&selector));
        }
        self.tui.set_focus(Some(as_component(&selector)));
        self.model_selector = Some(selector);
        self.dirty.set(true);
    }

    /// `showModelsSelector`: the scoped models model cycling steps through,
    /// with their effort and category, loaded from `scopedModels`.
    pub(super) fn show_models_selector(&mut self) {
        let all = self.session.get_available_models();
        if all.is_empty() {
            self.show_status("No models available");
            return;
        }
        let saved = self.session.settings().scoped_models();
        let scoped = saved.map(|entries| concrete_scoped_entries(&entries, &all));
        let selector = handle(ScopedModelsSelectorComponent::new(all, scoped));
        {
            let mut container = self.editor_container.borrow_mut();
            container.clear();
            container.add_child(as_component(&selector));
        }
        self.tui.set_focus(Some(as_component(&selector)));
        self.scoped_models_selector = Some(selector);
        self.dirty.set(true);
    }

    /// The picker's save (`Persist`): writes `scopedModels` (the project's when
    /// the project defines the key, else global) and applies the saved scope
    /// to the session. Refused while `--models` is set: that run's scope is the
    /// flag, and writing it would put the flag list on disk.
    fn persist_scoped_models(&mut self, picked: Vec<ScopedModel>) {
        if hoocode_code_subagents::instance::scoped_models_override_active() {
            self.show_error(
                "--models is set for this run, so the scope cannot be saved. Start without --models to save.",
            );
            return;
        }
        let available = self.session.get_available_models();
        let saved = self.session.settings().scoped_models().unwrap_or_default();
        let entries = entries_to_save(picked, &saved, &available);
        if let Err(error) = SettingsManager::validate_scoped_models(&entries) {
            self.show_error(&format!("Model selection not saved: {error}"));
            return;
        }
        self.session.settings().set_scoped_models(&entries);
        // Resolved entries keep each model's effort and category, so alt+m
        // cycling applies them now.
        self.session
            .set_resolved_scoped_models(resolve_scoped_models(&entries, &available));
        self.update_available_provider_count();
        self.show_status("Model selection saved to settings");
    }

    /// The model pickers' answers.
    pub(super) fn poll_model_selectors(&mut self) {
        let event = self
            .model_selector
            .as_ref()
            .and_then(|s| s.borrow_mut().take_events().into_iter().next());
        if let Some(event) = event {
            self.model_selector = None;
            self.restore_editor();
            if let ModelSelectorEvent::Select(model) = event {
                self.switch_model(*model);
            }
        }
        if let Some(selector) = self.scoped_models_selector.clone() {
            let events = selector.borrow_mut().take_events();
            for event in events {
                match event {
                    // A change is only the picker's own state. The session scope
                    // moves on a save, never on a change, so Cancel leaves it as it was.
                    ScopedModelsEvent::Change(_) => {}
                    ScopedModelsEvent::Persist(entries) => self.persist_scoped_models(entries),
                    ScopedModelsEvent::Cancel => {
                        self.scoped_models_selector = None;
                        self.restore_editor();
                        return;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod scoped_save_tests {
    use super::*;
    use hoocode_code_settings::ModelCategoryName;

    fn model(provider: &str, id: &str) -> Model {
        serde_json::from_value(serde_json::json!({
            "id": id, "name": id, "api": "openai-completions", "provider": provider,
            "baseUrl": "http://x", "contextWindow": 1000, "maxTokens": 100,
        }))
        .unwrap()
    }

    fn entry(model: &str) -> ScopedModel {
        ScopedModel {
            model: model.into(),
            effort: None,
            category: None,
            alias: None,
        }
    }

    fn available() -> Vec<Model> {
        vec![model("acme", "mid"), model("acme", "big")]
    }

    #[test]
    fn a_save_keeps_saved_entries_for_models_that_are_not_available() {
        // `other/gone` is logged out: the picker cannot show it, but saving must
        // not drop it. It goes after the picked entries, in its saved order.
        let saved = vec![entry("other/gone"), entry("acme/mid"), entry("gone/also")];
        let picked = vec![entry("acme/big")];
        let saved_list = entries_to_save(picked, &saved, &available());
        let models: Vec<&str> = saved_list.iter().map(|e| e.model.as_str()).collect();
        assert_eq!(models, vec!["acme/big", "other/gone", "gone/also"]);
    }

    #[test]
    fn a_model_the_user_turned_off_is_not_kept_back() {
        // `acme/mid` is available and was switched off in the picker: it is gone.
        let saved = vec![entry("acme/mid")];
        let saved_list = entries_to_save(vec![entry("acme/big")], &saved, &available());
        assert_eq!(saved_list, vec![entry("acme/big")]);
    }

    #[test]
    fn a_duplicate_model_keeps_its_alias_category_and_effort() {
        // The same model listed twice: the first row keeps what it has, and the
        // second fills in the alias, category and effort the first left unset.
        let list = vec![
            ScopedModel {
                effort: Some("high".into()),
                ..entry("acme/mid")
            },
            ScopedModel {
                model: "acme/mid".into(),
                effort: Some("low".into()),
                category: Some(ModelCategoryName::Fast),
                alias: Some("mid".into()),
            },
        ];
        let concrete = concrete_scoped_entries(&list, &available());
        assert_eq!(concrete.len(), 1);
        assert_eq!(concrete[0].effort.as_deref(), Some("high"));
        assert_eq!(concrete[0].category, Some(ModelCategoryName::Fast));
        assert_eq!(concrete[0].alias.as_deref(), Some("mid"));
    }
}

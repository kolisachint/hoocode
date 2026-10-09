//! Models and providers: `/model`, `/scoped-models`, cycling, and the Anthropic subscription notice.

use std::collections::HashSet;

use hoocode_ai_types::{Model, ThinkingLevel, Transport};
use hoocode_code_models::{find_exact_model_reference_match, resolve_model_scope};
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

    /// `showModelsSelector`: the enable set model cycling steps through.
    pub(super) fn show_models_selector(&mut self) {
        let all = self.session.get_available_models();
        if all.is_empty() {
            self.show_status("No models available");
            return;
        }
        let full_id = |m: &Model| format!("{}/{}", m.provider, m.id);
        let scoped = self.session.scoped_models();
        let enabled = if !scoped.is_empty() {
            Some(scoped.iter().map(|s| full_id(&s.model)).collect())
        } else {
            let patterns = self.session.settings().enabled_models();
            patterns.filter(|p| !p.is_empty()).map(|patterns| {
                resolve_model_scope(&patterns, &all)
                    .models
                    .iter()
                    .map(|s| full_id(&s.model))
                    .collect()
            })
        };
        let total = all.len();
        let selector = handle(ScopedModelsSelectorComponent::new(all, enabled));
        {
            let mut container = self.editor_container.borrow_mut();
            container.clear();
            container.add_child(as_component(&selector));
        }
        self.tui.set_focus(Some(as_component(&selector)));
        self.scoped_models_selector = Some((selector, total));
        self.dirty.set(true);
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
        if let Some((selector, total)) = &self.scoped_models_selector {
            let total = *total;
            let events = selector.borrow_mut().take_events();
            for event in events {
                match event {
                    ScopedModelsEvent::Change(enabled) => {
                        self.set_session_model_scope(enabled, total)
                    }
                    ScopedModelsEvent::Persist(enabled) => {
                        // Every model enabled clears the filter.
                        let patterns = enabled.filter(|ids| ids.len() != total);
                        self.session
                            .settings()
                            .set_enabled_models(patterns.as_deref());
                        self.show_status("Model selection saved to settings");
                    }
                    ScopedModelsEvent::Cancel => {
                        self.scoped_models_selector = None;
                        self.restore_editor();
                        return;
                    }
                }
            }
        }
    }

    /// The session's model scope from the picker (session-only): all or
    /// none enabled means no filter.
    fn set_session_model_scope(&mut self, enabled: Option<Vec<String>>, total: usize) {
        match enabled.filter(|ids| !ids.is_empty() && ids.len() < total) {
            Some(ids) => {
                let available = self.session.get_available_models();
                let scope = resolve_model_scope(&ids, &available);
                self.session.set_scoped_models(scope.models);
            }
            None => self.session.set_scoped_models(Vec::new()),
        }
        self.update_available_provider_count();
    }
}

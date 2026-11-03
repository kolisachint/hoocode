//! Default-model fallback: when hoocode picked the model itself (no
//! `--model`, no saved default, no resumed session model) and the provider
//! refuses it — no access on this account, no funds, deprecated, gone, or a
//! free tier that is used up for hours — the session switches to the next
//! provider default it has a key for and continues the turn. The first model
//! that answers is saved as the default, so the next start uses it directly.

use hoocode_agent_types::AgentMessage;
use hoocode_ai_types::{AssistantMessage, Model, StopReason};

use crate::session::{AgentSession, AgentSessionEvent};

/// The fallback list and the model to save once it answers.
#[derive(Default)]
pub struct FallbackState {
    /// Models to try next, in order. Empty unless the model was auto-picked.
    pub(crate) candidates: Vec<Model>,
    /// A model switched to by a fallback, saved as the default on its first answer.
    pub(crate) to_save: Option<Model>,
}

/// Messages that mean "this model is not usable here", not "this request failed".
const UNUSABLE_400_HINTS: [&str; 6] = [
    "model is unavailable",
    "model not found",
    "model_not_found",
    "does not exist",
    "deprecated",
    "not supported for model",
];

/// A 403 about this request (moderation, or a rate limit some providers send
/// as 403), not about the model.
const REQUEST_403_HINTS: [&str; 6] = [
    "flagged",
    "moderation",
    "content policy",
    "safety",
    "rate limit",
    "rate_limit",
];

/// Whether an errored response says the provider will not serve this model
/// to this account: HTTP 402, 403, 404 or 410, a 400 naming the model as
/// unavailable, or a 429 whose wait is hours (a used-up free tier). A bad
/// key (401), a transient failure or a context overflow is not one.
pub fn is_model_unusable_error(message: &AssistantMessage) -> bool {
    if message.stop_reason != StopReason::Error {
        return false;
    }
    let Some(error) = message.error_message.as_deref().filter(|e| !e.is_empty()) else {
        return false;
    };
    let status: String = error
        .trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    let lower = error.to_ascii_lowercase();
    match status.as_str() {
        "403" => !REQUEST_403_HINTS.iter().any(|h| lower.contains(h)),
        "402" | "404" | "410" => true,
        "400" => UNUSABLE_400_HINTS.iter().any(|h| lower.contains(h)),
        "429" => {
            hoocode_ai_util::is_long_retry_delay_error(Some(error))
                || lower.contains("free usage limit")
        }
        _ => false,
    }
}

fn label(model: &Model) -> String {
    format!("{}/{}", model.provider, model.id)
}

impl AgentSession {
    /// Arms the fallback for an auto-picked model (`create_agent_session`).
    pub fn set_model_fallbacks(&self, candidates: Vec<Model>) {
        self.model_fallback_state().candidates = candidates;
    }

    /// Whether a fallback is armed (tests and the UI).
    pub fn has_model_fallbacks(&self) -> bool {
        !self.model_fallback_state().candidates.is_empty()
    }

    /// The user chose a model: hoocode never switches away from it.
    pub(crate) fn clear_model_fallbacks(&self) {
        let mut state = self.model_fallback_state();
        state.candidates.clear();
        state.to_save = None;
    }

    /// Whether `message` will switch models (armed in `agent_end`, so the
    /// prompt waits for the continued turn).
    pub(crate) fn model_fallback_ready(&self, message: &AssistantMessage) -> bool {
        is_model_unusable_error(message) && self.has_model_fallbacks()
    }

    /// The current model answered: stop falling back, and save a model a
    /// fallback switched to as the default for the next start.
    pub(crate) fn on_model_answered(&self) {
        let to_save = {
            let mut state = self.model_fallback_state();
            state.candidates.clear();
            state.to_save.take()
        };
        if let Some(model) = to_save {
            self.settings()
                .set_default_model_and_provider(&model.provider, &model.id);
        }
    }

    /// Switches to the next candidate after an "unusable model" error and
    /// continues the turn. `false` when it does not apply.
    pub(crate) fn try_model_fallback(&self, message: &AssistantMessage) -> bool {
        if !is_model_unusable_error(message) {
            return false;
        }
        let Some(from) = self.model() else {
            return false;
        };
        let next = {
            let mut state = self.model_fallback_state();
            let mut next = None;
            while !state.candidates.is_empty() {
                let candidate = state.candidates.remove(0);
                let same = candidate.provider == from.provider && candidate.id == from.id;
                if !same && self.has_configured_auth(&candidate) {
                    next = Some(candidate);
                    break;
                }
            }
            if let Some(model) = &next {
                state.to_save = Some(model.clone());
            }
            next
        };
        let Some(next) = next else {
            return false;
        };
        // Keep `prompt()` waiting for the continued turn, and end any retry
        // streak the refused model was in.
        self.retry_state().pending = true;
        self.on_unretried_error(message);

        // Drop the error from the context (the session keeps it for history).
        let mut messages = self.messages();
        if matches!(messages.last(), Some(AgentMessage::Assistant(_))) {
            messages.pop();
            self.agent().set_messages(messages);
        }
        let thinking_level = self.thinking_level_for_model_switch(None);
        self.apply_model(&next);
        self.set_thinking_level(thinking_level);
        self.emit(AgentSessionEvent::ModelFallback {
            from: label(&from),
            to: label(&next),
            error_message: message.error_message.clone().unwrap_or_default(),
        });
        self.continue_agent(std::time::Duration::ZERO);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn errored(text: &str) -> AssistantMessage {
        AssistantMessage {
            stop_reason: StopReason::Error,
            error_message: Some(text.into()),
            ..Default::default()
        }
    }

    #[test]
    fn classifies_errors_that_rule_out_the_model() {
        for e in [
            "403 Upstream request failed: Model access is disabled",
            r#"402 {"type":"error","error":{"message":"Insufficient account funds"}}"#,
            "410 Model glm-5.1 has been deprecated. Use glm-5.3 instead.",
            "404 model not found",
            "400 Upstream request failed: Model is unavailable.",
            "429 Free usage limit reached. Free models work in OpenCode",
        ] {
            assert!(is_model_unusable_error(&errored(e)), "{e}");
        }
        for e in [
            "401 invalid api key",
            "400 messages: field required",
            "429 Too Many Requests",
            "500 internal error",
            "403 Your input was flagged by moderation",
            "403 You have exceeded a secondary rate limit",
            "connection refused",
        ] {
            assert!(!is_model_unusable_error(&errored(e)), "{e}");
        }
        let ok = AssistantMessage::default();
        assert!(!is_model_unusable_error(&ok));
    }
}
